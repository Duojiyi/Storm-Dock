use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client as ReqwestClient;
use reqwest::header::RETRY_AFTER;
use reqwest::redirect::Policy;
use reqwest::Method;
use thiserror::Error;

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
pub(crate) const USAGE_BUDGET: Duration = Duration::from_secs(15);
pub(crate) const SUBSCRIPTION_BUDGET: Duration = Duration::from_secs(12);
pub(crate) const TOOLS_BUDGET: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Retry {
    Transient,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportHint {
    Timeout,
    Connect,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryReason {
    Timeout,
    Unreachable,
    RateLimited,
    Status(u16),
    Deadline,
    Decode,
    Server,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum HttpError {
    #[error("连接超时，请检查网络后重试。")]
    Timeout,
    #[error("无法连接服务器，请检查网络或代理。")]
    Unreachable,
    #[error("请求过于频繁，请稍后重试。")]
    RateLimited,
    #[error("服务暂时不可用。")]
    Server,
    #[error("无法读取服务器响应。")]
    Decode,
    #[error("查询时间过长，请稍后重试。")]
    Deadline,
}

#[derive(Debug)]
pub(crate) struct HttpResponse {
    pub(crate) status: u16,
    pub(crate) bytes: Vec<u8>,
}

impl HttpResponse {
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    pub(crate) fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, HttpError> {
        serde_json::from_slice(&self.bytes).map_err(|_| HttpError::Decode)
    }
}

#[derive(Debug)]
pub(crate) struct Budget {
    deadline: Instant,
}

impl Budget {
    pub(crate) fn new(limit: Duration) -> Self {
        Self {
            deadline: Instant::now() + limit,
        }
    }

    pub(crate) fn remaining(&self) -> Option<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
    }
}

#[derive(Debug)]
pub(crate) enum Body {
    Empty,
    Json(serde_json::Value),
    Form(Vec<(String, String)>),
    Bytes(Vec<u8>),
}

impl Body {
    pub(crate) fn form(fields: &[(&str, &str)]) -> Self {
        Self::Form(
            fields
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        )
    }
}

#[derive(Debug)]
pub(crate) struct Call {
    pub(crate) method: Method,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) query: Vec<(String, String)>,
    pub(crate) body: Body,
}

pub(crate) struct Client {
    inner: ReqwestClient,
}

impl Client {
    pub(crate) fn shared() -> &'static Self {
        static CLIENT: OnceLock<Client> = OnceLock::new();
        CLIENT.get_or_init(|| Client {
            inner: build_client(Policy::default()),
        })
    }

    pub(crate) fn cursor_web() -> &'static Self {
        static CLIENT: OnceLock<Client> = OnceLock::new();
        CLIENT.get_or_init(|| Client {
            inner: build_client(Policy::custom(|attempt| {
                if attempt.url().host_str() == Some("cursor.com") && attempt.previous().len() < 5 {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            })),
        })
    }

    pub(crate) fn send(
        &self,
        call: &Call,
        budget: &Budget,
        retry: Retry,
    ) -> Result<HttpResponse, HttpError> {
        let mut last_transport = HttpError::Unreachable;
        for attempt in 0..=1 {
            let Some(remaining) = budget.remaining() else {
                return Err(HttpError::Deadline);
            };
            let timeout = remaining.min(REQUEST_TIMEOUT);
            match self.attempt(call, timeout) {
                Ok(response) if retryable_status(response.status) => {
                    let reason = RetryReason::Status(response.status);
                    let wait = status_wait(response.status, response.retry_after.as_deref());
                    if should_retry(reason, retry, attempt) && sleep_if_budget(budget, wait) {
                        continue;
                    }
                    return Ok(response.into_http());
                }
                Ok(response) => return Ok(response.into_http()),
                Err(error) => {
                    last_transport = error;
                    let reason = retry_reason(&last_transport);
                    let wait = jitter_wait();
                    if should_retry(reason, retry, attempt) && sleep_if_budget(budget, wait) {
                        continue;
                    }
                    return Err(last_transport);
                }
            }
        }
        Err(last_transport)
    }

    fn attempt(&self, call: &Call, timeout: Duration) -> Result<RawResponse, HttpError> {
        let mut request = match call.method {
            Method::POST => self.inner.post(&call.url),
            _ => self.inner.get(&call.url),
        }
        .timeout(timeout);
        if !call.query.is_empty() {
            request = request.query(&call.query);
        }
        for (name, value) in &call.headers {
            request = request.header(name.as_str(), value.as_str());
        }
        request = match &call.body {
            Body::Empty => request,
            Body::Json(body) => request.json(body),
            Body::Form(fields) => request.form(fields),
            Body::Bytes(bytes) => request.body(bytes.clone()),
        };
        let response = request
            .send()
            .map_err(|error| classify(hint_from_reqwest(&error)))?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = response
            .bytes()
            .map_err(|error| classify(hint_from_reqwest(&error)))?
            .to_vec();
        Ok(RawResponse {
            status,
            bytes,
            retry_after,
        })
    }
}

struct RawResponse {
    status: u16,
    bytes: Vec<u8>,
    retry_after: Option<String>,
}

impl RawResponse {
    fn into_http(self) -> HttpResponse {
        HttpResponse {
            status: self.status,
            bytes: self.bytes,
        }
    }
}

fn build_client(redirect: Policy) -> ReqwestClient {
    let mut builder = ReqwestClient::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(redirect);
    if let Some(proxy_url) = env_proxy_url() {
        if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
            builder = builder.proxy(proxy);
        }
    }
    builder
        .build()
        .unwrap_or_else(|_| ReqwestClient::new())
}

fn env_proxy_url() -> Option<String> {
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    macos_system_proxy_url()
}

#[cfg(target_os = "macos")]
fn macos_system_proxy_url() -> Option<String> {
    let output = std::process::Command::new("scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut https_enable = false;
    let mut http_enable = false;
    let mut https_host = None::<String>;
    let mut https_port = None::<String>;
    let mut http_host = None::<String>;
    let mut http_port = None::<String>;
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("HTTPSEnable : ") {
            https_enable = value.trim() == "1";
        } else if let Some(value) = line.strip_prefix("HTTPEnable : ") {
            http_enable = value.trim() == "1";
        } else if let Some(value) = line.strip_prefix("HTTPSProxy : ") {
            https_host = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("HTTPSPort : ") {
            https_port = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("HTTPProxy : ") {
            http_host = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("HTTPPort : ") {
            http_port = Some(value.trim().to_owned());
        }
    }
    if https_enable {
        if let (Some(host), Some(port)) = (https_host, https_port) {
            return Some(format!("http://{host}:{port}"));
        }
    }
    if http_enable {
        if let (Some(host), Some(port)) = (http_host, http_port) {
            return Some(format!("http://{host}:{port}"));
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn macos_system_proxy_url() -> Option<String> {
    None
}

pub(crate) fn hint_from_reqwest(error: &reqwest::Error) -> TransportHint {
    if error.is_timeout() {
        TransportHint::Timeout
    } else if error.is_connect() {
        TransportHint::Connect
    } else {
        TransportHint::Other
    }
}

pub(crate) fn classify(hint: TransportHint) -> HttpError {
    match hint {
        TransportHint::Timeout => HttpError::Timeout,
        TransportHint::Connect | TransportHint::Other => HttpError::Unreachable,
    }
}

pub(crate) fn should_retry(reason: RetryReason, retry: Retry, attempt: u8) -> bool {
    retry == Retry::Transient
        && attempt == 0
        && matches!(
            reason,
            RetryReason::Timeout
                | RetryReason::Unreachable
                | RetryReason::RateLimited
                | RetryReason::Status(429 | 502 | 503 | 504)
        )
}

pub(crate) fn retry_after_wait(header: Option<&str>) -> Duration {
    header
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(1))
}

fn retry_reason(error: &HttpError) -> RetryReason {
    match error {
        HttpError::Timeout => RetryReason::Timeout,
        HttpError::Unreachable => RetryReason::Unreachable,
        HttpError::RateLimited => RetryReason::RateLimited,
        HttpError::Server => RetryReason::Server,
        HttpError::Decode => RetryReason::Decode,
        HttpError::Deadline => RetryReason::Deadline,
    }
}

fn retryable_status(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}

fn status_wait(status: u16, retry_after: Option<&str>) -> Duration {
    if status == 429 {
        retry_after_wait(retry_after)
    } else {
        jitter_wait()
    }
}

fn jitter_wait() -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(0);
    Duration::from_millis(200 + u64::from(nanos % 201))
}

fn sleep_if_budget(budget: &Budget, wait: Duration) -> bool {
    match budget.remaining() {
        Some(left) if left >= wait => {
            thread::sleep(wait);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_maps_timeout_and_unreachable() {
        assert_eq!(classify(TransportHint::Timeout), HttpError::Timeout);
        assert_eq!(classify(TransportHint::Connect), HttpError::Unreachable);
        assert_eq!(classify(TransportHint::Other), HttpError::Unreachable);
    }

    #[test]
    fn transient_retries_once_for_retryable_failures() {
        for reason in [
            RetryReason::Timeout,
            RetryReason::Unreachable,
            RetryReason::RateLimited,
            RetryReason::Status(429),
            RetryReason::Status(502),
            RetryReason::Status(503),
            RetryReason::Status(504),
        ] {
            assert!(should_retry(reason, Retry::Transient, 0));
            assert!(!should_retry(reason, Retry::Transient, 1));
            assert!(!should_retry(reason, Retry::None, 0));
        }
        assert!(!should_retry(RetryReason::Deadline, Retry::Transient, 0));
        assert!(!should_retry(RetryReason::Decode, Retry::Transient, 0));
        assert!(!should_retry(RetryReason::Server, Retry::Transient, 0));
        assert!(!should_retry(RetryReason::Status(500), Retry::Transient, 0));
    }

    #[test]
    fn retry_after_parses_seconds_or_defaults() {
        assert_eq!(retry_after_wait(Some("2")), Duration::from_secs(2));
        assert_eq!(retry_after_wait(Some(" 3 ")), Duration::from_secs(3));
        assert_eq!(retry_after_wait(Some("nope")), Duration::from_secs(1));
        assert_eq!(retry_after_wait(None), Duration::from_secs(1));
    }

    #[test]
    fn exhausted_budget_has_no_remaining_time() {
        assert!(Budget::new(Duration::ZERO).remaining().is_none());
        assert!(Budget::new(Duration::from_secs(5))
            .remaining()
            .is_some_and(|left| left <= Duration::from_secs(5)));
    }

    #[test]
    fn body_variants_cover_json_form_and_bytes() {
        assert!(matches!(Body::Empty, Body::Empty));
        assert!(matches!(Body::Json(serde_json::json!({})), Body::Json(_)));
        assert!(matches!(
            Body::Form(vec![("a".into(), "b".into())]),
            Body::Form(_)
        ));
        assert!(matches!(Body::Bytes(vec![1, 2]), Body::Bytes(_)));
    }
}

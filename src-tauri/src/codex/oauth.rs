use std::time::Duration;

use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::codex::session::{oauth_auth_json, session_from_auth};
use crate::cursor::oauth::{
    emit_official_login_status, OauthLoginState, OfficialLoginStatus,
};
use crate::cursor::session::jwt_claims;
use crate::error::{AppError, Result};
use crate::http::{Body, Budget, Call, Client, Retry, USAGE_BUDGET};
use crate::models::{
    json_text, now, Account, ApplicationKind, ImportType, Session, SubscriptionSummary, UsageMetric,
};
use crate::store::AppState;
use crate::tray::refresh_tray;

const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEVICE_AUTH_USERCODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const DEVICE_AUTH_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const DEVICE_VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const USER_AGENT: &str = "storm-dock-codex-oauth";
const POLL_ATTEMPTS: u32 = 180;

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_auth_id: String,
    user_code: String,
    #[serde(default)]
    interval: Option<serde_json::Value>,
    #[serde(default)]
    #[allow(dead_code)]
    expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct DevicePollSuccess {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Clone, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

pub(crate) fn complete_codex_oauth(
    label: Option<String>,
    login_id: u64,
    app: AppHandle,
) -> Result<Account> {
    let oauth = app.state::<OauthLoginState>();
    let device = start_device_flow()?;
    oauth.set_url(login_id, DEVICE_VERIFICATION_URL.into())?;
    oauth.set_user_code(login_id, device.user_code.clone())?;
    emit_codex_login_status(
        &app,
        "started",
        Some(DEVICE_VERIFICATION_URL.into()),
        Some(device.user_code.clone()),
    );
    let _ = crate::browser::open(DEVICE_VERIFICATION_URL, &oauth.browser());
    emit_codex_login_status(
        &app,
        "waiting",
        Some(DEVICE_VERIFICATION_URL.into()),
        Some(device.user_code.clone()),
    );
    let tokens = poll_device_flow(&device, login_id, &app)?;
    if !oauth.is_active(login_id) {
        return Err(AppError::LoginCancelled);
    }
    emit_official_login_status(&app, "importing", None);
    let session = session_from_tokens(&tokens)?;
    let state = app.state::<AppState>();
    let mut controller = state
        .0
        .lock()
        .map_err(|_| AppError::Message("账户存储不可用".into()))?;
    let account = controller.save_imported_session(
        ApplicationKind::Codex,
        label,
        session,
        ImportType::OAuth,
    )?;
    drop(controller);
    refresh_tray(&app);
    let _ = app.emit("accounts-changed", ());
    crate::commands::spawn_imported_refresh(app.clone(), account.clone());
    oauth.finish(login_id);
    Ok(account)
}

pub(crate) fn refresh_session(
    session: &Session,
) -> Result<(Session, Option<(SubscriptionSummary, UsageMetric)>)> {
    let auth = crate::codex::session::auth_value(session)?;
    let Some(refresh_token) = auth
        .pointer("/tokens/refresh_token")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err(AppError::SecretMissing);
    };
    let tokens = refresh_tokens(refresh_token)?;
    let refreshed = session_from_tokens(&tokens)?;
    let quota = tokens.id_token.as_deref().and_then(|_| {
        fetch_quota(
            &tokens.access_token,
            crate::codex::session::chatgpt_account_id(
                &crate::codex::session::auth_value(&refreshed).ok()?,
            )
            .as_deref(),
        )
        .ok()
    });
    Ok((refreshed, quota))
}

fn start_device_flow() -> Result<DeviceCodeResponse> {
    let response = Client::shared().send(
        &Call {
            method: reqwest::Method::POST,
            url: DEVICE_AUTH_USERCODE_URL.into(),
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("User-Agent".into(), USER_AGENT.into()),
            ],
            query: Vec::new(),
            body: Body::Json(serde_json::json!({ "client_id": CODEX_CLIENT_ID })),
        },
        &Budget::new(USAGE_BUDGET),
        Retry::Transient,
    )?;
    if !(200..300).contains(&response.status) {
        return Err(AppError::Message(format!(
            "ChatGPT 登录请求失败: {} - {}",
            response.status,
            response.text()
        )));
    }
    response.json().map_err(Into::into)
}

fn poll_device_flow(
    device: &DeviceCodeResponse,
    login_id: u64,
    app: &AppHandle,
) -> Result<OAuthTokenResponse> {
    let interval = parse_interval(device.interval.as_ref());
    for _ in 0..POLL_ATTEMPTS {
        if !app.state::<OauthLoginState>().is_active(login_id) {
            return Err(AppError::LoginCancelled);
        }
        match poll_once(device) {
            Ok(tokens) => return Ok(tokens),
            Err(_) => crate::cursor::oauth::wait_if_active(app, login_id, interval)?,
        }
    }
    Err(AppError::LoginTimeout)
}

fn poll_once(device: &DeviceCodeResponse) -> Result<OAuthTokenResponse> {
    let response = Client::shared().send(
        &Call {
            method: reqwest::Method::POST,
            url: DEVICE_AUTH_TOKEN_URL.into(),
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("User-Agent".into(), USER_AGENT.into()),
            ],
            query: Vec::new(),
            body: Body::Json(serde_json::json!({
                "device_auth_id": device.device_auth_id,
                "user_code": device.user_code,
            })),
        },
        &Budget::new(USAGE_BUDGET),
        Retry::None,
    )?;
    if matches!(response.status, 403 | 404) {
        return Err(AppError::Message("等待用户授权".into()));
    }
    if response.status == 410 {
        return Err(AppError::LoginTimeout);
    }
    if !(200..300).contains(&response.status) {
        return Err(AppError::Message(format!(
            "ChatGPT 登录轮询失败: {} - {}",
            response.status,
            response.text()
        )));
    }
    let success: DevicePollSuccess = response.json()?;
    exchange_code(&success.authorization_code, &success.code_verifier)
}

fn exchange_code(code: &str, code_verifier: &str) -> Result<OAuthTokenResponse> {
    let response = Client::shared().send(
        &Call {
            method: reqwest::Method::POST,
            url: OAUTH_TOKEN_URL.into(),
            headers: vec![("User-Agent".into(), USER_AGENT.into())],
            query: Vec::new(),
            body: Body::form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", DEVICE_REDIRECT_URI),
                ("client_id", CODEX_CLIENT_ID),
                ("code_verifier", code_verifier),
            ]),
        },
        &Budget::new(USAGE_BUDGET),
        Retry::Transient,
    )?;
    if !(200..300).contains(&response.status) {
        return Err(AppError::Message(format!(
            "ChatGPT Token 交换失败: {} - {}",
            response.status,
            response.text()
        )));
    }
    response.json().map_err(Into::into)
}

fn refresh_tokens(refresh_token: &str) -> Result<OAuthTokenResponse> {
    let response = Client::shared().send(
        &Call {
            method: reqwest::Method::POST,
            url: OAUTH_TOKEN_URL.into(),
            headers: vec![("User-Agent".into(), USER_AGENT.into())],
            query: Vec::new(),
            body: Body::form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", CODEX_CLIENT_ID),
                ("scope", "openid profile email"),
            ]),
        },
        &Budget::new(USAGE_BUDGET),
        Retry::Transient,
    )?;
    if !(200..300).contains(&response.status) {
        return Err(AppError::Message(format!(
            "ChatGPT Token 刷新失败: {} - {}",
            response.status,
            response.text()
        )));
    }
    response.json().map_err(Into::into)
}

fn session_from_tokens(tokens: &OAuthTokenResponse) -> Result<Session> {
    let refresh_token = tokens
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Message("登录响应缺少 refresh_token".into()))?;
    let id_token = tokens
        .id_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Message("登录响应缺少 id_token".into()))?;
    let (account_id, _) = account_metadata(tokens);
    let account_id =
        account_id.ok_or_else(|| AppError::Message("无法从登录响应提取 ChatGPT 账号".into()))?;
    session_from_auth(
        oauth_auth_json(
            &account_id,
            &tokens.access_token,
            Some(id_token),
            refresh_token,
            &last_refresh_now(),
        ),
        None,
    )
}

fn account_metadata(tokens: &OAuthTokenResponse) -> (Option<String>, Option<String>) {
    let mut account_id = None;
    let mut email = None;
    for token in [
        tokens.id_token.as_deref(),
        Some(tokens.access_token.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        let Some(claims) = jwt_claims(token) else {
            continue;
        };
        if account_id.is_none() {
            account_id = json_text(&claims, &["chatgpt_account_id"]).or_else(|| {
                claims
                    .get("https://api.openai.com/auth")
                    .and_then(|value| json_text(value, &["chatgpt_account_id"]))
            });
        }
        if email.is_none() {
            email = json_text(&claims, &["email"]).filter(|value| value.contains('@'));
        }
    }
    (account_id, email)
}

fn fetch_quota(
    access_token: &str,
    account_id: Option<&str>,
) -> Result<(SubscriptionSummary, UsageMetric)> {
    let mut headers = vec![
        ("Authorization".into(), format!("Bearer {access_token}")),
        ("User-Agent".into(), "codex-cli".into()),
        ("Accept".into(), "application/json".into()),
    ];
    if let Some(account_id) = account_id {
        headers.push(("ChatGPT-Account-Id".into(), account_id.to_owned()));
    }
    let response = Client::shared().send(
        &Call {
            method: reqwest::Method::GET,
            url: "https://chatgpt.com/backend-api/wham/usage".into(),
            headers,
            query: Vec::new(),
            body: Body::Empty,
        },
        &Budget::new(USAGE_BUDGET),
        Retry::Transient,
    )?;
    if !(200..300).contains(&response.status) {
        return Err(AppError::Message("ChatGPT 用量查询失败".into()));
    }
    let body: serde_json::Value = response.json()?;
    let used = body
        .pointer("/rate_limit/primary_window/used_percent")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let reset_at = body
        .pointer("/rate_limit/primary_window/reset_at")
        .and_then(serde_json::Value::as_i64)
        .map(|value| value as u64);
    Ok((
        SubscriptionSummary {
            plan: Some("ChatGPT".into()),
            expires_at: reset_at,
            checked_at: Some(now()),
            ..Default::default()
        },
        UsageMetric {
            kind: "percent".into(),
            used,
            limit: Some(100.0),
            percent: used,
        },
    ))
}

fn last_refresh_now() -> String {
    time::OffsetDateTime::from_unix_timestamp(now() as i64)
        .ok()
        .and_then(|time| {
            time.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| now().to_string())
}

fn parse_interval(value: Option<&serde_json::Value>) -> Duration {
    let seconds = value
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            value
                .and_then(serde_json::Value::as_f64)
                .map(|value| value as u64)
        })
        .unwrap_or(5)
        .clamp(2, 15);
    Duration::from_secs(seconds + 3)
}

fn emit_codex_login_status(
    app: &AppHandle,
    stage: &'static str,
    login_url: Option<String>,
    user_code: Option<String>,
) {
    let _ = app.emit(
        "official-login-status",
        OfficialLoginStatus {
            stage,
            login_url,
            user_code,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_urls_match_official_codex_cli() {
        assert_eq!(CODEX_CLIENT_ID, "app_EMoamEEZ73f0CkXaXp7hrann");
        assert_eq!(
            DEVICE_VERIFICATION_URL,
            "https://auth.openai.com/codex/device"
        );
        assert!(parse_interval(Some(&serde_json::json!(5))) >= Duration::from_secs(8));
    }
}

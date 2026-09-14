use std::time::Duration;

use serde_json::Value;

use crate::error::{AppError, Result};
use crate::grok::session::{access_token, auth_value};
use crate::models::{now, Session, SubscriptionSummary};

const USER_AGENT: &str = "storm-dock-grok-subscription";
/// Official Grok Build CLI paywall path (relative `/user?include=subscription` on chat-proxy).
const USER_SUBSCRIPTION_URL: &str =
    "https://cli-chat-proxy.grok.com/v1/user?include=subscription";

/// Fetch Grok Build subscription tier and map it into Storm-Dock's plan field.
pub(crate) fn fetch_grok_subscription(session: &Session) -> Result<SubscriptionSummary> {
    let auth = auth_value(session)?;
    let token = access_token(&auth).ok_or(AppError::SecretMissing)?;
    let client = http_client()?;

    let response = client
        .get(USER_SUBSCRIPTION_URL)
        .bearer_auth(&token)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .send()
        .map_err(|error| AppError::Message(format!("Grok 订阅请求失败: {error}")))?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(AppError::Message(format!(
            "Grok 订阅接口失败 ({status}): {}",
            body.chars().take(160).collect::<String>()
        )));
    }
    let value: Value = serde_json::from_str(&body)
        .map_err(|error| AppError::Message(format!("Grok 订阅响应无效: {error}")))?;
    subscription_from_grok_payload(&value)
        .ok_or_else(|| AppError::Message("Grok 订阅响应缺少 tier/plan。".into()))
}

pub(crate) fn subscription_from_grok_payload(value: &Value) -> Option<SubscriptionSummary> {
    let tier = first_tier(value).unwrap_or_else(|| "free".into());
    let normalized = normalize_grok_tier(&tier);
    Some(SubscriptionSummary {
        plan: Some(normalized),
        expires_at: None,
        billing_cycle_end: text_at(
            value,
            &["billingPeriodEnd", "billing_cycle_end", "billingCycleEnd"],
        ),
        checked_at: Some(now()),
    })
}

fn first_tier(value: &Value) -> Option<String> {
    const KEYS: &[&str] = &[
        "subscriptionTier",
        "subscription_tier",
        "plan",
        "membershipType",
        "membership_type",
    ];
    for key in KEYS {
        match value.get(*key) {
            Some(Value::Null) => continue,
            Some(Value::String(text)) => {
                let text = text.trim();
                if !text.is_empty() {
                    return Some(text.to_owned());
                }
            }
            Some(Value::Number(number)) => return Some(number.to_string()),
            _ => {}
        }
    }
    // Do not use bare "tier" — management payloads use numeric API tier ids.
    if let Some(subscription) = value.get("subscription") {
        if let Some(found) = first_tier(subscription) {
            return Some(found);
        }
    }
    if let Some(user) = value.get("user") {
        if let Some(found) = first_tier(user) {
            return Some(found);
        }
    }
    if let Some(data) = value.get("data") {
        if let Some(found) = first_tier(data) {
            return Some(found);
        }
    }
    None
}

fn text_at(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Normalize Grok Build tier ids into Storm-Dock plan keys.
pub(crate) fn normalize_grok_tier(tier: &str) -> String {
    let lower = tier.trim().to_ascii_lowercase().replace('-', "_");
    match lower.as_str() {
        "free" | "free_tier" | "basic" | "x_basic" | "none" | "null" => "free".into(),
        "supergrok" | "super_grok" | "pro" => "supergrok".into(),
        "supergrok_plus" | "super_grok_plus" => "supergrok_plus".into(),
        "supergrok_heavy" | "super_grok_heavy" => "supergrok_heavy".into(),
        "supergrok_lite" | "super_grok_lite" => "supergrok_lite".into(),
        "x_premium" | "xpremium" => "x_premium".into(),
        "x_premium_plus" | "xpremium_plus" => "x_premium_plus".into(),
        other => other.to_owned(),
    }
}

pub(crate) fn http_client() -> Result<reqwest::blocking::Client> {
    let mut builder = reqwest::blocking::Client::builder().timeout(Duration::from_secs(12));
    if let Some(proxy_url) = env_proxy_url() {
        if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
            builder = builder.proxy(proxy);
        }
    }
    builder
        .build()
        .map_err(|error| AppError::Message(error.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_camel_case_subscription_tier() {
        let summary = subscription_from_grok_payload(&serde_json::json!({
            "subscriptionTier": "supergrok_plus",
            "hasGrokCodeAccess": true
        }))
        .expect("summary");
        assert_eq!(summary.plan.as_deref(), Some("supergrok_plus"));
    }

    #[test]
    fn null_tier_maps_to_free() {
        let summary = subscription_from_grok_payload(&serde_json::json!({
            "subscriptionTier": null,
            "hasGrokCodeAccess": true
        }))
        .expect("summary");
        assert_eq!(summary.plan.as_deref(), Some("free"));
    }

    #[test]
    fn reads_nested_subscription_tier() {
        let summary = subscription_from_grok_payload(&serde_json::json!({
            "user": { "subscription": { "subscription_tier": "supergrok_plus" } }
        }))
        .expect("summary");
        assert_eq!(summary.plan.as_deref(), Some("supergrok_plus"));
    }

    #[test]
    fn normalizes_free_aliases() {
        assert_eq!(normalize_grok_tier("Free"), "free");
        assert_eq!(normalize_grok_tier("x-basic"), "free");
        assert_eq!(normalize_grok_tier("SuperGrok"), "supergrok");
        assert_eq!(normalize_grok_tier("None"), "free");
    }
}

use serde_json::Value;

use crate::error::{AppError, Result};
use crate::grok::session::{access_token, auth_value};
use crate::http::{Body, Budget, Call, Client, HttpError, Retry, USAGE_BUDGET};
use crate::models::{now, Session, SubscriptionSummary};

const USER_AGENT: &str = "storm-dock-grok-subscription";
/// Official Grok Build CLI paywall path (relative `/user?include=subscription` on chat-proxy).
const USER_SUBSCRIPTION_URL: &str =
    "https://cli-chat-proxy.grok.com/v1/user?include=subscription";

/// Fetch Grok Build subscription tier and map it into Storm-Dock's plan field.
pub(crate) fn fetch_grok_subscription(session: &Session) -> Result<SubscriptionSummary> {
    let auth = auth_value(session)?;
    let token = access_token(&auth).ok_or(AppError::SecretMissing)?;
    let response = Client::shared().send(
        &Call {
            method: reqwest::Method::GET,
            url: USER_SUBSCRIPTION_URL.into(),
            headers: vec![
                ("Authorization".into(), format!("Bearer {token}")),
                ("User-Agent".into(), USER_AGENT.into()),
                ("Accept".into(), "application/json".into()),
            ],
            query: Vec::new(),
            body: Body::Empty,
        },
        &Budget::new(USAGE_BUDGET),
        Retry::Transient,
    )?;
    if !(200..300).contains(&response.status) {
        return Err(AppError::Message(format!(
            "Grok 订阅接口失败 ({}): {}",
            response.status,
            response.text().chars().take(160).collect::<String>()
        )));
    }
    let value: Value = response.json().map_err(|_| HttpError::Decode)?;
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

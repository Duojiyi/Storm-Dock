use serde_json::Value;
use std::time::Duration;

use crate::cursor::session::jwt_claims;
use crate::error::{AppError, Result};
use crate::grok::session::{access_token, auth_value, cli_proxy_headers, CLI_CHAT_PROXY};
use crate::http::{Body, Budget, Call, Client, HttpError, Retry};
use crate::models::{now, Session, SubscriptionSummary};

pub(crate) const GROK_SUBSCRIPTION_UA: &str = "storm-dock-grok-subscription";
pub(crate) const USER_SUBSCRIPTION_URL_PATH: &str = "/user?include=subscription";
pub(crate) const SETTINGS_URL_PATH: &str = "/settings";

/// Fetch Grok Build subscription tier without billing (Grok Bot list path).
pub(crate) fn fetch_grok_subscription(session: &Session) -> Result<SubscriptionSummary> {
    let auth = auth_value(session)?;
    let token = access_token(&auth).ok_or(AppError::SecretMissing)?;
    let user = grok_get(
        &format!("{CLI_CHAT_PROXY}{USER_SUBSCRIPTION_URL_PATH}"),
        &token,
        &[],
        GROK_SUBSCRIPTION_UA,
        crate::http::SUBSCRIPTION_BUDGET,
    )
    .ok();
    let settings = if user
        .as_ref()
        .and_then(subscription_tier)
        .is_some_and(|tier| is_concrete_plan(&normalize_grok_tier(&tier)))
    {
        None
    } else {
        grok_get(
            &format!("{CLI_CHAT_PROXY}{SETTINGS_URL_PATH}"),
            &token,
            &[],
            GROK_SUBSCRIPTION_UA,
            crate::http::SUBSCRIPTION_BUDGET,
        )
        .ok()
    };
    let plan = pick_plan(
        user.as_ref(),
        settings.as_ref(),
        jwt_plan(&token).as_deref(),
    )
    .ok_or_else(|| AppError::Message("Grok 订阅响应缺少 tier/plan。".into()))?;
    Ok(SubscriptionSummary {
        plan: Some(plan),
        expires_at: None,
        billing_cycle_end: None,
        checked_at: Some(now()),
    })
}

pub(crate) fn grok_get(
    url: &str,
    token: &str,
    extra_headers: &[(String, String)],
    user_agent: &str,
    budget: Duration,
) -> Result<Value> {
    let mut headers = cli_proxy_headers(token);
    headers.push(("User-Agent".into(), user_agent.into()));
    headers.extend(extra_headers.iter().cloned());
    let response = Client::shared().send(
        &Call {
            method: reqwest::Method::GET,
            url: url.into(),
            headers,
            query: Vec::new(),
            body: Body::Empty,
        },
        &Budget::new(budget),
        Retry::Transient,
    )?;
    if !(200..300).contains(&response.status) {
        return Err(AppError::Message(format!(
            "Grok 接口失败 ({}): {}",
            response.status,
            response.text().chars().take(160).collect::<String>()
        )));
    }
    response.json().map_err(|_| HttpError::Decode.into())
}

/// `/user` wire id beats `/settings` junk fields like a nested `plan: "plus"`.
pub(crate) fn pick_plan(
    user: Option<&Value>,
    settings: Option<&Value>,
    jwt: Option<&str>,
) -> Option<String> {
    let user_plan = user
        .and_then(subscription_tier)
        .map(|tier| normalize_grok_tier(&tier));
    if user_plan.as_deref().is_some_and(is_concrete_plan) {
        return user_plan;
    }
    settings
        .and_then(subscription_tier)
        .map(|tier| normalize_grok_tier(&tier))
        .filter(|plan| plan != "free" || user_plan.is_none())
        .or(user_plan)
        .or_else(|| jwt.map(str::to_owned))
}

pub(crate) fn is_concrete_plan(plan: &str) -> bool {
    matches!(
        plan,
        "supergrok"
            | "supergrok_plus"
            | "supergrok_heavy"
            | "supergrok_lite"
            | "x_premium"
            | "x_premium_plus"
    )
}

#[cfg(test)]
fn subscription_from_grok_payload(value: &Value) -> Option<SubscriptionSummary> {
    let tier = subscription_tier(value).unwrap_or_else(|| "free".into());
    Some(SubscriptionSummary {
        plan: Some(normalize_grok_tier(&tier)),
        expires_at: None,
        billing_cycle_end: None,
        checked_at: Some(now()),
    })
}

pub(crate) fn subscription_tier(value: &Value) -> Option<String> {
    const KEYS: &[&str] = &[
        "subscriptionTierDisplay",
        "subscription_tier_display",
        "subscriptionTier",
        "subscription_tier",
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
    for nest in ["subscription", "user"] {
        if let Some(found) = value.get(nest).and_then(subscription_tier) {
            return Some(found);
        }
    }
    None
}

pub(crate) fn jwt_plan(token: &str) -> Option<String> {
    let claims = jwt_claims(token)?;
    let tier = claims.get("tier")?.as_u64()?;
    Some(normalize_grok_tier(&tier.to_string()))
}

/// Normalize Grok Build tier ids / CCP display names into Storm-Dock plan keys.
/// `/user` wire names come from grok-build `jwt_claim_matches_user_subscription_tier`.
pub(crate) fn normalize_grok_tier(tier: &str) -> String {
    let lower = tier
        .trim()
        .to_ascii_lowercase()
        .replace('+', " plus")
        .replace(['-', '/'], " ");
    let collapsed = lower.split_whitespace().collect::<Vec<_>>().join("_");
    match collapsed.as_str() {
        "0" | "free" | "free_tier" | "basic" | "x_basic" | "xbasic" | "none" | "null" => {
            "free".into()
        }
        "1" | "supergrok" | "grokpro" | "grok_pro" => "supergrok".into(),
        "2" => "free".into(),
        "3" | "x_premium" | "xpremium" => "x_premium".into(),
        "4" | "x_premium_plus" | "xpremium_plus" => "x_premium_plus".into(),
        "5" | "supergrokpro" | "supergrok_pro" | "supergrok_heavy" | "super_grok_heavy" => {
            "supergrok_heavy".into()
        }
        "6" | "supergroklite" | "supergrok_lite" | "super_grok_lite" => "supergrok_lite".into(),
        "7" | "supergrokplus" | "supergrok_plus" | "super_grok_plus" => "supergrok_plus".into(),
        other => other.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_official_supergrok_pro_as_heavy() {
        let summary = subscription_from_grok_payload(&serde_json::json!({
            "userId": "u1",
            "subscriptionTier": "SuperGrokPro"
        }))
        .expect("summary");
        assert_eq!(summary.plan.as_deref(), Some("supergrok_heavy"));
    }

    #[test]
    fn user_wire_beats_settings_plan_junk() {
        let user = serde_json::json!({ "subscriptionTier": "SuperGrokPro" });
        let settings = serde_json::json!({
            "plan": "plus",
            "subscription_tier_display": "SuperGrok Plus"
        });
        assert_eq!(
            pick_plan(Some(&user), Some(&settings), None).as_deref(),
            Some("supergrok_heavy")
        );
    }

    #[test]
    fn settings_display_used_when_user_has_no_tier() {
        let settings = serde_json::json!({
            "subscription_tier_display": "SuperGrok Heavy"
        });
        assert_eq!(
            pick_plan(None, Some(&settings), None).as_deref(),
            Some("supergrok_heavy")
        );
    }

    #[test]
    fn ignores_unrelated_plan_field() {
        let summary = subscription_from_grok_payload(&serde_json::json!({
            "plan": "plus",
            "subscriptionTier": "SuperGrokPro"
        }))
        .expect("summary");
        assert_eq!(summary.plan.as_deref(), Some("supergrok_heavy"));
    }

    #[test]
    fn reads_nested_subscription_tier() {
        let summary = subscription_from_grok_payload(&serde_json::json!({
            "user": { "subscription": { "subscription_tier": "SuperGrokPlus" } }
        }))
        .expect("summary");
        assert_eq!(summary.plan.as_deref(), Some("supergrok_plus"));
    }

    #[test]
    fn null_tier_maps_to_free() {
        let summary = subscription_from_grok_payload(&serde_json::json!({
            "subscriptionTier": null
        }))
        .expect("summary");
        assert_eq!(summary.plan.as_deref(), Some("free"));
    }

    #[test]
    fn normalizes_official_aliases() {
        assert_eq!(normalize_grok_tier("SuperGrokPro"), "supergrok_heavy");
        assert_eq!(normalize_grok_tier("SuperGrokPlus"), "supergrok_plus");
        assert_eq!(normalize_grok_tier("GrokPro"), "supergrok");
        assert_eq!(normalize_grok_tier("SuperGrok Heavy"), "supergrok_heavy");
        assert_eq!(normalize_grok_tier("5"), "supergrok_heavy");
        assert_eq!(normalize_grok_tier("7"), "supergrok_plus");
    }
}

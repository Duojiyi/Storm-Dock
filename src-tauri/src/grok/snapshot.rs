use serde_json::Value;
use std::thread;

use crate::error::{AppError, Result};
use crate::grok::session::{access_token, auth_value, user_id, CLI_CHAT_PROXY};
use crate::grok::subscription::{
    grok_get, is_concrete_plan, jwt_plan, normalize_grok_tier, pick_plan, subscription_tier,
    GROK_SUBSCRIPTION_UA, SETTINGS_URL_PATH, USER_SUBSCRIPTION_URL_PATH,
};
use crate::grok::usage::usage_from_billing;
use crate::http::{SUBSCRIPTION_BUDGET, USAGE_BUDGET};
use crate::models::{
    now, parse_iso_timestamp, Account, CursorUsageDetails, ImportType, Session, SubscriptionSummary,
};
use crate::store::AppState;
use crate::tray::refresh_tray;
use tauri::{AppHandle, Emitter, Manager};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const CREDITS_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";
const SUBSCRIPTIONS_URL: &str = "https://grok.com/rest/subscriptions";
const USAGE_UA: &str = "storm-dock-grok-usage";

pub(crate) struct GrokSnapshot {
    pub(crate) subscription: SubscriptionSummary,
    pub(crate) usage: CursorUsageDetails,
    pub(crate) billing_raw: Value,
}

pub(crate) fn fetch_grok_snapshot(account: &Account, session: &Session) -> Result<GrokSnapshot> {
    let auth = auth_value(session)?;
    let token = access_token(&auth).ok_or(AppError::SecretMissing)?;
    let user_url = format!("{CLI_CHAT_PROXY}{USER_SUBSCRIPTION_URL_PATH}");
    let settings_url = format!("{CLI_CHAT_PROXY}{SETTINGS_URL_PATH}");
    let mut billing_headers = Vec::new();
    if let Some(uid) = user_id(&auth) {
        billing_headers.push(("x-userid".into(), uid));
    }
    let (user, credits, subscriptions) = thread::scope(|scope| {
        let user = scope.spawn(|| {
            grok_get(
                &user_url,
                &token,
                &[],
                GROK_SUBSCRIPTION_UA,
                SUBSCRIPTION_BUDGET,
            )
        });
        let credits = scope.spawn(|| {
            grok_get(
                CREDITS_URL,
                &token,
                &billing_headers,
                USAGE_UA,
                USAGE_BUDGET,
            )
        });
        let subscriptions = scope.spawn(|| {
            grok_get(
                SUBSCRIPTIONS_URL,
                &token,
                &[],
                GROK_SUBSCRIPTION_UA,
                SUBSCRIPTION_BUDGET,
            )
        });
        (
            user.join()
                .unwrap_or_else(|_| Err(AppError::Message("Grok 订阅请求中断。".into()))),
            credits
                .join()
                .unwrap_or_else(|_| Err(AppError::Message("Grok 用量请求中断。".into()))),
            subscriptions
                .join()
                .unwrap_or_else(|_| Err(AppError::Message("Grok 订阅有效期请求中断。".into()))),
        )
    });
    let credits = credits?;
    let user = user.ok();
    let subscriptions = subscriptions.ok();
    let settings = if user
        .as_ref()
        .and_then(subscription_tier)
        .is_some_and(|tier| is_concrete_plan(&normalize_grok_tier(&tier)))
    {
        None
    } else {
        grok_get(
            &settings_url,
            &token,
            &[],
            GROK_SUBSCRIPTION_UA,
            SUBSCRIPTION_BUDGET,
        )
        .ok()
    };
    grok_snapshot_from_payloads(
        account,
        user.as_ref(),
        settings.as_ref(),
        &credits,
        subscriptions.as_ref(),
        jwt_plan(&token).as_deref(),
    )
}

pub(crate) fn grok_snapshot_from_payloads(
    account: &Account,
    user: Option<&Value>,
    settings: Option<&Value>,
    credits: &Value,
    subscriptions: Option<&Value>,
    jwt: Option<&str>,
) -> Result<GrokSnapshot> {
    let plan = pick_plan(user, settings, jwt)
        .ok_or_else(|| AppError::Message("Grok 订阅响应缺少 tier/plan。".into()))?;
    let (billing_end, reset_at) = billing_window(credits, subscriptions);
    let mut usage = usage_from_billing(account, credits, Some(plan.clone()))?;
    usage.reset_at = reset_at.or(usage.reset_at);
    usage.membership_type = Some(plan.clone());
    Ok(GrokSnapshot {
        subscription: SubscriptionSummary {
            plan: Some(plan),
            expires_at: billing_end.as_deref().and_then(parse_iso_timestamp),
            billing_cycle_end: billing_end,
            checked_at: Some(now()),
        },
        usage,
        billing_raw: credits.clone(),
    })
}

fn billing_window(
    credits: &Value,
    subscriptions: Option<&Value>,
) -> (Option<String>, Option<String>) {
    (
        subscriptions.and_then(subscription_end),
        weekly_reset(credits),
    )
}

fn weekly_reset(credits: &Value) -> Option<String> {
    let cfg = config_of(credits);
    cfg.get("currentPeriod")
        .and_then(|period| iso_stamp(period.get("end").unwrap_or(&Value::Null)))
        .or_else(|| stamp_at(cfg, &["billingPeriodEnd", "billing_period_end"]))
}

fn subscription_end(raw: &Value) -> Option<String> {
    let subs = raw.get("subscriptions")?.as_array()?;
    let sub = subs.iter().find(|item| {
        item.get("status").and_then(Value::as_str) == Some("SUBSCRIPTION_STATUS_ACTIVE")
    })?;
    stamp_at(sub, &["billingPeriodEnd"])
        .or_else(|| {
            stamp_at(
                sub.get("stripe").unwrap_or(&Value::Null),
                &["currentPeriodEnd"],
            )
        })
        .or_else(|| stamp_at(sub.get("google").unwrap_or(&Value::Null), &["expiryTime"]))
}

fn config_of(raw: &Value) -> &Value {
    raw.get("config").unwrap_or(raw)
}

fn stamp_at(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(iso_stamp))
}

fn iso_stamp(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => {
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_owned())
        }
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().map(|n| n.max(0) as u64))
            .or_else(|| number.as_f64().map(|n| n as u64))
            .and_then(unix_stamp),
        Value::Object(map) => map
            .get("val")
            .or_else(|| map.get("value"))
            .and_then(iso_stamp)
            .or_else(|| {
                let seconds = map.get("seconds").and_then(|node| {
                    node.as_u64()
                        .or_else(|| node.as_i64().map(|n| n.max(0) as u64))
                        .or_else(|| node.as_str()?.parse().ok())
                })?;
                unix_stamp(seconds)
            }),
        _ => None,
    }
}

fn unix_stamp(value: u64) -> Option<String> {
    let seconds = if value > 10_000_000_000 {
        value / 1_000
    } else {
        value
    };
    OffsetDateTime::from_unix_timestamp(seconds as i64)
        .ok()?
        .format(&Rfc3339)
        .ok()
}

/// Background snapshot after import so the list does not sit on an unknown plan.
pub(crate) fn spawn_imported_refresh(app: AppHandle, account: Account) {
    if account.application != crate::models::ApplicationKind::Grok
        || account.import_type == ImportType::ApiKey
    {
        return;
    }
    tauri::async_runtime::spawn_blocking(move || {
        let session = app
            .state::<AppState>()
            .0
            .lock()
            .ok()
            .and_then(|mut controller| controller.subscription_session(&account.id).ok());
        let Some(session) = session else {
            return;
        };
        match fetch_grok_snapshot(&account, &session) {
            Ok(snapshot) => {
                if let Ok(mut controller) = app.state::<AppState>().0.lock() {
                    let _ = controller.save_grok_snapshot(&account.id, snapshot);
                }
            }
            Err(error)
                if error.to_string().contains("失效") || error.to_string().contains("过期") =>
            {
                if let Ok(mut controller) = app.state::<AppState>().0.lock() {
                    let _ = controller.mark_token_invalid(&account.id);
                }
            }
            Err(_) => {}
        }
        refresh_tray(&app);
        let _ = app.emit("accounts-changed", ());
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ApplicationKind, ImportType};

    fn sample_account() -> Account {
        Account {
            id: "acc_test".into(),
            label: "Grok".into(),
            email: Some("a@b.com".into()),
            application: ApplicationKind::Grok,
            import_type: ImportType::OAuth,
            subscription: SubscriptionSummary::default(),
            raw_export: serde_json::json!({}),
            created_at: 1,
            updated_at: 1,
            last_used_at: 1,
        }
    }

    fn weekly_credits() -> serde_json::Value {
        serde_json::json!({
            "config": {
                "creditUsagePercent": 40.0,
                "productUsage": [{"product": "GrokBuild", "usagePercent": 40.0}],
                "billingPeriodEnd": "2026-09-18T09:38:18.522579+00:00",
                "currentPeriod": {
                    "type": "USAGE_PERIOD_TYPE_WEEKLY",
                    "end": "2026-09-18T09:38:18.522579+00:00"
                }
            }
        })
    }

    #[test]
    fn grok_com_active_subscription_is_the_list_expiry() {
        let user = serde_json::json!({ "subscriptionTier": "SuperGrokPro" });
        let subscriptions = serde_json::json!({
            "subscriptions": [
                {
                    "tier": "SUBSCRIPTION_TIER_SUPER_GROK_LITE",
                    "status": "SUBSCRIPTION_STATUS_INACTIVE",
                    "billingPeriodEnd": "2026-08-21T09:38:41.350Z"
                },
                {
                    "tier": "SUBSCRIPTION_TIER_SUPER_GROK_PRO",
                    "status": "SUBSCRIPTION_STATUS_ACTIVE",
                    "billingPeriodEnd": "2026-09-21T09:38:26.885Z",
                    "google": { "expiryTime": "2026-09-21T09:38:26.885Z" }
                }
            ]
        });
        let snap = grok_snapshot_from_payloads(
            &sample_account(),
            Some(&user),
            None,
            &weekly_credits(),
            Some(&subscriptions),
            None,
        )
        .expect("snapshot");
        assert_eq!(snap.subscription.plan.as_deref(), Some("supergrok_heavy"));
        assert_eq!(
            snap.subscription.billing_cycle_end.as_deref(),
            Some("2026-09-21T09:38:26.885Z")
        );
        assert_eq!(
            snap.subscription.expires_at,
            parse_iso_timestamp("2026-09-21T09:38:26.885Z")
        );
        assert_eq!(
            snap.usage.reset_at.as_deref(),
            Some("2026-09-18T09:38:18.522579+00:00")
        );
    }

    #[test]
    fn weekly_pool_is_not_the_subscription_expiry() {
        let user = serde_json::json!({ "subscriptionTier": "SuperGrokPro" });
        let snap = grok_snapshot_from_payloads(
            &sample_account(),
            Some(&user),
            None,
            &weekly_credits(),
            None,
            None,
        )
        .expect("snapshot");
        assert_eq!(snap.subscription.billing_cycle_end, None);
        assert_eq!(snap.subscription.expires_at, None);
        assert_eq!(
            snap.usage.reset_at.as_deref(),
            Some("2026-09-18T09:38:18.522579+00:00")
        );
    }

    #[test]
    fn stripe_current_period_end_is_the_list_expiry() {
        let user = serde_json::json!({ "subscriptionTier": "GrokPro" });
        let credits = serde_json::json!({
            "config": { "currentPeriod": { "end": "2026-05-08T00:00:00Z" } }
        });
        let subscriptions = serde_json::json!({
            "subscriptions": [{
                "status": "SUBSCRIPTION_STATUS_ACTIVE",
                "stripe": { "currentPeriodEnd": "2026-06-01T00:00:00Z" }
            }]
        });
        let snap = grok_snapshot_from_payloads(
            &sample_account(),
            Some(&user),
            None,
            &credits,
            Some(&subscriptions),
            None,
        )
        .expect("snapshot");
        assert_eq!(
            snap.subscription.billing_cycle_end.as_deref(),
            Some("2026-06-01T00:00:00Z")
        );
        assert_eq!(snap.usage.reset_at.as_deref(), Some("2026-05-08T00:00:00Z"));
    }
}

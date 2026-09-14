use serde_json::Value;

use crate::error::{AppError, Result};
use crate::grok::session::{access_token, auth_value};
use crate::grok::subscription::http_client as grok_http_client;
use crate::models::{
    now, Account, CursorUsageDetails, Session, UsageEvent, UsageMetric, WeeklyUsageSummary,
};

const USER_AGENT: &str = "storm-dock-grok-usage";
const BILLING_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing";

/// Fetch Grok Build billing usage and map into the shared usage-details shape.
pub(crate) fn fetch_grok_usage(
    account: &Account,
    session: &Session,
) -> Result<(CursorUsageDetails, Value)> {
    let auth = auth_value(session)?;
    let token = access_token(&auth).ok_or(AppError::SecretMissing)?;
    let client = grok_http_client()?;
    let response = client
        .get(BILLING_URL)
        .bearer_auth(&token)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .send()
        .map_err(|error| AppError::Message(format!("Grok 用量请求失败: {error}")))?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(AppError::Message(format!(
            "Grok 用量接口失败 ({status}): {}",
            body.chars().take(160).collect::<String>()
        )));
    }
    let raw: Value = serde_json::from_str(&body)
        .map_err(|error| AppError::Message(format!("Grok 用量响应无效: {error}")))?;
    let details = usage_from_billing(account, &raw, account.subscription.plan.clone())?;
    Ok((details, raw))
}

pub(crate) fn usage_from_billing(
    account: &Account,
    raw: &Value,
    membership: Option<String>,
) -> Result<CursorUsageDetails> {
    let config = raw.get("config").cloned().unwrap_or_else(|| raw.clone());
    let used_dollars = money_val(&config, "used").unwrap_or(0.0);
    let limit_dollars = money_val(&config, "monthlyLimit");
    let on_demand_cap = money_val(&config, "onDemandCap");
    let period_end = config
        .get("billingPeriodEnd")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let history = config
        .get("history")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let primary = currency_metric(used_dollars, limit_dollars);
    let latest_on_demand = history
        .first()
        .and_then(|item| money_val(item, "onDemandUsed"));
    let on_demand = match (latest_on_demand, on_demand_cap) {
        (Some(used), Some(limit)) if limit > 0.0 || used > 0.0 => {
            Some(currency_metric(used, Some(limit)))
        }
        (Some(used), None) if used > 0.0 => Some(currency_metric(used, None)),
        (None, Some(limit)) if limit > 0.0 => Some(currency_metric(0.0, Some(limit))),
        _ => None,
    };

    let mut weekly = Vec::new();
    let mut events = Vec::new();
    for item in &history {
        let Some(cycle) = item.get("billingCycle") else {
            continue;
        };
        let Some(year) = cycle.get("year").and_then(Value::as_u64) else {
            continue;
        };
        let Some(month) = cycle.get("month").and_then(Value::as_u64) else {
            continue;
        };
        let included = money_val(item, "includedUsed").unwrap_or(0.0);
        let on_demand_used = money_val(item, "onDemandUsed").unwrap_or(0.0);
        let total = money_val(item, "totalUsed").unwrap_or(included + on_demand_used);
        let date = format!("{year:04}-{month:02}-01");
        weekly.push(WeeklyUsageSummary {
            date,
            requests: 0.0,
            on_demand_cents: dollars_to_cents(on_demand_used),
            is_on_demand: on_demand_used > 0.0,
        });
        let timestamp = rough_month_timestamp(year, month);
        if total > 0.0 || on_demand_used > 0.0 {
            events.push(UsageEvent {
                timestamp,
                model: Some(format!("{year:04}-{month:02}")),
                requests: 0.0,
                input_tokens: None,
                output_tokens: None,
                cost_usd: Some(total),
                charged_cents: Some(dollars_to_cents(total)),
                on_demand: on_demand_used > 0.0,
            });
        }
    }
    weekly.reverse();
    events.reverse();

    Ok(CursorUsageDetails {
        account_id: account.id.clone(),
        label: account.label.clone(),
        email: account.email.clone(),
        name: None,
        membership_type: membership.or_else(|| account.subscription.plan.clone()),
        primary,
        reset_at: period_end,
        on_demand,
        grok_bot: None,
        grok_bot_reset_at: None,
        models: Vec::new(),
        weekly,
        weekly_available: true,
        weekly_error: None,
        events,
        checked_at: now(),
    })
}

fn currency_metric(used_dollars: f64, limit_dollars: Option<f64>) -> UsageMetric {
    let used = dollars_to_cents(used_dollars);
    let limit = limit_dollars.map(dollars_to_cents);
    let percent = match limit {
        Some(limit) if limit > 0.0 => (used / limit) * 100.0,
        _ => 0.0,
    };
    UsageMetric {
        kind: "currency".into(),
        used,
        limit,
        percent,
    }
}

fn money_val(value: &Value, key: &str) -> Option<f64> {
    let node = value.get(key)?;
    if let Some(number) = node.as_f64().or_else(|| node.as_i64().map(|n| n as f64)) {
        return Some(number);
    }
    node.get("val")
        .and_then(|val| val.as_f64().or_else(|| val.as_i64().map(|n| n as f64)))
}

fn dollars_to_cents(dollars: f64) -> f64 {
    (dollars * 100.0).round()
}

fn rough_month_timestamp(year: u64, month: u64) -> u64 {
    let month = month.clamp(1, 12);
    (year.saturating_sub(1970) * 365 + (month - 1) * 30 + 15) * 86_400
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ApplicationKind, ImportType, SubscriptionSummary};

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

    #[test]
    fn maps_billing_payload() {
        let raw = serde_json::json!({
            "config": {
                "monthlyLimit": {"val": 30},
                "used": {"val": 12.5},
                "onDemandCap": {"val": 10},
                "billingPeriodEnd": "2026-10-01T00:00:00+00:00",
                "history": [{
                    "billingCycle": {"year": 2026, "month": 9},
                    "includedUsed": {"val": 12.5},
                    "onDemandUsed": {"val": 1.0},
                    "totalUsed": {"val": 13.5}
                }]
            }
        });
        let details = usage_from_billing(&sample_account(), &raw, Some("free".into())).unwrap();
        assert_eq!(details.primary.used, 1250.0);
        assert_eq!(details.primary.limit, Some(3000.0));
        assert!(details.on_demand.is_some());
        assert_eq!(details.weekly.len(), 1);
        assert_eq!(
            details.reset_at.as_deref(),
            Some("2026-10-01T00:00:00+00:00")
        );
    }
}

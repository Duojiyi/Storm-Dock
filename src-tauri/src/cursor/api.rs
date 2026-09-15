use std::{thread, time::Duration};

use crate::cursor::session::{jwt_claims, session_user_id};
use crate::error::{AppError, Result};
use crate::http::{
    Body, Budget, Call, Client, HttpError, Retry, SUBSCRIPTION_BUDGET, USAGE_BUDGET,
};
use crate::models::{now, parse_timestamp, Session, SubscriptionSummary, ACCESS_TOKEN_KEY};

const CURSOR_SUBSCRIPTION_URL: &str = "https://api2.cursor.sh/auth/full_stripe_profile";
const CURSOR_DASHBOARD_URL: &str = "https://cursor.com/api";
const CURSOR_CONNECT_URL: &str = "https://api2.cursor.sh";

#[derive(Debug, PartialEq)]
pub(crate) struct MarketplacePlugin {
    pub(crate) id: u64,
    pub(crate) slug: String,
    pub(crate) name: String,
    pub(crate) icon: Option<String>,
    pub(crate) enabled: bool,
    pub(crate) team_required: bool,
}

pub(crate) fn cursor_marketplace_plugins(session: &Session) -> Result<Vec<MarketplacePlugin>> {
    let bytes = cursor_connect_request(session, "GetEffectiveUserPlugins", &[])?;
    parse_effective_user_plugins(&bytes)
}

fn parse_effective_user_plugins(bytes: &[u8]) -> Result<Vec<MarketplacePlugin>> {
    let mut plugins = Vec::new();
    for effective in protobuf_fields(&bytes)?
        .into_iter()
        .filter_map(|field| (field.number == 1 && field.wire_type == 2).then_some(field.value))
    {
        let fields = protobuf_fields(effective)?;
        let Some(plugin) = fields
            .iter()
            .find(|field| field.number == 1 && field.wire_type == 2)
            .map(|field| field.value)
        else {
            continue;
        };
        let plugin_fields = protobuf_fields(plugin)?;
        let Some(id) = plugin_fields
            .iter()
            .find(|field| field.number == 1 && field.wire_type == 0)
            .and_then(|field| decode_varint(field.value).ok())
        else {
            continue;
        };
        let slug = protobuf_string(&plugin_fields, 2)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| id.to_string());
        let name = [3, 2]
            .into_iter()
            .find_map(|number| protobuf_string(&plugin_fields, number))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| id.to_string());
        let icon = protobuf_string(&plugin_fields, 10).filter(|value| !value.trim().is_empty());
        let enabled = fields
            .iter()
            .find(|field| field.number == 4 && field.wire_type == 0)
            .and_then(|field| decode_varint(field.value).ok())
            .is_some_and(|value| value != 0);
        let team_required = fields
            .iter()
            .find(|field| field.number == 3 && field.wire_type == 0)
            .and_then(|field| decode_varint(field.value).ok())
            .is_some_and(|value| value != 0);
        plugins.push(MarketplacePlugin {
            id,
            slug,
            name,
            icon,
            enabled,
            team_required,
        });
    }
    Ok(plugins)
}

pub(crate) fn set_cursor_marketplace_plugin_enabled(
    session: &Session,
    id: u64,
    enabled: bool,
) -> Result<()> {
    let mut body = protobuf_varint_field(1, id);
    body.extend(protobuf_varint_field(3, enabled.into()));
    cursor_connect_request(session, "UpdateUserPluginInstall", &body)?;
    Ok(())
}

pub(crate) fn uninstall_cursor_marketplace_plugin(session: &Session, id: u64) -> Result<()> {
    let current = cursor_marketplace_plugins(session)?;
    let plugin = current
        .iter()
        .find(|plugin| plugin.id == id)
        .ok_or_else(|| AppError::Message("找不到 Cursor Marketplace 插件。".into()))?;
    if plugin.team_required {
        return Err(AppError::Message("此插件由团队策略管理，无法删除。".into()));
    }
    cursor_connect_request(
        session,
        "UninstallUserPlugin",
        &protobuf_varint_field(1, id),
    )?;
    verify_marketplace_plugin_state(session, id, None)
}

fn verify_marketplace_plugin_state(
    session: &Session,
    id: u64,
    expected_enabled: Option<bool>,
) -> Result<()> {
    for attempt in 0..3 {
        let plugin = cursor_marketplace_plugins(session)?
            .into_iter()
            .find(|plugin| plugin.id == id);
        let matches = match expected_enabled {
            Some(enabled) => plugin.is_some_and(|plugin| plugin.enabled == enabled),
            None => plugin.is_none(),
        };
        if matches {
            return Ok(());
        }
        if attempt < 2 {
            thread::sleep(Duration::from_millis(250));
        }
    }
    Err(AppError::Message(match expected_enabled {
        Some(false) => "Cursor 未确认插件已禁用，可能受团队策略限制。".into(),
        Some(true) => "Cursor 未确认插件已启用，请稍后重试。".into(),
        None => "Cursor 未确认插件已删除，可能受团队策略限制。".into(),
    }))
}

fn cursor_connect_request(session: &Session, method: &str, body: &[u8]) -> Result<Vec<u8>> {
    let token = session
        .values
        .get(ACCESS_TOKEN_KEY)
        .ok_or(AppError::SecretMissing)?;
    let response = Client::shared().send(
        &Call {
            method: reqwest::Method::POST,
            url: format!("{CURSOR_CONNECT_URL}/aiserver.v1.DashboardService/{method}"),
            headers: vec![
                ("Authorization".into(), format!("Bearer {token}")),
                ("Content-Type".into(), "application/proto".into()),
                ("Accept".into(), "application/proto".into()),
                ("Connect-Protocol-Version".into(), "1".into()),
                ("X-Ghost-Mode".into(), "false".into()),
            ],
            query: Vec::new(),
            body: Body::Bytes(body.to_vec()),
        },
        &Budget::new(USAGE_BUDGET),
        Retry::Transient,
    )?;
    if matches!(response.status, 401 | 403) {
        return Err(AppError::Message(
            "Cursor 登录已失效，请在 Cursor 中重新登录后重试。".into(),
        ));
    }
    if !(200..300).contains(&response.status) {
        return Err(AppError::Message(format!(
            "Cursor 插件请求失败（HTTP {}）。",
            response.status
        )));
    }
    Ok(response.bytes)
}

struct ProtobufField<'a> {
    number: u64,
    wire_type: u64,
    value: &'a [u8],
}

fn protobuf_fields(bytes: &[u8]) -> Result<Vec<ProtobufField<'_>>> {
    let mut fields = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let key = read_varint(bytes, &mut offset)?;
        let number = key >> 3;
        let wire_type = key & 7;
        if number == 0 {
            return Err(AppError::Message("Cursor 插件响应格式无效。".into()));
        }
        let start = offset;
        match wire_type {
            0 => {
                read_varint(bytes, &mut offset)?;
            }
            1 => {
                offset = offset
                    .checked_add(8)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| AppError::Message("Cursor 插件响应格式无效。".into()))?
            }
            2 => {
                let len = read_varint(bytes, &mut offset)? as usize;
                let end = offset
                    .checked_add(len)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| AppError::Message("Cursor 插件响应格式无效。".into()))?;
                fields.push(ProtobufField {
                    number,
                    wire_type,
                    value: &bytes[offset..end],
                });
                offset = end;
                continue;
            }
            5 => {
                offset = offset
                    .checked_add(4)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| AppError::Message("Cursor 插件响应格式无效。".into()))?
            }
            _ => return Err(AppError::Message("Cursor 插件响应格式无效。".into())),
        }
        fields.push(ProtobufField {
            number,
            wire_type,
            value: &bytes[start..offset],
        });
    }
    Ok(fields)
}

fn protobuf_string(fields: &[ProtobufField<'_>], number: u64) -> Option<String> {
    fields
        .iter()
        .find(|field| field.number == number && field.wire_type == 2)
        .and_then(|field| std::str::from_utf8(field.value).ok())
        .map(str::to_owned)
}

fn read_varint(bytes: &[u8], offset: &mut usize) -> Result<u64> {
    let start = *offset;
    let value = decode_varint(&bytes[start..])?;
    let mut index = start;
    while bytes.get(index).is_some_and(|byte| byte & 0x80 != 0) {
        index += 1;
    }
    *offset = index + 1;
    Ok(value)
}

fn decode_varint(bytes: &[u8]) -> Result<u64> {
    let mut value = 0u64;
    for (index, byte) in bytes.iter().copied().enumerate().take(10) {
        value |= u64::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(AppError::Message("Cursor 插件响应格式无效。".into()))
}

fn protobuf_varint_field(number: u64, value: u64) -> Vec<u8> {
    let mut result = encode_varint(number << 3);
    result.extend(encode_varint(value));
    result
}

fn encode_varint(mut value: u64) -> Vec<u8> {
    let mut result = Vec::new();
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        result.push(byte);
        if value == 0 {
            return result;
        }
    }
}

fn plan_from_response(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(object) => ["membershipType", "stripeMembershipType"]
            .iter()
            .find_map(|key| object.get(*key).and_then(serde_json::Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| object.values().find_map(plan_from_response)),
        serde_json::Value::Array(items) => items.iter().find_map(plan_from_response),
        _ => None,
    }
}

pub(crate) fn subscription_from_response(value: &serde_json::Value) -> SubscriptionSummary {
    let billing_cycle_end = value
        .get("billingCycleEnd")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    SubscriptionSummary {
        plan: plan_from_response(value),
        expires_at: billing_cycle_end
            .as_deref()
            .and_then(crate::models::parse_iso_timestamp)
            .or_else(|| value.get("billingCycleEnd").and_then(parse_timestamp)),
        billing_cycle_end,
        checked_at: Some(now()),
    }
}

pub(crate) fn merge_subscription(
    primary: Option<SubscriptionSummary>,
    fallback: Option<SubscriptionSummary>,
) -> Option<SubscriptionSummary> {
    match (primary, fallback) {
        (None, None) => None,
        (Some(primary), None) => Some(primary),
        (None, Some(fallback)) => Some(fallback),
        (Some(primary), Some(fallback)) => Some(SubscriptionSummary {
            plan: primary.plan.or(fallback.plan),
            expires_at: primary.expires_at,
            billing_cycle_end: primary.billing_cycle_end,
            checked_at: primary.checked_at.or(fallback.checked_at),
        }),
    }
}

pub(crate) fn fetch_stripe_profile(session: &Session) -> Result<serde_json::Value> {
    fetch_stripe_profile_with(session, &Budget::new(USAGE_BUDGET), Retry::Transient)
}

pub(crate) fn fetch_stripe_profile_with(
    session: &Session,
    budget: &Budget,
    retry: Retry,
) -> Result<serde_json::Value> {
    let token = session
        .values
        .get(ACCESS_TOKEN_KEY)
        .ok_or(AppError::SecretMissing)?;
    let response = Client::shared().send(
        &Call {
            method: reqwest::Method::GET,
            url: CURSOR_SUBSCRIPTION_URL.into(),
            headers: vec![("Authorization".into(), format!("Bearer {token}"))],
            query: Vec::new(),
            body: Body::Empty,
        },
        budget,
        retry,
    )?;
    if matches!(response.status, 401 | 403) {
        return Err(AppError::Message(
            "Cursor 登录已失效，请在 Cursor 中重新登录后重新导入账号。".into(),
        ));
    }
    if !(200..300).contains(&response.status) {
        return Err(HttpError::Server.into());
    }
    serde_json::from_slice(&response.bytes).map_err(|_| HttpError::Decode.into())
}

pub(crate) fn fetch_cursor_subscription(session: &Session) -> Result<SubscriptionSummary> {
    let budget = Budget::new(SUBSCRIPTION_BUDGET);
    let usage = dashboard_cookie(session).ok().and_then(|cookie| {
        dashboard_request_with(&cookie, "/usage-summary", None, &budget, Retry::None).ok()
    });
    let stripe = fetch_stripe_profile_with(session, &budget, Retry::Transient);
    merge_subscription(
        usage.as_ref().map(subscription_from_response),
        stripe.as_ref().ok().map(subscription_from_response),
    )
    .ok_or_else(|| {
        stripe
            .err()
            .unwrap_or_else(|| AppError::Message("could not refresh Cursor subscription".into()))
    })
}

pub(crate) fn fetch_cursor_subscription_fast(session: &Session) -> Result<SubscriptionSummary> {
    fetch_stripe_profile(session).map(|profile| subscription_from_response(&profile))
}

pub(crate) fn dashboard_cookie(session: &Session) -> Result<String> {
    let token = session
        .values
        .get(ACCESS_TOKEN_KEY)
        .ok_or(AppError::SecretMissing)?;
    let user_id = session_user_id(session).ok_or_else(|| {
        AppError::Message("此账户不是可用于 Cursor 用量查询的 JWT，请重新导入账户。".into())
    })?;
    if let Some(claims) = jwt_claims(token) {
        if let Some(exp) = claims.get("exp").and_then(serde_json::Value::as_i64) {
            if exp <= now() as i64 + 60 {
                return Err(AppError::Message(
                    "Cursor 登录已过期，请在 Cursor 中重新登录后重新导入账户。".into(),
                ));
            }
        }
    }
    Ok(format!("WorkosCursorSessionToken={user_id}%3A%3A{token}"))
}

pub(crate) fn dashboard_request(
    cookie: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value> {
    dashboard_request_with(
        cookie,
        path,
        body,
        &Budget::new(USAGE_BUDGET),
        Retry::Transient,
    )
}

pub(crate) fn dashboard_request_with(
    cookie: &str,
    path: &str,
    body: Option<serde_json::Value>,
    budget: &Budget,
    retry: Retry,
) -> Result<serde_json::Value> {
    let mut headers = vec![
        ("Cookie".into(), cookie.to_owned()),
        ("User-Agent".into(), "Mozilla/5.0".into()),
        ("Accept".into(), "*/*".into()),
    ];
    if body.is_some() || path.contains("dashboard/") {
        headers.extend([
            ("Origin".into(), "https://cursor.com".into()),
            (
                "Referer".into(),
                "https://cursor.com/dashboard?tab=usage".into(),
            ),
            ("Sec-Fetch-Site".into(), "same-origin".into()),
            ("Sec-Fetch-Mode".into(), "cors".into()),
            ("Sec-Fetch-Dest".into(), "empty".into()),
        ]);
    }
    let has_body = body.is_some();
    let response = Client::cursor_web().send(
        &Call {
            method: if has_body {
                reqwest::Method::POST
            } else {
                reqwest::Method::GET
            },
            url: format!("{CURSOR_DASHBOARD_URL}{path}"),
            headers,
            query: Vec::new(),
            body: body.map(Body::Json).unwrap_or(Body::Empty),
        },
        budget,
        retry,
    )?;
    let status = response.status;
    if status == 204 || response.bytes.is_empty() {
        return Err(AppError::Message(
            "Cursor 登录已失效，请在 Cursor 中重新登录后重新导入账户。".into(),
        ));
    }
    let parsed = serde_json::from_slice::<serde_json::Value>(&response.bytes).ok();
    let api_error = parsed.as_ref().and_then(|value| {
        value
            .get("error")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    });
    match status {
        401 => {
            return Err(AppError::Message(
                "Cursor 登录已失效，请在 Cursor 中重新登录后重新导入账户。".into(),
            ))
        }
        403 if !has_body => {
            return Err(AppError::Message(
                "Cursor 登录已失效，请在 Cursor 中重新登录后重新导入账户。".into(),
            ))
        }
        status if !(200..300).contains(&status) => {
            return Err(match (status, api_error) {
                (429, None) => HttpError::RateLimited.into(),
                (500..=599, None) => HttpError::Server.into(),
                (_, Some(error)) => AppError::Message(format!("Cursor 用量查询失败（{error}）。")),
                (_, None) => AppError::Message(format!("Cursor 用量查询失败（HTTP {status}）。")),
            })
        }
        _ => {}
    }
    if let Some(error) = api_error {
        return Err(AppError::Message(format!(
            "Cursor 用量查询失败（{error}）。"
        )));
    }
    parsed.ok_or_else(|| HttpError::Decode.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::session::session_from_access_token;
    use crate::models::{subscription_from_session, Session, MEMBERSHIP_TYPE_KEY};
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use std::collections::BTreeMap;
    use time::{format_description::well_known::Rfc3339, OffsetDateTime};

    #[test]
    fn dashboard_cookie_can_use_session_user_id_without_jwt_claims() {
        let token = "a".repeat(40);
        let session =
            session_from_access_token(&token, Some("user_01ABCDEFGHJKMNPQRSTVWXYZ".into()));
        assert_eq!(
            dashboard_cookie(&session).unwrap(),
            format!("WorkosCursorSessionToken=user_01ABCDEFGHJKMNPQRSTVWXYZ%3A%3A{token}")
        );
    }

    #[test]
    fn dashboard_cookie_requires_a_live_cursor_jwt() {
        let claims = URL_SAFE_NO_PAD.encode(r#"{"sub":"auth0|user_123","exp":4102444800}"#);
        let session = Session {
            values: BTreeMap::from([(
                ACCESS_TOKEN_KEY.into(),
                format!("header.{claims}.signature"),
            )]),
            raw_export: None,
        };
        assert_eq!(
            dashboard_cookie(&session).unwrap(),
            format!("WorkosCursorSessionToken=user_123%3A%3Aheader.{claims}.signature")
        );
        assert!(dashboard_cookie(&Session {
            values: BTreeMap::from([(ACCESS_TOKEN_KEY.into(), "not-a-jwt".into())]),
            raw_export: None,
        })
        .is_err());
    }
    #[test]
    fn subscription_summary_uses_local_plan_and_cursor_response_dates() {
        let session = Session {
            values: BTreeMap::from([(MEMBERSHIP_TYPE_KEY.into(), "pro".into())]),
            raw_export: None,
        };
        assert_eq!(
            subscription_from_session(&session).plan.as_deref(),
            Some("pro")
        );
        let summary = subscription_from_response(&serde_json::json!({
            "stripeMembershipType": "free",
            "subscription": { "current_period_end": 1_800_000_000_000u64 }
        }));
        assert_eq!(summary.plan.as_deref(), Some("free"));
        assert_eq!(summary.expires_at, None);
        assert!(summary.checked_at.is_some());

        let live_shape =
            subscription_from_response(&serde_json::json!({ "membershipType": "free" }));
        assert_eq!(live_shape.plan.as_deref(), Some("free"));
        assert_eq!(live_shape.expires_at, None);

        let usage_summary = subscription_from_response(&serde_json::json!({
            "membershipType": "pro",
            "billingCycleEnd": "2026-08-27T00:00:00.000Z"
        }));
        assert_eq!(usage_summary.plan.as_deref(), Some("pro"));
        assert_eq!(
            usage_summary.billing_cycle_end.as_deref(),
            Some("2026-08-27T00:00:00.000Z")
        );
        assert_eq!(
            usage_summary.expires_at,
            Some(
                OffsetDateTime::parse("2026-08-27T00:00:00.000Z", &Rfc3339)
                    .unwrap()
                    .unix_timestamp() as u64
            )
        );
        let merged = merge_subscription(Some(usage_summary), Some(summary.clone()));
        assert_eq!(
            merged.unwrap().expires_at,
            Some(
                OffsetDateTime::parse("2026-08-27T00:00:00.000Z", &Rfc3339)
                    .unwrap()
                    .unix_timestamp() as u64
            )
        );
        let stripe_only_date = subscription_from_response(&serde_json::json!({
            "membershipType": "pro",
            "nested": { "billingCycleEnd": "2026-12-01T00:00:00.000Z" }
        }));
        assert_eq!(stripe_only_date.expires_at, None);
        let merged_without_usage_date = merge_subscription(
            Some(subscription_from_response(
                &serde_json::json!({ "membershipType": "pro" }),
            )),
            Some(summary),
        );
        assert_eq!(merged_without_usage_date.unwrap().expires_at, None);
        let fractional = subscription_from_response(&serde_json::json!({
            "membershipType": "pro",
            "billingCycleEnd": "2026-08-15T03:23:58.561Z"
        }));
        assert_eq!(
            fractional.expires_at,
            Some(
                OffsetDateTime::parse("2026-08-15T03:23:58.561Z", &Rfc3339)
                    .unwrap()
                    .unix_timestamp() as u64
            )
        );
    }

    #[test]
    fn missing_subscription_response_values_do_not_fabricate_an_expiry() {
        let summary = subscription_from_response(&serde_json::json!({ "status": "active" }));
        assert_eq!(summary.plan, None);
        assert_eq!(summary.expires_at, None);
    }

    #[test]
    fn effective_user_plugins_read_the_server_id_display_name_icon_and_enabled_state() {
        let plugin = [
            protobuf_varint_field(1, 47_051_883),
            protobuf_bytes_field(2, b"caveman"),
            protobuf_bytes_field(3, b"Caveman"),
            protobuf_bytes_field(10, b"https://example.test/caveman.png"),
        ]
        .concat();
        let effective = [
            protobuf_bytes_field(1, &plugin),
            protobuf_varint_field(4, 1),
        ]
        .concat();
        let response = protobuf_bytes_field(1, &effective);

        assert_eq!(
            parse_effective_user_plugins(&response).unwrap(),
            vec![MarketplacePlugin {
                id: 47_051_883,
                slug: "caveman".into(),
                name: "Caveman".into(),
                icon: Some("https://example.test/caveman.png".into()),
                enabled: true,
                team_required: false,
            }]
        );
    }

    fn protobuf_bytes_field(number: u64, value: &[u8]) -> Vec<u8> {
        let mut result = encode_varint((number << 3) | 2);
        result.extend(encode_varint(value.len() as u64));
        result.extend(value);
        result
    }
}

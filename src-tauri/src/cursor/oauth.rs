use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};

use crate::cursor::api::{dashboard_cookie, dashboard_request, fetch_cursor_subscription};
use crate::cursor::session::jwt_claims;
use crate::error::{AppError, Result};
use crate::http::{Body, Budget, Call, Client, HttpError, HttpResponse, Retry, USAGE_BUDGET};
use crate::models::{
    json_text, Account, ApplicationKind, ImportType, Session, SubscriptionSummary,
    ACCESS_TOKEN_KEY, EMAIL_KEY, MEMBERSHIP_TYPE_KEY,
};
use crate::store::AppState;
use crate::tray::refresh_tray;

const CURSOR_OAUTH_LOGIN_URL: &str = "https://cursor.com/loginDeepControl";
const CURSOR_OAUTH_POLL_URL: &str = "https://api2.cursor.sh/auth/poll";
const CURSOR_OAUTH_POLL_ATTEMPTS: u32 = 150;
const CURSOR_OAUTH_MAX_ERRORS: u32 = 3;

#[derive(Default)]
pub(crate) struct OauthLoginState {
    generation: AtomicU64,
    login_url: Mutex<Option<String>>,
    user_code: Mutex<Option<String>>,
    browser: Mutex<Option<String>>,
    capture_workos: Mutex<bool>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OfficialLoginStatus {
    pub(crate) stage: &'static str,
    pub(crate) login_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) user_code: Option<String>,
}

pub(crate) struct CursorOauthHandshake {
    uuid: String,
    verifier: String,
    login_url: String,
}

impl OauthLoginState {
    pub(crate) fn begin_with_browser(
        &self,
        browser: Option<String>,
        capture_workos: bool,
    ) -> u64 {
        let id = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *self
            .browser
            .lock()
            .unwrap_or_else(|error| error.into_inner()) =
            Some(crate::browser::normalize_id(browser.as_deref()));
        *self
            .capture_workos
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = capture_workos;
        id
    }

    pub(crate) fn capture_workos(&self) -> bool {
        *self
            .capture_workos
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn is_active(&self, id: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == id
    }

    pub(crate) fn set_url(&self, id: u64, url: String) -> Result<()> {
        if !self.is_active(id) {
            return Err(AppError::LoginCancelled);
        }
        *self
            .login_url
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(url);
        Ok(())
    }

    pub(crate) fn set_user_code(&self, id: u64, code: String) -> Result<()> {
        if !self.is_active(id) {
            return Err(AppError::LoginCancelled);
        }
        *self
            .user_code
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(code);
        Ok(())
    }

    pub(crate) fn url(&self) -> Option<String> {
        self.login_url
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) fn browser(&self) -> String {
        crate::browser::normalize_id(
            self.browser
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_deref(),
        )
    }

    pub(crate) fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        *self
            .login_url
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        *self
            .user_code
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        *self
            .browser
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        *self
            .capture_workos
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = false;
    }

    pub(crate) fn finish(&self, id: u64) {
        if self.is_active(id) {
            *self
                .login_url
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = None;
            *self
                .user_code
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = None;
            *self
                .browser
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = None;
            *self
                .capture_workos
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = false;
        }
    }
}

impl CursorOauthHandshake {
    pub(crate) fn generate() -> Self {
        let mut random = [0u8; 32];
        random[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        random[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        Self::from_parts(
            uuid::Uuid::new_v4().to_string(),
            URL_SAFE_NO_PAD.encode(random),
        )
    }

    pub(crate) fn from_parts(uuid: String, verifier: String) -> Self {
        let challenge = pkce_challenge(&verifier);
        let login_url = format!(
            "{CURSOR_OAUTH_LOGIN_URL}?challenge={challenge}&uuid={uuid}&mode=login&redirectTarget=cli"
        );
        Self {
            uuid,
            verifier,
            login_url,
        }
    }
}

pub(crate) fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub(crate) fn emit_official_login_status(
    app: &AppHandle,
    stage: &'static str,
    login_url: Option<String>,
) {
    let _ = app.emit(
        "official-login-status",
        OfficialLoginStatus {
            stage,
            login_url,
            user_code: None,
        },
    );
}

pub(crate) fn open_browser(url: &str) -> Result<()> {
    crate::browser::open(url, crate::browser::DEFAULT_BROWSER_ID)
}

pub(crate) fn wait_if_active(app: &AppHandle, login_id: u64, duration: Duration) -> Result<()> {
    const STEP: Duration = Duration::from_millis(50);
    let mut remaining = duration;
    while !remaining.is_zero() {
        if !app.state::<OauthLoginState>().is_active(login_id) {
            return Err(AppError::LoginCancelled);
        }
        let chunk = remaining.min(STEP);
        std::thread::sleep(chunk);
        remaining -= chunk;
    }
    Ok(())
}

pub(crate) fn poll_cursor_oauth(
    handshake: &CursorOauthHandshake,
    login_id: u64,
    app: &AppHandle,
) -> Result<serde_json::Value> {
    let mut delay = Duration::from_secs(1);
    let mut consecutive_errors = 0;
    for _ in 0..CURSOR_OAUTH_POLL_ATTEMPTS {
        if !app.state::<OauthLoginState>().is_active(login_id) {
            return Err(AppError::LoginCancelled);
        }
        match poll_cursor_oauth_once(handshake) {
            Ok(Some(value)) => return Ok(value),
            Ok(None) => {
                consecutive_errors = 0;
            }
            Err(AppError::LoginCancelled) => return Err(AppError::LoginCancelled),
            Err(_) => {
                consecutive_errors += 1;
                if consecutive_errors >= CURSOR_OAUTH_MAX_ERRORS {
                    return Err(AppError::Message(
                        "Cursor 官方登录轮询连续失败，请检查网络后重试。".into(),
                    ));
                }
            }
        }
        wait_if_active(app, login_id, delay)?;
        delay = delay.mul_f32(1.2).min(Duration::from_secs(10));
    }
    Err(AppError::LoginTimeout)
}

pub(crate) fn poll_cursor_oauth_once(
    handshake: &CursorOauthHandshake,
) -> Result<Option<serde_json::Value>> {
    let budget = Budget::new(USAGE_BUDGET);
    let headers = vec![("User-Agent".into(), "Storm Dock".into())];
    if let Ok(post) = Client::shared().send(
        &Call {
            method: reqwest::Method::POST,
            url: CURSOR_OAUTH_POLL_URL.into(),
            headers: headers.clone(),
            query: Vec::new(),
            body: Body::Json(serde_json::json!({
                "uuid": handshake.uuid,
                "verifier": handshake.verifier,
            })),
        },
        &budget,
        Retry::None,
    ) {
        if (200..300).contains(&post.status) {
            return read_oauth_poll_response(post);
        }
    }
    let get = Client::shared().send(
        &Call {
            method: reqwest::Method::GET,
            url: CURSOR_OAUTH_POLL_URL.into(),
            headers,
            query: vec![
                ("uuid".into(), handshake.uuid.clone()),
                ("verifier".into(), handshake.verifier.clone()),
            ],
            body: Body::Empty,
        },
        &budget,
        Retry::None,
    )?;
    read_oauth_poll_response(get)
}

pub(crate) fn read_oauth_poll_response(
    response: HttpResponse,
) -> Result<Option<serde_json::Value>> {
    if response.status == 404 {
        return Ok(None);
    }
    let value: serde_json::Value = response.json().map_err(|_| HttpError::Decode)?;
    if !(200..300).contains(&response.status) {
        return Err(AppError::Message(format!(
            "Cursor 登录轮询失败 (HTTP {})",
            response.status
        )));
    }
    Ok(Some(value))
}

pub(crate) fn session_from_oauth_poll(value: &serde_json::Value) -> Result<Session> {
    let access_token = json_text(value, &["accessToken", "access_token"])
        .filter(|token| token.len() >= 40)
        .ok_or(AppError::InvalidImport)?;
    let mut values = BTreeMap::from([(ACCESS_TOKEN_KEY.into(), access_token.clone())]);
    if let Some(refresh) = json_text(
        value,
        &["refreshToken", "refresh_token", "cursorAuth/refreshToken"],
    ) {
        values.insert("cursorAuth/refreshToken".into(), refresh);
    }
    if let Some(auth_id) = json_text(value, &["authId", "auth_id"]) {
        values.insert("glass.lastSignedInAuthId".into(), auth_id);
    }
    if let Some(email) = json_text(value, &["email", "cachedEmail", EMAIL_KEY]) {
        values.insert(EMAIL_KEY.into(), email.clone());
        values.insert(
            "cursorAuth/cachedScopedProfile".into(),
            serde_json::json!({ "displayName": email }).to_string(),
        );
    }
    if let Some(claims) = jwt_claims(&access_token) {
        if !values.contains_key(EMAIL_KEY) {
            if let Some(email) = claims
                .get("email")
                .and_then(serde_json::Value::as_str)
                .filter(|email| !email.is_empty())
            {
                values.insert(EMAIL_KEY.into(), email.into());
                values.insert(
                    "cursorAuth/cachedScopedProfile".into(),
                    serde_json::json!({ "displayName": email }).to_string(),
                );
            }
        }
        if !values.contains_key("glass.lastSignedInAuthId") {
            if let Some(sub) = claims
                .get("sub")
                .and_then(serde_json::Value::as_str)
                .filter(|sub| !sub.is_empty())
            {
                values.insert("glass.lastSignedInAuthId".into(), sub.into());
            }
        }
    }
    Ok(Session {
        values,
        raw_export: None,
    })
}

pub(crate) fn enrich_cursor_session(session: &mut Session) -> Option<SubscriptionSummary> {
    if let Ok(cookie) = dashboard_cookie(session) {
        if let Ok(me) = dashboard_request(&cookie, "/auth/me", None) {
            if let Some(email) = json_text(&me, &["email"]) {
                session.values.insert(EMAIL_KEY.into(), email.clone());
                session.values.insert(
                    "cursorAuth/cachedScopedProfile".into(),
                    serde_json::json!({ "displayName": email }).to_string(),
                );
            } else if !session
                .values
                .contains_key("cursorAuth/cachedScopedProfile")
            {
                if let Some(name) = json_text(&me, &["name", "displayName"]) {
                    session.values.insert(
                        "cursorAuth/cachedScopedProfile".into(),
                        serde_json::json!({ "displayName": name }).to_string(),
                    );
                }
            }
        }
    }
    let summary = fetch_cursor_subscription(session).ok();
    if let Some(plan) = summary.as_ref().and_then(|item| item.plan.clone()) {
        session.values.insert(MEMBERSHIP_TYPE_KEY.into(), plan);
    }
    summary
}

pub(crate) fn complete_cursor_oauth(
    label: Option<String>,
    login_id: u64,
    app: AppHandle,
) -> Result<Account> {
    let handshake = CursorOauthHandshake::generate();
    let oauth = app.state::<OauthLoginState>();
    oauth.set_url(login_id, handshake.login_url.clone())?;
    emit_official_login_status(&app, "started", Some(handshake.login_url.clone()));
    let browser_id = oauth.browser();
    let _ = crate::browser::open(&handshake.login_url, &browser_id);
    emit_official_login_status(&app, "waiting", Some(handshake.login_url.clone()));
    let tokens = poll_cursor_oauth(&handshake, login_id, &app)?;
    if !oauth.is_active(login_id) {
        return Err(AppError::LoginCancelled);
    }
    emit_official_login_status(&app, "importing", None);
    let mut session = session_from_oauth_poll(&tokens)?;
    let expect_user = crate::cursor::session::session_user_id(&session);
    if oauth.capture_workos() {
        if let Some(cookie) = crate::cursor::workos_cookie::capture_workos_session_token(
            &browser_id,
            expect_user.as_deref(),
        ) {
            session.values.insert(
                crate::cursor::workos_cookie::WORKOS_TOKEN_KEY.into(),
                cookie,
            );
        }
    }
    let subscription = enrich_cursor_session(&mut session);
    if !oauth.is_active(login_id) {
        return Err(AppError::LoginCancelled);
    }
    let state = app.state::<AppState>();
    let mut controller = state
        .0
        .lock()
        .map_err(|_| AppError::Message("账户存储不可用".into()))?;
    let account = controller.save_imported_session(
        ApplicationKind::Cursor,
        label,
        session,
        ImportType::OAuth,
    )?;
    let account_id = account.id.clone();
    if let Some(summary) = subscription {
        let _ = controller.save_subscription(&account_id, summary);
    }
    let account = controller.account(&account_id).unwrap_or(account);
    drop(controller);
    refresh_tray(&app);
    let _ = app.emit("accounts-changed", ());
    crate::commands::spawn_imported_refresh(app.clone(), account.clone());
    oauth.finish(login_id);
    Ok(account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{import_type, ImportType, ACCESS_TOKEN_KEY, EMAIL_KEY};
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    #[test]
    fn cursor_oauth_uses_s256_pkce_and_official_login_url() {
        let challenge = pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let handshake = CursorOauthHandshake::from_parts(
            "11111111-1111-4111-8111-111111111111".into(),
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into(),
        );
        assert!(handshake
            .login_url
            .starts_with("https://cursor.com/loginDeepControl?"));
        assert!(handshake
            .login_url
            .contains("challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"));
        assert!(handshake
            .login_url
            .contains("uuid=11111111-1111-4111-8111-111111111111"));
        assert!(handshake.login_url.contains("mode=login"));
        assert!(handshake.login_url.contains("redirectTarget=cli"));
        assert!(!handshake.login_url.contains("verifier"));
        assert!(!handshake.login_url.contains("dBjftJeZ4CVP"));
    }

    #[test]
    fn oauth_poll_response_builds_a_cursor_session() {
        let claims = URL_SAFE_NO_PAD
            .encode(r#"{"sub":"auth0|user_123","email":"me@example.com","exp":4102444800}"#);
        let token = format!("header.{claims}.signature-padding-for-length");
        let session = session_from_oauth_poll(&serde_json::json!({
            "accessToken": token,
            "refreshToken": "refresh-token-value",
            "authId": "auth0|user_123"
        }))
        .unwrap();
        assert_eq!(session.values.get(ACCESS_TOKEN_KEY), Some(&token));
        assert_eq!(
            session.values.get("cursorAuth/refreshToken"),
            Some(&"refresh-token-value".into())
        );
        assert_eq!(
            session.values.get("glass.lastSignedInAuthId"),
            Some(&"auth0|user_123".into())
        );
        assert_eq!(
            session.values.get(EMAIL_KEY),
            Some(&"me@example.com".into())
        );
        assert_eq!(import_type(&session), ImportType::OAuth);
    }
}

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aes::Aes128;
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use once_cell::sync::Lazy;
use pbkdf2::pbkdf2_hmac;
use rusqlite::{Connection, OpenFlags};
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::cursor::session::parse_cursor_session_token;

pub(crate) const WORKOS_TOKEN_KEY: &str = "stormDock/workosCursorSessionToken";
const COOKIE_NAME: &str = "WorkosCursorSessionToken";
const CAPTURE_ATTEMPTS: u32 = 6;
const CAPTURE_RETRY_DELAY: Duration = Duration::from_millis(350);

#[cfg(target_os = "macos")]
static KEYCHAIN_KEYS: Lazy<Mutex<HashMap<&'static str, Option<[u8; 16]>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
#[cfg(target_os = "windows")]
static WINDOWS_OSC_KEYS: Lazy<Mutex<HashMap<&'static str, Option<[u8; 32]>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static DEFAULT_BROWSER_ID: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

#[derive(Clone, Copy)]
struct ChromiumBrowser {
    id: &'static str,
    support_relative: &'static str,
    #[cfg(target_os = "macos")]
    keychain_service: &'static str,
}

#[cfg(target_os = "macos")]
const CHROMIUM_BROWSERS: &[ChromiumBrowser] = &[
    ChromiumBrowser {
        id: "chrome",
        support_relative: "Google/Chrome",
        keychain_service: "Chrome Safe Storage",
    },
    ChromiumBrowser {
        id: "edge",
        support_relative: "Microsoft Edge",
        keychain_service: "Microsoft Edge Safe Storage",
    },
    ChromiumBrowser {
        id: "brave",
        support_relative: "BraveSoftware/Brave-Browser",
        keychain_service: "Brave Safe Storage",
    },
    ChromiumBrowser {
        id: "arc",
        support_relative: "Arc/User Data",
        keychain_service: "Arc Safe Storage",
    },
];

#[cfg(target_os = "windows")]
const CHROMIUM_BROWSERS: &[ChromiumBrowser] = &[
    ChromiumBrowser {
        id: "chrome",
        support_relative: "Google/Chrome/User Data",
    },
    ChromiumBrowser {
        id: "edge",
        support_relative: "Microsoft/Edge/User Data",
    },
    ChromiumBrowser {
        id: "brave",
        support_relative: "BraveSoftware/Brave-Browser/User Data",
    },
];

/// Capture the official cookie for the login browser.
/// Retries briefly inside this call; keychain/AES key is resolved at most once per browser.
pub(crate) fn capture_workos_session_token(
    browser_id: &str,
    expect_user_id: Option<&str>,
) -> Option<String> {
    let id = resolve_browser_id(browser_id);
    if matches!(id.as_str(), "safari" | "firefox") {
        return None;
    }
    let browser = CHROMIUM_BROWSERS.iter().find(|item| item.id == id)?;
    let key = chromium_cookie_key(browser);
    let deadline = Instant::now() + CAPTURE_RETRY_DELAY * CAPTURE_ATTEMPTS;
    let mut attempt = 0;
    loop {
        attempt += 1;
        if let Some(token) = read_matching_cookie(browser, key.as_ref(), expect_user_id) {
            return Some(token);
        }
        if attempt >= CAPTURE_ATTEMPTS || Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(CAPTURE_RETRY_DELAY);
    }
}


fn cookie_db_paths(profile_dir: &Path) -> [PathBuf; 2] {
    [
        profile_dir.join("Network").join("Cookies"),
        profile_dir.join("Cookies"),
    ]
}

fn read_matching_cookie(
    browser: &ChromiumBrowser,
    key: Option<&CookieCryptoKey>,
    expect_user_id: Option<&str>,
) -> Option<String> {
    let root = chromium_user_data_root(browser)?;
    let mut best: Option<(u64, String)> = None;
    'profiles: for profile_dir in chromium_profile_dirs(&root) {
        for cookie_db in cookie_db_paths(&profile_dir) {
            if !cookie_db.is_file() {
                continue;
            }
            let Ok(rows) = load_cookie_rows(&cookie_db) else {
                continue;
            };
            for (host_key, encrypted, updated) in rows {
                let Some(plain) = decrypt_chromium_cookie(&encrypted, key, &host_key) else {
                    continue;
                };
                let Some(value) = extract_workos_token_from_decrypted(&plain) else {
                    continue;
                };
                if let Some(expected) = expect_user_id {
                    let Some((user_id, _)) = parse_cursor_session_token(&value) else {
                        continue;
                    };
                    if user_id != expected {
                        continue;
                    }
                }
                match &best {
                    Some((best_updated, _)) if *best_updated >= updated => {}
                    _ => best = Some((updated, value)),
                }
            }
        }
        // Prefer Default profile match when we already found one for the expected user.
        if expect_user_id.is_some()
            && best.is_some()
            && profile_dir.file_name().and_then(|n| n.to_str()) == Some("Default")
        {
            break 'profiles;
        }
    }
    best.map(|(_, value)| value)
}

pub(crate) fn resolved_default_browser_id() -> String {
    if let Ok(cache) = DEFAULT_BROWSER_ID.lock() {
        if let Some(cached) = cache.as_ref() {
            return cached.clone();
        }
    }
    let resolved = default_https_browser_id().unwrap_or_else(|| "chrome".into());
    if let Ok(mut cache) = DEFAULT_BROWSER_ID.lock() {
        *cache = Some(resolved.clone());
    }
    resolved
}

fn resolve_browser_id(browser_id: &str) -> String {
    let id = crate::browser::normalize_id(Some(browser_id));
    if id != crate::browser::DEFAULT_BROWSER_ID {
        return id;
    }
    resolved_default_browser_id()
}

fn default_https_browser_id() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        // Read LaunchServices HTTPS handler. Avoid osascript (no Automation prompt).
        let plist = dirs::home_dir()?.join(
            "Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist",
        );
        if !plist.is_file() {
            return None;
        }
        let output = Command::new("plutil")
            .args(["-convert", "json", "-o", "-", "--"])
            .arg(&plist)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        let mut bundle_id = None;
        if let Some(handlers) = value.get("LSHandlers").and_then(|item| item.as_array()) {
            for handler in handlers {
                let scheme = handler
                    .get("LSHandlerURLScheme")
                    .and_then(serde_json::Value::as_str);
                if scheme != Some("https") {
                    continue;
                }
                bundle_id = handler
                    .get("LSHandlerRoleAll")
                    .or_else(|| handler.get("LSHandlerRoleViewer"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
            }
        }
        let id = bundle_id?.to_ascii_lowercase();
        if id.contains("com.google.chrome") {
            return Some("chrome".into());
        }
        if id.contains("com.microsoft.edge") {
            return Some("edge".into());
        }
        if id.contains("brave") {
            return Some("brave".into());
        }
        if id.contains("arc") || id.contains("thebrowser") {
            return Some("arc".into());
        }
        if id.contains("safari") {
            return Some("safari".into());
        }
        if id.contains("firefox") {
            return Some("firefox".into());
        }
        None
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}


fn chromium_user_data_root(browser: &ChromiumBrowser) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir()?;
        let path = home
            .join("Library/Application Support")
            .join(browser.support_relative);
        path.is_dir().then_some(path)
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("LOCALAPPDATA")?;
        let path = PathBuf::from(base).join(browser.support_relative);
        path.is_dir().then_some(path)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = browser;
        None
    }
}

fn chromium_profile_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let default = root.join("Default");
    if default.is_dir() {
        dirs.push(default);
    }
    if let Ok(entries) = fs::read_dir(root) {
        let mut profiles = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_dir()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("Profile "))
            })
            .collect::<Vec<_>>();
        profiles.sort();
        dirs.extend(profiles);
    }
    dirs
}

fn load_cookie_rows(cookie_db: &Path) -> Result<Vec<(String, Vec<u8>, u64)>, ()> {
    if let Ok(rows) = query_cookie_rows(cookie_db, true) {
        return Ok(rows);
    }
    let temp_dir = std::env::temp_dir().join(format!(
        "storm-dock-cookies-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|item| item.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&temp_dir).map_err(|_| ())?;
    let temp_db = temp_dir.join("Cookies");
    fs::copy(cookie_db, &temp_db).map_err(|_| ())?;
    for suffix in ["-wal", "-shm", "-journal"] {
        let src = PathBuf::from(format!("{}{suffix}", cookie_db.display()));
        if src.is_file() {
            let _ = fs::copy(&src, temp_dir.join(format!("Cookies{suffix}")));
        }
    }
    let result = query_cookie_rows(&temp_db, false);
    let _ = fs::remove_dir_all(&temp_dir);
    result
}

fn query_cookie_rows(cookie_db: &Path, immutable: bool) -> Result<Vec<(String, Vec<u8>, u64)>, ()> {
    let conn = if immutable {
        let uri = format!("file:{}?mode=ro&immutable=1", cookie_db.display());
        Connection::open_with_flags(
            uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
    } else {
        Connection::open_with_flags(cookie_db, OpenFlags::SQLITE_OPEN_READ_ONLY)
    }
    .map_err(|_| ())?;
    let mut stmt = conn
        .prepare(
            "SELECT host_key, encrypted_value, COALESCE(last_update_utc, last_access_utc, 0)
             FROM cookies
             WHERE name = ?1
               AND host_key IN ('cursor.com', '.cursor.com')
             ORDER BY COALESCE(last_update_utc, last_access_utc, 0) DESC",
        )
        .map_err(|_| ())?;
    let rows = stmt
        .query_map([COOKIE_NAME], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2).unwrap_or(0).max(0) as u64,
            ))
        })
        .map_err(|_| ())?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}


#[derive(Clone, Copy)]
enum CookieCryptoKey {
    #[cfg(target_os = "macos")]
    MacAes128([u8; 16]),
    #[cfg(target_os = "windows")]
    WinAes256([u8; 32]),
}

fn chromium_cookie_key(browser: &ChromiumBrowser) -> Option<CookieCryptoKey> {
    #[cfg(target_os = "macos")]
    {
        let Ok(mut cache) = KEYCHAIN_KEYS.lock() else {
            return None;
        };
        if let Some(cached) = cache.get(browser.id) {
            return cached.map(CookieCryptoKey::MacAes128);
        }
        // Read via Security.framework in-process so macOS names Storm Dock in the prompt,
        // not the `security` CLI helper.
        let password = crate::macos_native::generic_password_for_service(browser.keychain_service);
        let key = password.map(|password| {
            let mut key = [0u8; 16];
            pbkdf2_hmac::<Sha1>(password.as_bytes(), b"saltysalt", 1003, &mut key);
            key
        });
        cache.insert(browser.id, key);
        key.map(CookieCryptoKey::MacAes128)
    }
    #[cfg(target_os = "windows")]
    {
        let Ok(mut cache) = WINDOWS_OSC_KEYS.lock() else {
            return None;
        };
        if let Some(cached) = cache.get(browser.id) {
            return cached.map(CookieCryptoKey::WinAes256);
        }
        let key = windows_os_crypt_key(browser);
        cache.insert(browser.id, key);
        key.map(CookieCryptoKey::WinAes256)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = browser;
        None
    }
}

#[cfg(target_os = "windows")]
fn windows_os_crypt_key(browser: &ChromiumBrowser) -> Option<[u8; 32]> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let root = chromium_user_data_root(browser)?;
    let local_state = root.join("Local State");
    let bytes = fs::read(local_state).ok()?;
    let root: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let encoded = root.pointer("/os_crypt/encrypted_key")?.as_str()?;
    let raw = STANDARD.decode(encoded).ok()?;
    const DPAPI_PREFIX: &[u8] = b"DPAPI";
    if !raw.starts_with(DPAPI_PREFIX) {
        // App-Bound Encryption (APPB / v20) is not supported yet.
        return None;
    }
    let unprotected = windows_dpapi_unprotect(&raw[DPAPI_PREFIX.len()..])?;
    unprotected.try_into().ok()
}

#[cfg(target_os = "windows")]
fn windows_dpapi_unprotect(blob: &[u8]) -> Option<Vec<u8>> {
    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptUnprotectData(
            data_in: *const DataBlob,
            description: *mut *mut u16,
            optional_entropy: *const DataBlob,
            reserved: *mut core::ffi::c_void,
            prompt: *mut core::ffi::c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(memory: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    }

    let input_blob = DataBlob {
        cb_data: blob.len() as u32,
        pb_data: blob.as_ptr() as *mut u8,
    };
    let mut output = DataBlob {
        cb_data: 0,
        pb_data: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &input_blob,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return None;
    }
    let bytes =
        unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize) }.to_vec();
    unsafe {
        LocalFree(output.pb_data.cast());
    }
    Some(bytes)
}

fn decrypt_chromium_cookie(
    encrypted: &[u8],
    key: Option<&CookieCryptoKey>,
    host_key: &str,
) -> Option<String> {
    if encrypted.len() < 3 {
        return None;
    }
    let prefix = &encrypted[..3];
    let body = &encrypted[3..];
    match key? {
        #[cfg(target_os = "macos")]
        CookieCryptoKey::MacAes128(key) => {
            if prefix != b"v10" && prefix != b"v11" {
                return None;
            }
            let mut buffer = body.to_vec();
            let plain = cbc::Decryptor::<Aes128>::new(key.into(), (&[b' '; 16]).into())
                .decrypt_padded_mut::<Pkcs7>(&mut buffer)
                .ok()?;
            let digest = Sha256::digest(host_key.as_bytes());
            let plain = if plain.len() >= 32 && plain[..32] == digest[..] {
                &plain[32..]
            } else {
                plain
            };
            String::from_utf8(plain.to_vec()).ok()
        }
        #[cfg(target_os = "windows")]
        CookieCryptoKey::WinAes256(key) => {
            // Classic Chromium OSCrypt on Windows: v10 + 12-byte nonce + ciphertext+tag.
            // v20 (app-bound) is unsupported.
            if prefix != b"v10" {
                return None;
            }
            decrypt_windows_v10_gcm(body, key)
        }
    }
}

/// Windows Chromium OSCrypt cookie body after the `v10` prefix: nonce || ciphertext||tag.
#[cfg(any(target_os = "windows", test))]
fn decrypt_windows_v10_gcm(body: &[u8], key: &[u8; 32]) -> Option<String> {
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
    if body.len() < 12 + 16 {
        return None;
    }
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(&body[..12]);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let unbound = UnboundKey::new(&AES_256_GCM, key).ok()?;
    let opening = LessSafeKey::new(unbound);
    let mut in_out = body[12..].to_vec();
    let plaintext = opening.open_in_place(nonce, Aad::empty(), &mut in_out).ok()?;
    String::from_utf8(plaintext.to_vec()).ok()
}

/// Keep the imported/captured cookie body unchanged after light cleanup.
/// Only strips an optional `WorkosCursorSessionToken=` prefix and surrounding whitespace.
pub(crate) fn preserve_workos_token(raw: &str) -> Option<String> {
    let trimmed = raw
        .strip_prefix("WorkosCursorSessionToken=")
        .unwrap_or(raw)
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    parse_cursor_session_token(trimmed)?;
    Some(trimmed.to_owned())
}

fn extract_workos_token_from_decrypted(plain: &str) -> Option<String> {
    if let Some(token) = preserve_workos_token(plain) {
        return Some(token);
    }
    let idx = plain.find("user_")?;
    preserve_workos_token(&plain[idx..])
}

pub(crate) fn stored_workos_token(
    values: &std::collections::BTreeMap<String, String>,
) -> Option<String> {
    values
        .get(WORKOS_TOKEN_KEY)
        .map(String::as_str)
        .and_then(preserve_workos_token)
}

pub(crate) fn workos_token_from_export_record(record: &serde_json::Value) -> Option<String> {
    record
        .get("workos_token")
        .and_then(serde_json::Value::as_str)
        .and_then(preserve_workos_token)
        .or_else(|| {
            record
                .get("WorkosCursorSessionToken")
                .and_then(serde_json::Value::as_str)
                .and_then(preserve_workos_token)
        })
}

pub(crate) fn append_workos_token(record: &mut serde_json::Value, token: Option<String>) {
    let Some(object) = record.as_object_mut() else {
        return;
    };
    object.remove("workos_token");
    if let Some(token) = token {
        object.insert("workos_token".into(), serde_json::Value::String(token));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserve_keeps_percent_encoded_body_verbatim() {
        let user_id = "user_01ABCDEFGHJKMNPQRSTVWXYZ";
        let token = "a".repeat(40);
        let value = format!("{user_id}%3A%3A{token}");
        assert_eq!(
            preserve_workos_token(&format!("WorkosCursorSessionToken={value}")).as_deref(),
            Some(value.as_str())
        );
        assert_eq!(preserve_workos_token(&value).as_deref(), Some(value.as_str()));
    }

    #[test]
    fn preserve_keeps_colon_separator_verbatim() {
        let user_id = "user_01ABCDEFGHJKMNPQRSTVWXYZ";
        let token = "b".repeat(40);
        let value = format!("{user_id}::{token}");
        assert_eq!(preserve_workos_token(&value).as_deref(), Some(value.as_str()));
    }

    #[test]
    fn preserve_rejects_invalid_values() {
        assert!(preserve_workos_token("not-a-cookie").is_none());
        assert!(preserve_workos_token("user_short::abc").is_none());
    }

    #[test]
    fn resolve_browser_id_keeps_explicit_choice() {
        assert_eq!(resolve_browser_id("chrome"), "chrome");
        assert_eq!(resolve_browser_id("edge"), "edge");
        assert_eq!(resolve_browser_id("safari"), "safari");
    }

    #[test]
    fn append_workos_token_places_field_last() {
        let mut record = serde_json::json!({
            "id": "a",
            "access_token": "x".repeat(40),
            "workos_token": "stale",
        });
        let user_id = "user_01ABCDEFGHJKMNPQRSTVWXYZ";
        let token = format!("{user_id}%3A%3A{}", "c".repeat(40));
        append_workos_token(&mut record, Some(token.clone()));
        let object = record.as_object().unwrap();
        assert_eq!(object.get("workos_token").and_then(|v| v.as_str()), Some(token.as_str()));
        assert_eq!(object.keys().last().map(String::as_str), Some("workos_token"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn decrypt_strips_host_key_sha256_prefix() {
        use cbc::cipher::BlockEncryptMut;
        let password = "peanuts";
        let host = "cursor.com";
        let user_id = "user_01ABCDEFGHJKMNPQRSTVWXYZ";
        let token = "d".repeat(40);
        let plain = format!("{user_id}%3A%3A{token}");
        let mut key = [0u8; 16];
        pbkdf2_hmac::<Sha1>(password.as_bytes(), b"saltysalt", 1003, &mut key);
        let mut payload = Sha256::digest(host.as_bytes()).to_vec();
        payload.extend_from_slice(plain.as_bytes());
        let len = payload.len();
        payload.resize(len + 16, 0);
        let encrypted = cbc::Encryptor::<Aes128>::new((&key).into(), (&[b' '; 16]).into())
            .encrypt_padded_mut::<Pkcs7>(&mut payload, len)
            .expect("pad");
        let mut wire = b"v10".to_vec();
        wire.extend_from_slice(encrypted);
        let crypto = CookieCryptoKey::MacAes128(key);
        let decrypted = decrypt_chromium_cookie(&wire, Some(&crypto), host).unwrap();
        assert_eq!(decrypted, plain);
        assert_eq!(
            extract_workos_token_from_decrypted(&decrypted).as_deref(),
            Some(plain.as_str())
        );
    }

    #[test]
    fn windows_v10_gcm_roundtrip_decrypts_cookie_body() {
        use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
        use ring::rand::{SecureRandom, SystemRandom};

        let key = [7u8; 32];
        let unbound = UnboundKey::new(&AES_256_GCM, &key).unwrap();
        let sealing = LessSafeKey::new(unbound);
        let mut nonce_bytes = [0u8; 12];
        SystemRandom::new().fill(&mut nonce_bytes).unwrap();
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);
        let user_id = "user_01ABCDEFGHJKMNPQRSTVWXYZ";
        let token = "d".repeat(40);
        let value = format!("{user_id}%3A%3A{token}");
        let mut in_out = value.as_bytes().to_vec();
        sealing
            .seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
            .unwrap();
        let mut body = Vec::new();
        body.extend_from_slice(&nonce_bytes);
        body.extend_from_slice(&in_out);
        let plain = decrypt_windows_v10_gcm(&body, &key);
        assert_eq!(plain.as_deref(), Some(value.as_str()));
        assert_eq!(
            extract_workos_token_from_decrypted(plain.as_deref().unwrap()).as_deref(),
            Some(value.as_str())
        );
    }

}

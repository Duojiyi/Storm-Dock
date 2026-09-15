use serde::Deserialize;
use serde_json::Value;
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use crate::codex_sessions::{CodexSession, CodexSessionMessage};
use crate::grok_bot;

const MAX_SESSIONS: usize = 1_000;
const MAX_MESSAGES: usize = 1_000;
const TITLE_MAX_CHARS: usize = 120;
const PERSISTENCE_DIR: &str = "sand-client-persistence";
const ACCOUNT_SLOT_KEY: &str = "sand.client.slice.client-meta.account-slot";
const ROSTER_SUFFIX: &str = ".roster.last-roster";
const TRANSCRIPT_MARKER: &str = ".transcript.replicas.";
const ACCOUNT_PREFIX: &str = "sand.client.slice.account.";

#[derive(Debug, Deserialize)]
struct PersistenceEnvelope<T> {
    #[serde(default)]
    value: T,
}

#[derive(Debug, Default, Deserialize)]
struct RosterValue {
    #[serde(default)]
    rows: Vec<RosterRow>,
}

#[derive(Debug, Deserialize)]
struct RosterRow {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    updated_at: Option<u64>,
    #[serde(rename = "updatedAt")]
    #[serde(default)]
    updated_at_camel: Option<u64>,
    #[serde(default)]
    last_entry: Option<Value>,
    #[serde(rename = "lastEntry")]
    #[serde(default)]
    last_entry_camel: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
struct TranscriptValue {
    #[serde(default)]
    entries: Vec<Value>,
}

pub(crate) fn list_sessions() -> Vec<CodexSession> {
    let Some(root) = grok_bot::user_data_dir() else {
        return Vec::new();
    };
    list_sessions_from(&root)
}

pub(crate) fn load_messages(id: &str) -> Vec<CodexSessionMessage> {
    if !is_valid_id(id) {
        return Vec::new();
    }
    let Some(root) = grok_bot::user_data_dir() else {
        return Vec::new();
    };
    load_messages_from_root(&root, id)
}

pub(crate) fn delete_session(id: &str) -> Result<(), String> {
    if !is_valid_id(id) {
        return Err("无效的会话标识。".into());
    }
    let root =
        grok_bot::user_data_dir().ok_or_else(|| "未检测到 Grok Bot 数据目录。".to_string())?;
    delete_session_from(&root, id)
}

pub(crate) fn delete_sessions(ids: &[String]) -> (Vec<String>, Vec<String>) {
    let mut deleted = Vec::with_capacity(ids.len());
    let mut failed = Vec::new();
    for id in ids {
        match delete_session(id) {
            Ok(()) => deleted.push(id.clone()),
            Err(_) => failed.push(id.clone()),
        }
    }
    (deleted, failed)
}

pub(crate) fn rename_session(id: &str, title: &str) -> Result<(), String> {
    if !is_valid_id(id) {
        return Err("无效的会话标识。".into());
    }
    let title = safe_title(title).ok_or_else(|| "会话标题无效。".to_string())?;
    let root =
        grok_bot::user_data_dir().ok_or_else(|| "未检测到 Grok Bot 数据目录。".to_string())?;
    rename_session_from(&root, id, &title)
}

fn list_sessions_from(root: &Path) -> Vec<CodexSession> {
    let persistence = root.join(PERSISTENCE_DIR);
    let mut sessions: Vec<CodexSession> = Vec::new();
    let preferred_slot = read_active_account_slot(&persistence);

    let mut roster_files = list_persistence_blobs(&persistence);
    roster_files.retain(|(key, _)| key.ends_with(ROSTER_SUFFIX));
    if let Some(slot) = preferred_slot.as_deref() {
        let preferred_key = format!("{ACCOUNT_PREFIX}{slot}{ROSTER_SUFFIX}");
        if roster_files.iter().any(|(key, _)| key == &preferred_key) {
            roster_files.retain(|(key, _)| key == &preferred_key);
        }
    }

    for (key, path) in roster_files {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(envelope) = serde_json::from_str::<PersistenceEnvelope<RosterValue>>(&text) else {
            continue;
        };
        let account = account_slot_from_key(&key);
        for row in envelope.value.rows {
            if !is_valid_id(&row.id) {
                continue;
            }
            if sessions.iter().any(|session| session.id == row.id) {
                continue;
            }
            let title = row
                .name
                .as_deref()
                .and_then(safe_title)
                .or_else(|| row.title.as_deref().and_then(safe_title))
                .or_else(|| {
                    row.last_entry_camel
                        .as_ref()
                        .or(row.last_entry.as_ref())
                        .and_then(last_entry_preview)
                })
                .unwrap_or_else(|| short_id(&row.id).to_owned());
            let updated_at = row
                .updated_at_camel
                .or(row.updated_at)
                .unwrap_or_else(|| modified_at(&path));
            sessions.push(CodexSession {
                id: row.id,
                title,
                project_dir: account.clone(),
                source_path: path.display().to_string(),
                updated_at,
            });
        }
    }

    // Also surface transcript-only chats that are missing from the roster.
    for (key, path) in list_persistence_blobs(&persistence) {
        let Some(id) = transcript_id_from_key(&key) else {
            continue;
        };
        if !is_valid_id(id) || sessions.iter().any(|session| session.id == id) {
            continue;
        }
        if let Some(slot) = preferred_slot.as_deref() {
            let prefix = format!("{ACCOUNT_PREFIX}{slot}{TRANSCRIPT_MARKER}");
            if !key.starts_with(&prefix) {
                continue;
            }
        }
        sessions.push(CodexSession {
            id: id.to_owned(),
            title: short_id(id).to_owned(),
            project_dir: account_slot_from_key(&key),
            source_path: path.display().to_string(),
            updated_at: modified_at(&path),
        });
    }

    sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
    sessions.truncate(MAX_SESSIONS);
    sessions
}

fn load_messages_from_root(root: &Path, id: &str) -> Vec<CodexSessionMessage> {
    let persistence = root.join(PERSISTENCE_DIR);
    let preferred_slot = read_active_account_slot(&persistence);
    let mut candidates = Vec::new();
    for (key, path) in list_persistence_blobs(&persistence) {
        let Some(transcript_id) = transcript_id_from_key(&key) else {
            continue;
        };
        if transcript_id != id {
            continue;
        }
        let rank = match preferred_slot.as_deref() {
            Some(slot) if key.contains(&format!("{ACCOUNT_PREFIX}{slot}{TRANSCRIPT_MARKER}")) => 0,
            _ => 1,
        };
        candidates.push((rank, modified_at(&path), path));
    }
    candidates.sort_by_key(|(rank, modified, _)| (*rank, std::cmp::Reverse(*modified)));
    let Some((_, _, path)) = candidates.into_iter().next() else {
        return Vec::new();
    };
    load_transcript_messages(&path)
}

fn load_transcript_messages(path: &Path) -> Vec<CodexSessionMessage> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(envelope) = serde_json::from_str::<PersistenceEnvelope<TranscriptValue>>(&text) else {
        return Vec::new();
    };
    envelope
        .value
        .entries
        .iter()
        .filter_map(parse_transcript_entry)
        .take(MAX_MESSAGES)
        .collect()
}

fn parse_transcript_entry(entry: &Value) -> Option<CodexSessionMessage> {
    let kind = entry.get("kind").and_then(Value::as_str)?;
    let timestamp = entry
        .get("timestampMs")
        .or_else(|| entry.get("timestamp"))
        .and_then(Value::as_u64);
    match kind {
        "message" => {
            let role = entry.get("role").and_then(Value::as_str)?;
            if !matches!(role, "user" | "assistant") {
                return None;
            }
            let content = entry
                .get("content")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())?
                .to_owned();
            Some(CodexSessionMessage {
                role: role.into(),
                content,
                timestamp,
            })
        }
        "send-message" => {
            let message = entry.get("message")?;
            let message_type = message
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("text");
            let content = match message_type {
                "text" => message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())?
                    .to_owned(),
                "widget" => {
                    let prompt = message
                        .get("widget")
                        .and_then(|widget| widget.get("prompt"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())?;
                    format!("[选项] {prompt}")
                }
                other => format!("[{other}]"),
            };
            Some(CodexSessionMessage {
                role: "assistant".into(),
                content,
                timestamp,
            })
        }
        _ => None,
    }
}

fn delete_session_from(root: &Path, id: &str) -> Result<(), String> {
    let persistence = root.join(PERSISTENCE_DIR);
    let mut removed = false;
    for (key, path) in list_persistence_blobs(&persistence) {
        if !key.ends_with(ROSTER_SUFFIX) {
            continue;
        }
        if remove_roster_row(&path, id)? {
            removed = true;
        }
    }
    for (key, path) in list_persistence_blobs(&persistence) {
        if transcript_id_from_key(&key) != Some(id) {
            continue;
        }
        fs::remove_file(&path).map_err(|error| error.to_string())?;
        removed = true;
    }
    if removed {
        Ok(())
    } else {
        Err("会话不存在。".into())
    }
}

fn rename_session_from(root: &Path, id: &str, title: &str) -> Result<(), String> {
    let persistence = root.join(PERSISTENCE_DIR);
    let preferred_slot = read_active_account_slot(&persistence);
    let mut renamed = false;
    let mut roster_files = list_persistence_blobs(&persistence);
    roster_files.retain(|(key, _)| key.ends_with(ROSTER_SUFFIX));
    if let Some(slot) = preferred_slot.as_deref() {
        let preferred_key = format!("{ACCOUNT_PREFIX}{slot}{ROSTER_SUFFIX}");
        if roster_files.iter().any(|(key, _)| key == &preferred_key) {
            roster_files.retain(|(key, _)| key == &preferred_key);
        }
    }
    for (_, path) in roster_files {
        if rename_roster_row(&path, id, title)? {
            renamed = true;
        }
    }
    if renamed {
        Ok(())
    } else {
        Err("会话不存在。".into())
    }
}

fn remove_roster_row(path: &Path, id: &str) -> Result<bool, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut value = serde_json::from_str::<Value>(&text).map_err(|error| error.to_string())?;
    let rows = value
        .get_mut("value")
        .and_then(|value| value.get_mut("rows"))
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "roster 格式无效。".to_string())?;
    let before = rows.len();
    rows.retain(|row| row.get("id").and_then(Value::as_str) != Some(id));
    if rows.len() == before {
        return Ok(false);
    }
    write_json(path, &value)?;
    Ok(true)
}

fn rename_roster_row(path: &Path, id: &str, title: &str) -> Result<bool, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut value = serde_json::from_str::<Value>(&text).map_err(|error| error.to_string())?;
    let rows = value
        .get_mut("value")
        .and_then(|value| value.get_mut("rows"))
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "roster 格式无效。".to_string())?;
    let mut changed = false;
    for row in rows {
        if row.get("id").and_then(Value::as_str) == Some(id) {
            row["name"] = Value::String(title.to_owned());
            changed = true;
        }
    }
    if !changed {
        return Ok(false);
    }
    write_json(path, &value)?;
    Ok(true)
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let payload = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    let mut file = File::create(path).map_err(|error| error.to_string())?;
    file.write_all(&payload)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn read_active_account_slot(persistence: &Path) -> Option<String> {
    for (key, path) in list_persistence_blobs(persistence) {
        if key != ACCOUNT_SLOT_KEY {
            continue;
        }
        let text = fs::read_to_string(path).ok()?;
        let value = serde_json::from_str::<Value>(&text).ok()?;
        let slot = value.get("value").and_then(|value| {
            value.as_str().map(str::to_owned).or_else(|| {
                value
                    .get("accountSlot")
                    .or_else(|| value.get("slot"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
        })?;
        let slot = slot.trim();
        if slot.is_empty() {
            return None;
        }
        // Persistence keys keep the Auth0 subject percent-encoded.
        return Some(slot.replace('|', "%7C"));
    }
    None
}

fn list_persistence_blobs(persistence: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(persistence) else {
        return Vec::new();
    };
    let mut blobs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("blob") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(key) = decode_base32_key(stem) else {
            continue;
        };
        blobs.push((key, path));
    }
    blobs
}

fn account_slot_from_key(key: &str) -> Option<String> {
    let rest = key.strip_prefix(ACCOUNT_PREFIX)?;
    let slot = rest
        .split_once(".roster.")
        .or_else(|| rest.split_once(".transcript."))
        .or_else(|| rest.split_once(".selection."))
        .or_else(|| rest.split_once(".composer-"))
        .map(|(slot, _)| slot)
        .unwrap_or(rest);
    let decoded = slot.replace("%7C", "|");
    (!decoded.is_empty()).then_some(decoded)
}

fn transcript_id_from_key(key: &str) -> Option<&str> {
    let (_, id) = key.split_once(TRANSCRIPT_MARKER)?;
    (!id.is_empty()).then_some(id)
}

fn last_entry_preview(value: &Value) -> Option<String> {
    value
        .get("text")
        .and_then(Value::as_str)
        .and_then(safe_title)
}

fn modified_at(path: &Path) -> u64 {
    fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && !id.contains(['/', '\\', '\0']) && !id.contains("..")
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn safe_title(value: &str) -> Option<String> {
    let title = value.trim();
    if title.is_empty() {
        return None;
    }
    Some(if title.chars().count() > TITLE_MAX_CHARS {
        title.chars().take(TITLE_MAX_CHARS).collect()
    } else {
        title.to_owned()
    })
}

fn decode_base32_key(input: &str) -> Option<String> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut bytes = Vec::with_capacity((input.len() * 5) / 8);
    let mut buffer: u64 = 0;
    let mut bits: u32 = 0;
    for ch in input.chars() {
        let upper = ch.to_ascii_uppercase();
        let value = alphabet.iter().position(|&item| item == upper as u8)?;
        buffer = (buffer << 5) | value as u64;
        bits += 5;
        while bits >= 8 {
            bits -= 8;
            bytes.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
fn encode_base32_key(input: &str) -> String {
    let alphabet = b"abcdefghijklmnopqrstuvwxyz234567";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let mut buffer: u64 = 0;
    let mut bits: u32 = 0;
    for &byte in bytes {
        buffer = (buffer << 8) | u64::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let index = ((buffer >> bits) & 0x1f) as usize;
            out.push(alphabet[index] as char);
        }
    }
    if bits > 0 {
        let index = ((buffer << (5 - bits)) & 0x1f) as usize;
        out.push(alphabet[index] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "storm-dock-grok-bot-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(root.join(PERSISTENCE_DIR)).unwrap();
        root
    }

    fn write_blob(root: &Path, key: &str, value: Value) {
        let path = root
            .join(PERSISTENCE_DIR)
            .join(format!("{}.blob", encode_base32_key(key)));
        let envelope = serde_json::json!({
            "schemaVersion": 1,
            "value": value,
        });
        fs::write(path, serde_json::to_vec(&envelope).unwrap()).unwrap();
    }

    #[test]
    fn lists_roster_sessions_for_active_account_slot() {
        let root = temp_root("list");
        write_blob(
            &root,
            ACCOUNT_SLOT_KEY,
            Value::String("auth0|user_ACTIVE".into()),
        );
        write_blob(
            &root,
            "sand.client.slice.account.auth0%7Cuser_ACTIVE.roster.last-roster",
            serde_json::json!({
                "rows": [{
                    "id": "11111111-1111-1111-1111-111111111111",
                    "name": "Alpha",
                    "updatedAt": 2000
                }]
            }),
        );
        write_blob(
            &root,
            "sand.client.slice.account.auth0%7Cuser_OTHER.roster.last-roster",
            serde_json::json!({
                "rows": [{
                    "id": "22222222-2222-2222-2222-222222222222",
                    "name": "Other",
                    "updatedAt": 3000
                }]
            }),
        );

        let sessions = list_sessions_from(&root);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "11111111-1111-1111-1111-111111111111");
        assert_eq!(sessions[0].title, "Alpha");
    }

    #[test]
    fn loads_transcript_messages_from_persistence_blob() {
        let root = temp_root("messages");
        let id = "33333333-3333-3333-3333-333333333333";
        write_blob(
            &root,
            &format!("sand.client.slice.account.auth0%7Cuser_A.transcript.replicas.{id}"),
            serde_json::json!({
                "entries": [
                    {
                        "kind": "send-message",
                        "timestampMs": 10,
                        "message": { "type": "text", "content": "你好" }
                    },
                    {
                        "kind": "message",
                        "role": "user",
                        "timestampMs": 20,
                        "content": "改会话列表"
                    }
                ]
            }),
        );

        let messages = load_messages_from_root(&root, id);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "assistant");
        assert_eq!(messages[0].content, "你好");
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[1].content, "改会话列表");
    }

    #[test]
    fn renames_and_deletes_roster_sessions() {
        let root = temp_root("edit");
        let id = "44444444-4444-4444-4444-444444444444";
        let roster_key = "sand.client.slice.account.auth0%7Cuser_A.roster.last-roster";
        write_blob(
            &root,
            roster_key,
            serde_json::json!({
                "rows": [{ "id": id, "name": "Old", "updatedAt": 1 }]
            }),
        );
        write_blob(
            &root,
            &format!("sand.client.slice.account.auth0%7Cuser_A.transcript.replicas.{id}"),
            serde_json::json!({ "entries": [] }),
        );

        rename_session_from(&root, id, "New Name").unwrap();
        let sessions = list_sessions_from(&root);
        assert_eq!(sessions[0].title, "New Name");

        delete_session_from(&root, id).unwrap();
        assert!(list_sessions_from(&root).is_empty());
    }

    #[test]
    fn round_trips_base32_persistence_keys() {
        let key = "sand.client.slice.account.auth0%7Cuser_01ABC.roster.last-roster";
        let encoded = encode_base32_key(key);
        assert_eq!(decode_base32_key(&encoded).as_deref(), Some(key));
    }
}

use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::error::{AppError, Result};
use crate::http::{Body, Budget, Call, Client, Retry, USAGE_BUDGET};
use std::time::Duration;

const ATTACHMENT_PROBE_BUDGET: Duration = Duration::from_secs(6);
use crate::models::{Session, ACCESS_TOKEN_KEY};

const CURSOR_CONNECT_URL: &str = "https://api2.cursor.sh";
const ATTACHMENT_METHOD: &str =
    "aiserver.v1.GrokBotService/ReadGrokBotAgentAttachmentChunk";
const AGENT_DATA_ROOT: &str = "/home/box/agent-data";
const SAND_DATA_ROOT: &str = "/home/box/sand-data";
const CHUNK_SIZE: u64 = 256 * 1024;
const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;
pub(crate) const ATTACHMENT_EXPIRED_PREFIX: &str = "attachment_expired:";
pub(crate) const ATTACHMENT_EXPIRED_MESSAGE: &str =
    "远程沙箱中找不到该附件（会话结束后文件可能已被清理）。";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AttachmentSource {
    Local(PathBuf),
    Remote(String),
}

pub(crate) fn resolve_attachment_source(source: &str) -> Result<AttachmentSource> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err(AppError::Message("无效的附件路径。".into()));
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Err(AppError::Message("远程 http(s) 附件请使用浏览器打开。".into()));
    }
    let absolute = if let Some(rest) = trimmed.strip_prefix("file://") {
        percent_decode(rest)
    } else {
        trimmed.to_owned()
    };
    if absolute.is_empty() || !absolute.starts_with('/') {
        return Err(AppError::Message("附件路径无效。".into()));
    }
    let local = PathBuf::from(&absolute);
    if local.is_file() {
        return Ok(AttachmentSource::Local(local));
    }
    if let Some(remote) = normalize_remote_box_path(&absolute) {
        return Ok(AttachmentSource::Remote(remote));
    }
    Err(AppError::Message(format!(
        "{ATTACHMENT_EXPIRED_PREFIX}附件文件不存在；也不是可拉取的 Grok Bot 远程沙箱路径。"
    )))
}

/// Remap model-visible `/home/box/agent-data/...` to `/home/box/sand-data/...`
/// (same rule as Grok Bot desktop) and accept already-remapped sand-data paths.
pub(crate) fn normalize_remote_box_path(path: &str) -> Option<String> {
    let normalized = path.trim().replace('\\', "/");
    let candidate = if let Some(rest) = normalized.strip_prefix(AGENT_DATA_ROOT) {
        format!("{SAND_DATA_ROOT}{rest}")
    } else if normalized.starts_with(SAND_DATA_ROOT) {
        normalized
    } else {
        return None;
    };
    is_agent_attachment_path(&candidate).then_some(candidate)
}

fn is_agent_attachment_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix(&format!("{SAND_DATA_ROOT}/agents/")) else {
        return false;
    };
    let mut parts = rest.split('/');
    let agent = parts.next().unwrap_or_default();
    let kind = parts.next().unwrap_or_default();
    let file = parts.next().unwrap_or_default();
    !agent.is_empty()
        && agent != "."
        && agent != ".."
        && !agent.contains('\0')
        && matches!(kind, "attachments" | "assets")
        && !file.is_empty()
        && parts.next().is_none()
}


fn expired_error_message() -> String {
    format!("{ATTACHMENT_EXPIRED_PREFIX}{ATTACHMENT_EXPIRED_MESSAGE}")
}

pub(crate) fn is_expired_error_message(message: &str) -> bool {
    let trimmed = message.trim();
    trimmed.starts_with(ATTACHMENT_EXPIRED_PREFIX)
        || trimmed.contains("远程沙箱中找不到该附件")
        || trimmed.to_ascii_lowercase().contains("not_found")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttachmentAvailability {
    pub(crate) status: AttachmentAvailabilityStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AttachmentAvailabilityStatus {
    Available,
    Expired,
    Unavailable,
}

impl AttachmentAvailability {
    pub(crate) fn available() -> Self {
        Self {
            status: AttachmentAvailabilityStatus::Available,
            reason: None,
        }
    }

    pub(crate) fn expired(reason: impl Into<String>) -> Self {
        Self {
            status: AttachmentAvailabilityStatus::Expired,
            reason: Some(reason.into()),
        }
    }

    pub(crate) fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            status: AttachmentAvailabilityStatus::Unavailable,
            reason: Some(reason.into()),
        }
    }
}

/// Classify whether an attachment can still be downloaded.
/// - local file present => available
/// - remote box path: tiny read-only probe (length=1); 404 => expired
/// - auth/plan/network => unavailable (not expired)
/// - unsupported missing paths (e.g. `/workspace/...`) => expired
pub(crate) fn probe_attachment_availability(
    session: Option<&Session>,
    source: &str,
) -> AttachmentAvailability {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return AttachmentAvailability::unavailable("无效的附件路径。");
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return AttachmentAvailability::available();
    }
    let absolute = if let Some(rest) = trimmed.strip_prefix("file://") {
        percent_decode(rest)
    } else {
        trimmed.to_owned()
    };
    if absolute.is_empty() || !absolute.starts_with('/') {
        return AttachmentAvailability::unavailable("附件路径无效。");
    }
    let local = PathBuf::from(&absolute);
    if local.is_file() {
        return AttachmentAvailability::available();
    }
    let Some(remote) = normalize_remote_box_path(&absolute) else {
        return AttachmentAvailability::expired(
            "该附件路径无法访问（非 Grok Bot 沙箱附件，或本地文件已不存在）。",
        );
    };
    let Some(session) = session else {
        return AttachmentAvailability::unavailable(
            "需要已登录的 Grok Bot 账号才能检查远程附件。",
        );
    };
    match read_attachment_chunk(session, &remote, 0, 1, ATTACHMENT_PROBE_BUDGET) {
        Ok(_) => AttachmentAvailability::available(),
        Err(AppError::Message(message)) if is_expired_error_message(&message) => {
            AttachmentAvailability::expired(ATTACHMENT_EXPIRED_MESSAGE)
        }
        Err(AppError::SecretMissing) => {
            AttachmentAvailability::unavailable("账号缺少有效登录凭据。")
        }
        Err(AppError::Message(message)) => AttachmentAvailability::unavailable(message),
        Err(error) => AttachmentAvailability::unavailable(error.to_string()),
    }
}

pub(crate) fn export_attachment(
    session: Option<&Session>,
    source: &str,
    destination: &Path,
) -> Result<()> {
    if destination.as_os_str().is_empty() {
        return Err(AppError::Message("无效的保存路径。".into()));
    }
    if let Some(parent) = destination.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    match resolve_attachment_source(source)? {
        AttachmentSource::Local(path) => {
            fs::copy(path, destination)?;
            Ok(())
        }
        AttachmentSource::Remote(remote) => {
            let session = session.ok_or_else(|| {
                AppError::Message("需要已登录的 Grok Bot 账号才能下载远程沙箱附件。".into())
            })?;
            let bytes = fetch_remote_attachment_bytes(session, &remote)?;
            let mut file = File::create(destination)?;
            file.write_all(&bytes)?;
            Ok(())
        }
    }
}

pub(crate) fn read_attachment_bytes(
    session: Option<&Session>,
    source: &str,
    max_bytes: u64,
) -> Result<Option<Vec<u8>>> {
    match resolve_attachment_source(source)? {
        AttachmentSource::Local(path) => {
            let metadata = fs::metadata(&path)?;
            if metadata.len() > max_bytes {
                return Ok(None);
            }
            Ok(Some(fs::read(path)?))
        }
        AttachmentSource::Remote(remote) => {
            let Some(session) = session else {
                return Ok(None);
            };
            let bytes = fetch_remote_attachment_bytes(session, &remote)?;
            if bytes.len() as u64 > max_bytes {
                return Ok(None);
            }
            Ok(Some(bytes))
        }
    }
}

pub(crate) fn fetch_remote_attachment_bytes(session: &Session, remote_path: &str) -> Result<Vec<u8>> {
    let first = read_attachment_chunk(session, remote_path, 0, CHUNK_SIZE, USAGE_BUDGET)?;
    let total = first.total_size;
    if total == 0 {
        return Ok(Vec::new());
    }
    if total > MAX_ATTACHMENT_BYTES {
        return Err(AppError::Message(format!(
            "附件过大（{} bytes），已跳过下载。",
            total
        )));
    }
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(&first.data);
    let mut offset = first.data.len() as u64;
    while offset < total {
        let length = CHUNK_SIZE.min(total - offset);
        let chunk = read_attachment_chunk(session, remote_path, offset, length, USAGE_BUDGET)?;
        if chunk.data.is_empty() {
            return Err(AppError::Message("远程附件读取中断。".into()));
        }
        out.extend_from_slice(&chunk.data);
        offset += chunk.data.len() as u64;
    }
    if out.len() as u64 != total {
        return Err(AppError::Message(format!(
            "远程附件不完整（{} / {} bytes）。",
            out.len(),
            total
        )));
    }
    Ok(out)
}

struct AttachmentChunk {
    data: Vec<u8>,
    total_size: u64,
}

fn read_attachment_chunk(
    session: &Session,
    path: &str,
    offset: u64,
    length: u64,
    budget: Duration,
) -> Result<AttachmentChunk> {
    let token = session
        .values
        .get(ACCESS_TOKEN_KEY)
        .ok_or(AppError::SecretMissing)?;
    let mut body = protobuf_string_field(1, path);
    body.extend(protobuf_varint_field(2, offset));
    body.extend(protobuf_varint_field(3, length));

    let response = Client::shared().send(
        &Call {
            method: reqwest::Method::POST,
            url: format!("{CURSOR_CONNECT_URL}/{ATTACHMENT_METHOD}"),
            headers: vec![
                ("Authorization".into(), format!("Bearer {token}")),
                ("Content-Type".into(), "application/proto".into()),
                ("Accept".into(), "application/proto".into()),
                ("Connect-Protocol-Version".into(), "1".into()),
                ("X-Ghost-Mode".into(), "false".into()),
                ("x-cursor-client-type".into(), "sand".into()),
                ("x-cursor-client-version".into(), "sand-desktop".into()),
            ],
            query: Vec::new(),
            body: Body::Bytes(body),
        },
        &Budget::new(budget),
        Retry::Transient,
    )?;

    if matches!(response.status, 401 | 403) {
        let detail = connect_error_message(&response.bytes)
            .unwrap_or_else(|| "Grok Bot 登录无效或当前套餐无法访问远程附件。".into());
        return Err(AppError::Message(detail));
    }
    if response.status == 404 {
        return Err(AppError::Message(expired_error_message()));
    }
    if !(200..300).contains(&response.status) {
        let detail = connect_error_message(&response.bytes).unwrap_or_else(|| {
            format!("拉取远程附件失败（HTTP {}）。", response.status)
        });
        return Err(AppError::Message(detail));
    }

    parse_attachment_chunk_response(&response.bytes)
}

fn parse_attachment_chunk_response(bytes: &[u8]) -> Result<AttachmentChunk> {
    let fields = protobuf_fields(bytes)?;
    let data = fields
        .iter()
        .find(|field| field.number == 1 && field.wire_type == 2)
        .map(|field| field.value.to_vec())
        .unwrap_or_default();
    let total_size = fields
        .iter()
        .find(|field| field.number == 2 && field.wire_type == 0)
        .and_then(|field| decode_varint(field.value).ok())
        .unwrap_or(data.len() as u64);
    Ok(AttachmentChunk { data, total_size })
}

fn connect_error_message(bytes: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    value
        .get("message")
        .and_then(|message| message.as_str())
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_owned)
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && bytes[index + 1].is_ascii_hexdigit()
            && bytes[index + 2].is_ascii_hexdigit()
        {
            let hi = hex_nibble(bytes[index + 1]);
            let lo = hex_nibble(bytes[index + 2]);
            out.push((hi << 4) | lo);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_owned())
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => 0,
    }
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
            return Err(AppError::Message("远程附件响应格式无效。".into()));
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
                    .ok_or_else(|| AppError::Message("远程附件响应格式无效。".into()))?;
            }
            2 => {
                let len = read_varint(bytes, &mut offset)? as usize;
                let end = offset
                    .checked_add(len)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| AppError::Message("远程附件响应格式无效。".into()))?;
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
                    .ok_or_else(|| AppError::Message("远程附件响应格式无效。".into()))?;
            }
            _ => return Err(AppError::Message("远程附件响应格式无效。".into())),
        }
        fields.push(ProtobufField {
            number,
            wire_type,
            value: &bytes[start..offset],
        });
    }
    Ok(fields)
}

fn protobuf_string_field(number: u64, value: &str) -> Vec<u8> {
    let bytes = value.as_bytes();
    let mut result = encode_varint((number << 3) | 2);
    result.extend(encode_varint(bytes.len() as u64));
    result.extend(bytes);
    result
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
    Err(AppError::Message("远程附件响应格式无效。".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaps_agent_data_paths_to_sand_data() {
        let path = "/home/box/agent-data/agents/abc/attachments/deadbeef.md";
        assert_eq!(
            normalize_remote_box_path(path).as_deref(),
            Some("/home/box/sand-data/agents/abc/attachments/deadbeef.md")
        );
        assert_eq!(
            normalize_remote_box_path(
                "/home/box/sand-data/agents/abc/attachments/deadbeef.md"
            )
            .as_deref(),
            Some("/home/box/sand-data/agents/abc/attachments/deadbeef.md")
        );
        assert!(normalize_remote_box_path("/tmp/other.md").is_none());
        assert!(normalize_remote_box_path("/home/box/sand-data/agents/abc/other/x.md").is_none());
    }

    #[test]
    fn builds_and_parses_attachment_chunk_roundtrip_shape() {
        let mut body = protobuf_string_field(1, "/home/box/sand-data/agents/a/attachments/b.md");
        body.extend(protobuf_varint_field(2, 0));
        body.extend(protobuf_varint_field(3, 1024));
        assert!(!body.is_empty());

        let mut response = protobuf_string_field(1, "hello");
        // field 1 is bytes; use string helper which is identical wire format
        response.extend(protobuf_varint_field(2, 5));
        let parsed = parse_attachment_chunk_response(&response).unwrap();
        assert_eq!(parsed.data, b"hello");
        assert_eq!(parsed.total_size, 5);
    }

    #[test]
    fn availability_maps_http_and_missing_local_paths() {
        assert_eq!(
            probe_attachment_availability(None, "https://example.com/a.png").status,
            AttachmentAvailabilityStatus::Available
        );
        assert_eq!(
            probe_attachment_availability(None, "").status,
            AttachmentAvailabilityStatus::Unavailable
        );
        assert_eq!(
            probe_attachment_availability(None, "/workspace/tmp/file.md").status,
            AttachmentAvailabilityStatus::Expired
        );
        assert_eq!(
            probe_attachment_availability(
                None,
                "file:///home/box/agent-data/agents/abc/attachments/deadbeef.md"
            )
            .status,
            AttachmentAvailabilityStatus::Unavailable
        );
    }

    #[test]
    fn expired_error_message_detection() {
        assert!(is_expired_error_message(&expired_error_message()));
        assert!(is_expired_error_message(ATTACHMENT_EXPIRED_MESSAGE));
        assert!(is_expired_error_message("Error: not_found"));
        assert!(!is_expired_error_message("Grok Bot requires an Ultra subscription"));
        assert!(!is_expired_error_message("网络错误"));
    }
}

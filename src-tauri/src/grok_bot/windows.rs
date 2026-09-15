use std::{fs, path::PathBuf};

use serde_json::Value;

use super::{encrypt_os_crypt_windows, write_json_file};
use crate::desktop::{self, DesktopApp};
use crate::error::{AppError, Result};

const DPAPI_PREFIX: &[u8] = b"DPAPI";

pub(crate) fn data_path() -> Result<PathBuf> {
    user_data_dir().map(|dir| dir.join("sand-secrets.json"))
}

fn user_data_dir() -> Result<PathBuf> {
    dirs::data_dir()
        .map(|dir| dir.join("Grok Bot"))
        .ok_or_else(|| AppError::Message("无法读取用户目录。".into()))
}

fn local_state_path() -> Result<PathBuf> {
    user_data_dir().map(|dir| dir.join("Local State"))
}

pub(crate) fn ensure_installed() -> Result<()> {
    desktop::ensure_installed(DesktopApp::GrokBot)
}

pub(crate) fn is_running() -> bool {
    desktop::is_running(DesktopApp::GrokBot)
}

pub(crate) fn launch() -> Result<()> {
    desktop::launch(DesktopApp::GrokBot)
}

pub(crate) fn quit_and_wait() -> Result<()> {
    desktop::quit_and_wait(DesktopApp::GrokBot)
}

pub(crate) fn encrypt_account_fields(
    access: &str,
    refresh: &str,
    profile: &str,
) -> Result<(String, String, String)> {
    let key = os_crypt_key()?;
    Ok((
        encrypt_os_crypt_windows(access, &key),
        encrypt_os_crypt_windows(refresh, &key),
        encrypt_os_crypt_windows(profile, &key),
    ))
}

fn os_crypt_key() -> Result<[u8; 32]> {
    let path = local_state_path()?;
    match fs::read(&path) {
        Ok(bytes) => {
            let mut root: Value = serde_json::from_slice(&bytes)
                .map_err(|_| AppError::Message("Grok Bot Local State 格式无效。".into()))?;
            if let Some(key) = read_encrypted_key(&root)? {
                return Ok(key);
            }
            let key = generate_os_crypt_key()?;
            write_encrypted_key(&mut root, &key)?;
            write_json_file(&path, &root)?;
            Ok(key)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let key = generate_os_crypt_key()?;
            let mut root = serde_json::json!({});
            write_encrypted_key(&mut root, &key)?;
            write_json_file(&path, &root)?;
            Ok(key)
        }
        Err(error) => Err(error.into()),
    }
}

fn read_encrypted_key(root: &Value) -> Result<Option<[u8; 32]>> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let Some(encoded) = root
        .pointer("/os_crypt/encrypted_key")
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let raw = STANDARD
        .decode(encoded)
        .map_err(|_| AppError::Message("Grok Bot OSCrypt 密钥格式无效。".into()))?;
    if !raw.starts_with(DPAPI_PREFIX) {
        return Err(AppError::Message(
            "Grok Bot 使用了无法适配的 Windows 加密方式。".into(),
        ));
    }
    let unprotected = dpapi_unprotect(&raw[DPAPI_PREFIX.len()..])?;
    let key: [u8; 32] = unprotected
        .try_into()
        .map_err(|_| AppError::Message("Grok Bot OSCrypt 密钥长度无效。".into()))?;
    Ok(Some(key))
}

fn write_encrypted_key(root: &mut Value, key: &[u8; 32]) -> Result<()> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let mut payload = DPAPI_PREFIX.to_vec();
    payload.extend(dpapi_protect(key)?);
    if !root.is_object() {
        *root = serde_json::json!({});
    }
    root["os_crypt"]["encrypted_key"] = Value::String(STANDARD.encode(payload));
    Ok(())
}

fn generate_os_crypt_key() -> Result<[u8; 32]> {
    use ring::rand::{SecureRandom, SystemRandom};
    let mut key = [0u8; 32];
    SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| AppError::Message("无法生成 Grok Bot 加密密钥。".into()))?;
    Ok(key)
}

fn dpapi_protect(plain: &[u8]) -> Result<Vec<u8>> {
    dpapi(plain, true)
}

#[cfg(test)]
pub(super) fn dpapi_unprotect_for_test(blob: &[u8]) -> Result<Vec<u8>> {
    dpapi_unprotect(blob)
}

#[cfg(test)]
pub(super) fn dpapi_protect_for_test(plain: &[u8]) -> Result<Vec<u8>> {
    dpapi_protect(plain)
}

fn dpapi_unprotect(blob: &[u8]) -> Result<Vec<u8>> {
    dpapi(blob, false)
}

fn dpapi(input: &[u8], protect: bool) -> Result<Vec<u8>> {
    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            data_in: *const DataBlob,
            description: *const u16,
            optional_entropy: *const DataBlob,
            reserved: *mut core::ffi::c_void,
            prompt: *mut core::ffi::c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
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
        cb_data: input.len() as u32,
        pb_data: input.as_ptr() as *mut u8,
    };
    let mut output = DataBlob {
        cb_data: 0,
        pb_data: std::ptr::null_mut(),
    };
    let ok = unsafe {
        if protect {
            CryptProtectData(
                &input_blob,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input_blob,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err(AppError::Message(
            "无法使用 Windows DPAPI 处理 Grok Bot 密钥。".into(),
        ));
    }
    let bytes =
        unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize) }.to_vec();
    unsafe {
        LocalFree(output.pb_data.cast());
    }
    Ok(bytes)
}

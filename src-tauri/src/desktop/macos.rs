use std::{
    collections::HashMap,
    path::PathBuf,
    process::Command,
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

use once_cell::sync::Lazy;

use super::{app_bundle_in, launch_failed, not_installed, quit_failed, quit_timeout, terminate_failed, DesktopApp};
use crate::error::{AppError, Result};

const PROC_ALL_PIDS: u32 = 1;
const KILL_WAIT: Duration = Duration::from_secs(5);
const QUIT_WAIT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(100);

static INSTALL_CACHE: Lazy<Mutex<HashMap<DesktopApp, PathBuf>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

unsafe extern "C" {
    fn proc_listpids(
        type_: u32,
        typeinfo: u32,
        buffer: *mut libc::c_void,
        buffersize: libc::c_int,
    ) -> libc::c_int;
    fn proc_name(pid: libc::c_int, buffer: *mut libc::c_void, buffersize: u32) -> libc::c_int;
}

pub(super) fn resolve(app: DesktopApp) -> Result<PathBuf> {
    if let Ok(cache) = INSTALL_CACHE.lock() {
        if let Some(path) = cache.get(&app) {
            if path.is_dir() {
                return Ok(path.clone());
            }
        }
    }
    let found = discover(app).ok_or_else(|| not_installed(app))?;
    if let Ok(mut cache) = INSTALL_CACHE.lock() {
        cache.insert(app, found.clone());
    }
    Ok(found)
}

fn discover(app: DesktopApp) -> Option<PathBuf> {
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    if let Some(path) = app_bundle_in(app.name(), &roots) {
        return Some(path);
    }
    ls_application_path(app)
}

fn ls_application_path(app: DesktopApp) -> Option<PathBuf> {
    let script = match app.macos_bundle_id() {
        Some(id) => format!("POSIX path of (path to application id \"{id}\")"),
        None => format!("POSIX path of (path to application \"{}\")", app.name()),
    };
    let output = Command::new("osascript").args(["-e", &script]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    path.is_dir().then_some(path)
}

pub(super) fn launch(app: DesktopApp) -> Result<()> {
    match Command::new("open").args(["-a", app.name()]).status() {
        Ok(status) if status.success() => Ok(()),
        _ => Err(launch_failed(app)),
    }
}

pub(super) fn is_running(app: DesktopApp) -> bool {
    process_named(app.name())
}

fn process_named(name: &str) -> bool {
    let needed = unsafe { proc_listpids(PROC_ALL_PIDS, 0, std::ptr::null_mut(), 0) };
    if needed <= 0 {
        return false;
    }
    let mut pids = vec![0i32; (needed as usize / std::mem::size_of::<i32>()) + 32];
    let filled = unsafe {
        proc_listpids(
            PROC_ALL_PIDS,
            0,
            pids.as_mut_ptr().cast(),
            (pids.len() * std::mem::size_of::<i32>()) as i32,
        )
    };
    if filled <= 0 {
        return false;
    }
    let count = (filled as usize) / std::mem::size_of::<i32>();
    let mut buf = [0u8; 256];
    for pid in pids.iter().take(count).copied().filter(|pid| *pid > 0) {
        let len = unsafe { proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
        if len <= 0 {
            continue;
        }
        if std::str::from_utf8(&buf[..len as usize]).is_ok_and(|proc| proc == name) {
            return true;
        }
    }
    false
}

pub(super) fn terminate(app: DesktopApp) -> Result<()> {
    if !is_running(app) {
        return Ok(());
    }
    match Command::new("pkill").args(["-x", app.name()]).status() {
        Ok(status) if status.success() => Ok(()),
        _ => Err(terminate_failed(app)),
    }
}

pub(super) fn wait_until_stopped(app: DesktopApp) -> Result<()> {
    wait_while_running(app, KILL_WAIT, || {
        AppError::Message(format!("{} 未在 5 秒内退出。", app.name()))
    })
}

pub(super) fn quit_and_wait(app: DesktopApp) -> Result<()> {
    if !is_running(app) {
        return Ok(());
    }
    let script = format!("tell application \"{}\" to quit", app.name());
    let status = Command::new("osascript")
        .args(["-e", &script])
        .status()
        .map_err(|_| quit_failed(app))?;
    if !status.success() {
        return Err(quit_failed(app));
    }
    wait_while_running(app, QUIT_WAIT, || quit_timeout(app))
}

fn wait_while_running(
    app: DesktopApp,
    limit: Duration,
    error: impl FnOnce() -> AppError,
) -> Result<()> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if !is_running(app) {
            return Ok(());
        }
        thread::sleep(POLL);
    }
    Err(error())
}

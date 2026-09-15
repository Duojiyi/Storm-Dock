use std::{
    collections::HashMap,
    fs,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

use once_cell::sync::Lazy;

use super::{
    launch_failed, not_installed, parse_lnk_target, quit_failed, quit_timeout, terminate_failed,
    DesktopApp,
};
use crate::error::{AppError, Result};

const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
const MAX_PATH: usize = 260;
const INVALID_HANDLE: isize = -1;
const PROCESS_TERMINATE: u32 = 0x0001;
const PROCESS_SYNCHRONIZE: u32 = 0x0010_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
const WM_CLOSE: u32 = 0x0010;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x0000_0102;
const KILL_WAIT: Duration = Duration::from_secs(5);
const QUIT_WAIT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(20);

static INSTALL_CACHE: Lazy<Mutex<HashMap<DesktopApp, PathBuf>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[repr(C)]
struct ProcessEntry32W {
    dw_size: u32,
    cnt_usage: u32,
    th32_process_id: u32,
    th32_default_heap_id: usize,
    th32_module_id: u32,
    cnt_threads: u32,
    th32_parent_process_id: u32,
    pc_pri_class_base: i32,
    dw_flags: u32,
    sz_exe_file: [u16; MAX_PATH],
}

#[repr(C)]
struct StartupInfoW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    dw_x: u32,
    dw_y: u32,
    dw_x_size: u32,
    dw_y_size: u32,
    dw_x_count_chars: u32,
    dw_y_count_chars: u32,
    dw_fill_attribute: u32,
    dw_flags: u32,
    w_show_window: u16,
    cb_reserved2: u16,
    lp_reserved2: *mut u8,
    h_std_input: *mut core::ffi::c_void,
    h_std_output: *mut core::ffi::c_void,
    h_std_error: *mut core::ffi::c_void,
}

#[repr(C)]
struct ProcessInformation {
    h_process: *mut core::ffi::c_void,
    h_thread: *mut core::ffi::c_void,
    dw_process_id: u32,
    dw_thread_id: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> *mut core::ffi::c_void;
    fn Process32FirstW(snapshot: *mut core::ffi::c_void, entry: *mut ProcessEntry32W) -> i32;
    fn Process32NextW(snapshot: *mut core::ffi::c_void, entry: *mut ProcessEntry32W) -> i32;
    fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> *mut core::ffi::c_void;
    fn TerminateProcess(process: *mut core::ffi::c_void, exit_code: u32) -> i32;
    fn WaitForSingleObject(handle: *mut core::ffi::c_void, milliseconds: u32) -> u32;
    fn CloseHandle(handle: *mut core::ffi::c_void) -> i32;
    fn CreateProcessW(
        application_name: *const u16,
        command_line: *mut u16,
        process_attributes: *mut core::ffi::c_void,
        thread_attributes: *mut core::ffi::c_void,
        inherit_handles: i32,
        creation_flags: u32,
        environment: *mut core::ffi::c_void,
        current_directory: *const u16,
        startup_info: *mut StartupInfoW,
        process_information: *mut ProcessInformation,
    ) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(
        callback: unsafe extern "system" fn(*mut core::ffi::c_void, isize) -> i32,
        lparam: isize,
    ) -> i32;
    fn GetWindowThreadProcessId(hwnd: *mut core::ffi::c_void, process_id: *mut u32) -> u32;
    fn PostMessageW(hwnd: *mut core::ffi::c_void, msg: u32, wparam: usize, lparam: isize) -> i32;
}

pub(super) fn resolve(app: DesktopApp) -> Result<PathBuf> {
    if let Ok(cache) = INSTALL_CACHE.lock() {
        if let Some(path) = cache.get(&app) {
            if path.is_file() {
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
    if let Some(path) = candidate_exes(app).into_iter().find(|path| path.is_file()) {
        return Some(path);
    }
    shortcut_paths(app).into_iter().find_map(|shortcut| {
        parse_lnk_target(&fs::read(shortcut).ok()?).filter(|path| path.is_file())
    })
}

fn candidate_exes(app: DesktopApp) -> Vec<PathBuf> {
    let exe = app.windows_exe();
    let mut paths = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        for dir in app.windows_dir_names() {
            paths.push(local.join("Programs").join(dir).join(exe));
            paths.push(local.join(dir).join(exe));
        }
    }
    for key in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(key).map(PathBuf::from) {
            for dir in app.windows_dir_names() {
                paths.push(root.join(dir).join(exe));
            }
        }
    }
    paths
}

fn shortcut_paths(app: DesktopApp) -> Vec<PathBuf> {
    let shortcut = app.windows_shortcut();
    let mut paths = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        paths.push(
            PathBuf::from(appdata)
                .join(r"Microsoft\Windows\Start Menu\Programs")
                .join(shortcut),
        );
    }
    if let Some(program_data) = std::env::var_os("ProgramData") {
        paths.push(
            PathBuf::from(program_data)
                .join(r"Microsoft\Windows\Start Menu\Programs")
                .join(shortcut),
        );
    }
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join("Desktop").join(shortcut));
    }
    if let Some(public) = std::env::var_os("PUBLIC") {
        paths.push(PathBuf::from(public).join("Desktop").join(shortcut));
    }
    paths
}

pub(super) fn launch(app: DesktopApp) -> Result<()> {
    let exe = resolve(app)?;
    let flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB;
    if create_detached(&exe, flags).is_ok() {
        return Ok(());
    }
    create_detached(&exe, DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .map_err(|_| launch_failed(app))
}

fn create_detached(exe: &Path, flags: u32) -> Result<()> {
    let app_name = wide_os(exe);
    let mut command = wide_quoted(exe);
    let directory = exe
        .parent()
        .map(wide_os)
        .unwrap_or_else(|| wide_os(Path::new(".")));
    let mut startup = unsafe { std::mem::zeroed::<StartupInfoW>() };
    startup.cb = std::mem::size_of::<StartupInfoW>() as u32;
    let mut info = ProcessInformation {
        h_process: std::ptr::null_mut(),
        h_thread: std::ptr::null_mut(),
        dw_process_id: 0,
        dw_thread_id: 0,
    };
    let ok = unsafe {
        CreateProcessW(
            app_name.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            flags,
            std::ptr::null_mut(),
            directory.as_ptr(),
            &mut startup,
            &mut info,
        )
    };
    if ok == 0 {
        return Err(AppError::Message("无法启动桌面应用。".into()));
    }
    unsafe {
        if !info.h_thread.is_null() {
            CloseHandle(info.h_thread);
        }
        if !info.h_process.is_null() {
            CloseHandle(info.h_process);
        }
    }
    Ok(())
}

fn wide_os(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn wide_quoted(path: &Path) -> Vec<u16> {
    let mut out: Vec<u16> = "\"".encode_utf16().collect();
    out.extend(path.as_os_str().encode_wide());
    out.extend("\"\0".encode_utf16());
    out
}

fn process_name_matches(wide: &[u16], expected: &str) -> bool {
    let end = wide
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end]).eq_ignore_ascii_case(expected)
}

fn process_ids(app: DesktopApp) -> Vec<u32> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot.is_null() || snapshot as isize == INVALID_HANDLE {
        return Vec::new();
    }
    let mut entry = ProcessEntry32W {
        dw_size: std::mem::size_of::<ProcessEntry32W>() as u32,
        cnt_usage: 0,
        th32_process_id: 0,
        th32_default_heap_id: 0,
        th32_module_id: 0,
        cnt_threads: 0,
        th32_parent_process_id: 0,
        pc_pri_class_base: 0,
        dw_flags: 0,
        sz_exe_file: [0; MAX_PATH],
    };
    let mut ids = Vec::new();
    let exe = app.windows_exe();
    unsafe {
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                if process_name_matches(&entry.sz_exe_file, exe) {
                    ids.push(entry.th32_process_id);
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }
    ids
}

pub(super) fn is_running(app: DesktopApp) -> bool {
    !process_ids(app).is_empty()
}

pub(super) fn terminate(app: DesktopApp) -> Result<()> {
    let ids = process_ids(app);
    if ids.is_empty() {
        return Ok(());
    }
    let mut killed_any = false;
    for pid in ids {
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
        if handle.is_null() {
            continue;
        }
        let ok = unsafe { TerminateProcess(handle, 1) };
        unsafe {
            CloseHandle(handle);
        }
        if ok != 0 {
            killed_any = true;
        }
    }
    if killed_any {
        Ok(())
    } else {
        Err(terminate_failed(app))
    }
}

pub(super) fn wait_until_stopped(app: DesktopApp) -> Result<()> {
    wait_while_running(app, KILL_WAIT, || {
        AppError::Message(format!("{} 未在 5 秒内退出。", app.name()))
    })
}

struct CloseTargets {
    pids: Vec<u32>,
    hwnds: Vec<*mut core::ffi::c_void>,
}

unsafe extern "system" fn enum_windows_proc(hwnd: *mut core::ffi::c_void, lparam: isize) -> i32 {
    let targets = unsafe { &mut *(lparam as *mut CloseTargets) };
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    if targets.pids.contains(&pid) {
        targets.hwnds.push(hwnd);
    }
    1
}

pub(super) fn quit_and_wait(app: DesktopApp) -> Result<()> {
    let pids = process_ids(app);
    if pids.is_empty() {
        return Ok(());
    }
    let mut handles = Vec::new();
    for pid in &pids {
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, *pid) };
        if !handle.is_null() && handle as isize != INVALID_HANDLE {
            handles.push(handle);
        }
    }
    let mut targets = CloseTargets {
        pids,
        hwnds: Vec::new(),
    };
    unsafe {
        EnumWindows(
            enum_windows_proc,
            &mut targets as *mut CloseTargets as isize,
        );
        for hwnd in targets.hwnds {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }
    }
    if handles.is_empty() {
        return if is_running(app) {
            Err(quit_failed(app))
        } else {
            Ok(())
        };
    }
    let deadline = Instant::now() + QUIT_WAIT;
    let mut remaining = handles;
    while !remaining.is_empty() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let wait_ms = u32::try_from(left.as_millis()).unwrap_or(u32::MAX);
        let handle = remaining[0];
        let status = unsafe { WaitForSingleObject(handle, wait_ms) };
        match status {
            WAIT_OBJECT_0 => {
                unsafe {
                    CloseHandle(handle);
                }
                remaining.remove(0);
            }
            WAIT_TIMEOUT => break,
            _ => {
                unsafe {
                    CloseHandle(handle);
                }
                remaining.remove(0);
            }
        }
    }
    for handle in remaining {
        unsafe {
            CloseHandle(handle);
        }
    }
    if Instant::now() < deadline {
        wait_while_running(
            app,
            deadline.saturating_duration_since(Instant::now()),
            || quit_timeout(app),
        )
        .ok();
    }
    if is_running(app) {
        Err(quit_timeout(app))
    } else {
        Ok(())
    }
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

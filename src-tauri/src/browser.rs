use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, Result};

pub(crate) const DEFAULT_BROWSER_ID: &str = "default";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct LoginBrowser {
    pub(crate) id: String,
    pub(crate) name: String,
}


struct Spec {
    id: &'static str,
    name: &'static str,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    macos_app: &'static str,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    windows_suffixes: &'static [&'static str],
}

const CATALOG: &[Spec] = &[
    Spec {
        id: "safari",
        name: "Safari",
        macos_app: "Safari",
        windows_suffixes: &[],
    },
    Spec {
        id: "chrome",
        name: "Google Chrome",
        macos_app: "Google Chrome",
        windows_suffixes: &[
            "Google/Chrome/Application/chrome.exe",
            "Google/Chrome/chrome.exe",
        ],
    },
    Spec {
        id: "edge",
        name: "Microsoft Edge",
        macos_app: "Microsoft Edge",
        windows_suffixes: &["Microsoft/Edge/Application/msedge.exe"],
    },
    Spec {
        id: "firefox",
        name: "Firefox",
        macos_app: "Firefox",
        windows_suffixes: &["Mozilla Firefox/firefox.exe", "Firefox/firefox.exe"],
    },
    Spec {
        id: "brave",
        name: "Brave",
        macos_app: "Brave Browser",
        windows_suffixes: &[
            "BraveSoftware/Brave-Browser/Application/brave.exe",
            "BraveSoftware/Brave-Browser/brave.exe",
        ],
    },
    Spec {
        id: "arc",
        name: "Arc",
        macos_app: "Arc",
        windows_suffixes: &["Arc/Arc.exe", "Programs/Arc/Arc.exe"],
    },
];

pub(crate) fn normalize_id(id: Option<&str>) -> String {
    match id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => value.to_string(),
        None => DEFAULT_BROWSER_ID.into(),
    }
}

pub(crate) fn list() -> Vec<LoginBrowser> {
    list_with(|path| path.exists())
}

pub(crate) fn open(url: &str, browser_id: &str) -> Result<()> {
    match resolve_with(browser_id, |path| path.exists())? {
        Target::Default => open_default(url),
        #[cfg(target_os = "macos")]
        Target::MacApp(name) => run(std::process::Command::new("open")
            .args(["-a", name, url])
            .status()),
        #[cfg(target_os = "windows")]
        Target::WindowsExe(path) => run(std::process::Command::new(path).arg(url).status()),
    }
}


fn list_with(exists: impl Fn(&Path) -> bool) -> Vec<LoginBrowser> {
    let mut browsers = vec![LoginBrowser {
        id: DEFAULT_BROWSER_ID.into(),
        name: "System default".into(),
    }];
    for spec in CATALOG {
        if spec.candidates().into_iter().any(|path| exists(&path)) {
            browsers.push(LoginBrowser {
                id: spec.id.into(),
                name: spec.name.into(),
            });
        }
    }
    browsers
}

enum Target {
    Default,
    #[cfg(target_os = "macos")]
    MacApp(&'static str),
    #[cfg(target_os = "windows")]
    WindowsExe(PathBuf),
}

fn resolve_with(browser_id: &str, exists: impl Fn(&Path) -> bool) -> Result<Target> {
    let id = normalize_id(Some(browser_id));
    if id == DEFAULT_BROWSER_ID {
        return Ok(Target::Default);
    }
    let spec = CATALOG
        .iter()
        .find(|spec| spec.id == id)
        .ok_or_else(|| AppError::Message("未知的浏览器。".into()))?;
    let path = spec
        .candidates()
        .into_iter()
        .find(|path| exists(path))
        .ok_or_else(|| AppError::Message(format!("未安装 {}。", spec.name)))?;
    #[cfg(target_os = "macos")]
    {
        let _ = path;
        return Ok(Target::MacApp(spec.macos_app));
    }
    #[cfg(target_os = "windows")]
    return Ok(Target::WindowsExe(path));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = path;
        Err(AppError::Message(
            "无法打开浏览器，请手动打开登录链接。".into(),
        ))
    }
}

impl Spec {
    fn candidates(&self) -> Vec<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            macos_roots()
                .into_iter()
                .map(|root| root.join(format!("{}.app", self.macos_app)))
                .collect()
        }
        #[cfg(target_os = "windows")]
        {
            if self.windows_suffixes.is_empty() {
                return Vec::new();
            }
            windows_roots()
                .into_iter()
                .flat_map(|root| {
                    self.windows_suffixes
                        .iter()
                        .map(move |suffix| root.join(suffix))
                })
                .collect()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
fn macos_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Cryptexes/App/System/Applications"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        roots.insert(1, PathBuf::from(home).join("Applications"));
    }
    roots
}

#[cfg(target_os = "windows")]
fn windows_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for key in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
        if let Some(value) = std::env::var_os(key) {
            roots.push(PathBuf::from(value));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("Programs"));
    }
    roots
}

fn open_default(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(url).status();
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .status();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let status: std::io::Result<std::process::ExitStatus> =
        Err(std::io::Error::other("unsupported OS"));
    run(status)
}

fn run(status: std::io::Result<std::process::ExitStatus>) -> Result<()> {
    match status {
        Ok(status) if status.success() => Ok(()),
        _ => Err(AppError::Message(
            "无法打开浏览器，请手动打开登录链接。".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_candidate(id: &str) -> PathBuf {
        CATALOG
            .iter()
            .find(|spec| spec.id == id)
            .expect("catalog id")
            .candidates()
            .into_iter()
            .next()
            .expect("candidate path")
    }

    #[test]
    fn default_is_first_when_nothing_is_installed() {
        let browsers = list_with(|_| false);
        assert_eq!(browsers[0].id, DEFAULT_BROWSER_ID);
        assert_eq!(browsers.len(), 1);
    }

    #[test]
    fn lists_only_browsers_that_exist() {
        let chrome = first_candidate("chrome");
        let ids: Vec<_> = list_with(|path| path == chrome)
            .into_iter()
            .map(|browser| browser.id)
            .collect();
        assert_eq!(ids, ["default", "chrome"]);
    }

    #[test]
    fn unknown_browser_id_fails_before_launch() {
        let error = open("https://example.invalid", "not-a-browser").unwrap_err();
        assert!(error.to_string().contains("未知"));
    }

    #[test]
    fn empty_id_resolves_to_default() {
        assert!(matches!(
            resolve_with("  ", |_| false).unwrap(),
            Target::Default
        ));
        assert_eq!(normalize_id(None), DEFAULT_BROWSER_ID);
        assert_eq!(normalize_id(Some(" chrome ")), "chrome");
    }
}

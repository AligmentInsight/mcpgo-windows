//! Windows path resolution shared by every McpGo module.
//!
//! Extracted from `main.rs` in Phase 3 so that both the environment checks
//! (`main.rs`) and the terminal engine (`terminal/`, `jobs/`) resolve
//! PowerShell, Chrome and PATH executables through exactly one
//! implementation.

use std::env;
use std::path::{Path, PathBuf};

/// One of the two supported PowerShell hosts.
#[derive(Debug, Clone)]
pub struct ShellKind {
    /// Stable machine-readable name recorded in job_status files
    /// ("pwsh-7" or "powershell-5.1").
    pub name: &'static str,
    /// Absolute path of the executable.
    pub exe: PathBuf,
}

/// Returns the value of a Windows environment variable (`LOCALAPPDATA`,
/// `ProgramFiles`, `SystemRoot`, ...) as an owned `PathBuf`, skipping empty
/// values.
pub fn env_path(var: &str) -> Option<PathBuf> {
    env::var_os(var)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// Checks whether a regular file exists at `path`.
pub fn file_exists(path: &Path) -> bool {
    path.is_file()
}

/// Checks whether a directory exists at `path`.
pub fn dir_exists(path: &Path) -> bool {
    path.is_dir()
}

/// Standard location of Windows PowerShell 5.1, present on every Windows 10+
/// installation: `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`.
/// Falls back to the canonical `C:\Windows` prefix when `SystemRoot` is not
/// set (a broken profile, not a normal scenario).
pub fn powershell51_path() -> PathBuf {
    let system_root = env_path("SystemRoot").unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    system_root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
}

/// Standard locations of PowerShell 7+ (64-bit and 32-bit Program Files),
/// plus a `PATH` lookup as a fallback (winget/scoop installs). PowerShell 7
/// is optional: the terminal engine prefers it when available and silently
/// falls back to Windows PowerShell 5.1 otherwise.
pub fn powershell7_path() -> Option<PathBuf> {
    let candidates: [Option<PathBuf>; 4] = [
        env_path("ProgramFiles").map(|p| p.join(r"PowerShell\7\pwsh.exe")),
        env_path("ProgramFiles(x86)").map(|p| p.join(r"PowerShell\7\pwsh.exe")),
        Some(PathBuf::from(r"C:\Program Files\PowerShell\7\pwsh.exe")),
        find_on_path("pwsh"),
    ];
    candidates.into_iter().flatten().find(|p| file_exists(p))
}

/// Standard locations of the Google Chrome executable (machine-wide installs
/// in Program Files, user-level install in LocalAppData), plus a `PATH`
/// lookup as a fallback.
pub fn chrome_executable_path() -> Option<PathBuf> {
    let candidates: [Option<PathBuf>; 4] = [
        env_path("ProgramFiles").map(|p| p.join(r"Google\Chrome\Application\chrome.exe")),
        env_path("ProgramFiles(x86)").map(|p| p.join(r"Google\Chrome\Application\chrome.exe")),
        env_path("LOCALAPPDATA").map(|p| p.join(r"Google\Chrome\Application\chrome.exe")),
        find_on_path("chrome"),
    ];
    candidates.into_iter().flatten().find(|p| file_exists(p))
}

/// Google Chrome user-data directory used by the Playwright/CDP integration:
/// `%LocalAppData%\Google\Chrome\User Data`. When this directory exists the
/// user has a real Chrome profile that McpGo can attach to.
pub fn chrome_user_data_path() -> Option<PathBuf> {
    let local = env_path("LOCALAPPDATA")?;
    let profile = local.join(r"Google\Chrome\User Data");
    dir_exists(&profile).then_some(profile)
}

/// Node.js runtime for the background service worker: resolved from `PATH`
/// (`node.exe`). Availability is informational (the environment report and
/// the services view show it); the service worker re-resolves the runtime
/// when a service actually starts.
pub fn node_path() -> Option<PathBuf> {
    find_on_path("node")
}

/// Python runtime: resolved from `PATH`, skipping the Microsoft Store
/// "execution alias" directory WindowsApps - the python.exe stub there is
/// usually a zero-byte launcher that only opens the Store and must not be
/// treated as an installed runtime.
pub fn python_path() -> Option<PathBuf> {
    find_on_path("python")
        .filter(|path| !path.to_string_lossy().to_lowercase().contains("windowsapps"))
}

/// Looks up an executable on `PATH`, honouring the Windows `PATHEXT` order
/// (`.COM;.EXE;.BAT;.CMD;...`). Returns the first existing match. The bare
/// name is tried first, which also covers callers that pass an explicit
/// extension such as `node.exe`.
pub fn find_on_path(program: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    let pathext = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());

    for dir in env::split_paths(&path_var) {
        let bare = dir.join(program);
        if file_exists(&bare) {
            return Some(bare);
        }
        for ext in pathext.split(';') {
            let ext = ext.trim();
            if ext.is_empty() {
                continue;
            }
            let candidate = dir.join(format!("{program}{ext}"));
            if file_exists(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Prefers PowerShell 7 (`pwsh.exe`) when installed, otherwise falls back to
/// Windows PowerShell 5.1. Returns `None` only on a system without either
/// (not a supported configuration; the environment report flags it).
pub fn preferred_shell() -> Option<ShellKind> {
    if let Some(exe) = powershell7_path() {
        return Some(ShellKind { name: "pwsh-7", exe });
    }
    let exe = powershell51_path();
    file_exists(&exe).then_some(ShellKind { name: "powershell-5.1", exe })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_kind_is_resolvable_on_supported_systems() {
        // On every supported Windows installation PowerShell 5.1 exists,
        // so preferred_shell() must return something.
        let shell = preferred_shell();
        assert!(shell.is_some(), "no PowerShell host found on this machine");
        let shell = shell.expect("checked above");
        assert!(file_exists(&shell.exe));
        assert!(shell.name == "pwsh-7" || shell.name == "powershell-5.1");
    }
}

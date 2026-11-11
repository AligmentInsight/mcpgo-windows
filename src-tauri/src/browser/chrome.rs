//! Chrome launch mechanics for the DevTools integration.
//!
//! The macOS original launches Chrome through the Playwright daemon with
//! the user's real profile (`~/Library/Application Support/Google/Chrome`).
//! This port launches `chrome.exe` directly - same flags Playwright would
//! pass, minus the Node.js daemon in the middle:
//!
//! | macOS original                        | Here                              |
//! |---------------------------------------|-----------------------------------|
//! | Playwright daemon builds the argv    | [`build_launch_args`]             |
//! | daemon spawns the browser            | [`launch`]                        |
//! | fixed CDP port per daemon config     | [`allocate_debug_port`]           |
//! | daemon owns the user profile         | managed profile dirs under        |
//! |                                       | `<data>\browser\profiles\<id>`    |
//!
//! ## Why managed profile directories (the Chrome 136 security change)
//!
//! Since Chrome 136 the `--remote-debugging-port` switch is silently
//! ignored when Chrome runs with its DEFAULT user data directory
//! (`%LocalAppData%\Google\Chrome\User Data`) - Google closed the cookie
//! theft vector behind the switch. A debug-enabled McpGo session therefore
//! ALWAYS runs with an explicit, non-default `--user-data-dir`; requests
//! for the default directory are rejected by `browser::mod` with a message
//! explaining the restriction, instead of launching a Chrome that silently
//! has no DevTools endpoint. Chromes that were already started debuggable
//! (by hand or by an earlier McpGo run) remain attachable through the
//! endpoint scan in `cdp::scan_debug_endpoints`.
//!
//! Managed profile directories are created per session and intentionally
//! never deleted automatically: Chrome writes caches and user downloads
//! into them, and silently removing user data is worse than leaving it
//! behind. The profiles directory is shown in the UI so it can be cleaned
//! by hand.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use super::cdp;

/// Windows process creation flag CREATE_NO_WINDOW (0x0800_0000): headless
/// launches must not flash a console window behind the GUI. Chrome's own
/// window is unaffected by this flag.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Chrome flag enabling the loopback DevTools endpoint; the value is the
/// port allocated by [`allocate_debug_port`].
pub const FLAG_DEBUG_PORT: &str = "--remote-debugging-port";
/// Chrome flag selecting the (non-default) user data directory.
pub const FLAG_USER_DATA_DIR: &str = "--user-data-dir";
/// Modern headless mode (the only headless mode in current Chrome builds;
/// accepted as an alias even though it is the default there).
pub const FLAG_HEADLESS: &str = "--headless=new";
/// Skip the first-run / import dialogues a fresh managed profile would show.
pub const FLAG_NO_FIRST_RUN: &str = "--no-first-run";
/// Skip the "make Chrome your default browser" prompt.
pub const FLAG_NO_DEFAULT_BROWSER_CHECK: &str = "--no-default-browser-check";

/// Argument prefixes McpGo must own exclusively: the debugging flags define
/// the session contract (which port, which profile), so user-supplied
/// duplicates are dropped by [`sanitize_extra_args`] rather than risking a
/// contradictory argv.
const RESERVED_PREFIXES: [&str; 3] = [
    FLAG_DEBUG_PORT,
    "--remote-debugging-pipe",
    FLAG_USER_DATA_DIR,
];

/// Builds the full Chrome argv of one session. Argument order is fixed and
/// part of the launch contract: McpGo's flags first, sanitized user extras
/// in the middle, the start URL last (Chrome treats a trailing URL as the
/// thing to open; flags after it may be interpreted as further URLs).
pub fn build_launch_args(
    port: u16,
    user_data_dir: &Path,
    headless: bool,
    start_url: Option<&str>,
    extra_args: &[String],
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        format!("{FLAG_DEBUG_PORT}={port}"),
        format!("{FLAG_USER_DATA_DIR}={}", user_data_dir.display()),
        FLAG_NO_FIRST_RUN.into(),
        FLAG_NO_DEFAULT_BROWSER_CHECK.into(),
    ];
    if headless {
        args.push(FLAG_HEADLESS.into());
    }
    args.extend(sanitize_extra_args(extra_args));
    if let Some(url) = start_url {
        args.push(url.to_owned());
    }
    args
}

/// Strips whitespace, drops empty tokens and drops every token whose
/// prefix is reserved for McpGo ([`RESERVED_PREFIXES`]). Dropped reserved
/// flags are logged - the launch log record is the natural place to audit
/// what the effective argv turned out to be.
pub fn sanitize_extra_args(args: &[String]) -> Vec<String> {
    args.iter()
        .map(|arg| arg.trim().to_owned())
        .filter(|arg| {
            if arg.is_empty() {
                return false;
            }
            let reserved = RESERVED_PREFIXES.iter().any(|prefix| arg.starts_with(prefix));
            if reserved {
                log::debug!("[browser] dropped reserved argument: {arg}");
            }
            !reserved
        })
        .collect()
}

/// Spawns `chrome.exe` with `args`. All three standard streams are null:
/// the DevTools endpoint (not the console) is McpGo's observability channel
/// for this process.
pub fn launch(exe: &Path, args: &[String]) -> std::io::Result<Child> {
    let mut builder = Command::new(exe);
    builder
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        builder.creation_flags(CREATE_NO_WINDOW);
    }

    builder.spawn()
}

/// Finds a free loopback debugging port. Candidates are probed in the
/// McpGo range ([`cdp::PORT_RANGE_START`] ..= [`cdp::PORT_RANGE_END`]):
///
/// 1. a port already serving a DevTools endpoint is skipped - two Chromes
///    can never share one endpoint;
/// 2. a port another process listens on is skipped (the bind probe);
/// 3. the first free candidate wins.
///
/// The released-bind race between this probe and Chrome's own bind is
/// closed by the session supervisor: a stolen port means the DevTools
/// endpoint never answers, the launch times out and the session records a
/// Failed state with a precise error message.
pub fn allocate_debug_port() -> Option<u16> {
    for port in cdp::PORT_RANGE_START..=cdp::PORT_RANGE_END {
        if cdp::endpoint_alive(port) {
            continue;
        }
        if TcpListener::bind((cdp::HOST, port)).is_ok() {
            return Some(port);
        }
    }
    None
}

/// `true` when `dir` is the machine's default Chrome user data directory
/// (case-insensitive comparison - Windows paths are case-insensitive and the
/// user may type any casing into the launch form). See the module docs for
/// why launching a debug session against this directory is rejected.
pub fn is_default_chrome_profile(dir: &Path) -> bool {
    match crate::paths::chrome_user_data_path() {
        Some(default_dir) => {
            let normalize = |path: &Path| path.to_string_lossy().to_lowercase().replace('/', "\\");
            normalize(dir) == normalize(&default_dir)
        }
        None => false,
    }
}

/// Managed profile directory of one session: `<profiles_dir>\<session_id>`.
/// Created by the caller before the launch so the spawn can never fail on
/// a missing directory.
pub fn managed_profile_dir(profiles_dir: &Path, session_id: &str) -> PathBuf {
    profiles_dir.join(session_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_flag_contract_in_order() {
        let args = build_launch_args(
            9222,
            Path::new(r"C:\Data\profiles\br_1"),
            true,
            Some("https://example.com/"),
            &["--incognito".into(), "--window-size=1280,800".into()],
        );
        assert_eq!(args[0], "--remote-debugging-port=9222");
        assert_eq!(
            args[1],
            format!("--user-data-dir=C:\\Data\\profiles\\br_1")
        );
        assert!(args.contains(&"--no-first-run".to_owned()));
        assert!(args.contains(&"--no-default-browser-check".to_owned()));
        assert!(args.contains(&"--headless=new".to_owned()));
        // The start URL is always the LAST argument.
        assert_eq!(args.last().map(String::as_str), Some("https://example.com/"));
        // User extras survive (in the middle, before the URL).
        assert!(args.contains(&"--incognito".into()));
    }

    #[test]
    fn headless_flag_follows_user_data_dir() {
        let args = build_launch_args(
            9300,
            Path::new(r"C:\Data\profiles\br_2"),
            false,
            None,
            &[],
        );
        assert!(!args.contains(&"--headless=new".to_owned()));
        assert!(args.iter().any(|arg| arg.starts_with("--user-data-dir=")));
    }

    #[test]
    fn sanitize_drops_reserved_flags_and_empty_tokens() {
        let raw = vec![
            "  --incognito  ".to_owned(),
            "--remote-debugging-port=6666".to_owned(),
            "--user-data-dir=C:\\SomewhereElse".to_owned(),
            "--remote-debugging-pipe".to_owned(),
            String::new(),
            "   ".to_owned(),
            "--window-position=10,10".to_owned(),
        ];
        let clean = sanitize_extra_args(&raw);
        assert_eq!(clean, vec!["--incognito", "--window-position=10,10"]);
    }

    #[test]
    fn managed_profile_dir_is_named_after_the_session() {
        let dir = managed_profile_dir(Path::new(r"C:\Data\profiles"), "br_1731234567890_3");
        assert_eq!(
            dir,
            PathBuf::from(r"C:\Data\profiles\br_1731234567890_3")
        );
    }
}

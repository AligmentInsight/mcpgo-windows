//! tunnel-client launch mechanics.
//!
//! The macOS roadmap originally planned a Node.js `tunnel-agent` service.
//! This port instead supervises the OFFICIAL OpenAI `tunnel-client`
//! executable - the same reasoning as the browser domain, which launches
//! `chrome.exe` instead of porting Playwright: McpGo owns profiles,
//! lifecycle and health monitoring, while the protocol itself (the
//! control-plane poll/response queue over `https://api.openai.com`) stays
//! implemented by its vendor.
//!
//! | macOS plan                       | Here                              |
//! |----------------------------------|-----------------------------------|
//! | `scripts/tunnel-agent/` daemon   | one `tunnel-client.exe` per       |
//! | (Node.js)                        | started tunnel ([`launch`])       |
//! | daemon config                    | generated profile                 |
//! |                                  | (`profile.rs`, rewritten on every |
//! |                                  | start)                            |
//! | (daemon internals)               | health probing (`health.rs`)      |
//!
//! ## Executable resolution ([`resolve`])
//!
//! 1. the definition's explicit `client_path` (absolute path);
//! 2. `PATH` lookup (`tunnel-client` / `tunnel-client.exe`);
//! 3. the managed drop-in location `<tunnels>\bin\tunnel-client.exe` -
//!    created by the store, shown in the view header so the user knows
//!    where to unzip a downloaded release.
//!
//! The executable ships in the release archives of
//! `github.com/openai/tunnel-client` (a `bin\tunnel-client.exe` plus the
//! bundled companion); McpGo never downloads or updates it - installing a
//! runtime that carries secrets is an explicit user action.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::error::{McpGoError, McpGoResult};
use crate::paths;

/// Program name of the official client (with and without extension, for
/// the PATH lookup).
const CLIENT_PROGRAM: &str = "tunnel-client";

/// Windows process creation flag CREATE_NO_WINDOW (0x0800_0000): the
/// runtime is a background daemon and must not flash a console window
/// behind the GUI.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Subcommand of the client that runs one profile.
const RUN_SUBCOMMAND: &str = "run";

/// Flag selecting an explicit profile file (instead of the profile
/// directory lookup).
const PROFILE_FILE_FLAG: &str = "--profile-file";

/// Resolves the tunnel-client executable for one start:
/// definition override, then `PATH`, then the managed `bin\` directory.
/// The returned path is what the caller spawns - errors name the three
/// looked-up locations.
pub fn resolve(explicit: Option<&str>, bin_dir: &Path) -> McpGoResult<PathBuf> {
    if let Some(explicit) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
        let path = PathBuf::from(explicit);
        if !path.is_file() {
            return Err(McpGoError::invalid_input(format!(
                "the configured tunnel-client path does not exist: {explicit}"
            )));
        }
        return Ok(path);
    }
    if let Some(found) = paths::find_on_path(CLIENT_PROGRAM) {
        return Ok(found);
    }
    let managed = bin_dir.join(format!("{CLIENT_PROGRAM}.exe"));
    if managed.is_file() {
        return Ok(managed);
    }
    Err(McpGoError::ComponentMissing {
        component: "tunnel-client".into(),
        detail: format!(
            "not found on PATH or in {} - download a release from \
             github.com/openai/tunnel-client and extract it into that \
             directory (or set an explicit client path on the tunnel)",
            bin_dir.display()
        ),
    })
}

/// The default resolution (without a definition override), used by the
/// machine profile of the view header.
pub fn resolve_default(bin_dir: &Path) -> Option<PathBuf> {
    paths::find_on_path(CLIENT_PROGRAM)
        .filter(|path| path.is_file())
        .or_else(|| {
            let managed = bin_dir.join(format!("{CLIENT_PROGRAM}.exe"));
            managed.is_file().then_some(managed)
        })
}

/// Builds the argv of one runtime start: the `run` subcommand with an
/// explicit `--profile-file`. Order is fixed and part of the contract.
pub fn build_run_args(profile_path: &Path) -> Vec<String> {
    vec![
        RUN_SUBCOMMAND.into(),
        PROFILE_FILE_FLAG.into(),
        profile_path.to_string_lossy().into_owned(),
    ]
}

/// Spawns the tunnel-client runtime. All three standard streams are null:
/// the runtime writes its ndjson log into the file configured by the
/// profile (`log.file`), which is the observability channel McpGo reads
/// back. The working directory is the tunnels data directory - the GUI
/// process starts in the install directory (e.g. Program Files), which is
/// not writable and must not receive stray relative-path output.
pub fn launch(exe: &Path, args: &[String], working_dir: &Path) -> std::io::Result<Child> {
    let mut builder = Command::new(exe);
    builder
        .args(args)
        .current_dir(working_dir)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_args_lead_with_the_subcommand_and_end_with_the_profile() {
        let args = build_run_args(Path::new(
            r"C:\Data\com.mcpgo.windows\tunnels\profiles\files.yaml",
        ));
        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "run");
        assert_eq!(args[1], "--profile-file");
        assert_eq!(
            args[2],
            r"C:\Data\com.mcpgo.windows\tunnels\profiles\files.yaml"
        );
    }

    #[test]
    fn an_explicit_but_missing_client_path_is_rejected() {
        let error = resolve(
            Some(r"C:\Nowhere\tunnel-client.exe"),
            Path::new(r"C:\Data\tunnels\bin"),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("does not exist"), "unexpected: {error}");
    }

    #[test]
    fn the_missing_client_error_names_the_managed_location() {
        // In the sandbox nothing resolves, so the ComponentMissing branch
        // fires and must name the managed bin directory.
        let bin_dir = Path::new(r"C:\Data\com.mcpgo.windows\tunnels\bin");
        let error = resolve(None, bin_dir).unwrap_err().to_string();
        assert!(error.contains("tunnel-client"), "unexpected: {error}");
        assert!(
            error.contains(bin_dir.display().to_string().as_str()),
            "managed location missing: {error}"
        );
    }
}

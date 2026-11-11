//! # McpGo for Windows - application entry point.
//!
//! Windows port of McpGo, originally a macOS application built with
//! SwiftUI/AppKit, LaunchAgents and Bash/Zsh scripting. This crate turns the
//! original architecture into a Tauri 2 desktop application:
//!
//! | Original (macOS)                 | This port (Windows)                     |
//! |----------------------------------|-----------------------------------------|
//! | SwiftUI/AppKit GUI (`native/`)   | React + TypeScript frontend (`src/`)     |
//! | LaunchAgents (`launchagents/`)   | Windows services via SCM (`services/`)   |
//! | Bash/Zsh terminal                | PowerShell 5.1/7 (`terminal/`, `jobs/`)  |
//! | Chrome management                | Playwright/CDP (`browser/`)              |
//! | (none)                           | OpenAI MCP tunnels (`tunnels/`)          |
//!
//! This file owns the application bootstrap and nothing else:
//!
//! 1. Service-worker mode dispatch: when the SCM starts this executable
//!    with `--service-host <id> --data-dir <dir>`, the process runs the
//!    headless service worker (`services::worker`) and exits - no GUI is
//!    built. See `services/worker.rs` for the full architecture.
//! 2. Windows environment detection (path checks for PowerShell, Chrome,
//!    Node.js and Python) - see [`build_environment_report`]; the path
//!    resolution itself lives in [`paths`].
//! 3. Application directory initialisation - see [`ensure_app_directories`].
//! 4. Structured file logging - see [`init_logging`].
//! 5. The [`jobs::JobRegistry`] (the job_status/job_logs tracking system
//!    ported from macOS), created here and managed as Tauri state, the
//!    [`browser::BrowserRegistry`] (the Chrome/CDP domain), the
//!    [`services::ServiceStore`] (the LaunchAgents port) and the
//!    [`tunnels::TunnelRegistry`] (the OpenAI Secure MCP Tunnel domain,
//!    supervising the official tunnel-client runtime).
//! 6. Registration of the Tauri plugins that back the permission model
//!    declared in `src-tauri/capabilities/default.json` (fs, shell, process,
//!    http) and of every IPC command (this file, `commands/*`).
//! 7. Exit cleanup: running jobs are terminated when the application exits
//!    (see [`jobs::JobRegistry::shutdown`]), so no orphan PowerShell
//!    processes survive the GUI, invisible headless browser sessions are
//!    terminated as well (see [`browser::BrowserRegistry::shutdown`]) while
//!    windowed Chromes keep running, and tunnel runtimes are terminated
//!    too (see [`tunnels::TunnelRegistry::shutdown`]) - they are invisible
//!    background daemons. Installed Windows services outlive the
//!    GUI by design - they are supervised by the SCM, not by this process.
//!
//! New domains arrive as new modules under `src-tauri/src/` plus one
//! registration line here.

// Release builds on Windows must not flash a console window behind the GUI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// McpGo-Windows targets Windows 10 (build 19041) and later only: the UI needs
// WebView2 and the service layer needs the Service Control Manager. Building
// the crate for any other operating system is a configuration error, not a
// runtime fallback scenario.
#[cfg(not(target_os = "windows"))]
compile_error!(
    "McpGo-Windows targets Windows only. Build with the MSVC toolchain (x86_64-pc-windows-msvc)."
);

mod browser;
mod commands;
mod error;
mod jobs;
mod paths;
mod services;
mod terminal;
mod time;
mod tunnels;

use std::fs;
use std::path::PathBuf;

use serde::Serialize;
use tauri::{Manager, State};
use tauri_plugin_log::{Target, TargetKind};

use crate::browser::BrowserRegistry;
use crate::jobs::JobRegistry;

/// Application identifier. MUST stay in sync with `identifier` in
/// `src-tauri/tauri.conf.json`: Tauri derives every application directory
/// (data, cache, logs) from this value on Windows.
const APP_IDENTIFIER: &str = "com.mcpgo.windows";

/// Human-readable application name used in startup log records.
const APP_NAME: &str = "McpGo-Windows";

// ----------------------------------------------------------------------------
// IPC payload types (environment report)
// ----------------------------------------------------------------------------

/// Result of a single Windows component check, serialised to the frontend
/// with camelCase field names.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentCheck {
    /// Short machine-readable id, e.g. `"powershell-5.1"`.
    pub id: &'static str,
    /// Human-readable label for the UI status panel.
    pub label: &'static str,
    /// `true` when the component was found on this machine.
    pub available: bool,
    /// Absolute path of the component, when found.
    pub path: Option<String>,
    /// Explanatory note shown in the UI (why the component matters).
    pub note: &'static str,
    /// Whether McpGo refuses to operate without this component.
    pub required: bool,
}

/// Full environment snapshot computed once at startup and cached in Tauri
/// managed state. The React frontend receives it through the
/// `environment_report` IPC command.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentReport {
    /// Windows PowerShell 5.1 - the mandatory terminal backend.
    pub powershell51: ComponentCheck,
    /// PowerShell 7+ - optional, preferred when present.
    pub powershell7: ComponentCheck,
    /// Google Chrome executable (Playwright/CDP launch target).
    pub chrome_exe: ComponentCheck,
    /// Google Chrome user-data directory (`...\User Data`).
    pub chrome_user_data: ComponentCheck,
    /// Node.js runtime for background service workers.
    pub node_js: ComponentCheck,
    /// Python runtime - optional alternative worker runtime.
    pub python: ComponentCheck,
    /// Directory for job_status files (port of the macOS job_status files).
    pub jobs_status_dir: String,
    /// Directory for job_logs files (port of the macOS job_logs files).
    pub jobs_logs_dir: String,
    /// Directory with service definitions and generated PowerShell scripts.
    pub services_dir: String,
}

// ----------------------------------------------------------------------------
// Application directories and logging
// ----------------------------------------------------------------------------

/// Fallback used only when Tauri's path resolver fails (a broken user
/// profile): mirrors the resolver's own layout, `%APPDATA%\com.mcpgo.windows`.
fn default_app_data_dir() -> PathBuf {
    paths::env_path("APPDATA")
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_IDENTIFIER)
}

/// Computes the log directory that Tauri itself would resolve on Windows:
/// `%LOCALAPPDATA%\com.mcpgo.windows\logs`. It is resolved manually because
/// the log plugin is registered on the builder, before an app handle (and
/// therefore `app.path().app_log_dir()`) exists.
fn windows_log_dir() -> PathBuf {
    paths::env_path("LOCALAPPDATA")
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join(APP_IDENTIFIER)
        .join("logs")
}

/// Creates the McpGo application directory tree under Tauri's
/// `app_data_dir()` (on Windows: `%APPDATA%\com.mcpgo.windows`).
/// The call is idempotent: existing directories are left untouched, so it is
/// safe to run on every application start.
fn ensure_app_directories(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir: PathBuf = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("cannot resolve the application data directory: {e}"))?;

    // One subdirectory per architectural domain:
    //   jobs/status - job_status files (one JSON per background job)
    //   jobs/logs   - job_logs files   (one log file per background job)
    //   services    - service definitions and generated PowerShell scripts
    //   browser     - Playwright/CDP profiles and screenshots
    //   tunnels     - OpenAI Secure MCP Tunnel definitions plus the
    //                generated profiles\, secrets\, logs\ and bin\ tree
    //                (materialised by the tunnel store)
    let subdirectories = [
        data_dir.join("jobs").join("status"),
        data_dir.join("jobs").join("logs"),
        data_dir.join("services"),
        data_dir.join("browser"),
        data_dir.join("tunnels"),
    ];

    for dir in &subdirectories {
        fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create directory {}: {e}", dir.display()))?;
        log::debug!("directory ready: {}", dir.display());
    }

    Ok(())
}

/// Configures `tauri-plugin-log` with three targets:
///
/// * `Stdout`  - console output for `npm run tauri dev`;
/// * `Webview` - forwards records to the devtools console of the webview;
/// * `Folder`  - rotating files under `%LOCALAPPDATA%\com.mcpgo.windows\logs`.
///
/// The log level can be overridden at runtime through the `RUST_LOG`
/// environment variable (e.g. `RUST_LOG=debug`); the default is `info`.
/// Files rotate after roughly 512 KB, keeping only the most recent one.
///
/// Generic over the Tauri runtime because the plugin type returned by
/// `Builder::build` is `TauriPlugin<R>`; `main` instantiates it with `Wry`.
fn init_logging<R: tauri::Runtime>(log_dir: PathBuf) -> tauri::plugin::TauriPlugin<R> {
    let level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|v| v.parse::<log::LevelFilter>().ok())
        .unwrap_or(log::LevelFilter::Info);

    // Fail fast: an unwritable log directory is a broken installation and
    // must be visible immediately, not silently ignored.
    fs::create_dir_all(&log_dir).expect("cannot create the McpGo log directory");

    tauri_plugin_log::Builder::new()
        .level(level)
        .max_file_size(512_000)
        .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne)
        .targets([
            Target::new(TargetKind::Stdout),
            Target::new(TargetKind::Webview),
            Target::new(TargetKind::Folder {
                path: log_dir,
                file_name: Some("mcpgo".into()),
            }),
        ])
        .build()
}

// ----------------------------------------------------------------------------
// Environment detection
// ----------------------------------------------------------------------------

/// Runs every Windows component check and assembles the report rendered by
/// the frontend status panel. All checks are cheap synchronous file-system
/// probes; they run exactly once per application start.
fn build_environment_report(app: &tauri::AppHandle) -> EnvironmentReport {
    let ps51 = paths::powershell51_path();
    let ps7 = paths::powershell7_path();
    let chrome_exe = paths::chrome_executable_path();
    let chrome_data = paths::chrome_user_data_path();
    // WindowsApps Store stubs are filtered inside paths::python_path.
    let node = paths::node_path();
    let python = paths::python_path();

    // Tauri's resolver is authoritative; the manual fallback mirrors it and
    // only fires on a broken user profile.
    let data_dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_resolver_error| default_app_data_dir());

    EnvironmentReport {
        powershell51: ComponentCheck {
            id: "powershell-5.1",
            label: "Windows PowerShell 5.1",
            available: paths::file_exists(&ps51),
            path: ps51.to_str().map(str::to_owned),
            note: "Terminal backend; ships with every Windows 10+ installation.",
            required: true,
        },
        powershell7: ComponentCheck {
            id: "powershell-7",
            label: "PowerShell 7+ (pwsh)",
            available: ps7.is_some(),
            path: ps7.as_deref().and_then(std::path::Path::to_str).map(str::to_owned),
            note: "Optional; preferred by the terminal engine when installed.",
            required: false,
        },
        chrome_exe: ComponentCheck {
            id: "chrome-exe",
            label: "Google Chrome (executable)",
            available: chrome_exe.is_some(),
            path: chrome_exe.as_deref().and_then(std::path::Path::to_str).map(str::to_owned),
            note: "Launch target for the Playwright/CDP integration.",
            required: false,
        },
        chrome_user_data: ComponentCheck {
            id: "chrome-user-data",
            label: "Google Chrome user profile",
            available: chrome_data.is_some(),
            path: chrome_data.as_deref().and_then(std::path::Path::to_str).map(str::to_owned),
            note: "Directory %LocalAppData%\\Google\\Chrome\\User Data used by CDP.",
            required: false,
        },
        node_js: ComponentCheck {
            id: "node-js",
            label: "Node.js",
            available: node.is_some(),
            path: node.as_deref().and_then(std::path::Path::to_str).map(str::to_owned),
            note: "Runtime for the background service worker.",
            required: false,
        },
        python: ComponentCheck {
            id: "python",
            label: "Python",
            available: python.is_some(),
            path: python.as_deref().and_then(std::path::Path::to_str).map(str::to_owned),
            note: "Optional alternative runtime for service workers.",
            required: false,
        },
        jobs_status_dir: data_dir
            .join("jobs")
            .join("status")
            .to_string_lossy()
            .into_owned(),
        jobs_logs_dir: data_dir
            .join("jobs")
            .join("logs")
            .to_string_lossy()
            .into_owned(),
        services_dir: data_dir.join("services").to_string_lossy().into_owned(),
    }
}

/// Writes the environment report into the application log so that every
/// support request carries the machine state. Missing optional components
/// are logged as warnings; missing required components as errors.
fn log_report(report: &EnvironmentReport) {
    let checks = [
        &report.powershell51,
        &report.powershell7,
        &report.chrome_exe,
        &report.chrome_user_data,
        &report.node_js,
        &report.python,
    ];
    for check in checks {
        let path = check.path.as_deref().unwrap_or("not found");
        if check.available {
            log::info!("[env] {}: OK ({})", check.label, path);
        } else if check.required {
            log::error!("[env] {}: MISSING ({})", check.label, path);
        } else {
            log::warn!("[env] {}: not found ({})", check.label, path);
        }
    }
}

// ----------------------------------------------------------------------------
// IPC command
// ----------------------------------------------------------------------------

/// IPC command: returns the environment report computed once during
/// application setup. The call is cheap - it clones cached state and never
/// touches the file system again.
///
/// Frontend usage (TypeScript):
/// `const report = await invoke<EnvironmentReport>("environment_report");`
#[tauri::command]
fn environment_report(report: State<'_, EnvironmentReport>) -> EnvironmentReport {
    report.inner().clone()
}

// ----------------------------------------------------------------------------
// Entry point
// ----------------------------------------------------------------------------

fn main() {
    // Service-worker mode dispatch: the SCM starts this very executable
    // with `--service-host <id> --data-dir <dir>` (the ImagePath built by
    // services/installer.rs). In that mode the process must never build
    // the GUI - it connects to the SCM, supervises the runtime process
    // of the definition and exits when the service stops.
    if let Some(exit_code) = services::worker::maybe_run_service_host() {
        std::process::exit(exit_code);
    }

    // The log directory must be known before the Tauri app exists, because
    // the log plugin is registered on the builder, not on a running app.
    let log_dir = windows_log_dir();

    let app = tauri::Builder::default()
        // Logging is registered first so that failures in every subsequent
        // plugin and setup step end up in the log file.
        .plugin(init_logging(log_dir))
        // Plugins backing the permission model in capabilities/default.json.
        .plugin(tauri_plugin_shell::init())   // shell: scoped PowerShell / Chrome
        .plugin(tauri_plugin_process::init()) // process: exit / relaunch
        .plugin(tauri_plugin_fs::init())      // fs: application data access
        .plugin(tauri_plugin_http::init())    // http: OpenAI Platform API
        .setup(|app| {
            // Create %APPDATA%\com.mcpgo.windows\{jobs\status, jobs\logs,
            // services, browser, tunnels} before anything writes there.
            // (`app` is `&mut App` here; both helpers want `&AppHandle`.)
            ensure_app_directories(app.handle())?;

            // Job registry (job_status / job_logs tracking): owns the
            // PowerShell job lifecycle. Reconciles orphaned jobs from a
            // previous application run during construction.
            let data_dir: PathBuf = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("cannot resolve the application data directory: {e}"))?;
            app.manage(JobRegistry::new(app.handle().clone(), data_dir.clone()));

            // Browser registry (Chrome/CDP domain): owns the debug session
            // lifecycle and the managed profile directories. Starts the
            // endpoint discovery scan on a background thread during
            // construction; discovered sessions surface through events.
            app.manage(BrowserRegistry::new(
                app.handle().clone(),
                data_dir.join("browser").join("profiles"),
            ));

            // Service store (LaunchAgents port): owns the service
            // definition files under <data>\services.
            app.manage(services::ServiceStore::new(data_dir.join("services")));

            // Tunnel registry (OpenAI Secure MCP Tunnels): owns the tunnel
            // definitions under <data>\tunnels (plus the generated
            // profiles\secrets\logs\bin tree) and supervises one
            // tunnel-client runtime process per started tunnel. Tunnels
            // marked autostart are started on a background thread.
            app.manage(tunnels::TunnelRegistry::new(
                app.handle().clone(),
                data_dir.join("tunnels"),
            ));

            // Run every Windows environment check exactly once and cache the
            // result: the frontend queries it through the
            // `environment_report` command without re-probing the disk.
            let report = build_environment_report(app.handle());
            log_report(&report);
            app.manage(report);

            log::info!(
                "{} v{} started (identifier: {})",
                APP_NAME,
                env!("CARGO_PKG_VERSION"),
                APP_IDENTIFIER
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            environment_report,
            commands::terminal::start_job,
            commands::terminal::cancel_job,
            commands::terminal::get_job,
            commands::terminal::list_jobs,
            commands::terminal::read_job_log,
            commands::browser::browser_profile,
            commands::browser::list_browser_sessions,
            commands::browser::get_browser_session,
            commands::browser::launch_browser,
            commands::browser::stop_browser_session,
            commands::browser::read_browser_targets,
            commands::services::list_services,
            commands::services::get_service_definition,
            commands::services::create_service_definition,
            commands::services::update_service_definition,
            commands::services::delete_service_definition,
            commands::services::install_service,
            commands::services::uninstall_service,
            commands::services::start_service,
            commands::services::stop_service,
            commands::services::restart_service,
            commands::services::read_service_log,
            commands::tunnels::tunnels_profile,
            commands::tunnels::list_tunnels,
            commands::tunnels::get_tunnel_definition,
            commands::tunnels::create_tunnel_definition,
            commands::tunnels::update_tunnel_definition,
            commands::tunnels::delete_tunnel_definition,
            commands::tunnels::set_tunnel_api_key,
            commands::tunnels::start_tunnel,
            commands::tunnels::stop_tunnel,
            commands::tunnels::read_tunnel_log
        ])
        .build(tauri::generate_context!())
        .expect("error while building the McpGo Tauri application");

    // Exit cleanup: Windows does not terminate child processes together
    // with their parent, so running PowerShell jobs are killed explicitly
    // before the process exits, as are invisible headless browser sessions
    // (windowed Chromes keep their windows and are re-attached through the
    // endpoint scan on the next start) and tunnel runtimes (invisible
    // background daemons, one orphan tunnel-client per tunnel otherwise).
    // Job_status files that miss their final update (the process dies
    // before the waiter threads can finalize them) are reconciled on the
    // next application start.
    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            app_handle.state::<JobRegistry>().shutdown();
            app_handle.state::<BrowserRegistry>().shutdown();
            app_handle.state::<tunnels::TunnelRegistry>().shutdown();
        }
    });
}

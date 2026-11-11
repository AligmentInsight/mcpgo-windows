//! Read-only Service Control Manager queries for services owned by McpGo.
//!
//! Why PowerShell instead of `sc.exe`: the text labels printed by `sc.exe`
//! are localized on non-English Windows installations, while the .NET enum
//! names produced by `Get-Service` (`Status`, `StartType`) are
//! culture-invariant - parsing them is deterministic on every machine.
//!
//! One invocation queries every `McpGo-*` service at once (the prefix is
//! owned by this application and doubles as its namespace), so refreshing
//! the whole services view costs a single PowerShell process. The status
//! values are returned verbatim; their meaning for the UI is documented on
//! [`ScmServiceState`].
//!
//! The write side of the SCM lives in `installer.rs` (elevated install /
//! uninstall / start / stop through the PowerShell management scripts);
//! this module stays read-only and quote-free.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use crate::error::{McpGoError, McpGoResult};
use crate::paths;
use crate::services::definition::SERVICE_NAME_PREFIX;
use crate::terminal;

/// Budget for one SCM status query, PowerShell cold start included. A
/// healthy `Get-Service` call returns in well under a second; the generous
/// margin only catches genuinely hung machines.
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);

/// SCM snapshot of one Windows service, as projected by the PowerShell
/// pipeline below (camelCase keys match the `pscustomobject` fields).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScmServiceState {
    /// Windows service name, e.g. `"McpGo-my-server"`.
    pub name: String,
    /// `ServiceControllerStatus` enum name: `"Running"`, `"Stopped"`,
    /// `"StartPending"`, `"StopPending"`, `"ContinuePending"`,
    /// `"PausePending"` or `"Paused"`.
    pub status: String,
    /// `ServiceStartMode` enum name: `"Automatic"`, `"Manual"` or
    /// `"Disabled"`.
    pub start_type: String,
}

/// Builds the status query pipeline for one service-name filter. Kept as
/// a function (not a const) so callers can pass either the whole McpGo
/// namespace (`McpGo-*`) or one exact service name.
///
/// Shape notes:
/// * `Get-Service` with a name filter that matches nothing raises a
///   non-terminating error - suppressed by `-ErrorAction SilentlyContinue`,
///   the pipeline then produces no output at all;
/// * `ConvertTo-Json -Compress` emits a single JSON object for exactly one
///   match and an array for several - both shapes are handled by
///   [`parse_scm_states`];
/// * the enum values are cast to `[string]` explicitly, which yields their
///   culture-invariant names.
fn status_pipeline(name_filter: &str) -> String {
    format!(
        "Get-Service -Name '{name_filter}' -ErrorAction SilentlyContinue \
         | Sort-Object Name \
         | ForEach-Object {{ [pscustomobject]@{{ \
             name = $_.Name; \
             status = [string]$_.Status; \
             startType = [string]$_.StartType }} }} \
         | ConvertTo-Json -Compress"
    )
}

/// Parses the stdout of the status pipeline into a lookup map keyed by the
/// lowercased service name (Windows service names are case-insensitive, so
/// lookups from definitions never miss on casing).
fn parse_scm_states(stdout: &str) -> McpGoResult<HashMap<String, ScmServiceState>> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(HashMap::new());
    }
    let value: serde_json::Value = serde_json::from_str(trimmed).map_err(|e| {
        McpGoError::internal(format!("SCM status query returned invalid JSON: {e}"))
    })?;
    let states: Vec<ScmServiceState> = match value {
        value @ serde_json::Value::Object(_) => {
            vec![serde_json::from_value(value).map_err(|e| {
                McpGoError::internal(format!("SCM status query returned invalid JSON: {e}"))
            })?]
        }
        serde_json::Value::Array(items) => serde_json::from_value(
            serde_json::Value::Array(items),
        )
        .map_err(|e| {
            McpGoError::internal(format!("SCM status query returned invalid JSON: {e}"))
        })?,
        _ => {
            return Err(McpGoError::internal(
                "SCM status query returned unexpected JSON",
            ));
        }
    };
    let mut map = HashMap::with_capacity(states.len());
    for state in states {
        map.insert(state.name.to_lowercase(), state);
    }
    Ok(map)
}

/// Queries the SCM state of every `McpGo-*` service in one PowerShell run.
/// Returns an empty map when no McpGo service is installed yet.
pub fn query_mcpgo_services() -> McpGoResult<HashMap<String, ScmServiceState>> {
    query_states(&format!("{SERVICE_NAME_PREFIX}*"))
}

/// Queries the SCM state of ONE exact service name (e.g. `McpGo-weather`).
/// Returns `None` when the SCM does not know that service. Used by the
/// write-side operations (install / uninstall / control) to confirm their
/// result without paying for a whole-namespace query.
pub fn query_service_state(service_name: &str) -> McpGoResult<Option<ScmServiceState>> {
    // Service names are case-insensitive; the map key is lowercased, so
    // the (at most one) entry matches regardless of the query casing.
    let states = query_states(service_name)?;
    Ok(states.into_values().next())
}

/// Shared engine of the two public queries: runs the pipeline for one
/// name filter and parses the result map.
fn query_states(name_filter: &str) -> McpGoResult<HashMap<String, ScmServiceState>> {
    let shell = paths::preferred_shell().ok_or_else(|| McpGoError::ComponentMissing {
        component: "powershell".into(),
        detail: "neither PowerShell 7 nor Windows PowerShell 5.1 was found".into(),
    })?;
    let captured = terminal::run_capture(&shell, &status_pipeline(name_filter), None, QUERY_TIMEOUT)
        .map_err(|e| McpGoError::io("run the SCM status query", &e))?;
    if captured.timed_out {
        return Err(McpGoError::internal(
            "SCM status query timed out (PowerShell did not finish in time)",
        ));
    }
    if captured.exit_code != Some(0) {
        return Err(McpGoError::internal(format!(
            "SCM status query failed (exit code {:?}): {}",
            captured.exit_code,
            captured.stderr.trim()
        )));
    }
    parse_scm_states(&captured.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_output_means_no_services() {
        let states = parse_scm_states("").expect("parseable");
        assert!(states.is_empty());
        let states = parse_scm_states("  \r\n").expect("parseable");
        assert!(states.is_empty());
    }

    #[test]
    fn single_object_is_unwrapped() {
        let states = parse_scm_states(
            r#"{"name":"McpGo-my-server","status":"Running","startType":"Automatic"}"#,
        )
        .expect("parseable");
        let state = states.get("mcpgo-my-server").expect("lowercased key");
        assert_eq!(state.status, "Running");
        assert_eq!(state.start_type, "Automatic");
    }

    #[test]
    fn arrays_keep_every_service() {
        let states = parse_scm_states(
            r#"[{"name":"McpGo-a","status":"Stopped","startType":"Manual"},
                {"name":"McpGo-b","status":"Running","startType":"Automatic"}]"#,
        )
        .expect("parseable");
        assert_eq!(states.len(), 2);
        assert_eq!(states["mcpgo-a"].status, "Stopped");
        assert_eq!(states["mcpgo-b"].status, "Running");
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(parse_scm_states("not json").is_err());
        // Valid JSON of the wrong shape is rejected as well.
        assert!(parse_scm_states("42").is_err());
    }

    #[test]
    fn pipelines_filter_by_exact_name_or_namespace() {
        let all = status_pipeline("McpGo-*");
        assert!(all.contains("Get-Service -Name 'McpGo-*'"));
        let one = status_pipeline("McpGo-weather");
        assert!(one.contains("Get-Service -Name 'McpGo-weather'"));
        // Both share the invariant projection of the status fields.
        for pipeline in [all, one] {
            assert!(pipeline.contains("status = [string]$_.Status"));
            assert!(pipeline.contains("startType = [string]$_.StartType"));
        }
    }
}

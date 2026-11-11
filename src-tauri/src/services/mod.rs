//! Windows service management - the port of the macOS LaunchAgents domain.
//!
//! Four isolated submodules, one per responsibility:
//!
//! * [`definition`] - the persisted model (`services/<id>.json`) and its
//!   validation, the direct analogue of the LaunchAgent property lists;
//! * [`scm`] - read-only Service Control Manager queries;
//! * [`installer`] - the write side of the SCM: install / uninstall /
//!   start / stop / restart through elevated PowerShell management
//!   scripts (self-elevating through a UAC prompt);
//! * [`worker`] - the SCM-facing service host: the application executable
//!   re-invoked with `--service-host`, supervising the runtime process
//!   (node / python / powershell) of one definition.
//!
//! This module joins the definition model with the SCM state into
//! [`ServiceSummary`] (one row of the services view) and implements the
//! guard that refuses to delete a definition while its service is still
//! registered in the SCM (removing only the file would leave an
//! unmanaged orphan service behind).
//!
//! Architecture decision (vs. the macOS original): a Windows service runs
//! the McpGo service worker, which supervises the actual runtime process
//! (node / python / powershell). Plain runtimes are not SCM-aware - they
//! do not implement the service control protocol - so they cannot be
//! registered with the SCM directly.

pub mod definition;
pub mod installer;
pub mod scm;
pub mod worker;

use std::fs;
use std::path::PathBuf;

use serde::Serialize;

use crate::error::{McpGoError, McpGoResult};
use crate::jobs::{self, JobLogLine};

pub use definition::{RuntimeKind, ServiceDefinition, ServiceDefinitionInput, ServiceStore};
pub use installer::ServiceAction;
pub use scm::ScmServiceState;

/// A service definition joined with its live SCM state - one row of the
/// services view.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSummary {
    /// The persisted definition.
    pub definition: ServiceDefinition,
    /// SCM name of the service (`McpGo-<id>`).
    pub service_name: String,
    /// Whether the SCM knows a service under that name.
    pub installed: bool,
    /// `ServiceControllerStatus` name when installed ("Running",
    /// "Stopped", ...), `None` otherwise.
    pub scm_status: Option<String>,
    /// `ServiceStartMode` name when installed ("Automatic", "Manual",
    /// "Disabled"), `None` otherwise.
    pub start_type: Option<String>,
    /// Whether the definition's runtime is resolvable on this machine.
    pub runtime_available: bool,
    /// Absolute path of the definition file.
    pub definition_path: String,
    /// Absolute path of the service log file written by the worker
    /// (`<data>\services\logs\<id>.log`); shown by the log panel footer.
    pub log_path: String,
}

impl ServiceStore {
    /// Assembles the summary row of one definition. Extracted so the
    /// whole-list refresh ([`ServiceStore::list_summaries`]) and the
    /// single-service refresh ([`ServiceStore::summary_with_live_scm`],
    /// used by the write-side operations to confirm their result) build
    /// the identical shape.
    fn build_summary(
        &self,
        definition: ServiceDefinition,
        scm_state: Option<&ScmServiceState>,
    ) -> ServiceSummary {
        ServiceSummary {
            service_name: definition.service_name(),
            installed: scm_state.is_some(),
            scm_status: scm_state.map(|state| state.status.clone()),
            start_type: scm_state.map(|state| state.start_type.clone()),
            runtime_available: definition.runtime.is_available(),
            definition_path: self
                .definition_path(&definition.id)
                .to_string_lossy()
                .into_owned(),
            log_path: self
                .log_path(&definition.id)
                .to_string_lossy()
                .into_owned(),
            definition,
        }
    }

    /// Absolute path of the log file the service worker writes for `id`
    /// (see `worker::service_log_path` - both sides must agree on
    /// `<services>\logs\<id>.log`).
    pub fn log_path(&self, id: &str) -> PathBuf {
        self.dir.join("logs").join(format!("{id}.log"))
    }

    /// Lists every definition joined with its SCM state, sorted by display
    /// name (id as a stable tie-breaker). Costs exactly one PowerShell
    /// invocation regardless of how many services exist - see
    /// [`scm::query_mcpgo_services`].
    pub fn list_summaries(&self) -> McpGoResult<Vec<ServiceSummary>> {
        let definitions = self.list();
        let scm_states = scm::query_mcpgo_services()?;
        let mut summaries: Vec<ServiceSummary> = definitions
            .into_iter()
            .map(|definition| {
                let service_name = definition.service_name();
                let scm_state = scm_states.get(&service_name.to_lowercase());
                self.build_summary(definition, scm_state)
            })
            .collect();
        summaries.sort_by(|a, b| {
            a.definition
                .name
                .to_lowercase()
                .cmp(&b.definition.name.to_lowercase())
                .then_with(|| a.definition.id.cmp(&b.definition.id))
        });
        Ok(summaries)
    }

    /// Refreshes the summary of ONE definition with its live SCM state
    /// (one exact-name query instead of the whole-namespace scan). Used
    /// by the write-side operations to return the post-operation truth.
    pub fn summary_with_live_scm(&self, id: &str) -> McpGoResult<ServiceSummary> {
        let definition = self.get(id)?;
        let scm_state = scm::query_service_state(&definition.service_name())?;
        Ok(self.build_summary(definition, scm_state.as_ref()))
    }

    /// Deletes one definition, refusing while its service is still
    /// registered in the SCM: removing only the file would leave an
    /// unmanaged orphan service behind, invisible to every later McpGo
    /// reinstall of the same id.
    pub fn delete_definition(&self, id: &str) -> McpGoResult<()> {
        let definition = self.get(id)?;
        let service_name = definition.service_name();
        let scm_states = scm::query_mcpgo_services()?;
        if scm_states.contains_key(&service_name.to_lowercase()) {
            return Err(McpGoError::invalid_input(format!(
                "the Windows service \"{service_name}\" is still installed; \
                 stop and uninstall it before deleting the definition"
            )));
        }
        self.delete(id)
    }

    /// Returns the parsed log records of one service, oldest first. The
    /// log file is written by the service worker (`worker.rs`) using the
    /// same canonical `[RFC3339-UTC] OUT|ERR|SYS <text>` grammar as the
    /// job logs, so the shared parser applies; a service that never ran
    /// has no file yet, which reads as an empty log (not an error).
    pub fn read_log(&self, id: &str) -> McpGoResult<ServiceLog> {
        // Validating the definition first guards against fabricated ids
        // (same contract as `read_job_log`).
        self.get(id)?;
        let raw = fs::read_to_string(self.log_path(id)).unwrap_or_default();
        let lines = raw.lines().map(jobs::parse_log_line).collect();
        Ok(ServiceLog {
            id: id.to_owned(),
            lines,
        })
    }
}

/// Response of the `read_service_log` command. The lines reuse the job
/// log record type: the service worker writes the same canonical
/// grammar, so one wire type covers both domains (see `jobs::JobLogLine`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceLog {
    /// Definition id whose log was read.
    pub id: String,
    /// Parsed log records, oldest first.
    pub lines: Vec<JobLogLine>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_flags_mirror_the_scm_state() {
        // Pure assembly check: the join logic must not depend on the
        // number of installed services.
        let store = ServiceStore::new(std::path::PathBuf::from(
            r"C:\Users\dev\AppData\Roaming\com.mcpgo.windows\services",
        ));
        let definition = ServiceDefinition {
            id: "weather".into(),
            name: "Weather".into(),
            description: String::new(),
            runtime: RuntimeKind::Node,
            command: "server.js".into(),
            args: Vec::new(),
            working_dir: None,
            env: Default::default(),
            autostart: true,
            keep_alive: true,
            runtime_path: None,
            created_at: 0,
            updated_at: 0,
        };

        let installed = store.build_summary(
            definition.clone(),
            Some(&ScmServiceState {
                name: "McpGo-weather".into(),
                status: "Running".into(),
                start_type: "Automatic".into(),
            }),
        );
        assert!(installed.installed);
        assert_eq!(installed.scm_status.as_deref(), Some("Running"));
        assert_eq!(installed.start_type.as_deref(), Some("Automatic"));
        assert_eq!(installed.service_name, "McpGo-weather");
        assert_eq!(
            installed.definition_path,
            r"C:\Users\dev\AppData\Roaming\com.mcpgo.windows\services\weather.json"
        );
        // The log path must mirror worker::service_log_path (both derive
        // from the same <data>\services root: definitions next to logs\).
        assert_eq!(
            installed.log_path,
            r"C:\Users\dev\AppData\Roaming\com.mcpgo.windows\services\logs\weather.log"
        );

        let uninstalled = store.build_summary(definition, None);
        assert!(!uninstalled.installed);
        assert!(uninstalled.scm_status.is_none());
        assert!(uninstalled.start_type.is_none());
    }
}

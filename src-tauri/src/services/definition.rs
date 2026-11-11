//! Service definitions - the Windows port of the macOS LaunchAgent property
//! lists (`launchagents/*.plist` in the original).
//!
//! One definition is one JSON file in
//! `%APPDATA%\com.mcpgo.windows\services\<id>.json`, written atomically
//! (tmp file + rename, the same strategy as the job_status files). The
//! actual Windows service (the SCM object) is created from a definition
//! by `installer.rs` (elevated PowerShell scripts); the running service
//! is hosted by `worker.rs`. This module owns the persisted model only.
//!
//! Mapping to the macOS original:
//!
//! | macOS LaunchAgent plist     | `ServiceDefinition` field       |
//! |-----------------------------|---------------------------------|
//! | `Label`                     | `id` (SCM name: `McpGo-<id>`)   |
//! | `Program`/`ProgramArguments`| `runtime` + `command` + `args` |
//! | `WorkingDirectory`          | `working_dir`                   |
//! | `EnvironmentVariables`      | `env`                           |
//! | `RunAtLoad`                 | `autostart`                     |
//! | `KeepAlive`                 | `keep_alive`                    |
//! | (n/a: launchd resolves the  | `runtime_path` (absolute        |
//! |  binary from the plist)     |  executable override)           |
//!
//! Naming convention: every SCM service owned by McpGo is named
//! `McpGo-<id>` ([`SERVICE_NAME_PREFIX`]). The prefix doubles as the
//! namespace the read-only SCM status query filters by (see `scm.rs`), so
//! it must never collide with foreign services.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{McpGoError, McpGoResult};
use crate::paths;
use crate::time;

/// Prefix of every SCM service name owned by McpGo (`McpGo-<id>`).
pub const SERVICE_NAME_PREFIX: &str = "McpGo-";

/// Maximum id length (also the character budget of [`slugify`]).
const MAX_ID_CHARS: usize = 40;
/// Maximum display-name length.
const MAX_NAME_CHARS: usize = 80;
/// Maximum description length.
const MAX_DESCRIPTION_CHARS: usize = 256;
/// Maximum command length (bytes).
const MAX_COMMAND_CHARS: usize = 4096;
/// Maximum number of command-line arguments.
const MAX_ARGUMENTS: usize = 32;
/// Maximum length of one command-line argument.
const MAX_ARGUMENT_CHARS: usize = 1024;
/// Maximum number of environment entries.
const MAX_ENV_ENTRIES: usize = 32;
/// Maximum length of one environment value.
const MAX_ENV_VALUE_CHARS: usize = 2048;

// ----------------------------------------------------------------------------
// Wire types (shared with the frontend; keep src/types/service.ts in sync)
// ----------------------------------------------------------------------------

/// Runtime that executes the service command. Serialized lowercase on the
/// wire ("node" | "python" | "powershell").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeKind {
    Node,
    Python,
    PowerShell,
}

impl RuntimeKind {
    /// Human-readable label for log records and the UI.
    pub fn label(self) -> &'static str {
        match self {
            RuntimeKind::Node => "Node.js",
            RuntimeKind::Python => "Python",
            RuntimeKind::PowerShell => "PowerShell",
        }
    }

    /// Whether the runtime executable is resolvable on this machine right
    /// now. Availability is informational: it powers the warning chip in
    /// the services view. The service worker re-resolves the runtime when
    /// the service actually starts, because PATH may change between the
    /// definition being saved and the service running.
    pub fn is_available(self) -> bool {
        match self {
            RuntimeKind::Node => paths::node_path().is_some(),
            RuntimeKind::Python => paths::python_path().is_some(),
            RuntimeKind::PowerShell => paths::preferred_shell().is_some(),
        }
    }
}

/// Contents of one definition file, also exchanged over IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDefinition {
    /// Slug identifying the definition (`McpGo-<id>` in the SCM).
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Free-form description (may be empty).
    pub description: String,
    /// Runtime that executes the command.
    pub runtime: RuntimeKind,
    /// Entry command: a script path (node/python) or a PowerShell command.
    pub command: String,
    /// Command-line arguments appended after the command.
    pub args: Vec<String>,
    /// Absolute working directory, when one is set.
    pub working_dir: Option<String>,
    /// Extra environment variables for the runtime process.
    pub env: BTreeMap<String, String>,
    /// Whether the service starts automatically at system boot.
    pub autostart: bool,
    /// Whether the service worker restarts the runtime process when it
    /// exits (port of the LaunchAgent `KeepAlive` flag). Crashes of the
    /// worker itself are additionally covered by the SCM failure
    /// recovery actions configured at install time.
    #[serde(default)]
    pub keep_alive: bool,
    /// Optional absolute path of the runtime executable, overriding the
    /// PATH lookup. Services run as LocalSystem whose PATH differs from
    /// the installing user's, so an absolute path is the reliable choice
    /// for per-user runtime installs.
    #[serde(default)]
    pub runtime_path: Option<String>,
    /// Creation time, epoch milliseconds.
    pub created_at: u64,
    /// Last modification time, epoch milliseconds.
    pub updated_at: u64,
}

impl ServiceDefinition {
    /// Name of the Windows service (SCM object) this definition describes:
    /// `McpGo-<id>`. All McpGo-managed services share this prefix, which is
    /// also the wildcard the SCM status query filters by.
    pub fn service_name(&self) -> String {
        format!("{SERVICE_NAME_PREFIX}{}", self.id)
    }
}

/// Caller-supplied payload of the create/update IPC commands. Unlike
/// [`ServiceDefinition`] it carries no `id`/timestamps: those are owned by
/// the store.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDefinitionInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub runtime: RuntimeKind,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub keep_alive: bool,
    #[serde(default)]
    pub runtime_path: Option<String>,
}

// ----------------------------------------------------------------------------
// Validation and id derivation
// ----------------------------------------------------------------------------

/// Validates a service id: 1..=40 characters, lowercase letters, digits and
/// inner hyphens; must start and end with a letter or digit. Ids become
/// file names and part of SCM service names, so the alphabet is kept tight
/// on purpose.
pub fn is_valid_id(id: &str) -> bool {
    let count = id.chars().count();
    if count == 0 || count > MAX_ID_CHARS {
        return false;
    }
    let is_border = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    let first = id.chars().next().expect("non-empty checked above");
    let last = id.chars().next_back().expect("non-empty checked above");
    is_border(first)
        && is_border(last)
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Derives an id from a display name: lowercase ASCII alphanumerics are
/// kept, every other character collapses into a single hyphen; leading and
/// trailing hyphens are dropped and the result is truncated to
/// [`MAX_ID_CHARS`] characters. Returns an empty string when nothing
/// survives (e.g. a fully non-ASCII name) - callers turn that into an
/// "provide an id explicitly" error.
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut pending_hyphen = true; // also swallows leading separators
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            pending_hyphen = false;
        } else if !pending_hyphen {
            slug.push('-');
            pending_hyphen = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    // Enforce the id budget on a character boundary, then re-trim in case
    // the cut landed on a hyphen.
    if slug.chars().count() > MAX_ID_CHARS {
        slug = slug.chars().take(MAX_ID_CHARS).collect();
        while slug.ends_with('-') {
            slug.pop();
        }
    }
    slug
}

/// Environment variable keys follow the Windows convention: a letter or
/// underscore first, then letters, digits and underscores.
fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Validates a create/update payload. Every rule produces a specific,
/// actionable message - the frontend mirrors the cheap ones client-side,
/// but this is the authoritative gate.
pub fn validate_input(input: &ServiceDefinitionInput) -> McpGoResult<()> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(McpGoError::invalid_input("service name must not be empty"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(McpGoError::invalid_input(format!(
            "service name is too long ({} characters, limit {MAX_NAME_CHARS})",
            name.chars().count()
        )));
    }
    if input.description.trim().chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(McpGoError::invalid_input(format!(
            "description is too long (limit {MAX_DESCRIPTION_CHARS} characters)"
        )));
    }
    let command = input.command.trim();
    if command.is_empty() {
        return Err(McpGoError::invalid_input("command must not be empty"));
    }
    if command.len() > MAX_COMMAND_CHARS {
        return Err(McpGoError::invalid_input(format!(
            "command is too long ({} bytes, limit {MAX_COMMAND_CHARS} bytes)",
            command.len()
        )));
    }
    if input.args.len() > MAX_ARGUMENTS {
        return Err(McpGoError::invalid_input(format!(
            "too many arguments ({}, limit {MAX_ARGUMENTS})",
            input.args.len()
        )));
    }
    if input
        .args
        .iter()
        .any(|arg| arg.trim().is_empty() || arg.len() > MAX_ARGUMENT_CHARS)
    {
        return Err(McpGoError::invalid_input(format!(
            "every argument must be non-empty and at most {MAX_ARGUMENT_CHARS} characters"
        )));
    }
    if input.env.len() > MAX_ENV_ENTRIES {
        return Err(McpGoError::invalid_input(format!(
            "too many environment entries ({}, limit {MAX_ENV_ENTRIES})",
            input.env.len()
        )));
    }
    for (key, value) in &input.env {
        if !is_valid_env_key(key) {
            return Err(McpGoError::invalid_input(format!(
                "invalid environment variable name \"{key}\" (letters, digits, underscores; must start with a letter or underscore)"
            )));
        }
        if value.len() > MAX_ENV_VALUE_CHARS {
            return Err(McpGoError::invalid_input(format!(
                "environment value for {key} is too long (limit {MAX_ENV_VALUE_CHARS} characters)"
            )));
        }
    }
    if let Some(dir) = input.working_dir.as_deref().map(str::trim) {
        if !dir.is_empty() && !Path::new(dir).is_absolute() {
            return Err(McpGoError::invalid_input(
                "working directory must be an absolute path",
            ));
        }
    }
    if let Some(runtime_path) = normalized_runtime_path(input) {
        if runtime_path.len() > MAX_COMMAND_CHARS {
            return Err(McpGoError::invalid_input(format!(
                "runtime path is too long ({} bytes, limit {MAX_COMMAND_CHARS} bytes)",
                runtime_path.len()
            )));
        }
        if !Path::new(&runtime_path).is_absolute() {
            return Err(McpGoError::invalid_input(
                "runtime path must be an absolute path (e.g. C:\\Program Files\\nodejs\\node.exe)",
            ));
        }
    }
    Ok(())
}

/// Trims an input's `runtime_path`: `None` when unset or blank, so the
/// normalized value is what gets persisted and what the worker reads.
fn normalized_runtime_path(input: &ServiceDefinitionInput) -> Option<String> {
    input
        .runtime_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

// ----------------------------------------------------------------------------
// Persistence (the store)
// ----------------------------------------------------------------------------

/// Owns the `services` directory and the definition files inside it.
/// Managed as Tauri state in `main.rs`. The store is stateless besides the
/// directory path: every operation goes straight to the file system, which
/// keeps the files the single source of truth (no cache to invalidate;
/// definitions edited by hand in Explorer are picked up on the next list).
pub struct ServiceStore {
    dir: PathBuf,
}

impl ServiceStore {
    /// Creates the store for `dir` (`%APPDATA%\com.mcpgo.windows\services`).
    /// The directory itself is created by `ensure_app_directories` at
    /// startup; a missing directory simply yields an empty list.
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The services directory this store owns (definitions live directly
    /// inside it; the installer materializes `scripts\` and the worker
    /// writes `logs\` next to them).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The McpGo data directory (the parent of the services directory).
    /// Baked into the service ImagePath by the installer so the worker -
    /// running as LocalSystem with a different `%APPDATA%` - still finds
    /// the definition files.
    pub fn data_dir(&self) -> PathBuf {
        self.dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.dir.clone())
    }

    /// Absolute path of the definition file for `id`.
    pub fn definition_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Lists every parseable definition. Corrupt files are skipped with a
    /// warning instead of failing the whole view - one hand-edited file
    /// must not hide the remaining services.
    pub fn list(&self) -> Vec<ServiceDefinition> {
        let mut definitions = Vec::new();
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(_) => return definitions,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = fs::read_to_string(&path) else { continue };
            match serde_json::from_str::<ServiceDefinition>(&raw) {
                Ok(definition) => definitions.push(definition),
                Err(e) => {
                    log::warn!(
                        "[services] skipping corrupt definition {}: {e}",
                        path.display()
                    );
                }
            }
        }
        definitions
    }

    /// Loads one definition by id.
    pub fn get(&self, id: &str) -> McpGoResult<ServiceDefinition> {
        if !is_valid_id(id) {
            return Err(McpGoError::invalid_input(
                "invalid service id (lowercase letters, digits and inner hyphens)",
            ));
        }
        let path = self.definition_path(id);
        match fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).map_err(|e| {
                McpGoError::internal(format!("corrupt service definition for {id}: {e}"))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(McpGoError::item_not_found("service definition", id))
            }
            Err(e) => Err(McpGoError::io("read service definition", &e)),
        }
    }

    /// Creates a definition. `requested_id` is optional: when omitted, the
    /// id is derived from the (slugified) name. Fails when the derived or
    /// requested id is invalid or already taken.
    pub fn create(
        &self,
        requested_id: Option<&str>,
        input: &ServiceDefinitionInput,
    ) -> McpGoResult<ServiceDefinition> {
        validate_input(input)?;
        let id = match requested_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(explicit) => {
                if !is_valid_id(explicit) {
                    return Err(McpGoError::invalid_input(format!(
                        "invalid service id \"{explicit}\" (lowercase letters, digits and inner hyphens, at most {MAX_ID_CHARS} characters)"
                    )));
                }
                explicit.to_owned()
            }
            None => {
                let derived = slugify(&input.name);
                if derived.is_empty() {
                    return Err(McpGoError::invalid_input(
                        "cannot derive a service id from this name; provide an id explicitly",
                    ));
                }
                derived
            }
        };
        if self.definition_path(&id).exists() {
            return Err(McpGoError::invalid_input(format!(
                "a service definition with id \"{id}\" already exists"
            )));
        }
        let now = time::now_epoch_millis();
        let definition = ServiceDefinition {
            id,
            name: input.name.trim().to_owned(),
            description: input.description.trim().to_owned(),
            runtime: input.runtime,
            command: input.command.trim().to_owned(),
            args: input.args.clone(),
            working_dir: input
                .working_dir
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            env: input.env.clone(),
            autostart: input.autostart,
            keep_alive: input.keep_alive,
            runtime_path: normalized_runtime_path(input),
            created_at: now,
            updated_at: now,
        };
        self.write_definition(&definition)?;
        log::info!(
            "[services] definition {} created (runtime {}, autostart {}, keep-alive {})",
            definition.id,
            definition.runtime.label(),
            definition.autostart,
            definition.keep_alive
        );
        Ok(definition)
    }

    /// Updates an existing definition in place. The `id` is immutable (it
    /// names both the file and the SCM service); `created_at` is preserved
    /// and `updated_at` bumped.
    pub fn update(
        &self,
        id: &str,
        input: &ServiceDefinitionInput,
    ) -> McpGoResult<ServiceDefinition> {
        let mut existing = self.get(id)?;
        validate_input(input)?;
        existing.name = input.name.trim().to_owned();
        existing.description = input.description.trim().to_owned();
        existing.runtime = input.runtime;
        existing.command = input.command.trim().to_owned();
        existing.args = input.args.clone();
        existing.working_dir = input
            .working_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        existing.env = input.env.clone();
        existing.autostart = input.autostart;
        existing.keep_alive = input.keep_alive;
        existing.runtime_path = normalized_runtime_path(input);
        existing.updated_at = time::now_epoch_millis();
        self.write_definition(&existing)?;
        log::info!("[services] definition {} updated", existing.id);
        Ok(existing)
    }

    /// Deletes one definition file. The SCM-installed guard lives in
    /// `ServiceStore::delete_definition` (mod.rs) - this method is the raw
    /// file operation.
    pub fn delete(&self, id: &str) -> McpGoResult<()> {
        self.get(id)?;
        match fs::remove_file(self.definition_path(id)) {
            Ok(()) => {
                log::info!("[services] definition {id} deleted");
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(McpGoError::item_not_found("service definition", id))
            }
            Err(e) => Err(McpGoError::io("delete service definition", &e)),
        }
    }

    /// Writes a definition file atomically: serialize, write a `.tmp`
    /// sibling, rename over the target - the same strategy as the
    /// job_status files, so a crash never leaves a half-written JSON.
    fn write_definition(&self, definition: &ServiceDefinition) -> McpGoResult<()> {
        let path = self.definition_path(&definition.id);
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(definition)
            .map_err(|e| McpGoError::internal(format!("serialize service definition: {e}")))?;
        fs::write(&tmp, json)
            .map_err(|e| McpGoError::io("write service definition", &e))?;
        fs::rename(&tmp, &path)
            .map_err(|e| McpGoError::io("replace service definition", &e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_maps_names_to_service_ids() {
        assert_eq!(slugify("My MCP Server!"), "my-mcp-server");
        assert_eq!(slugify("  --Weird__Name..  "), "weird-name");
        assert_eq!(slugify("server.js"), "server-js");
        assert_eq!(slugify("a"), "a");
        // Fully non-ASCII names derive nothing: the caller then requires
        // an explicit id.
        assert_eq!(slugify("Мой сервер"), "");
    }

    #[test]
    fn slugify_truncates_on_a_character_boundary() {
        let slug = slugify(&"x".repeat(80));
        assert_eq!(slug.chars().count(), MAX_ID_CHARS);
        assert!(!slug.ends_with('-'));
    }

    #[test]
    fn ids_are_validated() {
        assert!(is_valid_id("my-service"));
        assert!(is_valid_id("s"));
        assert!(is_valid_id("svc2"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("-leading"));
        assert!(!is_valid_id("trailing-"));
        assert!(!is_valid_id("Has-Upper"));
        assert!(!is_valid_id("with space"));
        assert!(!is_valid_id(&"x".repeat(MAX_ID_CHARS + 1)));
    }

    #[test]
    fn env_keys_are_validated() {
        assert!(is_valid_env_key("_KEY"));
        assert!(is_valid_env_key("KEY_1"));
        assert!(!is_valid_env_key("1KEY"));
        assert!(!is_valid_env_key("KEY-1"));
        assert!(!is_valid_env_key(""));
    }

    #[test]
    fn service_names_carry_the_namespace_prefix() {
        let definition = ServiceDefinition {
            id: "my-server".into(),
            name: "My Server".into(),
            description: String::new(),
            runtime: RuntimeKind::Node,
            command: "server.js".into(),
            args: Vec::new(),
            working_dir: None,
            env: BTreeMap::new(),
            autostart: false,
            keep_alive: false,
            runtime_path: None,
            created_at: 0,
            updated_at: 0,
        };
        assert_eq!(definition.service_name(), "McpGo-my-server");
    }

    #[test]
    fn definition_files_without_the_new_fields_still_parse() {
        // Definitions written before keep_alive/runtime_path existed
        // carry neither field; #[serde(default)] must fill them.
        let legacy = r#"{
            "id": "legacy",
            "name": "Legacy",
            "description": "",
            "runtime": "node",
            "command": "server.js",
            "args": [],
            "workingDir": null,
            "env": {},
            "autostart": false,
            "createdAt": 1,
            "updatedAt": 2
        }"#;
        let parsed: ServiceDefinition =
            serde_json::from_str(legacy).expect("legacy definition parses");
        assert!(!parsed.keep_alive);
        assert!(parsed.runtime_path.is_none());
    }

    #[test]
    fn runtime_paths_are_normalized_and_validated() {
        fn input_with(runtime_path: Option<&str>) -> ServiceDefinitionInput {
            ServiceDefinitionInput {
                name: "s".into(),
                description: String::new(),
                runtime: RuntimeKind::Node,
                command: "server.js".into(),
                args: Vec::new(),
                working_dir: None,
                env: BTreeMap::new(),
                autostart: false,
                keep_alive: false,
                runtime_path: runtime_path.map(str::to_owned),
            }
        }
        // Blank paths collapse to None (not stored, not validated).
        assert_eq!(normalized_runtime_path(&input_with(Some("   "))), None);
        assert!(validate_input(&input_with(Some("   "))).is_ok());
        assert!(validate_input(&input_with(Some(r"C:\nodejs\node.exe"))).is_ok());
        // Relative paths are rejected: the worker must not resolve them
        // against an arbitrary working directory.
        assert!(validate_input(&input_with(Some("node.exe"))).is_err());
    }

    #[test]
    fn data_dir_is_the_parent_of_the_services_dir() {
        let store = ServiceStore::new(PathBuf::from(
            r"C:\Users\dev\AppData\Roaming\com.mcpgo.windows\services",
        ));
        assert_eq!(
            store.data_dir(),
            PathBuf::from(r"C:\Users\dev\AppData\Roaming\com.mcpgo.windows")
        );
        assert_eq!(store.dir(), Path::new(r"C:\Users\dev\AppData\Roaming\com.mcpgo.windows\services"));
    }
}

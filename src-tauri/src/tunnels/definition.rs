//! Tunnel definitions - the persisted model of the OpenAI Secure MCP Tunnel
//! domain (roadmap stage 5 of the Windows port).
//!
//! One definition is one JSON file in
//! `%APPDATA%\com.mcpgo.windows\tunnels\<id>.json`, written atomically
//! (tmp file + rename, the same strategy as the job_status files and the
//! service definitions). A definition describes everything the generated
//! `tunnel-client` profile needs:
//!
//! | `tunnel-client` profile key        | `TunnelDefinition` field          |
//! |-------------------------------------|-----------------------------------|
//! | `control_plane.tunnel_id`           | `tunnel_id`                       |
//! | `control_plane.base_url`            | `residency` (endpoint catalog)    |
//! | `control_plane.api_key`             | `api_key` (managed/env/file ref)  |
//! | `mcp.commands[main].command`        | `target_kind=Stdio`: `command` +  |
//! |                                     | `args` + `working_dir`            |
//! | `mcp.server_urls[main].url`         | `target_kind=Http`: `server_url`  |
//! | `health.listen_addr`                | `health_port` (or allocated)      |
//! | `log.file`                          | `<tunnels>\logs\<id>.log` (fixed) |
//!
//! ## Secret handling (the architecture rule)
//!
//! OpenAI API keys live ONLY in the Rust core; the webview never receives
//! them. A definition therefore stores an [`ApiKeySource`] *reference*, never
//! key material:
//!
//! * `Managed` - McpGo stores the key in
//!   `<tunnels>\secrets\<id>.key`, written by the `set_tunnel_api_key`
//!   command (which returns unit - the key is never echoed back). The
//!   generated profile references it as `file:<path>`.
//! * `Env` - the profile references `env:<VARNAME>`; the variable must
//!   exist in the environment the runtime inherits from this GUI process.
//! * `File` - the profile references `file:<absolute path>` to a key file
//!   the user maintains (the pattern the tunnel-client documentation
//!   recommends for checked-in configs).
//!
//! Deleting a definition removes the definition file, the generated profile
//! and (for `Managed`) the secret file; the log file is deliberately left
//! behind as post-mortem evidence, exactly like the services domain.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{McpGoError, McpGoResult};
use crate::time;

/// Maximum id length (also the character budget of [`slugify`]).
const MAX_ID_CHARS: usize = 40;
/// Maximum display-name length.
const MAX_NAME_CHARS: usize = 80;
/// Maximum description length.
const MAX_DESCRIPTION_CHARS: usize = 256;
/// Maximum command length (bytes) of a stdio target.
const MAX_COMMAND_CHARS: usize = 4096;
/// Maximum number of command-line arguments of a stdio target.
const MAX_ARGUMENTS: usize = 32;
/// Maximum length of one command-line argument.
const MAX_ARGUMENT_CHARS: usize = 1024;
/// Maximum server URL length (bytes) of an HTTP target.
const MAX_URL_CHARS: usize = 2048;
/// Prefix every OpenAI tunnel id carries (`tunnel_` + base32-ish suffix).
const TUNNEL_ID_PREFIX: &str = "tunnel_";
/// Maximum total length of a tunnel id (the canonical form is
/// `tunnel_` + 32 characters; namespaced aliases may be longer).
const MAX_TUNNEL_ID_CHARS: usize = 64;
/// Maximum length of the stored API key literal (bytes, `set_tunnel_api_key`).
const MAX_API_KEY_CHARS: usize = 4096;

// ----------------------------------------------------------------------------
// Wire types (shared with the frontend; keep src/types/tunnel.ts in sync)
// ----------------------------------------------------------------------------

/// Where the local MCP server of the tunnel lives. `Stdio` spawns the server
/// as a child of the tunnel-client runtime; `Http` forwards to an existing
/// streamable-HTTP endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelTargetKind {
    Stdio,
    Http,
}

impl TunnelTargetKind {
    /// Human-readable label for the UI chip.
    pub fn label(self) -> &'static str {
        match self {
            TunnelTargetKind::Stdio => "stdio",
            TunnelTargetKind::Http => "HTTP",
        }
    }
}

/// How the runtime API key reaches the generated profile. See the module
/// docs for the secret-handling contract: the definition stores a reference,
/// never the key itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiKeySourceKind {
    /// McpGo owns the key file under `<tunnels>\secrets\<id>.key`.
    Managed,
    /// The key is read from an environment variable of the runtime.
    Env,
    /// The key is read from an existing key file.
    File,
}

/// Reference to the runtime API key (never key material).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeySource {
    pub kind: ApiKeySourceKind,
    /// Environment variable name (`Env`) or absolute file path (`File`);
    /// `None` for `Managed` (the path is derived from the definition id).
    #[serde(default)]
    pub reference: Option<String>,
}

/// Control-plane endpoint region. The variants mirror the tunnel-client
/// residency catalog (`global` is the default `https://api.openai.com`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Residency {
    Global,
    Us,
    Eu,
    Au,
    Ca,
    Gb,
    In,
    Jp,
    Kr,
    Sg,
    Ae,
}

impl Residency {
    /// HTTPS base URL written into `control_plane.base_url`.
    pub fn base_url(self) -> &'static str {
        match self {
            Residency::Global => "https://api.openai.com",
            Residency::Us => "https://us.api.openai.com",
            Residency::Eu => "https://eu.api.openai.com",
            Residency::Au => "https://au.api.openai.com",
            Residency::Ca => "https://ca.api.openai.com",
            Residency::Gb => "https://gb.api.openai.com",
            Residency::In => "https://in.api.openai.com",
            Residency::Jp => "https://jp.api.openai.com",
            Residency::Kr => "https://kr.api.openai.com",
            Residency::Sg => "https://sg.api.openai.com",
            Residency::Ae => "https://ae.api.openai.com",
        }
    }
}

impl Default for Residency {
    fn default() -> Self {
        Residency::Global
    }
}

/// Contents of one tunnel definition file, also exchanged over IPC (minus
/// every secret byte - see [`ApiKeySource`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelDefinition {
    /// Slug identifying the definition (file name + registry key).
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Free-form description (may be empty).
    pub description: String,
    /// OpenAI tunnel id (`tunnel_<suffix>`) from Platform tunnel settings.
    pub tunnel_id: String,
    /// Where the local MCP server lives.
    pub target_kind: TunnelTargetKind,
    /// Stdio target: the MCP server executable (None for HTTP targets).
    #[serde(default)]
    pub command: Option<String>,
    /// Stdio target: command-line arguments appended to `command`.
    #[serde(default)]
    pub args: Vec<String>,
    /// Stdio target: absolute working directory of the MCP server.
    #[serde(default)]
    pub working_dir: Option<String>,
    /// HTTP target: streamable-HTTP URL of the MCP server
    /// (e.g. `http://127.0.0.1:3000/mcp`; None for stdio targets).
    #[serde(default)]
    pub server_url: Option<String>,
    /// Reference to the runtime API key (never the key itself).
    pub api_key: ApiKeySource,
    /// Control-plane endpoint region (default `global`).
    #[serde(default)]
    pub residency: Residency,
    /// Whether the runtime starts automatically with McpGo.
    #[serde(default)]
    pub autostart: bool,
    /// Whether the supervisor restarts the runtime after unexpected exits.
    #[serde(default)]
    pub keep_alive: bool,
    /// Optional absolute path of `tunnel-client.exe`, overriding the
    /// resolution order (PATH, then `<tunnels>\bin`).
    #[serde(default)]
    pub client_path: Option<String>,
    /// Pinned health-listener port; `None` allocates the next free port of
    /// the McpGo range on every start.
    #[serde(default)]
    pub health_port: Option<u16>,
    /// Creation time, epoch milliseconds.
    pub created_at: u64,
    /// Last modification time, epoch milliseconds.
    pub updated_at: u64,
}

/// Caller-supplied payload of the create/update IPC commands. Mirrors
/// [`TunnelDefinition`] minus id and timestamps (owned by the store).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelDefinitionInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub tunnel_id: String,
    pub target_kind: TunnelTargetKind,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub server_url: Option<String>,
    pub api_key: ApiKeySource,
    #[serde(default)]
    pub residency: Residency,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub keep_alive: bool,
    #[serde(default)]
    pub client_path: Option<String>,
    #[serde(default)]
    pub health_port: Option<u16>,
}

// ----------------------------------------------------------------------------
// Validation and id derivation
// ----------------------------------------------------------------------------

/// Validates a tunnel definition id: 1..=40 characters, lowercase letters,
/// digits and inner hyphens; must start and end with a letter or digit (the
/// same alphabet as the service ids - both become file names).
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

/// Derives an id from a display name (same slug grammar as the services
/// domain; duplicated on purpose - modules stay isolated from each other's
/// internals).
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
    if slug.chars().count() > MAX_ID_CHARS {
        slug = slug.chars().take(MAX_ID_CHARS).collect();
        while slug.ends_with('-') {
            slug.pop();
        }
    }
    slug
}

/// Validates an OpenAI tunnel id: the `tunnel_` prefix plus 1..=57
/// lowercase letters, digits or underscores (the canonical form is 32
/// characters; namespaced aliases accepted, uppercase rejected - the
/// service assigns lowercase ids only).
pub fn is_valid_tunnel_id(tunnel_id: &str) -> bool {
    let Some(suffix) = tunnel_id.strip_prefix(TUNNEL_ID_PREFIX) else {
        return false;
    };
    !suffix.is_empty()
        && tunnel_id.chars().count() <= MAX_TUNNEL_ID_CHARS
        && suffix.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Environment variable keys follow the Windows convention (same rule as
/// the services domain).
fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `true` when `value` contains a double quote. Stdio commands and arguments
/// are joined into the single YAML `command:` string of the profile; embedded
/// double quotes would depend on the client's private splitting rules, so
/// they are rejected up front with an actionable message.
fn contains_double_quote(value: &str) -> bool {
    value.contains('"')
}

/// Validates a create/update payload. Every rule produces a specific,
/// actionable message - the frontend mirrors the cheap ones client-side,
/// but this is the authoritative gate.
pub fn validate_input(input: &TunnelDefinitionInput) -> McpGoResult<()> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(McpGoError::invalid_input("tunnel name must not be empty"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(McpGoError::invalid_input(format!(
            "tunnel name is too long ({} characters, limit {MAX_NAME_CHARS})",
            name.chars().count()
        )));
    }
    if input.description.trim().chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(McpGoError::invalid_input(format!(
            "description is too long (limit {MAX_DESCRIPTION_CHARS} characters)"
        )));
    }

    let tunnel_id = input.tunnel_id.trim();
    if !is_valid_tunnel_id(tunnel_id) {
        return Err(McpGoError::invalid_input(format!(
            "invalid tunnel id \"{tunnel_id}\" (expected \"{TUNNEL_ID_PREFIX}<suffix>\", \
             lowercase letters, digits and underscores, at most {MAX_TUNNEL_ID_CHARS} \
             characters - create it in Platform tunnel settings)"
        )));
    }

    match input.target_kind {
        TunnelTargetKind::Stdio => {
            let Some(command) = input.command.as_deref().map(str::trim).filter(|c| !c.is_empty())
            else {
                return Err(McpGoError::invalid_input(
                    "a stdio tunnel needs the MCP server command",
                ));
            };
            if command.len() > MAX_COMMAND_CHARS {
                return Err(McpGoError::invalid_input(format!(
                    "command is too long ({} bytes, limit {MAX_COMMAND_CHARS} bytes)",
                    command.len()
                )));
            }
            if contains_double_quote(command) {
                return Err(McpGoError::invalid_input(
                    "the command must not contain double quotes (the profile joins it \
                     into one YAML command line)",
                ));
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
            if input.args.iter().any(|arg| contains_double_quote(arg)) {
                return Err(McpGoError::invalid_input(
                    "arguments must not contain double quotes (the profile joins them \
                     into one YAML command line)",
                ));
            }
            if let Some(dir) = input.working_dir.as_deref().map(str::trim) {
                if !dir.is_empty() && !Path::new(dir).is_absolute() {
                    return Err(McpGoError::invalid_input(
                        "working directory must be an absolute path",
                    ));
                }
            }
        }
        TunnelTargetKind::Http => {
            let Some(url) = input
                .server_url
                .as_deref()
                .map(str::trim)
                .filter(|u| !u.is_empty())
            else {
                return Err(McpGoError::invalid_input(
                    "an HTTP tunnel needs the MCP server URL",
                ));
            };
            let lowered = url.to_lowercase();
            if !lowered.starts_with("http://") && !lowered.starts_with("https://") {
                return Err(McpGoError::invalid_input(
                    "the MCP server URL must start with http:// or https://",
                ));
            }
            if url.len() > MAX_URL_CHARS {
                return Err(McpGoError::invalid_input(format!(
                    "server URL is too long ({} bytes, limit {MAX_URL_CHARS} bytes)",
                    url.len()
                )));
            }
        }
    }

    match input.api_key.kind {
        ApiKeySourceKind::Managed => {
            if input.api_key.reference.as_deref().map(str::trim).is_some() {
                return Err(McpGoError::invalid_input(
                    "a managed key takes no reference - set it with the Set key action",
                ));
            }
        }
        ApiKeySourceKind::Env => {
            let Some(reference) = input
                .api_key
                .reference
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty())
            else {
                return Err(McpGoError::invalid_input(
                    "an environment-variable key needs the variable name",
                ));
            };
            if !is_valid_env_key(reference) {
                return Err(McpGoError::invalid_input(
                    "invalid environment variable name (letters, digits, underscores; \
                     must start with a letter or underscore)",
                ));
            }
        }
        ApiKeySourceKind::File => {
            let Some(reference) = input
                .api_key
                .reference
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty())
            else {
                return Err(McpGoError::invalid_input(
                    "a file key needs the absolute path of the key file",
                ));
            };
            if !Path::new(reference).is_absolute() {
                return Err(McpGoError::invalid_input(
                    "the key file path must be absolute",
                ));
            }
            if reference.len() > MAX_COMMAND_CHARS {
                return Err(McpGoError::invalid_input(format!(
                    "key file path is too long ({} bytes, limit {MAX_COMMAND_CHARS} bytes)",
                    reference.len()
                )));
            }
        }
    }

    if let Some(client_path) = input.client_path.as_deref().map(str::trim) {
        if !client_path.is_empty() {
            if !Path::new(client_path).is_absolute() {
                return Err(McpGoError::invalid_input(
                    "the tunnel-client path must be absolute (e.g. C:\\Tools\\tunnel-client.exe)",
                ));
            }
            if client_path.len() > MAX_COMMAND_CHARS {
                return Err(McpGoError::invalid_input(format!(
                    "tunnel-client path is too long ({} bytes, limit {MAX_COMMAND_CHARS} bytes)",
                    client_path.len()
                )));
            }
        }
    }

    if let Some(port) = input.health_port {
        if !(1024..=65535).contains(&port) {
            return Err(McpGoError::invalid_input(
                "the health port must be between 1024 and 65535",
            ));
        }
    }

    Ok(())
}

// ----------------------------------------------------------------------------
// Persistence (the store)
// ----------------------------------------------------------------------------

/// Owns the `tunnels` directory and everything below it: definition files
/// directly inside, plus the generated `profiles\`, `secrets\`, `logs\` and
/// `bin\` subdirectories. Managed as Tauri state (through the registry) in
/// `main.rs`. Stateless besides the directory path - the files are the
/// single source of truth. `Clone` is a plain path copy: supervisor threads
/// own a clone so they can resolve paths and respawn runtimes.
#[derive(Clone)]
pub struct TunnelStore {
    dir: PathBuf,
}

impl TunnelStore {
    /// Creates the store for `dir` (`%APPDATA%\com.mcpgo.windows\tunnels`)
    /// and its subdirectories. Idempotent: existing directories are left
    /// untouched, so it is safe to run on every application start.
    pub fn new(dir: PathBuf) -> Self {
        for subdirectory in ["profiles", "secrets", "logs", "bin"] {
            if let Err(e) = fs::create_dir_all(dir.join(subdirectory)) {
                log::warn!(
                    "[tunnels] cannot create directory {}: {e}",
                    dir.join(subdirectory).display()
                );
            }
        }
        Self { dir }
    }

    /// The tunnels directory this store owns.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Absolute path of the definition file for `id`.
    pub fn definition_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Absolute path of the generated tunnel-client profile for `id`
    /// (written by `profile.rs` on every start).
    pub fn profile_path(&self, id: &str) -> PathBuf {
        self.dir.join("profiles").join(format!("{id}.yaml"))
    }

    /// Absolute path of the managed key file for `id` (only meaningful for
    /// definitions whose `api_key.kind` is `Managed`).
    pub fn secret_path(&self, id: &str) -> PathBuf {
        self.dir.join("secrets").join(format!("{id}.key"))
    }

    /// Absolute path of the runtime log file for `id`
    /// (`log.file` of the generated profile - both sides must agree).
    pub fn log_path(&self, id: &str) -> PathBuf {
        self.dir.join("logs").join(format!("{id}.log"))
    }

    /// Directory the user can drop `tunnel-client.exe` into so McpGo finds
    /// it without a PATH entry.
    pub fn bin_dir(&self) -> PathBuf {
        self.dir.join("bin")
    }

    /// Lists every parseable definition. Corrupt files are skipped with a
    /// warning instead of failing the whole view.
    pub fn list(&self) -> Vec<TunnelDefinition> {
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
            match serde_json::from_str::<TunnelDefinition>(&raw) {
                Ok(definition) => definitions.push(definition),
                Err(e) => {
                    log::warn!(
                        "[tunnels] skipping corrupt definition {}: {e}",
                        path.display()
                    );
                }
            }
        }
        definitions
    }

    /// Loads one definition by id.
    pub fn get(&self, id: &str) -> McpGoResult<TunnelDefinition> {
        if !is_valid_id(id) {
            return Err(McpGoError::invalid_input(
                "invalid tunnel id (lowercase letters, digits and inner hyphens)",
            ));
        }
        let path = self.definition_path(id);
        match fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).map_err(|e| {
                McpGoError::internal(format!("corrupt tunnel definition for {id}: {e}"))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(McpGoError::item_not_found("tunnel definition", id))
            }
            Err(e) => Err(McpGoError::io("read tunnel definition", &e)),
        }
    }

    /// Creates a definition. `requested_id` is optional: when omitted, the
    /// id is derived from the slugified name.
    pub fn create(
        &self,
        requested_id: Option<&str>,
        input: &TunnelDefinitionInput,
    ) -> McpGoResult<TunnelDefinition> {
        validate_input(input)?;
        let id = match requested_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(explicit) => {
                if !is_valid_id(explicit) {
                    return Err(McpGoError::invalid_input(format!(
                        "invalid tunnel id \"{explicit}\" (lowercase letters, digits and inner \
                         hyphens, at most {MAX_ID_CHARS} characters)"
                    )));
                }
                explicit.to_owned()
            }
            None => {
                let derived = slugify(&input.name);
                if derived.is_empty() {
                    return Err(McpGoError::invalid_input(
                        "cannot derive an id from this name - provide one explicitly",
                    ));
                }
                derived
            }
        };
        let path = self.definition_path(&id);
        if path.exists() {
            return Err(McpGoError::invalid_input(format!(
                "tunnel id \"{id}\" is already taken"
            )));
        }
        let now = time::now_epoch_millis();
        let definition = TunnelDefinition {
            id,
            name: input.name.trim().to_owned(),
            description: input.description.trim().to_owned(),
            tunnel_id: input.tunnel_id.trim().to_owned(),
            target_kind: input.target_kind,
            command: normalized(input.command.as_deref()),
            args: input.args.iter().map(|a| a.trim().to_owned()).collect(),
            working_dir: normalized(input.working_dir.as_deref()),
            server_url: normalized(input.server_url.as_deref()),
            api_key: normalized_api_key(input),
            residency: input.residency,
            autostart: input.autostart,
            keep_alive: input.keep_alive,
            client_path: normalized(input.client_path.as_deref()),
            health_port: input.health_port,
            created_at: now,
            updated_at: now,
        };
        self.write(&definition)
            .map_err(|e| McpGoError::io("write tunnel definition", &e))?;
        log::info!("[tunnels] definition {} created", definition.id);
        Ok(definition)
    }

    /// Updates an existing definition. The id is immutable (it names the
    /// definition file and the registry slot).
    pub fn update(&self, id: &str, input: &TunnelDefinitionInput) -> McpGoResult<TunnelDefinition> {
        let mut definition = self.get(id)?;
        validate_input(input)?;
        definition.name = input.name.trim().to_owned();
        definition.description = input.description.trim().to_owned();
        definition.tunnel_id = input.tunnel_id.trim().to_owned();
        definition.target_kind = input.target_kind;
        definition.command = normalized(input.command.as_deref());
        definition.args = input.args.iter().map(|a| a.trim().to_owned()).collect();
        definition.working_dir = normalized(input.working_dir.as_deref());
        definition.server_url = normalized(input.server_url.as_deref());
        definition.api_key = normalized_api_key(input);
        definition.residency = input.residency;
        definition.autostart = input.autostart;
        definition.keep_alive = input.keep_alive;
        definition.client_path = normalized(input.client_path.as_deref());
        definition.health_port = input.health_port;
        definition.updated_at = time::now_epoch_millis();
        self.write(&definition)
            .map_err(|e| McpGoError::io("write tunnel definition", &e))?;
        log::info!("[tunnels] definition {} updated", definition.id);
        Ok(definition)
    }

    /// Deletes the definition file, the generated profile and the managed
    /// secret; the log file stays behind as post-mortem evidence. The
    /// run-state guard (refusing while the runtime is active) is enforced by
    /// the registry, which owns the live slots.
    pub fn delete(&self, id: &str) -> McpGoResult<()> {
        self.get(id)?; // validates the id and the existence
        let definition_path = self.definition_path(id);
        fs::remove_file(&definition_path)
            .map_err(|e| McpGoError::io("delete tunnel definition", &e))?;
        // Best-effort cleanup: a missing profile or secret is not an error.
        let _ = fs::remove_file(self.profile_path(id));
        let _ = fs::remove_file(self.secret_path(id));
        log::info!("[tunnels] definition {id} deleted (log file kept)");
        Ok(())
    }

    /// Writes the managed key material to `<tunnels>\secrets\<id>.key`,
    /// atomically. The key is never logged and never returned to the caller.
    pub fn set_managed_key(&self, id: &str, key: &str) -> McpGoResult<()> {
        let key = key.trim();
        if key.is_empty() {
            return Err(McpGoError::invalid_input("the API key must not be empty"));
        }
        if key.len() > MAX_API_KEY_CHARS {
            return Err(McpGoError::invalid_input(format!(
                "the API key is too long ({} bytes, limit {MAX_API_KEY_CHARS} bytes)",
                key.len()
            )));
        }
        if key.contains(['\r', '\n']) {
            return Err(McpGoError::invalid_input(
                "the API key must be a single line",
            ));
        }
        self.get(id)?; // validates the id and the existence
        write_atomic(&self.secret_path(id), key.as_bytes())
            .map_err(|e| McpGoError::io("write the managed key file", &e))?;
        log::info!("[tunnels] managed key of {id} replaced (value not logged)");
        Ok(())
    }

    /// Writes one definition file atomically (tmp file + rename - the same
    /// strategy as the job_status files).
    fn write(&self, definition: &TunnelDefinition) -> std::io::Result<()> {
        let raw = serde_json::to_string_pretty(definition)
            .map_err(|e| std::io::Error::other(format!("serialize definition: {e}")))?;
        write_atomic(&self.definition_path(&definition.id), raw.as_bytes())
    }
}

/// Trims an optional string field: `None` when unset or blank.
fn normalized(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Normalizes the api-key reference of an input: trims, and forces `None`
/// for the Managed kind (the path is derived, not user-supplied).
fn normalized_api_key(input: &TunnelDefinitionInput) -> ApiKeySource {
    ApiKeySource {
        kind: input.api_key.kind,
        reference: if input.api_key.kind == ApiKeySourceKind::Managed {
            None
        } else {
            normalized(input.api_key.reference.as_deref())
        },
    }
}

/// Writes `bytes` to `path` atomically: a tmp file in the same directory
/// plus a rename (POSIX-atomic on NTFS for same-volume moves).
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    // Windows rename refuses an existing target: remove it first.
    let _ = fs::remove_file(path);
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str) -> TunnelDefinitionInput {
        TunnelDefinitionInput {
            name: name.into(),
            description: String::new(),
            tunnel_id: "tunnel_0123456789abcdef0123456789abcdef".into(),
            target_kind: TunnelTargetKind::Stdio,
            command: Some("node C:\\servers\\mcp-server.js".into()),
            args: Vec::new(),
            working_dir: None,
            server_url: None,
            api_key: ApiKeySource {
                kind: ApiKeySourceKind::Managed,
                reference: None,
            },
            residency: Residency::Global,
            autostart: false,
            keep_alive: false,
            client_path: None,
            health_port: None,
        }
    }

    #[test]
    fn tunnel_ids_are_validated() {
        assert!(is_valid_tunnel_id("tunnel_0123456789abcdef0123456789abcdef"));
        // Namespaced aliases keep the prefix and lowercase alphabet.
        assert!(is_valid_tunnel_id("tunnel_abcd_extra_suffix"));
        assert!(!is_valid_tunnel_id("0123456789abcdef0123456789abcdef"));
        assert!(!is_valid_tunnel_id("tunnel_"));
        assert!(!is_valid_tunnel_id("tunnel_ABCDEF"));
        assert!(!is_valid_tunnel_id(
            "tunnel_0123456789abcdef0123456789abcdef_extra_padding"
        ));
    }

    #[test]
    fn slugify_matches_the_service_grammar() {
        assert_eq!(slugify("My Files MCP"), "my-files-mcp");
        assert_eq!(slugify("  --Weather--  "), "weather");
        assert_eq!(slugify("Сервер"), "");
    }

    #[test]
    fn stdio_targets_need_a_command() {
        let mut payload = input("Files");
        payload.command = None;
        assert!(validate_input(&payload).is_err());

        payload.command = Some("node \"C:\\split path\\server.js\"".into());
        let error = validate_input(&payload).unwrap_err().to_string();
        assert!(error.contains("double quotes"), "unexpected: {error}");
    }

    #[test]
    fn http_targets_need_a_url() {
        let mut payload = input("Files");
        payload.target_kind = TunnelTargetKind::Http;
        payload.command = None;
        payload.server_url = Some("http://127.0.0.1:3000/mcp".into());
        assert!(validate_input(&payload).is_ok());

        payload.server_url = Some("ftp://example.com/mcp".into());
        assert!(validate_input(&payload).is_err());

        payload.server_url = None;
        assert!(validate_input(&payload).is_err());
    }

    #[test]
    fn managed_keys_take_no_reference() {
        let mut payload = input("Files");
        payload.api_key = ApiKeySource {
            kind: ApiKeySourceKind::Managed,
            reference: Some("sk-secret".into()),
        };
        assert!(validate_input(&payload).is_err());
    }

    #[test]
    fn env_keys_need_a_valid_variable_name() {
        let mut payload = input("Files");
        payload.api_key = ApiKeySource {
            kind: ApiKeySourceKind::Env,
            reference: Some("CONTROL_PLANE_API_KEY".into()),
        };
        assert!(validate_input(&payload).is_ok());

        payload.api_key.reference = Some("1BAD".into());
        assert!(validate_input(&payload).is_err());

        payload.api_key.reference = None;
        assert!(validate_input(&payload).is_err());
    }

    #[test]
    fn residency_urls_follow_the_endpoint_catalog() {
        assert_eq!(Residency::default().base_url(), "https://api.openai.com");
        assert_eq!(Residency::Eu.base_url(), "https://eu.api.openai.com");
        assert_eq!(
            serde_json::to_string(&Residency::Us).expect("serialize"),
            "\"us\""
        );
    }

    #[test]
    fn definitions_round_trip_through_json() {
        let definition = TunnelDefinition {
            id: "files".into(),
            name: "Files".into(),
            description: "Local file tools".into(),
            tunnel_id: "tunnel_0123456789abcdef0123456789abcdef".into(),
            target_kind: TunnelTargetKind::Http,
            command: None,
            args: Vec::new(),
            working_dir: None,
            server_url: Some("http://127.0.0.1:3000/mcp".into()),
            api_key: ApiKeySource {
                kind: ApiKeySourceKind::Env,
                reference: Some("CONTROL_PLANE_API_KEY".into()),
            },
            residency: Residency::Global,
            autostart: true,
            keep_alive: true,
            client_path: None,
            health_port: Some(18080),
            created_at: 1,
            updated_at: 2,
        };
        let raw = serde_json::to_string(&definition).expect("serialize");
        let parsed: TunnelDefinition = serde_json::from_str(&raw).expect("deserialize");
        assert_eq!(parsed.id, "files");
        assert_eq!(
            parsed.api_key.reference.as_deref(),
            Some("CONTROL_PLANE_API_KEY")
        );
        assert_eq!(parsed.health_port, Some(18080));
        // The wire format is camelCase and carries no key material.
        assert!(raw.contains("\"targetKind\""));
        assert!(raw.contains("\"tunnelId\""));
        assert!(!raw.to_lowercase().contains("sk-"));
    }
}

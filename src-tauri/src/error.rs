//! Unified error type for every McpGo IPC command.
//!
//! Commands return `Result<T, McpGoError>`; because `McpGoError` implements
//! `Serialize` (as its human-readable message), the React frontend receives
//! a plain string in the rejected promise - see `src/api/*.ts`, which wrap
//! the message into native `Error` objects.

use std::fmt;

use serde::Serializer;

/// Convenient alias used across command handlers and modules.
pub type McpGoResult<T> = Result<T, McpGoError>;

/// Every failure mode McpGo reports to the frontend or logs.
#[derive(Debug, Clone)]
pub enum McpGoError {
    /// A required Windows component is missing (e.g. PowerShell itself).
    ComponentMissing { component: String, detail: String },
    /// The referenced job id does not exist.
    JobNotFound { job_id: String },
    /// The referenced item of any other tracked kind (e.g. a service
    /// definition) does not exist.
    ItemNotFound { item: String, id: String },
    /// Caller-provided input failed validation.
    InvalidInput { detail: String },
    /// An elevated management operation (install / uninstall / start /
    /// stop of a Windows service) failed, e.g. the UAC prompt was
    /// declined or the SCM rejected the change.
    Operation { operation: String, detail: String },
    /// A file system operation failed (wraps the io error message).
    Io { operation: String, detail: String },
    /// An unexpected internal condition (should never escape to users).
    Internal { detail: String },
}

impl McpGoError {
    /// Builds an [`McpGoError::InvalidInput`] from any string-like value.
    pub fn invalid_input(detail: impl Into<String>) -> Self {
        McpGoError::InvalidInput { detail: detail.into() }
    }

    /// Builds an [`McpGoError::Operation`] naming the failed operation
    /// ("installing service my-id", ...) with the script's detail line.
    pub fn operation(operation: impl Into<String>, detail: impl Into<String>) -> Self {
        McpGoError::Operation { operation: operation.into(), detail: detail.into() }
    }

    /// Builds an [`McpGoError::Internal`] from any string-like value.
    pub fn internal(detail: impl Into<String>) -> Self {
        McpGoError::Internal { detail: detail.into() }
    }

    /// Builds an [`McpGoError::Io`] with the failed operation name.
    pub fn io(operation: impl Into<String>, error: &std::io::Error) -> Self {
        McpGoError::Io { operation: operation.into(), detail: error.to_string() }
    }

    /// Builds an [`McpGoError::JobNotFound`].
    pub fn job_not_found(job_id: impl Into<String>) -> Self {
        McpGoError::JobNotFound { job_id: job_id.into() }
    }

    /// Builds an [`McpGoError::ItemNotFound`] for any tracked item kind
    /// (the message names the kind, e.g. "service definition").
    pub fn item_not_found(item: impl Into<String>, id: impl Into<String>) -> Self {
        McpGoError::ItemNotFound { item: item.into(), id: id.into() }
    }
}

impl fmt::Display for McpGoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            McpGoError::ComponentMissing { component, detail } => {
                write!(f, "required component missing: {component} ({detail})")
            }
            McpGoError::JobNotFound { job_id } => {
                write!(f, "job not found: {job_id}")
            }
            McpGoError::ItemNotFound { item, id } => {
                write!(f, "not found: {item} {id}")
            }
            McpGoError::InvalidInput { detail } => {
                write!(f, "invalid input: {detail}")
            }
            McpGoError::Operation { operation, detail } => {
                write!(f, "service operation failed: {operation}: {detail}")
            }
            McpGoError::Io { operation, detail } => {
                write!(f, "operation failed: {operation}: {detail}")
            }
            McpGoError::Internal { detail } => {
                write!(f, "internal error: {detail}")
            }
        }
    }
}

impl std::error::Error for McpGoError {}

impl From<std::io::Error> for McpGoError {
    fn from(error: std::io::Error) -> Self {
        McpGoError::io("io", &error)
    }
}

/// Serialized as a plain message string: the frontend only needs to display
/// it, and a string keeps the wire format stable for future structured
/// variants.
impl serde::Serialize for McpGoError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages_are_human_readable() {
        assert_eq!(
            McpGoError::job_not_found("job_1").to_string(),
            "job not found: job_1"
        );
        assert_eq!(
            McpGoError::invalid_input("command must not be empty").to_string(),
            "invalid input: command must not be empty"
        );
    }

    #[test]
    fn item_not_found_message_names_the_item() {
        assert_eq!(
            McpGoError::item_not_found("service definition", "svc-1").to_string(),
            "not found: service definition svc-1"
        );
    }

    #[test]
    fn operation_messages_name_the_operation() {
        assert_eq!(
            McpGoError::operation("installing service weather-mcp", "UAC elevation declined")
                .to_string(),
            "service operation failed: installing service weather-mcp: UAC elevation declined"
        );
    }

    #[test]
    fn serializes_as_a_string() {
        let json = serde_json::to_string(&McpGoError::job_not_found("job_9")).expect("serializable");
        assert_eq!(json, "\"job not found: job_9\"");
    }
}

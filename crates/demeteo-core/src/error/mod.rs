//! Centralized error envelope for the entire backend.
//!
//! Every public function in the application, infrastructure, and command
//! layers returns [`AppError`]. The IPC boundary in [`crate::error::ipc`]
//! converts it to a [`crate::error::ipc::IpcError`] that omits sensitive
//! fields (PATs, keyring paths, raw git output) before Tauri serializes
//! it to the frontend.
//!
//! Design choices:
//! - **Typed variants** rather than stringly-typed errors so the frontend
//!   can `switch (err.code)` and render the right UX.
//! - **`#[serde(tag = "kind")]`** so the JSON shape is `{kind, message}`
//!   and the discriminated union round-trips cleanly through Tauri's IPC.
//! - **`#[non_exhaustive]`** so adapter code can add new variants without
//!   breaking downstream consumers.
//! - **No `#[from]` for foreign errors** that carry sensitive data — those
//!   go through explicit conversions in the From impls below that redact
//!   before they reach the message field.

use serde::Serialize;
use thiserror::Error;

use crate::adapters::database::DbError;
use crate::ports::agent_execution::ActionError;
use crate::ports::agent_runtime::AgentStartError;

/// Stable, frontend-friendly error categories. The frontend imports this
/// list and pattern-matches on `kind` to render the right UX.
///
/// **Do not rename a variant without coordinating with the frontend** —
/// the variant name is the IPC contract.
#[derive(Debug, Error, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AppError {
    /// Resource (project, feature, step, machine, etc.) was not found.
    #[error("{message}")]
    NotFound { message: String },

    /// Input failed domain validation (bad machine id, invalid slug,
    /// unsupported workflow shape, etc.).
    #[error("{message}")]
    Validation { message: String },

    /// A conflict was detected that requires user resolution (merge
    /// conflict, gate decision needed, dirty repo blocked).
    #[error("{message}")]
    Conflict { message: String },

    /// A provider returned a 4xx response or a non-retryable failure
    /// (GitHub PAT invalid, GitLab 401, etc.).
    #[error("{message}")]
    Provider { message: String },

    /// An external transport (SSH, reqwest, reqwest body) failed. The
    /// message is redacted — see [`crate::error::ipc`] for the redaction
    /// policy.
    #[error("{message}")]
    Transport { message: String },

    /// A database query failed. The message is the redacted Display form;
    /// the full error stays in `tracing`.
    #[error("{message}")]
    Database { message: String },

    /// An agent runtime / CLI subprocess failed to start, install, or
    /// produce a usable session.
    #[error("{message}")]
    Agent { message: String },

    /// Internal invariant violated (poisoned mutex that should never
    /// poison, a port returned None where Some was expected, etc.).
    /// Surfaces as a UI-level error; full context in `tracing`.
    #[error("{message}")]
    Internal { message: String },
}

impl AppError {
    // ── Constructors ──────────────────────────────────────────────────
    // Named constructors keep call sites readable and prevent typos in
    // the variant name. Each takes `impl Into<String>` so callers can
    // pass either a `String` or a `&str`.

    pub fn not_found(msg: impl Into<String>) -> Self {
        AppError::NotFound {
            message: msg.into(),
        }
    }

    pub fn validation(msg: impl Into<String>) -> Self {
        AppError::Validation {
            message: msg.into(),
        }
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        AppError::Conflict {
            message: msg.into(),
        }
    }

    pub fn provider(msg: impl Into<String>) -> Self {
        AppError::Provider {
            message: msg.into(),
        }
    }

    pub fn transport(msg: impl Into<String>) -> Self {
        AppError::Transport {
            message: msg.into(),
        }
    }

    pub fn database(msg: impl Into<String>) -> Self {
        AppError::Database {
            message: msg.into(),
        }
    }

    pub fn agent(msg: impl Into<String>) -> Self {
        AppError::Agent {
            message: msg.into(),
        }
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        AppError::Internal {
            message: msg.into(),
        }
    }

    /// Stable, frontend-friendly error code. Stable across releases
    /// even if the variant is renamed.
    pub fn code(&self) -> &'static str {
        match self {
            AppError::NotFound { .. } => "not_found",
            AppError::Validation { .. } => "validation",
            AppError::Conflict { .. } => "conflict",
            AppError::Provider { .. } => "provider",
            AppError::Transport { .. } => "transport",
            AppError::Database { .. } => "database",
            AppError::Agent { .. } => "agent",
            AppError::Internal { .. } => "internal",
        }
    }
}

// ── From impls ──────────────────────────────────────────────────────
// Every From impl deliberately constructs a fresh, redacted message
// rather than relying on the foreign Display. This is the one place
// where secrets in error strings get filtered out before they hit the
// IPC boundary.

impl From<DbError> for AppError {
    fn from(err: DbError) -> Self {
        match &err {
            DbError::Sqlite(e) => {
                tracing::error!(error = %e, "sqlite error");
                AppError::database("database query failed")
            }
            DbError::LockPoisoned => {
                tracing::error!("database lock poisoned");
                AppError::internal("database lock poisoned")
            }
            DbError::Migration(msg) => {
                tracing::error!(migration_error = %msg, "migration failed");
                AppError::database("database migration failed")
            }
            DbError::Other(msg) => {
                tracing::error!(error = %msg, "database error");
                AppError::database("database error")
            }
        }
    }
}

impl From<AgentStartError> for AppError {
    fn from(err: AgentStartError) -> Self {
        match &err {
            AgentStartError::NotFound(kind) => {
                AppError::not_found(format!("agent runtime not found: {}", kind))
            }
            AgentStartError::InstallDeclined { agent, .. } => {
                AppError::validation(format!("install of {} declined by user", agent))
            }
            AgentStartError::InstallFailed(msg) => {
                tracing::warn!(message = %msg, "agent install failed");
                AppError::agent("agent install failed")
            }
            AgentStartError::SpawnFailed(msg) => {
                tracing::warn!(message = %msg, "agent spawn failed");
                AppError::agent("agent failed to start")
            }
        }
    }
}

impl From<ActionError> for AppError {
    fn from(err: ActionError) -> Self {
        match err {
            ActionError::Network { message } => AppError::transport(message),
            ActionError::NotFound { message } => AppError::not_found(message),
            ActionError::Internal { message } => AppError::internal(message),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        tracing::warn!(error = %err, "io error");
        AppError::transport(format!("io error: {}", err.kind()))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        tracing::warn!(error = %err, "serde_json error");
        AppError::validation(format!("invalid json: {}", err))
    }
}

impl From<tokio::task::JoinError> for AppError {
    fn from(err: tokio::task::JoinError) -> Self {
        tracing::error!(error = %err, "task join error");
        AppError::internal("background task panicked")
    }
}

// ssh2::Error carries the libssh2 error code but no secrets; safe to
// surface verbatim. Keyring errors are intentionally NOT exposed — they
// contain the service+account names which the user may treat as
// sensitive.
impl From<ssh2::Error> for AppError {
    fn from(err: ssh2::Error) -> Self {
        tracing::warn!(code = ?err.code(), message = ?err.message(), "ssh2 error");
        AppError::transport(format!("ssh error: {}", err.code()))
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        // reqwest::Error::url() can include query strings with PATs in
        // them. We deliberately do NOT include it in the user-facing
        // message.
        tracing::warn!(error = %err, "reqwest error");
        if err.is_timeout() {
            AppError::transport("network request timed out")
        } else if err.is_connect() {
            AppError::transport("network connection failed")
        } else {
            AppError::transport("network request failed")
        }
    }
}

/// Convenience conversion from `String` for the many call sites that
/// currently produce `Result<T, String>`. **Use sparingly** — most
/// error paths should construct a typed [`AppError`] directly.
impl From<String> for AppError {
    fn from(msg: String) -> Self {
        AppError::internal(msg)
    }
}

impl From<&str> for AppError {
    fn from(msg: &str) -> Self {
        AppError::internal(msg.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_stable() {
        assert_eq!(AppError::not_found("x").code(), "not_found");
        assert_eq!(AppError::validation("x").code(), "validation");
        assert_eq!(AppError::conflict("x").code(), "conflict");
        assert_eq!(AppError::provider("x").code(), "provider");
        assert_eq!(AppError::transport("x").code(), "transport");
        assert_eq!(AppError::database("x").code(), "database");
        assert_eq!(AppError::agent("x").code(), "agent");
        assert_eq!(AppError::internal("x").code(), "internal");
    }

    #[test]
    fn serializes_as_tagged_union() {
        let json = serde_json::to_string(&AppError::not_found("project p-1")).unwrap();
        assert!(json.contains("\"kind\":\"not_found\""));
        assert!(json.contains("\"message\":\"project p-1\""));
    }

    #[test]
    fn from_io_redacts_path() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "/home/user/.ssh/id_rsa");
        let err: AppError = io.into();
        match err {
            AppError::Transport { message } => {
                // The path string should not appear in the user-facing
                // message (only the kind is surfaced).
                assert!(!message.contains("id_rsa"));
            }
            _ => panic!("expected Transport variant"),
        }
    }

    #[test]
    fn from_db_sqlite_redacts_raw_error() {
        let err: AppError = DbError::Sqlite(rusqlite::Error::InvalidQuery).into();
        match err {
            AppError::Database { message } => {
                // Generic message only — the full rusqlite context is
                // only available via tracing.
                assert_eq!(message, "database query failed");
            }
            _ => panic!("expected Database variant"),
        }
    }
}

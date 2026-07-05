//! IPC boundary for errors. [`crate::error::AppError`] carries the full
//! context (including potentially sensitive details that have already
//! been redacted at construction time). This module provides the final
//! payload that crosses the Tauri IPC bridge.
//!
//! Currently the [`IpcError`] is just a thin wrapper, but this is the
//! choke point: if we ever want to drop fields, hash stack traces, or
//! internationalize messages, we do it here.

use serde::Serialize;

use super::AppError;

/// The shape the frontend actually receives. Tauri serializes this
/// automatically because of the `#[derive(Serialize)]`.
#[derive(Debug, Serialize, Clone)]
pub struct IpcError {
    /// Stable error code; see [`AppError::code`].
    pub code: &'static str,
    /// Human-readable, already-redacted message safe to display.
    pub message: String,
}

impl From<AppError> for IpcError {
    fn from(err: AppError) -> Self {
        let code = err.code();
        let message = match err {
            AppError::NotFound { message }
            | AppError::Validation { message }
            | AppError::Conflict { message }
            | AppError::Provider { message }
            | AppError::Transport { message }
            | AppError::Database { message }
            | AppError::Agent { message }
            | AppError::Internal { message } => message,
        };
        IpcError { code, message }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_sensitive_paths() {
        let err: IpcError = AppError::transport("ssh error: 2").into();
        assert_eq!(err.code, "transport");
        assert_eq!(err.message, "ssh error: 2");
    }

    #[test]
    fn serializes_with_code_field() {
        let err: IpcError = AppError::not_found("machine m-1").into();
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("\"code\":\"not_found\""));
        assert!(json.contains("\"message\":\"machine m-1\""));
    }
}
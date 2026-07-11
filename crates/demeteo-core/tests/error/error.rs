// Tests extracted from `crates/demeteo-core/src/error/mod.rs` (mirrored-tests convention). `super` = that module.

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

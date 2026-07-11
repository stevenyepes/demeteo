// Tests extracted from `crates/demeteo-core/src/error/ipc.rs` (mirrored-tests convention). `super` = that module.

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

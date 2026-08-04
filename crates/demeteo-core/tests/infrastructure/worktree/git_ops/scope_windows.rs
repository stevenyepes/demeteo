use super::GitOpsHelper;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::local::execution::LocalSubprocessAdapter;
use crate::ports::db::AppSettingsRepository;
use std::path::PathBuf;
use std::sync::Arc;

fn helper() -> GitOpsHelper {
    let db = Arc::new(
        SqliteAdapter::new(rusqlite::Connection::open_in_memory().expect("in-memory database"))
            .expect("database adapter"),
    ) as Arc<dyn AppSettingsRepository>;
    GitOpsHelper::new(db, Arc::new(LocalSubprocessAdapter::new()))
}

#[tokio::test]
async fn windows_acl_scope_blocks_protected_writes_and_restores_them() {
    let root = std::env::temp_dir().join(format!("demeteo acl scope {}", std::process::id()));
    let protected = root.join("src").join("main.rs");
    let allowed = root.join("artifacts").join("report.md");
    std::fs::create_dir_all(protected.parent().expect("protected parent"))
        .expect("protected directory");
    std::fs::create_dir_all(allowed.parent().expect("allowed parent")).expect("allowed directory");
    std::fs::write(&protected, "original").expect("protected file");

    let helper = helper();
    helper
        .apply_artifact_scope(None, &root.to_string_lossy(), &[PathBuf::from("artifacts")])
        .await
        .expect("ACL scope setup");

    let protected_write = std::fs::write(&protected, "blocked");
    let allowed_write = std::fs::write(&allowed, "allowed");
    helper
        .restore_artifact_scope(None, &root.to_string_lossy())
        .await
        .expect("ACL scope restore");
    let restored_write = std::fs::write(&protected, "restored");
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        protected_write.is_err(),
        "ACL fence allowed a protected write"
    );
    assert!(allowed_write.is_ok(), "ACL fence blocked an allowed write");
    assert!(
        restored_write.is_ok(),
        "ACL restore left the protected path fenced"
    );
}

use super::GitOpsHelper;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::local::execution::LocalSubprocessAdapter;
use crate::ports::db::AppSettingsRepository;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn helper() -> GitOpsHelper {
    let db = Arc::new(
        SqliteAdapter::new(rusqlite::Connection::open_in_memory().expect("in-memory database"))
            .expect("database adapter"),
    ) as Arc<dyn AppSettingsRepository>;
    GitOpsHelper::new(db, Arc::new(LocalSubprocessAdapter::new()))
}

fn worktree(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("demeteo {name} {}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).expect("source directory");
    std::fs::create_dir_all(root.join("artifacts")).expect("artifact directory");
    std::fs::write(root.join("src").join("main.rs"), "original").expect("source file");
    root
}

/// The fence has to deny every way an agent reaches a file it may not touch.
/// A mask covering the overwrite alone lets `src/main.rs` be deleted instead,
/// and a fence that only stamps what already exists lets the agent create the
/// file first.
#[tokio::test]
async fn the_fence_denies_writes_creations_and_deletes_outside_the_scope() {
    let root = worktree("fence");
    let protected = root.join("src").join("main.rs");
    let helper = helper();

    helper
        .apply_artifact_scope(None, &root.to_string_lossy(), &[PathBuf::from("artifacts")])
        .await
        .expect("fence");

    let overwrite = std::fs::write(&protected, "blocked");
    let create = std::fs::write(root.join("src").join("new.rs"), "blocked");
    let delete = std::fs::remove_file(&protected);
    let allowed = std::fs::write(root.join("artifacts").join("report.md"), "allowed");

    helper
        .restore_artifact_scope(None, &root.to_string_lossy())
        .await
        .expect("teardown");
    let after = std::fs::write(&protected, "restored");
    let _ = std::fs::remove_dir_all(&root);

    assert!(overwrite.is_err(), "the fence allowed a protected write");
    assert!(create.is_err(), "the fence allowed a protected creation");
    assert!(delete.is_err(), "the fence allowed a protected delete");
    assert!(allowed.is_ok(), "the fence blocked a declared artifact");
    assert!(after.is_ok(), "teardown left the protected path fenced");
}

/// The ACE is inheritable, so a file the agent creates *inside* a fenced
/// directory is fenced the moment it exists. Nothing walks the tree to make
/// that true, which is what lets the fence cost one call per top-level entry.
#[tokio::test]
async fn a_file_created_under_a_fenced_directory_is_fenced_too() {
    let root = worktree("inherit");
    let helper = helper();
    let writable = [PathBuf::from("artifacts")];

    helper
        .apply_artifact_scope(None, &root.to_string_lossy(), &writable)
        .await
        .expect("fence");
    std::fs::create_dir_all(root.join("artifacts").join("nested")).expect("artifact subdirectory");
    let nested = std::fs::write(root.join("artifacts").join("nested").join("out.md"), "ok");

    helper
        .restore_artifact_scope(None, &root.to_string_lossy())
        .await
        .expect("teardown");
    let _ = std::fs::remove_dir_all(&root);

    assert!(nested.is_ok(), "inheritance reached into the writable path");
}

/// Nothing the fence does may leave a file in the worktree. Such a file is a
/// path `git merge` fails on the moment the fence denies it, and one the
/// post-step diff guard has to be told to ignore — and an exemption in the
/// diff guard is a hole in the only layer that decides what reaches the
/// feature branch.
#[tokio::test]
async fn the_fence_writes_no_file_into_the_worktree() {
    let root = worktree("bookkeeping");
    let helper = helper();

    helper
        .apply_artifact_scope(None, &root.to_string_lossy(), &[PathBuf::from("artifacts")])
        .await
        .expect("fence");
    let after_fence = entry_names(&root);
    helper
        .restore_artifact_scope(None, &root.to_string_lossy())
        .await
        .expect("teardown");
    let after_teardown = entry_names(&root);
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(
        after_fence,
        vec!["artifacts".to_string(), "src".to_string()]
    );
    assert_eq!(after_teardown, after_fence);
}

/// Teardown runs on the failure path too, and a step killed mid-turn can have
/// deleted the worktree already.
#[tokio::test]
async fn teardown_on_a_worktree_that_is_gone_is_not_a_failure() {
    let root = std::env::temp_dir().join(format!("demeteo absent {}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    helper()
        .restore_artifact_scope(None, &root.to_string_lossy())
        .await
        .expect("teardown of an absent worktree");
}

fn entry_names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .expect("read worktree")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

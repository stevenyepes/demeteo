use super::super::common::*;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ExecutionPort;
use rusqlite::Connection;
use std::sync::Arc;

#[tokio::test]
async fn test_detect_worktree_strategy_local() {
    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_gitops_detect_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    // Run git init and config
    let local_exec = fresh_exec();
    let _ = local_exec
        .run_command(
            "local",
            &format!("git -C \"{}\" init -b main", temp_dir.to_string_lossy()),
        )
        .await;
    // Create mock files
    local_exec
        .write_file(
            "local",
            &format!("{}/package.json", temp_dir.to_string_lossy()),
            "{}",
        )
        .await
        .unwrap();
    local_exec
        .write_file(
            "local",
            &format!(
                "{}/.github/pull_request_template.md",
                temp_dir.to_string_lossy()
            ),
            "PR Template Content",
        )
        .await
        .unwrap();
    // Commit so HEAD branch is set
    let _ = local_exec
        .run_command(
            "local",
            &format!(
                "git -C \"{}\" config user.email \"test@demeteo.com\"",
                temp_dir.to_string_lossy()
            ),
        )
        .await;
    let _ = local_exec
        .run_command(
            "local",
            &format!(
                "git -C \"{}\" config user.name \"test\"",
                temp_dir.to_string_lossy()
            ),
        )
        .await;
    let _ = local_exec
        .run_command(
            "local",
            &format!("git -C \"{}\" add .", temp_dir.to_string_lossy()),
        )
        .await;
    let _ = local_exec
        .run_command(
            "local",
            &format!(
                "git -C \"{}\" commit -m \"Initial commit\"",
                temp_dir.to_string_lossy()
            ),
        )
        .await;

    // Initialize helper
    let conn = Connection::open_in_memory().unwrap();
    let db_adapter = Arc::new(SqliteAdapter::new(conn).unwrap()) as Arc<dyn AppSettingsRepository>;
    let git_ops = GitOpsHelper::new(db_adapter, Arc::new(local_exec));

    let strategy = git_ops
        .detect_worktree_strategy(None, &temp_dir.to_string_lossy())
        .await
        .unwrap();
    assert_eq!(strategy.default_branch, "main");
    assert_eq!(strategy.test_command, Some("npm test".to_string()));
    assert_eq!(
        strategy.pr_template,
        Some("PR Template Content".to_string())
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(temp_dir);
}

/// A polyglot repo (here: a Tauri-shaped `package.json` + `Cargo.toml`) must
/// have *every* ecosystem's suite chained into the detected test command.
/// Regression guard for the first-match-wins bug where detection returned only
/// `npm test` (or only `cargo test`), so the verifier harness ran one language's
/// suite and greenlit changes whose real gate never executed.
#[tokio::test]
async fn test_detect_worktree_strategy_polyglot_chains_all_suites() {
    let (dir, helper) = make_repo("detect_polyglot").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();
    exec.write_file("local", &format!("{repo}/package.json"), "{}")
        .await
        .unwrap();
    exec.write_file("local", &format!("{repo}/Cargo.toml"), "[package]")
        .await
        .unwrap();

    let strategy = helper.detect_worktree_strategy(None, &repo).await.unwrap();

    // Clean up before asserting so a regression doesn't leak the repo dir.
    let _ = std::fs::remove_dir_all(&dir);

    let tc = strategy
        .test_command
        .expect("polyglot repo must detect a suite");
    assert!(
        tc.contains("npm test") && tc.contains("cargo test"),
        "polyglot repo must run both the JS and Rust suites, not just the first match; got: {tc}"
    );
    // Build command must chain the same way (regression guard for the
    // first-match-wins bug that survived in build detection).
    let bc = strategy
        .build_command
        .expect("polyglot repo must detect a build command");
    assert!(
        bc.contains("npm run build") && bc.contains("cargo build"),
        "polyglot repo must build both the JS and Rust sides; got: {bc}"
    );
}

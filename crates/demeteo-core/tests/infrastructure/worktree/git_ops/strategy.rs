use super::super::common::*;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ExecutionPort;
use rusqlite::Connection;
use std::sync::Arc;

/// Write `content` at `repo/rel`, creating the parent directories.
async fn put(exec: &impl ExecutionPort, repo: &str, rel: &str, content: &str) {
    exec.write_file("local", &format!("{repo}/{rel}"), content)
        .await
        .unwrap();
}

/// A minimal `package.json` with the given `scripts` body.
fn pkg(scripts: &str) -> String {
    format!(r#"{{"name":"fixture","scripts":{{{scripts}}}}}"#)
}

/// Every command a detected strategy emits, so a single assertion can range
/// over the whole output.
fn all_commands(s: &crate::domain::models::WorktreeStrategy) -> Vec<String> {
    let mut out: Vec<String> = s
        .harnesses
        .as_ref()
        .map(|h| h.values().cloned().collect())
        .unwrap_or_default();
    out.extend(s.test_command.clone());
    out.extend(s.build_command.clone());
    out.extend(s.prepare_command.clone());
    out
}

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
    let repo = temp_dir.to_string_lossy().to_string();
    put(
        &local_exec,
        &repo,
        "package.json",
        &pkg(r#""test":"vitest run""#),
    )
    .await;
    put(
        &local_exec,
        &repo,
        ".github/pull_request_template.md",
        "PR Template Content",
    )
    .await;
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
    // One ecosystem, so tier 3 still carries the single command a workflow
    // binding `{{test_command}}` renders.
    assert_eq!(strategy.test_command, Some("npm test".to_string()));
    assert_eq!(
        strategy.pr_template,
        Some("PR Template Content".to_string())
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(temp_dir);
}

/// The Stratosbar layout, and the reason HB3 exists: a root `package.json` plus
/// a `Cargo.toml` that lives **only** under `src-tauri/`. Detection stat-ed
/// `{repo}/Cargo.toml` and nothing deeper, so the entire Rust half of the
/// project was invisible and the emitted command silently covered half the repo.
#[tokio::test]
async fn a_tauri_layout_detects_both_ecosystems_as_named_gates() {
    let (dir, helper) = make_repo("detect_tauri").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();
    put(
        &exec,
        &repo,
        "package.json",
        &pkg(r#""test":"vitest run","build":"vite build""#),
    )
    .await;
    put(&exec, &repo, "package-lock.json", "{}").await;
    put(&exec, &repo, "src-tauri/Cargo.toml", "[package]").await;

    let strategy = helper.detect_worktree_strategy(None, &repo).await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    let harnesses = strategy
        .harnesses
        .clone()
        .expect("a polyglot repo must emit named harnesses");
    assert_eq!(
        harnesses.get("js-test").map(String::as_str),
        Some("npm test")
    );
    assert_eq!(
        harnesses.get("rust-test").map(String::as_str),
        Some("(cd src-tauri && cargo test)"),
        "the Rust half lives below the root and must still be found"
    );
    // Pre-ticked, or the map is dead config (HB5's tier 2).
    assert_eq!(
        strategy.validation_gates,
        Some(vec!["js-test".to_string(), "rust-test".to_string()])
    );
    // A fresh `git worktree add` has neither `node_modules` nor `target/`.
    assert_eq!(
        strategy.prepare_command.as_deref(),
        Some("npm ci && (cd src-tauri && cargo fetch)")
    );
    // The string this task deletes rather than fixes.
    for cmd in all_commands(&strategy) {
        assert!(
            !cmd.contains("rc=") && !cmd.contains("set +e"),
            "no accumulator may survive anywhere in the output; got: {cmd}"
        );
    }
}

/// `"test": "vitest"` with no `run` never exits: since S10 that terminates at
/// the wall-clock ceiling with remediation naming watch mode — a far better
/// failure than the old hang, but still a wasted ceiling on every single run.
#[tokio::test]
async fn a_watch_mode_test_script_is_corrected_before_it_is_ever_run() {
    let (dir, helper) = make_repo("detect_watch").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();
    put(&exec, &repo, "package.json", &pkg(r#""test":"vitest""#)).await;

    let strategy = helper.detect_worktree_strategy(None, &repo).await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(strategy.test_command.as_deref(), Some("npm test -- --run"));
}

/// Fixing the root-only stat means looking below the root; it must not become a
/// tree walk. A marker two levels down belongs to a layout a human should be
/// describing in settings, and one inside `node_modules` belongs to somebody
/// else entirely — every npm dependency ships a `package.json`.
#[tokio::test]
async fn the_scan_stops_one_level_down_and_never_enters_a_dependency_tree() {
    let (dir, helper) = make_repo("detect_depth").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();
    put(&exec, &repo, "Cargo.toml", "[package]").await;
    // Depth 2 — past the bound.
    put(&exec, &repo, "packages/api/go.mod", "module x").await;
    // Depth 1, but inside a directory that is never entered.
    put(&exec, &repo, "node_modules/go.mod", "module dep").await;
    put(&exec, &repo, "target/package.json", &pkg("")).await;

    let strategy = helper.detect_worktree_strategy(None, &repo).await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        strategy.validation_gates,
        Some(vec!["rust-test".to_string()]),
        "only the root Cargo.toml is real; got {:?}",
        strategy.harnesses
    );
    let names: Vec<&String> = strategy
        .harnesses
        .as_ref()
        .map(|h| h.keys().collect())
        .unwrap_or_default();
    assert_eq!(
        names.len(),
        2,
        "only the root Cargo.toml's two gates may exist; got {names:?}"
    );
    for cmd in all_commands(&strategy) {
        assert!(
            !cmd.starts_with("go ") && !cmd.contains("npm"),
            "a marker below the depth bound or inside a skipped directory must \
             not reach the output; got: {cmd}"
        );
    }
}

/// A repository with nothing recognisable in it must say so rather than
/// guessing: an invented `npm test` is blocked at launch by HB1's preflight,
/// which is the false positive that whole module is built to avoid.
#[tokio::test]
async fn a_repo_with_no_markers_emits_nothing_rather_than_a_guess() {
    let (dir, helper) = make_repo("detect_empty").await;
    let repo = dir.to_string_lossy().to_string();

    let strategy = helper.detect_worktree_strategy(None, &repo).await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(strategy.test_command, None);
    assert_eq!(strategy.harnesses, None);
    assert_eq!(strategy.validation_gates, None);
    assert_eq!(strategy.prepare_command, None);
}

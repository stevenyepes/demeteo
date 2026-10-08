//! Publishing a sync resolution runs the *worktree's* pre-push hook.
//!
//! A pre-push hook runs in the tree the push is made from. The project this was
//! found on guards its protected branch with a tracked hook, and the clone's
//! main checkout is the one tree that hook always refuses — so a publish made
//! from the clone failed on a hook that had nothing to say about the branch
//! being pushed. The sync worktree has the resolved feature branch checked out,
//! and the same tracked hook passes there.
//!
//! Real git through [`LocalSubprocessAdapter`] and a local bare origin, no
//! network. The negative control pushes the same commit from the clone first:
//! without it a hook that never fires would pass this test just as well.
//!
//! The hook is POSIX shell, so the whole module is Unix-only.

#![cfg(unix)]

use std::sync::Arc;

use rusqlite::Connection;

use crate::adapters::database::SqliteAdapter;
use crate::adapters::git_push::push_request;
use crate::adapters::local::execution::LocalSubprocessAdapter;
use crate::application::sync_session::{publish, SyncPorts};
use crate::application::sync_turns::SyncTurns;
use crate::domain::git_push::{classify_push_failure, PushFailure};
use crate::domain::ids::FeatureId;
use crate::domain::sync_session::SyncSessionStatus;
use crate::paths::shell_escape_posix;
use crate::ports::execution::ExecutionPort;
use crate::ports::sync_session::{SyncSession, SyncSessionPort};
use crate::support::test_dir::TestDir;

const MACHINE: &str = "local";
const FEATURE_BRANCH: &str = "feature/f-1";
const HOOK_REFUSAL: &str = "PREPUSH-HOOK-REFUSED";

const PRE_PUSH: &str = "#!/bin/sh\n\
    branch=$(git rev-parse --abbrev-ref HEAD)\n\
    if [ \"$branch\" = \"main\" ]; then\n\
    echo \"PREPUSH-HOOK-REFUSED: refusing to push from $branch\"\n\
    exit 1\n\
    fi\n\
    exit 0\n";

async fn sh(exec: &dyn ExecutionPort, cmd: &str) -> String {
    exec.run_command(MACHINE, cmd)
        .await
        .unwrap_or_else(|e| panic!("`{cmd}` failed: {e}"))
}

async fn git(exec: &dyn ExecutionPort, dir: &str, args: &str) -> String {
    sh(
        exec,
        &format!("git -C {} {}", shell_escape_posix(dir), args),
    )
    .await
}

struct Fixture {
    origin: String,
    repo: String,
    worktree: String,
    head_before: String,
    resolution: String,
}

/// A clone on `main` whose `core.hooksPath` is a tracked `.githooks/` refusing
/// any push made while `main` is checked out, and a sync worktree on the
/// feature branch holding a real merge commit that origin does not have.
async fn fixture(exec: &dyn ExecutionPort, root: &str) -> Fixture {
    let origin = format!("{root}/origin.git");
    let repo = format!("{root}/repo");
    let worktree = format!("{root}/repo_wt_sync_f-1");

    sh(
        exec,
        &format!("git init --bare -b main {}", shell_escape_posix(&origin)),
    )
    .await;
    exec.create_dir_all(MACHINE, &format!("{repo}/.githooks"))
        .await
        .unwrap();
    git(exec, &repo, "init -b main").await;
    git(exec, &repo, "config user.email demeteo@local").await;
    git(exec, &repo, "config user.name demeteo").await;
    let hook = format!("{repo}/.githooks/pre-push");
    exec.write_file(MACHINE, &hook, PRE_PUSH).await.unwrap();
    sh(exec, &format!("chmod +x {}", shell_escape_posix(&hook))).await;
    exec.write_file(MACHINE, &format!("{repo}/README.md"), "# fixture\n")
        .await
        .unwrap();
    git(exec, &repo, "add -A").await;
    git(exec, &repo, "commit -m seed").await;
    git(
        exec,
        &repo,
        &format!("remote add origin {}", shell_escape_posix(&origin)),
    )
    .await;
    // Seeded before the hook is wired so the setup itself is not refused.
    git(exec, &repo, "push origin main").await;
    git(exec, &repo, "config core.hooksPath .githooks").await;

    git(
        exec,
        &repo,
        &format!(
            "worktree add -b {FEATURE_BRANCH} {} main",
            shell_escape_posix(&worktree)
        ),
    )
    .await;
    exec.write_file(MACHINE, &format!("{worktree}/feature.txt"), "feature\n")
        .await
        .unwrap();
    git(exec, &worktree, "add -A").await;
    git(exec, &worktree, "commit -m feature").await;
    let head_before = git(exec, &worktree, "rev-parse HEAD")
        .await
        .trim()
        .to_string();
    // Origin has the feature branch as it was before the sync; only the
    // resolution is missing from it.
    git(
        exec,
        &worktree,
        &format!("-c core.hooksPath=/dev/null push origin {FEATURE_BRANCH}"),
    )
    .await;

    exec.write_file(MACHINE, &format!("{repo}/main.txt"), "main moved\n")
        .await
        .unwrap();
    git(exec, &repo, "add -A").await;
    git(exec, &repo, "commit -m main-moved").await;
    git(exec, &worktree, "merge --no-edit main").await;
    let resolution = git(exec, &worktree, "rev-parse HEAD")
        .await
        .trim()
        .to_string();
    assert_ne!(resolution, head_before, "the merge must have made a commit");

    Fixture {
        origin,
        repo,
        worktree,
        head_before,
        resolution,
    }
}

async fn origin_has(exec: &dyn ExecutionPort, fx: &Fixture) -> bool {
    exec.run_command(
        MACHINE,
        &format!(
            "git -C {} merge-base --is-ancestor {} refs/heads/{FEATURE_BRANCH}",
            shell_escape_posix(&fx.origin),
            shell_escape_posix(&fx.resolution),
        ),
    )
    .await
    .is_ok()
}

#[tokio::test]
async fn publish_runs_the_sync_worktrees_pre_push_hook_not_the_clones() {
    let root = TestDir::new("demeteo-sync-publish-hook");
    let root_path = root.path().to_str().expect("utf-8 test dir").to_string();
    let exec: Arc<dyn ExecutionPort> = Arc::new(LocalSubprocessAdapter);
    let fx = fixture(&*exec, &root_path).await;

    // Negative control: the same commit pushed from the clone's `main`
    // checkout is refused by the hook, so the hook demonstrably fires here and
    // the success below is not a hook that never ran.
    let refused = exec
        .run_program(MACHINE, push_request(&fx.repo, FEATURE_BRANCH, None, None))
        .await
        .expect_err("the clone's hook must refuse a push made from main");
    assert!(
        refused.contains(HOOK_REFUSAL),
        "the failure must carry the hook's output: {refused}"
    );
    assert_eq!(classify_push_failure(&refused), PushFailure::HookFailed);
    assert!(
        !origin_has(&*exec, &fx).await,
        "a refused push must leave origin without the resolution"
    );

    let db = Arc::new(SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap());
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO projects (id, name, created_at) VALUES ('p-1', 'demeteo', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO features (id, project_id, title, status, created_at)
             VALUES ('f-1', 'p-1', 'sync me', 'completed', 0)",
            [],
        )
        .unwrap();
    }
    let feature_id = FeatureId::from("f-1".to_string());
    db.open(&SyncSession {
        feature_id: "f-1".to_string(),
        machine_id: MACHINE.to_string(),
        repo_dir: fx.repo.clone(),
        feature_branch: FEATURE_BRANCH.to_string(),
        base_branch: "main".to_string(),
        status: SyncSessionStatus::Resolved,
        worktree_path: Some(fx.worktree.clone()),
        head_before: Some(fx.head_before.clone()),
        merge_commit_sha: Some(fx.resolution.clone()),
        conflict_files: Vec::new(),
        raw_error: None,
        blocked_stage: None,
        pushed_at: None,
        attempts: 0,
        created_at: 100,
        updated_at: 100,
    })
    .unwrap();

    let sessions: Arc<dyn SyncSessionPort> = db.clone();
    let features: Arc<dyn crate::ports::db::FeatureRepository> = db.clone();
    let app_settings: Arc<dyn crate::ports::db::AppSettingsRepository> = db;
    let turns = Arc::new(SyncTurns::default());
    let published = publish(
        SyncPorts {
            sessions: &sessions,
            exec: &exec,
            features: &features,
            turns: &turns,
            app_settings: &app_settings,
        },
        &feature_id,
    )
    .await
    .expect("publish must pass the worktree's hook")
    .expect("the session exists");

    assert!(published.session.pushed_at.is_some());
    assert!(
        origin_has(&*exec, &fx).await,
        "origin's feature branch must contain the resolution commit"
    );
}

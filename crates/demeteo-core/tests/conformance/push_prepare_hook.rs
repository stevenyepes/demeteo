//! A push from the clone prepares the clone first when a `pre-push` hook will
//! judge it, and only then.
//!
//! The hook here stands in for a repository gate that needs installed
//! dependencies: it refuses unless the prepare command has left its marker.
//! The negative control pushes before preparing, so a hook that never fires
//! cannot pass this test.
//!
//! The hook is POSIX shell, so the whole module is Unix-only.

#![cfg(unix)]

use rusqlite::Connection;

use crate::adapters::database::SqliteAdapter;
use crate::adapters::git_push::{prepare_push_tree, push_request};
use crate::adapters::local::execution::LocalSubprocessAdapter;
use crate::domain::git_push::{classify_push_failure, PushFailure};
use crate::paths::shell_escape_posix;
use crate::ports::execution::ExecutionPort;
use crate::support::test_dir::TestDir;

const MACHINE: &str = "local";
const BRANCH: &str = "feature/f-1";
const MARKER: &str = "prepared.marker";
const HOOK_REFUSAL: &str = "PREPUSH-NEEDS-PREPARE";
const PREPARE: &str = "touch prepared.marker";

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
    _root: TestDir,
    origin: String,
    repo: String,
}

/// A clone with `BRANCH` checked out and one commit origin lacks. With
/// `hooked`, `core.hooksPath` names a tracked `.githooks/` whose `pre-push`
/// refuses until [`MARKER`] exists.
async fn fixture(exec: &dyn ExecutionPort, hooked: bool) -> Fixture {
    let root = TestDir::new("demeteo-push-prepare-hook");
    let root_path = root.path().to_str().expect("utf-8 test dir").to_string();
    let origin = format!("{root_path}/origin.git");
    let repo = format!("{root_path}/repo");

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
    let body = format!(
        "#!/bin/sh\n[ -f {MARKER} ] && exit 0\necho \"{HOOK_REFUSAL}: run the prepare command\"\nexit 1\n"
    );
    exec.write_file(MACHINE, &hook, &body).await.unwrap();
    sh(exec, &format!("chmod +x {}", shell_escape_posix(&hook))).await;
    exec.write_file(
        MACHINE,
        &format!("{repo}/.gitignore"),
        &format!("{MARKER}\n"),
    )
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
    git(exec, &repo, "push origin main").await;
    if hooked {
        git(exec, &repo, "config core.hooksPath .githooks").await;
    }
    git(exec, &repo, &format!("checkout -b {BRANCH}")).await;
    exec.write_file(MACHINE, &format!("{repo}/feature.txt"), "feature\n")
        .await
        .unwrap();
    git(exec, &repo, "add -A").await;
    git(exec, &repo, "commit -m feature").await;

    Fixture {
        _root: root,
        origin,
        repo,
    }
}

fn settings() -> SqliteAdapter {
    SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap()
}

async fn marker_exists(exec: &dyn ExecutionPort, fx: &Fixture) -> bool {
    exec.get_metadata(MACHINE, &format!("{}/{MARKER}", fx.repo))
        .await
        .is_ok()
}

async fn origin_has_branch(exec: &dyn ExecutionPort, fx: &Fixture) -> bool {
    exec.run_command(
        MACHINE,
        &format!(
            "git -C {} rev-parse --verify -q refs/heads/{BRANCH}",
            shell_escape_posix(&fx.origin)
        ),
    )
    .await
    .is_ok()
}

#[tokio::test]
async fn a_hooked_clone_is_prepared_before_its_push() {
    let exec = LocalSubprocessAdapter;
    let fx = fixture(&exec, true).await;

    let refused = exec
        .run_program(MACHINE, push_request(&fx.repo, BRANCH, None, None))
        .await
        .expect_err("an unprepared clone's hook must refuse the push");
    assert!(refused.contains(HOOK_REFUSAL), "{refused}");
    assert_eq!(classify_push_failure(&refused), PushFailure::HookFailed);

    prepare_push_tree(&exec, &settings(), MACHINE, &fx.repo, Some(PREPARE))
        .await
        .expect("prepare must run in the clone");
    exec.run_program(MACHINE, push_request(&fx.repo, BRANCH, None, None))
        .await
        .expect("the prepared clone must pass its own hook");
    assert!(origin_has_branch(&exec, &fx).await);
}

#[tokio::test]
async fn a_clone_without_a_hook_is_not_prepared() {
    let exec = LocalSubprocessAdapter;
    let fx = fixture(&exec, false).await;

    prepare_push_tree(&exec, &settings(), MACHINE, &fx.repo, Some(PREPARE))
        .await
        .expect("no hook is not an error");
    assert!(
        !marker_exists(&exec, &fx).await,
        "nothing judges the clone, so nothing may be installed into it"
    );
}

#[tokio::test]
async fn a_failed_prepare_refuses_the_push_and_names_the_command() {
    let exec = LocalSubprocessAdapter;
    let fx = fixture(&exec, true).await;

    let err = prepare_push_tree(&exec, &settings(), MACHINE, &fx.repo, Some("exit 3"))
        .await
        .expect_err("a failed prepare must not be pushed through");
    assert!(err.contains("`exit 3`"), "{err}");
    assert!(err.contains("pre-push hook"), "{err}");
}

// Tests extracted from `crates/demeteo-runner/src/rpc/branch.rs`
// (mirrored-tests convention). `super` = that module.
//
// Against real repositories: a bare `origin`, the runner's clone, and a second
// clone standing in for the desktop's. What is under test is what git does
// with the refs these functions write, which no double can answer.

use super::refresh_clone;
use crate::run::push_leased;
use demeteo_core::domain::push_lease::PushLease;
use demeteo_core::domain::runner_branch_refresh::BranchRefreshOutcome;
use demeteo_core::test_dir::TestDir;
use std::path::{Path, PathBuf};
use std::process::Command;

const BRANCH: &str = "demeteo/features/f-1";

/// Never consulted: every remote here is a path, which asks for no credential.
fn askpass() -> PathBuf {
    PathBuf::from("/nonexistent/askpass")
}

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(dir: &Path, file: &str, body: &str) -> String {
    std::fs::write(dir.join(file), body).expect("write the file");
    sh(dir, &["add", file]);
    sh(dir, &["commit", "-q", "-m", body]);
    sh(dir, &["rev-parse", "HEAD"])
}

struct Fixture {
    _root: TestDir,
    origin: PathBuf,
    runner: PathBuf,
    desktop: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let root = TestDir::new(&format!("demeteo_runner_branch_{tag}"));
        let origin = root.path().join("origin.git");
        let seed = root.path().join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        sh(
            root.path(),
            &["init", "-q", "--bare", "-b", "main", "origin.git"],
        );
        sh(&seed, &["init", "-q", "-b", "main"]);
        commit(&seed, "README", "base");
        sh(
            &seed,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        sh(&seed, &["push", "-q", "origin", "main"]);
        let runner = root.path().join("runner");
        let desktop = root.path().join("desktop");
        for clone in [&runner, &desktop] {
            sh(
                root.path(),
                &[
                    "clone",
                    "-q",
                    origin.to_str().unwrap(),
                    clone.to_str().unwrap(),
                ],
            );
        }
        Self {
            _root: root,
            origin,
            runner,
            desktop,
        }
    }

    /// The run's branch, committed to the way the runner leaves it: cut
    /// beside the primary checkout, not in it.
    fn runner_commit(&self, file: &str, body: &str) -> String {
        sh(&self.runner, &["checkout", "-q", "-B", BRANCH]);
        let sha = commit(&self.runner, file, body);
        sh(&self.runner, &["checkout", "-q", "main"]);
        sha
    }

    /// A sync published from the desktop's clone, with a plain push.
    fn desktop_publishes(&self, body: &str) -> String {
        sh(&self.desktop, &["fetch", "-q", "origin", BRANCH]);
        sh(
            &self.desktop,
            &["checkout", "-q", "-B", BRANCH, &format!("origin/{BRANCH}")],
        );
        let sha = commit(&self.desktop, "SYNC", body);
        sh(&self.desktop, &["push", "-q", "origin", BRANCH]);
        sha
    }

    async fn runner_push(&self) -> Result<(), String> {
        push_leased(&askpass(), self.runner.to_str().unwrap(), BRANCH, "unused").await
    }

    async fn refresh(&self, status: &str) -> Result<BranchRefreshOutcome, String> {
        refresh_clone(
            &askpass(),
            self.runner.to_str().unwrap(),
            BRANCH,
            status,
            None,
        )
        .await
    }

    fn origin_tip(&self) -> String {
        sh(&self.origin, &["rev-parse", BRANCH])
    }

    fn runner_tip(&self) -> String {
        sh(&self.runner, &["rev-parse", BRANCH])
    }

    fn runner_tracking(&self) -> String {
        sh(&self.runner, &["rev-parse", &format!("origin/{BRANCH}")])
    }
}

#[tokio::test]
async fn the_lease_passes_a_new_branch_and_every_repush_of_its_own() {
    let fx = Fixture::new("own");
    fx.runner_commit("A", "first");
    fx.runner_push()
        .await
        .expect("a branch origin lacks may be created");

    // finalize-style rewrite: the lease compares origin's tip, not ancestry.
    sh(&fx.runner, &["checkout", "-q", BRANCH]);
    sh(&fx.runner, &["commit", "-q", "--amend", "-m", "squashed"]);
    sh(&fx.runner, &["checkout", "-q", "main"]);
    fx.runner_push()
        .await
        .expect("a rewrite of its own push passes");
    fx.runner_push()
        .await
        .expect("the second push of a run agrees with the first");

    assert_eq!(fx.origin_tip(), fx.runner_tip());
}

#[tokio::test]
async fn the_lease_refuses_to_overwrite_what_another_clone_published() {
    let fx = Fixture::new("moved");
    fx.runner_commit("A", "first");
    fx.runner_push().await.expect("first push");
    let ours = fx.runner_tip();
    let published = fx.desktop_publishes("resolution");
    sh(&fx.runner, &["checkout", "-q", BRANCH]);
    sh(&fx.runner, &["commit", "-q", "--amend", "-m", "replayed"]);
    sh(&fx.runner, &["checkout", "-q", "main"]);

    let refused = fx
        .runner_push()
        .await
        .expect_err("origin moved under the runner");

    let expected = PushLease {
        branch: BRANCH.to_string(),
        expected: Some(ours),
    }
    .refusal();
    assert_eq!(refused, expected);
    assert_eq!(fx.origin_tip(), published);
}

#[tokio::test]
async fn the_lease_refuses_a_branch_origin_gained_after_the_clone() {
    let fx = Fixture::new("gained");
    sh(&fx.desktop, &["checkout", "-q", "-b", BRANCH]);
    let theirs = commit(&fx.desktop, "B", "theirs");
    sh(&fx.desktop, &["push", "-q", "origin", BRANCH]);
    fx.runner_commit("A", "ours");

    let refused = fx
        .runner_push()
        .await
        .expect_err("the branch must not exist");

    assert!(refused.contains("expected it absent"), "{refused}");
    assert_eq!(fx.origin_tip(), theirs);
}

#[tokio::test]
async fn a_refresh_moves_a_settled_branch_and_the_next_push_keeps_the_sync() {
    let fx = Fixture::new("refresh");
    let before = fx.runner_commit("A", "first");
    fx.runner_push().await.expect("first push");
    let published = fx.desktop_publishes("resolution");

    let outcome = fx.refresh("failed").await.expect("the refresh runs");

    assert_eq!(
        outcome,
        BranchRefreshOutcome::Updated {
            from: before,
            to: published.clone()
        }
    );
    assert_eq!(fx.runner_tip(), published);
    assert_eq!(fx.runner_tracking(), published);
    fx.runner_push()
        .await
        .expect("the lease now expects what origin holds");
    assert_eq!(fx.origin_tip(), published);
    assert_eq!(
        fx.refresh("failed").await,
        Ok(BranchRefreshOutcome::UpToDate { tip: published })
    );
}

#[tokio::test]
async fn a_clean_checkout_is_fast_forwarded_with_its_files() {
    let fx = Fixture::new("checkout");
    fx.runner_commit("A", "first");
    fx.runner_push().await.expect("first push");
    let published = fx.desktop_publishes("resolution");
    let wt = fx.runner.with_file_name("runner_wt");
    sh(
        &fx.runner,
        &["worktree", "add", "-q", wt.to_str().unwrap(), BRANCH],
    );

    let outcome = fx.refresh("completed").await.expect("the refresh runs");

    assert!(
        matches!(outcome, BranchRefreshOutcome::Updated { .. }),
        "{outcome:?}"
    );
    assert_eq!(sh(&wt, &["rev-parse", "HEAD"]), published);
    assert!(wt.join("SYNC").exists());
}

#[tokio::test]
async fn a_dirty_checkout_is_refused_and_the_lease_still_holds() {
    let fx = Fixture::new("dirty");
    let before = fx.runner_commit("A", "first");
    fx.runner_push().await.expect("first push");
    fx.desktop_publishes("resolution");
    let wt = fx.runner.with_file_name("runner_wt");
    sh(
        &fx.runner,
        &["worktree", "add", "-q", wt.to_str().unwrap(), BRANCH],
    );
    std::fs::write(wt.join("A"), "edited").unwrap();

    let outcome = fx.refresh("completed").await.expect("the refresh runs");

    let BranchRefreshOutcome::Refused { reason } = outcome else {
        panic!("a dirty checkout moved: {outcome:?}");
    };
    assert!(reason.contains("uncommitted changes"), "{reason}");
    assert_eq!(fx.runner_tip(), before);
    assert_eq!(fx.runner_tracking(), before);
    assert_eq!(std::fs::read_to_string(wt.join("A")).unwrap(), "edited");
}

#[tokio::test]
async fn unpushed_runner_work_is_refused() {
    let fx = Fixture::new("unpushed");
    fx.runner_commit("A", "first");
    fx.runner_push().await.expect("first push");
    let pushed = fx.runner_tip();
    fx.desktop_publishes("resolution");
    let unpushed = fx.runner_commit("C", "never pushed");

    let outcome = fx.refresh("interrupted").await.expect("the refresh runs");

    let BranchRefreshOutcome::Refused { reason } = outcome else {
        panic!("unpushed work was moved: {outcome:?}");
    };
    assert!(reason.contains("never pushed"), "{reason}");
    assert_eq!(fx.runner_tip(), unpushed);
    assert_eq!(fx.runner_tracking(), pushed);
}

#[tokio::test]
async fn a_running_run_is_refused_without_touching_the_clone() {
    let fx = Fixture::new("running");
    let before = fx.runner_commit("A", "first");
    fx.runner_push().await.expect("first push");
    fx.desktop_publishes("resolution");

    let outcome = fx.refresh("running").await.expect("the refresh answers");

    assert!(
        matches!(&outcome, BranchRefreshOutcome::Refused { reason } if reason.contains("still running")),
        "{outcome:?}"
    );
    assert_eq!(fx.runner_tip(), before);
    assert_eq!(fx.runner_tracking(), before);
}

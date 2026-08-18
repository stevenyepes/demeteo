//! Which branch a sync merges in, asserted as the ref git was asked for, and
//! which class the failure of that fetch comes back as.
//!
//! The half of a run's origin that is easiest to leave behind: the branch
//! *name* moved to the row while the merge base stayed
//! `worktree_strategy.default_branch`, and a run cut from `origin/release/2.0`
//! then merges trunk into itself — a sync that reports success while pulling
//! in every commit the release branch was deliberately without.
//!
//! Nothing here is scripted, so the fetch fails and the flow stops at its
//! first git call. That call is the whole subject — plus what the caller is
//! handed when it fails, which used to be a conflict report with no conflict
//! in it.

use std::sync::Arc;

use super::harness::{build_test_executor_in, scratch_dir, FakeNotif};
use crate::adapters::database::SqliteAdapter;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{FeatureId, ProjectId, ProviderId, RepositoryId};
use crate::domain::models::{ConflictFile, ConflictReport};
use crate::domain::sync_failure::SyncBlockedStage;
use crate::paths;
use crate::ports::db::{FeatureRepository, MergeAuditRepository, ProjectRepository};
use crate::ports::step_executor::SyncOutcomeView;

const REPO_PATH: &str = "demeteo/sync-base";

/// The git a "Sync with main" on a feature with this `origin` and
/// `diff_base_branch` issues.
async fn sync_git(
    label: &str,
    origin: FeatureOrigin,
    diff_base_branch: Option<&str>,
) -> (Vec<String>, Result<SyncOutcomeView, String>) {
    let temp_dir = scratch_dir(label);
    let exec = Arc::new(ScriptedExec::new(&[]));
    let (executor, db) =
        build_test_executor_in(temp_dir.clone(), Arc::new(FakeNotif), exec.clone()).await;
    let feature_id = seed_feature(&db, label, origin, diff_base_branch);

    let view = executor.feature_sync_impl(&feature_id).await;

    let ran = exec.programs();
    let _ = std::fs::remove_dir_all(temp_dir);
    (ran, view)
}

/// One project, one repository, one feature — the least a sync needs before it
/// can name a base branch.
fn seed_feature(
    db: &Arc<SqliteAdapter>,
    label: &str,
    origin: FeatureOrigin,
    diff_base_branch: Option<&str>,
) -> String {
    let project_id = seed_project(db, label, "local", None);
    seed_repository(db, label, &project_id);
    seed_feature_row(db, label, &project_id, origin, diff_base_branch)
}

fn seed_project(
    db: &Arc<SqliteAdapter>,
    label: &str,
    compute_type: &str,
    remote_host: Option<&str>,
) -> String {
    let project_id = format!("p-{label}");
    let projects: &dyn ProjectRepository = &**db;
    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from(project_id.clone()),
            name: label.to_string(),
            compute_type: compute_type.to_string(),
            remote_host: remote_host.map(|h| crate::domain::ids::MachineId::from(h.to_string())),
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: paths::now_ms(),
        })
        .expect("add project");
    project_id
}

fn seed_repository(db: &Arc<SqliteAdapter>, label: &str, project_id: &str) {
    let projects: &dyn ProjectRepository = &**db;
    projects
        .add_repository(crate::domain::models::Repository {
            id: RepositoryId::from(format!("r-{label}")),
            project_id: ProjectId::from(project_id.to_string()),
            provider_id: ProviderId::from("sync-base-provider"),
            repo_path: REPO_PATH.to_string(),
        })
        .expect("add repository");
}

fn seed_feature_row(
    db: &Arc<SqliteAdapter>,
    label: &str,
    project_id: &str,
    origin: FeatureOrigin,
    diff_base_branch: Option<&str>,
) -> String {
    let feature_id = format!("f-{label}");
    let features: &dyn FeatureRepository = &**db;
    features
        .add(crate::domain::models::Feature {
            id: FeatureId::from(feature_id.clone()),
            project_id: ProjectId::from(project_id.to_string()),
            workflow_id: None,
            workflow_version_id: None,
            title: "Sync base".to_string(),
            description: String::new(),
            status: "running".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            created_at: paths::now_ms(),
            agent_kind: None,
            model: None,
            effort: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin,
            diff_base_branch: diff_base_branch.map(str::to_string),
            resolved_branch: Some(format!("demeteo/features/{feature_id}")),
        })
        .expect("add feature");

    feature_id
}

fn fetched_branch(ran: &[String]) -> Option<String> {
    ran.iter()
        .find_map(|argv| argv.split(" fetch origin -- ").nth(1))
        .map(str::to_string)
}

#[tokio::test]
async fn a_run_cut_from_a_release_branch_syncs_from_that_branch() {
    let (ran, _) = sync_git(
        "sync_base_branch",
        FeatureOrigin::Branch {
            base: "release/2.0".to_string(),
        },
        None,
    )
    .await;
    assert_eq!(
        fetched_branch(&ran).as_deref(),
        Some("release/2.0"),
        "syncing from the project default merges trunk into a release branch: {ran:?}"
    );
}

#[tokio::test]
async fn a_declared_base_is_what_a_pull_request_run_syncs_from() {
    let (ran, _) = sync_git(
        "sync_base_declared",
        FeatureOrigin::Ref {
            fetch_spec: "refs/pull/9/head".to_string(),
            label: "PR #9".to_string(),
        },
        Some("release/2.0"),
    )
    .await;
    assert_eq!(
        fetched_branch(&ran).as_deref(),
        Some("release/2.0"),
        "a PR head offers no base, so the declared one is the only answer: {ran:?}"
    );
}

#[tokio::test]
async fn a_run_that_declared_nothing_still_syncs_from_the_project_default() {
    let (ran, _) = sync_git("sync_base_default", FeatureOrigin::DefaultBranch, None).await;
    assert_eq!(fetched_branch(&ran).as_deref(), Some("main"), "{ran:?}");
}

/// A fetch that never ran reached the banner as "Merge conflict in 0 file(s)"
/// beside a button that spawned an agent into a tree with no merge in it. The
/// class has to survive `git_ops` -> the merge executor -> the view, and this
/// is the only test that walks all three.
#[tokio::test]
async fn an_unreachable_origin_comes_back_blocked_rather_than_conflicted() {
    let (_, view) = sync_git("sync_blocked_fetch", FeatureOrigin::DefaultBranch, None).await;
    match view.expect("a failed fetch is an outcome, not a command error") {
        SyncOutcomeView::Blocked { stage, raw_error } => {
            assert_eq!(stage, SyncBlockedStage::Fetch);
            assert!(
                raw_error.contains("Could not fetch origin/main"),
                "git's own words are the only evidence the user gets: {raw_error}"
            );
        }
        other => panic!("a fetch that never ran rendered as {other:?}"),
    }
}

/// The audit-row spelling `adapters::merge` documents as load-bearing,
/// asserted through the real repository: an earlier conflict's worktree stays
/// the answer after a later attempt fails without ever merging.
#[tokio::test]
async fn a_blocked_attempt_does_not_hide_an_earlier_conflicts_worktree() {
    let label = "sync_blocked_shadow";
    let temp_dir = scratch_dir(label);
    let (executor, db) = build_test_executor_in(
        temp_dir.clone(),
        Arc::new(FakeNotif),
        Arc::new(ScriptedExec::new(&[])),
    )
    .await;
    let feature_id = seed_feature(&db, label, FeatureOrigin::DefaultBranch, None);
    let fid = FeatureId::from(feature_id.clone());

    let audit: &dyn MergeAuditRepository = &*db;
    let report = ConflictReport {
        source_branch: "origin/main".to_string(),
        target_branch: format!("demeteo/features/{feature_id}"),
        files: vec![ConflictFile {
            path: "README.md".to_string(),
            kind: "both-modified".to_string(),
        }],
        raw_error: "CONFLICT (content): Merge conflict in README.md".to_string(),
        detected_at: paths::now_ms() - 60_000,
        worktree_path: Some("/w/sync-earlier".to_string()),
    };
    audit
        .record_sync_outcome(
            &fid,
            &report.target_branch,
            "main",
            "conflict",
            None,
            Some(&serde_json::to_string(&report).expect("report serializes")),
            paths::now_ms() - 60_000,
        )
        .expect("record the earlier conflict");

    let _ = executor.feature_sync_impl(&feature_id).await;

    assert_eq!(
        audit
            .get_last_sync_worktree_path(&fid)
            .expect("read the last conflict"),
        Some("/w/sync-earlier".to_string()),
        "a sync that never merged must not be filed as the newest conflict"
    );
    let _ = std::fs::remove_dir_all(temp_dir);
}

/// The one failure that issues no git at all: with no `repositories` row there
/// is no directory to run `git -C` in. It has to arrive as the same class as
/// the rest, or the banner offers an agent a tree that was never located.
#[tokio::test]
async fn a_feature_with_no_repository_row_is_blocked_before_any_git() {
    let label = "sync_no_repo";
    let temp_dir = scratch_dir(label);
    let exec = Arc::new(ScriptedExec::new(&[]));
    let (executor, db) =
        build_test_executor_in(temp_dir.clone(), Arc::new(FakeNotif), exec.clone()).await;
    let project_id = seed_project(&db, label, "local", None);
    let feature_id = seed_feature_row(
        &db,
        label,
        &project_id,
        FeatureOrigin::DefaultBranch,
        Some("main"),
    );

    let view = executor.feature_sync_impl(&feature_id).await;

    match view.expect("a missing repository row is an outcome, not a command error") {
        SyncOutcomeView::Blocked { stage, raw_error } => {
            assert_eq!(stage, SyncBlockedStage::RepoContext);
            assert!(raw_error.contains("repository"), "{raw_error}");
        }
        other => panic!("a feature with no repository rendered as {other:?}"),
    }
    assert!(
        exec.programs().is_empty(),
        "nothing may be run against a repo dir that was never resolved: {:?}",
        exec.programs()
    );
    let _ = std::fs::remove_dir_all(temp_dir);
}

/// The remote half of the same stage: the row is there, and resolving the
/// machine's home — the first thing the repo dir is built from — is what fails.
#[tokio::test]
async fn a_remote_whose_repo_dir_will_not_resolve_is_blocked_too() {
    let label = "sync_remote_repo_dir";
    let temp_dir = scratch_dir(label);
    let exec = Arc::new(ScriptedExec::new(&[]));
    let (executor, db) =
        build_test_executor_in(temp_dir.clone(), Arc::new(FakeNotif), exec.clone()).await;
    let project_id = seed_project(&db, label, "remote", Some("box"));
    seed_repository(&db, label, &project_id);
    let feature_id = seed_feature_row(
        &db,
        label,
        &project_id,
        FeatureOrigin::DefaultBranch,
        Some("main"),
    );

    let view = executor.feature_sync_impl(&feature_id).await;

    match view.expect("an unresolvable repo dir is an outcome, not a command error") {
        SyncOutcomeView::Blocked { stage, .. } => assert_eq!(stage, SyncBlockedStage::RepoContext),
        other => panic!("an unresolvable remote repo dir rendered as {other:?}"),
    }
    let _ = std::fs::remove_dir_all(temp_dir);
}

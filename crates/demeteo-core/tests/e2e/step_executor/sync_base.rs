//! Which branch a sync merges in, asserted as the ref git was asked for.
//!
//! The half of a run's origin that is easiest to leave behind: the branch
//! *name* moved to the row while the merge base stayed
//! `worktree_strategy.default_branch`, and a run cut from `origin/release/2.0`
//! then merges trunk into itself — a sync that reports success while pulling
//! in every commit the release branch was deliberately without.
//!
//! Nothing here is scripted, so the fetch fails and the flow stops at its
//! first git call. That call is the whole subject.

use std::sync::Arc;

use super::harness::{build_test_executor_in, scratch_dir, FakeNotif};
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{FeatureId, ProjectId, ProviderId, RepositoryId};
use crate::paths;
use crate::ports::db::{FeatureRepository, ProjectRepository};

const REPO_PATH: &str = "demeteo/sync-base";

/// The git a "Sync with main" on a feature with this `origin` and
/// `diff_base_branch` issues.
async fn sync_git(
    label: &str,
    origin: FeatureOrigin,
    diff_base_branch: Option<&str>,
) -> Vec<String> {
    let temp_dir = scratch_dir(label);
    let project_id = format!("p-{label}");
    let feature_id = format!("f-{label}");
    let exec = Arc::new(ScriptedExec::new(&[]));
    let (executor, db) =
        build_test_executor_in(temp_dir.clone(), Arc::new(FakeNotif), exec.clone()).await;

    let projects: &dyn ProjectRepository = &*db;
    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from(project_id.clone()),
            name: label.to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: paths::now_ms(),
        })
        .expect("add project");
    projects
        .add_repository(crate::domain::models::Repository {
            id: RepositoryId::from(format!("r-{label}")),
            project_id: ProjectId::from(project_id.clone()),
            provider_id: ProviderId::from("sync-base-provider"),
            repo_path: REPO_PATH.to_string(),
        })
        .expect("add repository");

    let features: &dyn FeatureRepository = &*db;
    features
        .add(crate::domain::models::Feature {
            id: FeatureId::from(feature_id.clone()),
            project_id: ProjectId::from(project_id.clone()),
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

    let _ = executor.feature_sync_impl(&feature_id, None).await;

    let ran = exec.programs();
    let _ = std::fs::remove_dir_all(temp_dir);
    ran
}

fn fetched_branch(ran: &[String]) -> Option<String> {
    ran.iter()
        .find_map(|argv| argv.split(" fetch origin -- ").nth(1))
        .map(str::to_string)
}

#[tokio::test]
async fn a_run_cut_from_a_release_branch_syncs_from_that_branch() {
    let ran = sync_git(
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
    let ran = sync_git(
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
    let ran = sync_git("sync_base_default", FeatureOrigin::DefaultBranch, None).await;
    assert_eq!(fetched_branch(&ran).as_deref(), Some("main"), "{ran:?}");
}

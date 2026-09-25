// Where finalize's summary range starts.
// `super` = `domain::finalize::summary_base`.

use super::*;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{FeatureId, ProjectId};

fn feature(origin: FeatureOrigin, diff_base_branch: Option<&str>) -> Feature {
    Feature {
        id: FeatureId::from("f-1".to_string()),
        project_id: ProjectId::from("p-1".to_string()),
        workflow_id: None,
        workflow_version_id: None,
        title: "Fix the auth spec".to_string(),
        description: String::new(),
        status: "running".to_string(),
        total_cost: 0.0,
        duration: String::new(),
        tokens: 0,
        created_at: 0,
        agent_kind: None,
        model: None,
        effort: None,
        mr_url: None,
        mr_state: None,
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
        resolved_branch: None,
    }
}

/// A fix run launched on a same-repo pull request is cut from the PR's head
/// and measured against its target. The summary must start at the head: from
/// the target, the range holds every commit of the reviewed PR, and the agent
/// titles the fix PR after work the published commit does not contain.
#[test]
fn a_branch_run_with_a_declared_diff_base_is_summarised_from_its_cut() {
    let f = feature(
        FeatureOrigin::Branch {
            base: "pr-head".to_string(),
        },
        Some("main"),
    );
    assert_eq!(summary_base(&f, "main"), "pr-head");
}

#[test]
fn a_ref_run_is_summarised_from_the_fetched_ref_not_the_target() {
    let origin = FeatureOrigin::Ref {
        fetch_spec: "refs/pull/12/head".to_string(),
        label: "PR #12".to_string(),
    };
    let expected = origin.squash_base("main");
    let f = feature(origin, Some("main"));
    assert_eq!(summary_base(&f, "main"), expected);
    assert_ne!(summary_base(&f, "main"), "main");
}

#[test]
fn a_default_branch_run_is_summarised_from_the_default_branch_whatever_it_declares() {
    let f = feature(FeatureOrigin::DefaultBranch, Some("release/2.x"));
    assert_eq!(summary_base(&f, "main"), "main");
}

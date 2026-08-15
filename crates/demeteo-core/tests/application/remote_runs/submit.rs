// Tests for `src/application/remote_runs/submit.rs` (mirrored-tests
// convention). `super` resolves to that module.
//
// What a detached launch says about where it starts has to survive two
// separate journeys — onto the laptop's own shadow row, and over the wire —
// and the failure when it survives neither is a run that completes, pushes,
// and opens a PR whose diff is against a tree nobody chose. Nothing observes
// that but the diff, so these assert the two destinations directly.

use super::*;
use crate::domain::ids::WorkflowVersionId;
use crate::domain::models::WorkflowVersion;

fn submit_input(origin: Option<FeatureOrigin>, diff_base_branch: Option<&str>) -> SubmitInput {
    SubmitInput {
        machine_id: "runner-1".to_string(),
        project_id: "p-1".to_string(),
        workflow_id: "w-1".to_string(),
        title: "Ship it".to_string(),
        description: "A detached run".to_string(),
        agent_kind: None,
        model: None,
        effort: None,
        commit_artifacts: None,
        loop_iterations: None,
        max_budget_usd: None,
        step_overrides: None,
        staged_attachments: None,
        target_repo_id: None,
        unattended: false,
        max_cost_usd: None,
        max_wall_clock_secs: None,
        origin,
        diff_base_branch: diff_base_branch.map(str::to_string),
    }
}

fn resolved() -> ResolvedSubmit {
    ResolvedSubmit {
        project_id: ProjectId::from("p-1"),
        workflow: ResolvedWorkflow {
            id: WorkflowId::from("w-1"),
            version: WorkflowVersion {
                id: WorkflowVersionId::from("wv-1"),
                workflow_id: WorkflowId::from("w-1"),
                version: 1,
                steps_json: "[]".to_string(),
                definition_json: None,
                created_at: 1_700_000_000,
                note: None,
            },
            json: serde_json::json!({ "name": "W", "description": "", "steps": [] }),
        },
        provider: RunSpecProvider {
            kind: "github".to_string(),
            host: "github.com".to_string(),
        },
        repo_path: "demeteo/demeteo".to_string(),
        project_settings: None,
        attachments: Vec::new(),
        budget: None,
        feature_id: "f-1".to_string(),
        step_overrides: Vec::new(),
        now: 1_700_000_000,
    }
}

fn pr_head() -> FeatureOrigin {
    FeatureOrigin::Ref {
        fetch_spec: "refs/pull/42/head".to_string(),
        label: "PR #42".to_string(),
    }
}

#[test]
fn the_shadow_row_records_the_origin_the_launch_chose() {
    let input = submit_input(Some(pr_head()), Some("release/2.0"));
    let feature = resolved().shadow_feature(&input);

    assert_eq!(feature.origin, pr_head());
    assert_eq!(feature.diff_base_branch.as_deref(), Some("release/2.0"));
}

#[test]
fn the_wire_spec_carries_the_origin_the_launch_chose() {
    let input = submit_input(Some(pr_head()), Some("release/2.0"));
    let spec = resolved().run_spec(&input);

    assert_eq!(spec.origin, Some(RunOrigin::Supported(pr_head())));
    assert_eq!(spec.diff_base_branch.as_deref(), Some("release/2.0"));
}

/// The runner does not read `RunSpec::origin` directly, so the assertion
/// above is only half the claim: what matters is that the JSON it decodes
/// resolves to the same origin rather than to `RunOrigin::Unsupported`, which
/// is what an encoding the runner cannot name looks like from here.
#[test]
fn the_encoded_spec_resolves_on_the_runner_to_the_origin_that_was_sent() {
    let input = submit_input(Some(pr_head()), None);
    let wire = serde_json::to_value(resolved().run_spec(&input)).expect("spec serializes");
    let decoded: RunSpec = serde_json::from_value(wire).expect("spec round-trips");

    assert_eq!(decoded.origin_to_honour(), Ok(pr_head()));
}

/// A launch that named nothing is the pre-V41 launch, and both records have
/// to say so in their own spelling: the row stores the default branch, the
/// wire stays absent so a runner that predates the field is unaffected.
#[test]
fn naming_no_origin_submits_the_default_branch() {
    let input = submit_input(None, None);
    let resolved = resolved();

    assert_eq!(
        resolved.shadow_feature(&input).origin,
        FeatureOrigin::DefaultBranch
    );
    let spec = resolved.run_spec(&input);
    assert_eq!(spec.origin, None);
    assert_eq!(spec.origin_to_honour(), Ok(FeatureOrigin::DefaultBranch));
}

#[test]
fn the_shadow_row_and_the_wire_agree_on_where_the_run_starts() {
    let input = submit_input(
        Some(FeatureOrigin::Branch {
            base: "release/2.0".to_string(),
        }),
        None,
    );
    let resolved = resolved();

    assert_eq!(
        resolved.run_spec(&input).origin_to_honour(),
        Ok(resolved.shadow_feature(&input).origin)
    );
}

// `super` = `commands::remote_runner`. The command takes `State<'_, AppContext>`,
// which a test cannot construct, so this covers the payload it deserializes.

use super::*;
use crate::domain::feature_origin::FeatureOrigin;

fn payload() -> serde_json::Value {
    serde_json::json!({
        "machineId": "m-1",
        "projectId": "p-1",
        "workflowId": "wf-1",
        "title": "Ship it",
        "description": "",
        "agentKind": null,
        "model": null,
        "effort": null,
        "commitArtifacts": null,
        "loopIterations": null,
        "maxBudgetUsd": null,
        "stepOverrides": null,
        "stagedAttachments": null,
        "targetRepoId": "repo-1",
        "unattended": true,
        "maxCostUsd": null,
        "maxWallClockSecs": 600,
    })
}

#[test]
fn submit_args_without_origin_or_diff_base_branch_leave_both_unset() {
    let args: RemoteSubmitRunArgs = serde_json::from_value(payload()).unwrap();

    let input = args.into_submit_input("1.2.0".into());
    assert_eq!(input.origin, None);
    assert_eq!(input.diff_base_branch, None);
    assert_eq!(input.machine_id, "m-1");
    assert_eq!(input.target_repo_id.as_deref(), Some("repo-1"));
    assert!(input.unattended);
    assert_eq!(input.max_wall_clock_secs, Some(600));
    assert_eq!(input.app_version, "1.2.0");
}

#[test]
fn submit_args_read_origin_and_diff_base_branch_in_camel_case() {
    let mut json = payload();
    json["origin"] = serde_json::json!({ "kind": "branch", "base": "develop" });
    json["diffBaseBranch"] = serde_json::json!("main");

    let input = serde_json::from_value::<RemoteSubmitRunArgs>(json)
        .unwrap()
        .into_submit_input("1.2.0".into());
    assert_eq!(
        input.origin,
        Some(FeatureOrigin::Branch {
            base: "develop".into()
        })
    );
    assert_eq!(input.diff_base_branch.as_deref(), Some("main"));
}

#[test]
fn submit_args_with_every_optional_key_omitted_leave_each_unset() {
    let json = serde_json::json!({
        "machineId": "m-1",
        "projectId": "p-1",
        "workflowId": "wf-1",
        "title": "Ship it",
        "description": "",
        "unattended": false,
    });

    let input = serde_json::from_value::<RemoteSubmitRunArgs>(json)
        .unwrap()
        .into_submit_input("1.2.0".into());
    assert_eq!(input.agent_kind, None);
    assert_eq!(input.model, None);
    assert_eq!(input.effort, None);
    assert_eq!(input.commit_artifacts, None);
    assert_eq!(input.loop_iterations, None);
    assert_eq!(input.max_budget_usd, None);
    assert!(input.step_overrides.is_none());
    assert!(input.staged_attachments.is_none());
    assert_eq!(input.target_repo_id, None);
    assert_eq!(input.max_cost_usd, None);
    assert_eq!(input.max_wall_clock_secs, None);
    assert_eq!(input.origin, None);
    assert_eq!(input.diff_base_branch, None);
    assert!(!input.unattended);
}

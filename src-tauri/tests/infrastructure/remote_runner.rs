// `super` = `commands::remote_runner`. The commands take `State<'_, AppContext>`,
// which a test cannot construct, so this covers the payloads they deserialize.

use super::*;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::MachineId;
use crate::domain::run_placement::RunPlacement;

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

fn launch_request_with_machine(machine_id: serde_json::Value) -> LaunchRequest {
    let mut json = payload();
    json["machineId"] = machine_id;
    serde_json::from_value::<LaunchRunArgs>(json)
        .unwrap()
        .into_launch_request()
}

#[test]
fn launch_args_without_a_machine_launch_locally() {
    let mut json = payload();
    json.as_object_mut().unwrap().remove("machineId");
    let req = serde_json::from_value::<LaunchRunArgs>(json)
        .unwrap()
        .into_launch_request();
    assert_eq!(req.placement, RunPlacement::Local);

    assert_eq!(
        launch_request_with_machine(serde_json::Value::Null).placement,
        RunPlacement::Local
    );
}

#[test]
fn launch_args_with_a_blank_or_local_machine_launch_locally() {
    for id in ["", "   ", "local", " local "] {
        assert_eq!(
            launch_request_with_machine(serde_json::json!(id)).placement,
            RunPlacement::Local,
            "machineId {id:?}"
        );
    }
}

#[test]
fn launch_args_with_a_padded_machine_launch_detached_on_the_trimmed_id() {
    assert_eq!(
        launch_request_with_machine(serde_json::json!(" build-box ")).placement,
        RunPlacement::Detached {
            machine_id: MachineId::from("build-box")
        }
    );
}

#[test]
fn launch_args_with_a_machine_id_launch_detached_with_every_field_carried() {
    let mut json = payload();
    json["origin"] = serde_json::json!({ "kind": "branch", "base": "develop" });
    json["diffBaseBranch"] = serde_json::json!("main");
    json["maxCostUsd"] = serde_json::json!(2.5);

    let req = serde_json::from_value::<LaunchRunArgs>(json)
        .unwrap()
        .into_launch_request();
    assert_eq!(
        req.placement,
        RunPlacement::Detached {
            machine_id: MachineId::from("m-1")
        }
    );
    assert_eq!(req.launch.project_id, "p-1");
    assert_eq!(req.launch.workflow_id, "wf-1");
    assert_eq!(req.launch.title, "Ship it");
    assert_eq!(
        req.launch.origin,
        FeatureOrigin::Branch {
            base: "develop".into()
        }
    );
    assert_eq!(req.launch.diff_base_branch.as_deref(), Some("main"));
    assert_eq!(
        req.detached,
        DetachedOptions {
            target_repo_id: Some("repo-1".into()),
            unattended: Some(true),
            max_cost_usd: Some(2.5),
            max_wall_clock_secs: Some(600),
        }
    );
}

#[test]
fn launch_args_with_every_optional_key_omitted_leave_each_unset() {
    let json = serde_json::json!({
        "projectId": "p-1",
        "workflowId": "wf-1",
        "title": "Ship it",
        "description": "",
    });

    let req = serde_json::from_value::<LaunchRunArgs>(json)
        .unwrap()
        .into_launch_request();
    assert_eq!(req.placement, RunPlacement::Local);
    assert_eq!(req.detached, DetachedOptions::default());
    assert_eq!(req.launch.agent_kind, None);
    assert_eq!(req.launch.effort, None);
    assert_eq!(req.launch.max_budget_usd, None);
    assert!(req.launch.step_overrides.is_empty());
    assert!(req.launch.staged_attachments.is_empty());
    assert_eq!(req.launch.origin, FeatureOrigin::default());
}

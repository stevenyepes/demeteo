// Tests extracted from `crates/demeteo-runner/src/hub/runs.rs` (mirrored-tests convention). `super` = that module.

use super::{HubRunCredentials, RunnerHubRuns, HUB_CLIENT_ID};
use crate::test_services::Rig;
use async_trait::async_trait;
use demeteo_core::application::attachments::StagedAttachmentInput;
use demeteo_core::domain::ids::FeatureId;
use demeteo_core::domain::models::{LaunchedBy, StepConfig};
use demeteo_core::domain::run_spec::RunSpec;
use demeteo_core::ports::hub::{
    HubLaunch, HubRunControl, RefusalReason, StartFeature, StartSupport,
};
use demeteo_core::ports::step_executor::FeatureLaunch;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn the_hub_cancels_a_run_whichever_desktop_submitted_it() {
    let rig = Rig::new("runner_hub_runs_any_client");
    rig.seed_run("run-1", "f-1", "laptop-1", "running");
    rig.seed_run("run-2", "f-2", "laptop-2", "running");
    let runs = RunnerHubRuns::new(rig.svc.clone(), None);

    runs.cancel(&FeatureId::from("f-2"))
        .await
        .expect("cancelled");

    assert_eq!(rig.run("run-2").status, "cancelled");
    assert_eq!(rig.event_kinds("run-2"), ["cancelled"]);
    assert_eq!(rig.svc.creds.get("run-2"), None);
    assert_eq!(rig.run("run-1").status, "running", "only the run it named");
    assert_eq!(rig.executor.cancels(), ["f-2"]);
}

#[tokio::test]
async fn a_feature_with_no_run_here_is_not_found_and_nothing_is_stopped() {
    let rig = Rig::new("runner_hub_runs_absent");
    rig.seed_run("run-1", "f-1", "laptop-1", "running");
    let runs = RunnerHubRuns::new(rig.svc.clone(), None);

    let error = runs
        .cancel(&FeatureId::from("run-1"))
        .await
        .expect_err("a run id names no feature");

    assert!(error.to_string().contains("run-1"), "{error}");
    assert!(rig.executor.cancels().is_empty());
    assert_eq!(rig.run("run-1").status, "running");
}

#[tokio::test]
async fn a_runner_with_no_credential_seam_refuses_hub_starts() {
    let rig = Rig::new("runner_hub_runs_starts");
    assert!(matches!(
        RunnerHubRuns::new(rig.svc.clone(), None).starts(),
        StartSupport::Refused
    ));
}

/// Records every run it was asked to deliver for, and delivers nothing.
#[derive(Default)]
struct RecordingCredentials {
    delivered: Mutex<Vec<String>>,
}

#[async_trait]
impl HubRunCredentials for RecordingCredentials {
    async fn deliver(&self, run_id: &str) -> Result<(), String> {
        self.delivered.lock().unwrap().push(run_id.to_owned());
        Ok(())
    }
}

fn hub_start(agent_kind: Option<&str>) -> StartFeature {
    let steps = serde_json::to_string(&[StepConfig {
        id: "s-build".into(),
        kind: "agent".into(),
        title: "Build".into(),
        ..StepConfig::default()
    }])
    .unwrap();
    serde_json::from_value(serde_json::json!({
        "project_id": "p-1",
        "workflow_id": "w-1",
        "title": "From the Hub",
        "description": "Started from a phone",
        "agent_kind": agent_kind,
        "max_budget_cents": 250,
        "max_wall_clock_secs": 600,
        "runner": {
            "source_instance_id": "i-desktop",
            "repository": {
                "provider_kind": "github",
                "host": "github.com",
                "full_name": "acme/app"
            },
            "workflow": {
                "workflow_id": "w-1",
                "version": 3,
                "name": "Ship",
                "steps_json": steps
            }
        }
    }))
    .expect("a runner start")
}

fn hub_launch(start: StartFeature) -> HubLaunch {
    HubLaunch {
        start,
        launch: FeatureLaunch {
            staged_attachments: vec![StagedAttachmentInput {
                source_path: String::new(),
                mime: Some("text/plain".into()),
                source_filename: Some("../notes.txt".into()),
                bytes: Some(b"the brief".to_vec()),
            }],
            ..FeatureLaunch::default()
        },
    }
}

fn with_seam(rig: &Rig) -> (RunnerHubRuns, Arc<RecordingCredentials>) {
    let credentials = Arc::new(RecordingCredentials::default());
    let runs = RunnerHubRuns::new(rig.svc.clone(), Some(credentials.clone()));
    (runs, credentials)
}

fn spool_of(rig: &Rig, run_id: &str) -> std::path::PathBuf {
    rig.svc
        .ctx
        .app_data_dir
        .join("attachment-spool")
        .join(run_id)
}

#[tokio::test]
async fn a_hub_start_is_submitted_under_the_hub_owner_and_committed_before_any_feature() {
    let rig = Rig::new("runner_hub_runs_launch");
    let (runs, credentials) = with_seam(&rig);
    assert!(matches!(runs.starts(), StartSupport::Launches));
    let start = hub_start(None);
    assert_eq!(runs.resolve(&start), Ok(()));

    let launched = runs.launch(hub_launch(start)).await.expect("launched");

    let all = rig.svc.ctx.runner_runs.list().unwrap();
    assert_eq!(all.len(), 1);
    let run = &all[0];
    assert_eq!(run.owner_client_id, HUB_CLIENT_ID);
    assert_eq!(run.status, "running");
    assert_eq!(*credentials.delivered.lock().unwrap(), [run.run_id.clone()]);

    let spec: RunSpec = serde_json::from_str(&run.spec_json).unwrap();
    assert_eq!(
        spec.feature_id.as_deref(),
        Some(launched.feature_id.as_str())
    );
    assert_eq!(spec.launched_by, LaunchedBy::Hub);
    assert_eq!(spec.repo_path, "acme/app");
    let [attachment] = spec.attachments.as_slice() else {
        panic!("one attachment: {:?}", spec.attachments);
    };
    let staged = Path::new(&attachment.staged_path);
    assert_eq!(staged, spool_of(&rig, &run.run_id).join("0-notes.txt"));
    assert_eq!(std::fs::read(staged).unwrap(), b"the brief");

    assert!(rig
        .svc
        .ctx
        .features
        .get(&launched.feature_id)
        .unwrap()
        .is_none());
    assert_eq!(runs.committed_cents_since(0), Ok(250));
    assert_eq!(launched.note, None);
}

#[tokio::test]
async fn a_submit_that_fails_is_an_error_that_commits_nothing_and_leaves_no_spool() {
    let rig = Rig::new("runner_hub_runs_failed_submit");
    let (runs, credentials) = with_seam(&rig);

    runs.launch(hub_launch(hub_start(Some("no-such-agent"))))
        .await
        .expect_err("the agent is not installed");

    let all = rig.svc.ctx.runner_runs.list().unwrap();
    let [run] = all.as_slice() else {
        panic!("one run row: {all:?}");
    };
    assert_eq!(run.status, "failed");
    assert_eq!(runs.committed_cents_since(0), Ok(0));
    assert!(!spool_of(&rig, &run.run_id).exists());
    assert!(credentials.delivered.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_start_without_its_runner_part_is_refused_at_resolution() {
    let rig = Rig::new("runner_hub_runs_no_runner_part");
    let (runs, _) = with_seam(&rig);
    let mut start = hub_start(None);
    start.runner = None;
    assert_eq!(runs.resolve(&start), Err(RefusalReason::MalformedBody));
}

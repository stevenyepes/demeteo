// Tests extracted from `crates/demeteo-runner/src/hub/runs.rs` (mirrored-tests convention). `super` = that module.

use super::{HubRunCredentials, RunnerHubRuns, HUB_CLIENT_ID};
use crate::driver::{Acquire, DriverGuard};
use crate::test_services::Rig;
use async_trait::async_trait;
use demeteo_core::application::attachments::StagedAttachmentInput;
use demeteo_core::domain::hub_request::log::parse_scope;
use demeteo_core::domain::hub_request::{runner_run_spec, HubRequestLogEntry, LogKind, LogOutcome};
use demeteo_core::domain::ids::{FeatureId, StepExecutionId};
use demeteo_core::domain::models::{LaunchedBy, StepConfig};
use demeteo_core::domain::run_spec::RunSpec;
use demeteo_core::error::AppError;
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

/// Records every `(run, request, feature)` it was asked to deliver or
/// redeliver for, and delivers nothing.
#[derive(Default)]
struct RecordingCredentials {
    delivered: Mutex<Vec<(String, String, String)>>,
    redelivered: Mutex<Vec<(String, String, String)>>,
}

fn recorded(run_id: &str, request_id: &str, feature_id: &FeatureId) -> (String, String, String) {
    (
        run_id.to_owned(),
        request_id.to_owned(),
        feature_id.to_string(),
    )
}

#[async_trait]
impl HubRunCredentials for RecordingCredentials {
    async fn deliver(
        &self,
        run_id: &str,
        request_id: &str,
        feature_id: &FeatureId,
    ) -> Result<(), String> {
        self.delivered
            .lock()
            .unwrap()
            .push(recorded(run_id, request_id, feature_id));
        Ok(())
    }

    async fn redeliver(
        &self,
        run_id: &str,
        request_id: &str,
        feature_id: &FeatureId,
    ) -> Result<(), String> {
        self.redelivered
            .lock()
            .unwrap()
            .push(recorded(run_id, request_id, feature_id));
        Ok(())
    }
}

const REQUEST_ID: &str = "req-7f3a";

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
        request_id: REQUEST_ID.into(),
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
    assert_eq!(
        *credentials.delivered.lock().unwrap(),
        [(
            run.run_id.clone(),
            REQUEST_ID.to_owned(),
            launched.feature_id.to_string()
        )]
    );

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
    assert!(
        !rig.svc.creds.deliver(&run.run_id, "late".into()),
        "a refused start closes the window it opened"
    );
}

#[tokio::test]
async fn a_start_without_its_runner_part_is_refused_at_resolution() {
    let rig = Rig::new("runner_hub_runs_no_runner_part");
    let (runs, _) = with_seam(&rig);
    let mut start = hub_start(None);
    start.runner = None;
    assert_eq!(runs.resolve(&start), Err(RefusalReason::MalformedBody));
}

/// A run of `f-1` whose step failed and ended it, as `owner` submitted it,
/// with the accepted start the Hub's token is fetched under logged. The
/// driver slot is held, so the retry's re-open attaches and no pass runs.
fn seed_ended_run(rig: &Rig, owner: &str) -> DriverGuard {
    let start = hub_start(None);
    let runner = start.runner.as_deref().expect("a runner part");
    let spec = runner_run_spec(&start, runner, "f-1", Vec::new()).expect("a spec");
    let runs = &rig.svc.ctx.runner_runs;
    runs.get_or_create("run-1", &serde_json::to_string(&spec).unwrap(), owner, 1)
        .unwrap();
    runs.update_status("run-1", "failed", Some("p-1"), Some("f-1"), None, None, 2)
        .unwrap();
    rig.svc
        .ctx
        .hub_requests
        .append_log(&HubRequestLogEntry {
            request_id: REQUEST_ID.into(),
            kind: LogKind::StartFeature,
            scope: parse_scope("spend"),
            outcome: LogOutcome::Accepted,
            reason: None,
            feature_id: Some("f-1".into()),
            received_at_ms: 1,
        })
        .unwrap();
    match rig.svc.drivers.try_acquire("run-1") {
        Acquire::Driver(guard) => guard,
        Acquire::Attached => panic!("no driver was live"),
    }
}

#[tokio::test]
async fn a_hub_retry_reruns_the_step_and_fetches_a_hub_runs_token_again() {
    let rig = Rig::new("runner_hub_runs_retry");
    let _slot = seed_ended_run(&rig, HUB_CLIENT_ID);
    rig.executor.answer_retries(Ok(()));
    let (runs, credentials) = with_seam(&rig);

    runs.retry(&FeatureId::from("f-1"), &StepExecutionId::from("se-build"))
        .await
        .expect("retried");

    assert_eq!(rig.executor.retries(), ["se-build"]);
    assert_eq!(rig.event_kinds("run-1"), ["retried"]);
    assert_eq!(
        *credentials.redelivered.lock().unwrap(),
        [recorded("run-1", REQUEST_ID, &FeatureId::from("f-1"))]
    );
    assert!(credentials.delivered.lock().unwrap().is_empty());
    assert!(
        rig.svc.creds.deliver("run-1", "token".into()),
        "the window is open"
    );
}

#[tokio::test]
async fn a_hub_retry_of_a_desktop_submitted_run_fetches_no_hub_token() {
    let rig = Rig::new("runner_hub_runs_retry_desktop");
    let _slot = seed_ended_run(&rig, "laptop-1");
    rig.executor.answer_retries(Ok(()));
    let (runs, credentials) = with_seam(&rig);

    runs.retry(&FeatureId::from("f-1"), &StepExecutionId::from("se-build"))
        .await
        .expect("retried");

    assert_eq!(rig.executor.retries(), ["se-build"]);
    assert!(credentials.redelivered.lock().unwrap().is_empty());
    assert!(!rig.svc.creds.deliver("run-1", "token".into()));
}

#[tokio::test]
async fn a_retry_the_executor_declines_stays_a_validation_error_and_reopens_nothing() {
    let rig = Rig::new("runner_hub_runs_retry_declined");
    let _slot = seed_ended_run(&rig, HUB_CLIENT_ID);
    rig.executor
        .answer_retries(Err("Step 's-plan' is still running".into()));
    let (runs, credentials) = with_seam(&rig);

    let error = runs
        .retry(&FeatureId::from("f-1"), &StepExecutionId::from("se-build"))
        .await
        .expect_err("declined");

    assert!(matches!(error, AppError::Validation { .. }), "{error:?}");
    assert!(rig.event_kinds("run-1").is_empty());
    assert!(credentials.redelivered.lock().unwrap().is_empty());
    assert!(
        !rig.svc.creds.deliver("run-1", "token".into()),
        "the window is closed"
    );
    assert_eq!(rig.run("run-1").status, "failed");
}

#[tokio::test]
async fn a_retry_of_a_feature_with_no_run_here_is_not_found() {
    let rig = Rig::new("runner_hub_runs_retry_absent");
    rig.executor.answer_retries(Ok(()));
    let (runs, _) = with_seam(&rig);

    let error = runs
        .retry(
            &FeatureId::from("f-missing"),
            &StepExecutionId::from("se-build"),
        )
        .await
        .expect_err("no run");

    assert!(matches!(error, AppError::NotFound { .. }), "{error:?}");
    assert!(rig.executor.retries().is_empty());
}

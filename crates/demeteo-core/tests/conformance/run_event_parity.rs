//! Unified-event-log parity gate (P1.13, `docs/TASKS_DAG_WORKFLOWS.md`).
//!
//! The Done-when for P1.13: *a local stub run's `run_events` rows replay
//! into the same ordered story the Tauri events told*. This suite runs
//! one deterministic stub feature with the real local wiring — a
//! [`RunEventRecorder`] decorating a capturing "UI" port, exactly as
//! `src-tauri/src/lib.rs` wires the Tauri emitter — and asserts three
//! directions of parity:
//!
//! 1. every durable row is one of the live events, in order (no
//!    invented rows);
//! 2. every narrative transition the live events told is in the durable
//!    log (nothing dropped except throttled same-status telemetry);
//! 3. the `RunEventAppended` live pushes mirror the durable rows
//!    byte-for-byte and in order (the UI's `run_event` stream *is* the
//!    log).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::adapters::agent::stub_runtime::STUB_AGENT_ENV;
use crate::adapters::run_event_log::{run_event_record, RunEventRecorder};
use crate::application::{bootstrap, projects, workflows};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{FeatureId, ProviderId};
use crate::domain::models::ProviderInstance;
use crate::paths;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::state::AppContext;

const ARTIFACT_PATH: &str = "artifacts/parity-report.md";
const REPO_PATH: &str = "demeteo/run-event-parity";
const PROVIDER_ID: &str = "parity-provider";

/// Stands in for the Tauri emitter: records every event it is handed —
/// this capture *is* "the story the Tauri events told".
#[derive(Default)]
struct CapturingNotif {
    events: Mutex<Vec<DomainEvent>>,
}
impl NotificationPort for CapturingNotif {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }
}

/// One deterministic agent step (same shape as the C5 topology fixture).
fn minimal_workflow() -> serde_json::Value {
    serde_json::json!({
        "name": "Run-Event Parity",
        "description": "Single deterministic agent step for the P1.13 parity gate.",
        "steps": [
            {
                "id": "s-report",
                "kind": "agent",
                "title": "Produce report",
                "agent_kind": "stub",
                "prompt_template": format!(
                    "Produce the parity report.\n\n\
                     Feature description: {{{{feature_description}}}}\n\n\
                     @stub-write {ARTIFACT_PATH}\n"
                ),
                "capability": "artifacts",
                "allow_shell": true,
                "artifacts": [
                    {
                        "name": "parity-report",
                        "capture": { "kind": "last_write_to", "path": ARTIFACT_PATH },
                        "mode": "full"
                    }
                ],
                "on_failure": null,
                "max_iterations": 1
            }
        ]
    })
}

fn init_local_repo(workspace_dir: &Path, project_id: &str) {
    let dir = paths::repo_target_dir_local(workspace_dir, project_id, REPO_PATH);
    std::fs::create_dir_all(&dir).expect("create repo dir");
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "demeteo@local"]);
    git(&["config", "user.name", "demeteo"]);
    std::fs::write(dir.join("README.md"), "# parity fixture\n").expect("seed README");
    git(&["add", "-A"]);
    git(&["commit", "-m", "seed"]);
}

async fn poll_terminal(ctx: &AppContext, feature_id: &FeatureId) -> String {
    const MAX_WAIT: Duration = Duration::from_secs(60);
    let started = Instant::now();
    loop {
        let feature = ctx
            .features
            .get(feature_id)
            .expect("feature read")
            .expect("feature exists");
        if matches!(
            feature.status.as_str(),
            "completed" | "awaiting_mr" | "failed" | "interrupted"
        ) {
            return feature.status;
        }
        assert!(
            started.elapsed() <= MAX_WAIT,
            "feature did not settle; last status {}",
            feature.status
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// P1.16 exit criterion (PRD §9): every failure in `run_events` names
/// its failure class and the retry-policy rule that answered it. A stub
/// step that declares an artifact it never writes fails
/// deterministically (`agent_failure`, answered by the derived
/// `agent_failure.fail` rule since there is no `on_failure`), and the
/// durable log must carry a `retry_decision` row saying exactly that.
#[tokio::test]
async fn failed_run_logs_failure_class_and_policy_rule() {
    std::env::set_var(STUB_AGENT_ENV, "1");
    let tmp = std::env::temp_dir().join(format!(
        "demeteo-run-event-fail-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).expect("create app data dir");
    let ui = Arc::new(CapturingNotif::default());
    let recorder = Arc::new(RunEventRecorder::new(ui.clone()));
    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: tmp.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        recorder.clone(),
        tokio::runtime::Handle::current(),
    );
    recorder.wire(ctx.run_events.clone());

    ctx.app_settings
        .add_provider_instance(ProviderInstance {
            id: ProviderId::from("parity-fail-provider"),
            kind: "github".to_string(),
            host: "github.com".to_string(),
            username: String::new(),
            avatar_url: String::new(),
            created_at: paths::now_ms(),
        })
        .expect("register provider");
    let project = projects::create(
        &ctx,
        projects::ProjectConfig {
            name: "parity-fail".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            repos: vec![projects::RepositoryConfig {
                repo_path: REPO_PATH.to_string(),
                provider_id: "parity-fail-provider".to_string(),
            }],
        },
    )
    .expect("create project");
    init_local_repo(&ctx.workspace_dir, project.id.as_str());
    bootstrap::bootstrap_project(&ctx, project.id.0.clone())
        .await
        .expect("bootstrap project");

    // Same shape as `minimal_workflow`, but with no `@stub-write` — the
    // declared artifact is never produced, so the step fails.
    let mut wf = minimal_workflow();
    wf["steps"][0]["prompt_template"] =
        serde_json::json!("Produce the parity report, but write nothing.");
    let workflow_id = workflows::create_from_json(&ctx.workflows, &wf).expect("ingest workflow");

    let feature = ctx
        .executor
        .feature_start(
            None,
            project.id.as_str(),
            workflow_id.as_str(),
            "Parity Failing Feature",
            "Fail deterministically for the exit-gate check.",
            Some("stub"),
            None,
            None,
            None,
            None,
            None,
            vec![],
            vec![],
        )
        .await
        .expect("feature_start");
    let status = poll_terminal(&ctx, &feature.id).await;
    assert_eq!(status, "failed", "the artifact-less stub step must fail");

    let rows = ctx
        .run_events
        .list_since(feature.id.as_str(), 0)
        .expect("list run events");
    let decision = rows
        .iter()
        .find(|r| r.kind == "retry_decision")
        .expect("a failed run must log the retry decision that answered it");
    let payload: serde_json::Value =
        serde_json::from_str(decision.payload_json.as_deref().unwrap()).expect("payload json");
    assert_eq!(payload["error_class"], "agent_failure");
    assert_eq!(payload["rule_id"], "agent_failure.fail");
    assert_eq!(payload["action"], "fail");
    assert_eq!(payload["step_id"], "s-report");

    let _ = std::fs::remove_dir_all(&tmp);
}

/// The parity gate itself.
#[tokio::test]
async fn local_run_events_replay_the_live_story() {
    std::env::set_var(STUB_AGENT_ENV, "1");
    let tmp = std::env::temp_dir().join(format!(
        "demeteo-run-event-parity-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).expect("create app data dir");

    // Wire exactly like `src-tauri/src/lib.rs`: recorder decorates the UI
    // port, sink late-bound after the context exists.
    let ui = Arc::new(CapturingNotif::default());
    let recorder = Arc::new(RunEventRecorder::new(ui.clone()));
    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: tmp.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        recorder.clone(),
        tokio::runtime::Handle::current(),
    );
    recorder.wire(ctx.run_events.clone());

    ctx.app_settings
        .add_provider_instance(ProviderInstance {
            id: ProviderId::from(PROVIDER_ID),
            kind: "github".to_string(),
            host: "github.com".to_string(),
            username: String::new(),
            avatar_url: String::new(),
            created_at: paths::now_ms(),
        })
        .expect("register provider");
    let project = projects::create(
        &ctx,
        projects::ProjectConfig {
            name: "parity".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            repos: vec![projects::RepositoryConfig {
                repo_path: REPO_PATH.to_string(),
                provider_id: PROVIDER_ID.to_string(),
            }],
        },
    )
    .expect("create project");
    init_local_repo(&ctx.workspace_dir, project.id.as_str());
    bootstrap::bootstrap_project(&ctx, project.id.0.clone())
        .await
        .expect("bootstrap project");
    let workflow_id =
        workflows::create_from_json(&ctx.workflows, &minimal_workflow()).expect("ingest workflow");

    let feature = ctx
        .executor
        .feature_start(
            None,
            project.id.as_str(),
            workflow_id.as_str(),
            "Parity Feature",
            "Produce a deterministic parity report.",
            Some("stub"),
            None,
            None,
            None,
            None,
            None,
            vec![],
            vec![],
        )
        .await
        .expect("feature_start");
    let status = poll_terminal(&ctx, &feature.id).await;
    assert!(
        matches!(status.as_str(), "completed" | "awaiting_mr"),
        "stub run must succeed; got {status}"
    );

    let rows = ctx
        .run_events
        .list_since(feature.id.as_str(), 0)
        .expect("list run events");
    assert!(
        !rows.is_empty(),
        "a local run must write the unified event log"
    );
    let story = ui.events.lock().unwrap().clone();

    // ── 3. The live `run_event` pushes mirror the rows exactly. ──────
    let echoes: Vec<(String, String, i64)> = story
        .iter()
        .filter_map(|e| match e {
            DomainEvent::RunEventAppended {
                run_id,
                offset,
                event_kind,
                payload_json,
                ..
            } => {
                assert_eq!(run_id, feature.id.as_str(), "local rows key by feature id");
                Some((event_kind.clone(), payload_json.clone(), *offset))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        echoes
            .iter()
            .map(|(k, p, _)| (k.as_str(), Some(p.as_str())))
            .collect::<Vec<_>>(),
        rows.iter()
            .map(|r| (r.kind.as_str(), r.payload_json.as_deref()))
            .collect::<Vec<_>>(),
        "the pushed record stream must be byte-identical to the durable log"
    );
    assert!(
        echoes.windows(2).all(|w| w[0].2 < w[1].2),
        "offsets must be strictly increasing"
    );

    // ── 1. Every durable row is one of the live events, in order. ────
    // The recorder appends the translation of events it saw, so the row
    // sequence must be an order-preserving subsequence of the translated
    // story (throttling only ever *drops* — never reorders or invents).
    let translated: Vec<(String, String)> = story
        .iter()
        .filter_map(run_event_record)
        .map(|r| (r.kind.to_string(), r.payload.to_string()))
        .collect();
    let mut cursor = 0usize;
    for row in &rows {
        let payload = row.payload_json.as_deref().unwrap_or_default();
        let found = translated[cursor..]
            .iter()
            .position(|(k, p)| k == &row.kind && p == payload);
        match found {
            Some(i) => cursor += i + 1,
            None => panic!(
                "durable row (kind={}, payload={}) is not part of the live story \
                 (or out of order); story tail: {:?}",
                row.kind,
                payload,
                &translated[cursor..]
            ),
        }
    }

    // ── 2. Every narrative *transition* the live events told is in the
    // durable log — only same-status telemetry refreshes may be dropped.
    let mut expected_transitions: Vec<(String, String)> = Vec::new();
    for e in &story {
        if let DomainEvent::StepProgress {
            step_id, status, ..
        } = e
        {
            let key = (step_id.clone(), status.clone());
            if !expected_transitions.contains(&key) {
                expected_transitions.push(key);
            }
        }
    }
    for (step_id, status) in &expected_transitions {
        assert!(
            rows.iter().any(|r| {
                r.kind == "step_progress"
                    && r.payload_json.as_deref().is_some_and(|p| {
                        serde_json::from_str::<serde_json::Value>(p).is_ok_and(|v| {
                            v["step_id"] == *step_id.as_str() && v["status"] == *status.as_str()
                        })
                    })
            }),
            "step transition ({step_id} -> {status}) missing from the durable log"
        );
    }
    let live_feature_statuses = story
        .iter()
        .filter(|e| matches!(e, DomainEvent::FeatureStatusChanged { .. }))
        .count();
    let logged_feature_statuses = rows.iter().filter(|r| r.kind == "feature_status").count();
    assert_eq!(
        logged_feature_statuses, live_feature_statuses,
        "every feature status change must be durably logged"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

//! Tests for the `DomainEvent -> RunEvent` bridge. The throttle is pure
//! over the injected status/clock and needs no ports; the resolve+append
//! path is exercised through in-memory fakes for the two wired ports.

use super::*;
use demeteo_core::ports::run_events::RunEvent;
use demeteo_core::ports::runner_run::RunnerRun;

// ---- fakes ---------------------------------------------------------------

#[derive(Default)]
struct FakeRunEvents {
    appended: Mutex<Vec<(String, String, Option<String>)>>, // (run_id, kind, payload)
}

impl RunEventsPort for FakeRunEvents {
    fn append(
        &self,
        run_id: &str,
        kind: &str,
        payload_json: Option<&str>,
        _now: i64,
    ) -> Result<i64, String> {
        let mut v = self.appended.lock().unwrap();
        v.push((
            run_id.to_string(),
            kind.to_string(),
            payload_json.map(str::to_string),
        ));
        Ok(v.len() as i64)
    }
    fn list_since(&self, _run_id: &str, _from_offset: i64) -> Result<Vec<RunEvent>, String> {
        Ok(vec![])
    }
}

struct FakeRunnerRuns {
    rows: Vec<RunnerRun>,
}

impl FakeRunnerRuns {
    fn with(run_id: &str, feature_id: Option<&str>) -> Self {
        Self {
            rows: vec![RunnerRun {
                run_id: run_id.to_string(),
                project_id: None,
                feature_id: feature_id.map(str::to_string),
                spec_json: String::new(),
                status: "running".to_string(),
                error: None,
                created_at: 0,
                updated_at: 0,
                resume_count: 0,
                pushed_branch: None,
                owner_client_id: String::new(),
            }],
        }
    }
}

impl RunnerRunPort for FakeRunnerRuns {
    fn get_or_create(&self, _: &str, _: &str, _: &str, _: i64) -> Result<RunnerRun, String> {
        Err("unused".into())
    }
    fn update_status(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
        _: i64,
    ) -> Result<(), String> {
        Ok(())
    }
    fn get(&self, run_id: &str) -> Result<Option<RunnerRun>, String> {
        Ok(self.rows.iter().find(|r| r.run_id == run_id).cloned())
    }
    fn list(&self) -> Result<Vec<RunnerRun>, String> {
        Ok(self.rows.clone())
    }
    fn mark_all_running_interrupted(&self, _: i64) -> Result<(), String> {
        Ok(())
    }
    fn bump_resume_count(&self, _: &str) -> Result<i64, String> {
        Ok(0)
    }
    fn cancel_if_active(&self, _: &str, _: i64) -> Result<Option<RunnerRun>, String> {
        Ok(None)
    }
}

fn step_progress(feature_id: &str, step_id: &str, status: &str) -> DomainEvent {
    DomainEvent::StepProgress {
        feature_id: demeteo_core::domain::ids::FeatureId::from(feature_id.to_string()),
        step_id: step_id.to_string(),
        status: status.to_string(),
        cost_usd: Some(0.01),
        tokens: Some(1_000),
        wall_clock_secs: Some(3),
        cache_read_input_tokens: Some(0),
        cache_creation_input_tokens: Some(0),
    }
}

// ---- throttle (pure) -----------------------------------------------------

#[test]
fn throttle_drops_repeated_same_status_within_window() {
    let b = RunEventBridge::new();
    assert!(b.should_emit_progress("f1", "s-impl", "running"));
    // Immediately again, same status → suppressed.
    assert!(!b.should_emit_progress("f1", "s-impl", "running"));
}

#[test]
fn throttle_always_emits_on_status_change() {
    let b = RunEventBridge::new();
    assert!(b.should_emit_progress("f1", "s-impl", "running"));
    // A real transition is never suppressed, even inside the window.
    assert!(b.should_emit_progress("f1", "s-impl", "completed"));
}

#[test]
fn throttle_is_independent_per_step() {
    let b = RunEventBridge::new();
    assert!(b.should_emit_progress("f1", "s-a", "running"));
    // A different step's first sighting is its own transition.
    assert!(b.should_emit_progress("f1", "s-b", "running"));
}

// ---- resolve + append ----------------------------------------------------

fn wire(bridge: &RunEventBridge, runs: FakeRunnerRuns) -> Arc<FakeRunEvents> {
    let events = Arc::new(FakeRunEvents::default());
    bridge.wire(events.clone(), Arc::new(runs));
    events
}

#[test]
fn bridges_step_progress_to_owning_run() {
    let bridge = RunEventBridge::new();
    let events = wire(&bridge, FakeRunnerRuns::with("run-1", Some("feat-1")));

    bridge
        .emit(&step_progress("feat-1", "s-impl", "running"))
        .unwrap();

    let appended = events.appended.lock().unwrap();
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].0, "run-1");
    assert_eq!(appended[0].1, "step_progress");
    let payload = appended[0].2.as_deref().unwrap();
    assert!(payload.contains("\"step_id\":\"s-impl\""));
    assert!(payload.contains("\"status\":\"running\""));
}

#[test]
fn drops_event_for_unresolved_feature() {
    let bridge = RunEventBridge::new();
    // The only run row has no feature_id yet (mid-bootstrap) → unresolvable.
    let events = wire(&bridge, FakeRunnerRuns::with("run-1", None));

    bridge
        .emit(&step_progress("feat-1", "s-impl", "running"))
        .unwrap();

    assert!(events.appended.lock().unwrap().is_empty());
}

#[test]
fn unwired_bridge_drops_silently() {
    let bridge = RunEventBridge::new();
    // Never wired — must behave like the noop adapter, not panic.
    assert!(bridge
        .emit(&step_progress("feat-1", "s-impl", "running"))
        .is_ok());
}

#[test]
fn bridges_retry_exhausted_with_attempt_counts() {
    let bridge = RunEventBridge::new();
    let events = wire(&bridge, FakeRunnerRuns::with("run-1", Some("feat-1")));

    bridge
        .emit(&DomainEvent::RetryBudgetExhausted {
            feature_id: demeteo_core::domain::ids::FeatureId::from("feat-1".to_string()),
            step_id: "s-impl".to_string(),
            target_id: "s-implement".to_string(),
            attempt: 3,
            max: 3,
            reason: "verify kept failing".to_string(),
        })
        .unwrap();

    let appended = events.appended.lock().unwrap();
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].1, "retry_exhausted");
    let payload = appended[0].2.as_deref().unwrap();
    assert!(payload.contains("\"attempt\":3"));
    assert!(payload.contains("\"max\":3"));
}

// ---- coalesced step output ----------------------------------------------

fn agent_stream(feature_id: &str, step_exec_id: &str, content: &str) -> DomainEvent {
    DomainEvent::AgentStream {
        feature_id: demeteo_core::domain::ids::FeatureId::from(feature_id.to_string()),
        step_execution_id: demeteo_core::domain::ids::StepExecutionId::from(
            step_exec_id.to_string(),
        ),
        content: content.to_string(),
    }
}

#[test]
fn small_stream_deltas_are_coalesced_not_flushed_per_token() {
    let bridge = RunEventBridge::new();
    let events = wire(&bridge, FakeRunnerRuns::with("run-1", Some("feat-1")));

    // A handful of tiny deltas, each well under the size trigger and within
    // the time window → nothing written yet (they're buffered).
    for delta in ["editing ", "src/", "auth.rs"] {
        bridge.emit(&agent_stream("feat-1", "se-1", delta)).unwrap();
    }
    assert!(events.appended.lock().unwrap().is_empty());
}

#[test]
fn large_stream_burst_flushes_on_size_trigger() {
    let bridge = RunEventBridge::new();
    let events = wire(&bridge, FakeRunnerRuns::with("run-1", Some("feat-1")));

    // One delta past OUTPUT_FLUSH_BYTES (2_000) trips the size flush, and
    // past OUTPUT_CHUNK_CAP (4_000) so it's also truncated.
    let big = "x".repeat(5_000);
    bridge.emit(&agent_stream("feat-1", "se-1", &big)).unwrap();

    let appended = events.appended.lock().unwrap();
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].0, "run-1");
    assert_eq!(appended[0].1, "step_output");
    let payload = appended[0].2.as_deref().unwrap();
    assert!(payload.contains("\"step_execution_id\":\"se-1\""));
    // Truncated to the chunk cap with a marker.
    assert!(payload.contains("(truncated)"));
}

#[test]
fn feature_status_change_drains_pending_output_before_status() {
    let bridge = RunEventBridge::new();
    let events = wire(&bridge, FakeRunnerRuns::with("run-1", Some("feat-1")));

    // Buffered (below triggers), then the run goes terminal.
    bridge
        .emit(&agent_stream("feat-1", "se-1", "final summary line"))
        .unwrap();
    bridge
        .emit(&DomainEvent::FeatureStatusChanged {
            feature_id: demeteo_core::domain::ids::FeatureId::from("feat-1".to_string()),
            status: "completed".to_string(),
        })
        .unwrap();

    let appended = events.appended.lock().unwrap();
    // Output is drained first, then the status line — order matters for the
    // "output then outcome" reading in the timeline.
    assert_eq!(appended.len(), 2);
    assert_eq!(appended[0].1, "step_output");
    assert!(appended[0]
        .2
        .as_deref()
        .unwrap()
        .contains("final summary line"));
    assert_eq!(appended[1].1, "feature_status");
}

#[test]
fn whitespace_only_stream_never_emits() {
    let bridge = RunEventBridge::new();
    let events = wire(&bridge, FakeRunnerRuns::with("run-1", Some("feat-1")));

    bridge
        .emit(&agent_stream("feat-1", "se-1", "\n\n  \n"))
        .unwrap();
    bridge
        .emit(&DomainEvent::FeatureStatusChanged {
            feature_id: demeteo_core::domain::ids::FeatureId::from("feat-1".to_string()),
            status: "completed".to_string(),
        })
        .unwrap();

    // Only the status line — the blank output is dropped, not emitted empty.
    let appended = events.appended.lock().unwrap();
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].1, "feature_status");
}

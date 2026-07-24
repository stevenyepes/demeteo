// Tests extracted from `crates/demeteo-core/src/adapters/run_event_log.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::ids::{FeatureId, StepExecutionId};
use rusqlite::Connection;

/// Inner port that records every event it is handed, standing in for the
/// Tauri emitter.
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

fn feature() -> FeatureId {
    FeatureId::new("feat-1")
}

fn progress(status: &str) -> DomainEvent {
    DomainEvent::StepProgress {
        feature_id: feature(),
        step_id: "s-implement".to_string(),
        status: status.to_string(),
        cost_usd: Some(0.5),
        tokens: Some(100),
        wall_clock_secs: Some(3),
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
    }
}

fn wired_recorder() -> (Arc<RunEventRecorder>, Arc<CapturingNotif>, Arc<SqliteAdapter>) {
    let inner = Arc::new(CapturingNotif::default());
    let db = Arc::new(SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap());
    let recorder = Arc::new(RunEventRecorder::new(inner.clone()));
    recorder.wire(db.clone());
    (recorder, inner, db)
}

/// The translation covers exactly the narrative events, each keyed by
/// the owning feature and carrying the documented kind.
#[test]
fn translation_maps_narrative_events() {
    let cases: Vec<(DomainEvent, &str)> = vec![
        (progress("running"), "step_progress"),
        (
            DomainEvent::FeatureStatusChanged {
                feature_id: feature(),
                status: "completed".into(),
            },
            "feature_status",
        ),
        (
            DomainEvent::AgentSpawned {
                feature_id: feature(),
                step_execution_id: StepExecutionId::new("se-1"),
                agent_kind: "stub".into(),
                model: None,
                effort: None,
            },
            "agent_spawned",
        ),
        (
            DomainEvent::RetryDecision {
                feature_id: feature(),
                step_id: "s-validate".into(),
                error_class: "verdict".into(),
                rule_id: "verdict.redirect".into(),
                action: "redirect".into(),
                target_id: Some("s-implement".into()),
                attempt: 2,
                max: 3,
                reason: "tests failed".into(),
            },
            "retry_decision",
        ),
        (
            DomainEvent::RetryBudgetExhausted {
                feature_id: feature(),
                step_id: "s-validate".into(),
                target_id: "s-implement".into(),
                attempt: 3,
                max: 3,
                reason: "gave up".into(),
            },
            "retry_exhausted",
        ),
        (
            DomainEvent::EnvironmentNotReady {
                feature_id: feature(),
                step_id: "s-implement".into(),
                reason: "install gdk".into(),
            },
            "env_not_ready",
        ),
        (
            DomainEvent::GateRequired {
                feature_id: feature(),
                step_execution_id: StepExecutionId::new("se-2"),
            },
            "gate_required",
        ),
        (
            DomainEvent::GateDecided {
                feature_id: feature(),
                step_execution_id: StepExecutionId::new("se-2"),
                decision: "approve".into(),
                feedback: None,
            },
            "gate_decided",
        ),
        (
            DomainEvent::BootstrapProgress {
                feature_id: feature(),
                phase: "connecting".into(),
                label: "Connecting".into(),
                status: "running".into(),
                detail: None,
            },
            "bootstrap_progress",
        ),
    ];
    for (event, want_kind) in cases {
        let rec = run_event_record(&event)
            .unwrap_or_else(|| panic!("expected a record for {event:?}"));
        assert_eq!(rec.kind, want_kind);
        assert_eq!(rec.feature_id, "feat-1");
    }
}

/// Non-narrative events translate to nothing — including `AgentStream`
/// (transport-specific handling) and `RunEventAppended` (would recurse).
#[test]
fn translation_skips_non_narrative_events() {
    let skipped = vec![
        DomainEvent::AgentStream {
            feature_id: feature(),
            step_execution_id: StepExecutionId::new("se-1"),
            content: "hello".into(),
        },
        DomainEvent::RunEventAppended {
            run_id: "feat-1".into(),
            offset: 1,
            event_kind: "step_progress".into(),
            payload_json: "{}".into(),
            created_at: 0,
        },
        DomainEvent::MrMerged {
            feature_id: feature(),
            project_id: "p-1".into(),
            feature_title: "t".into(),
            mr_url: "u".into(),
        },
        DomainEvent::ConflictDetected {
            feature_id: feature(),
            subtask_id: "st-1".into(),
        },
        DomainEvent::TerminalAwaitingApproval {
            session_id: "sess-1".into(),
            label: None,
        },
    ];
    for event in skipped {
        assert!(
            run_event_record(&event).is_none(),
            "expected None for {event:?}"
        );
    }
}

/// The recorder appends rows keyed by the feature id and pushes a
/// `RunEventAppended` whose `payload_json` is byte-identical to the
/// stored row — the UI receives exactly what a poller would read.
#[test]
fn recorder_appends_and_pushes_stored_shape() {
    let (recorder, inner, db) = wired_recorder();

    recorder.emit(&progress("running")).unwrap();
    recorder
        .emit(&DomainEvent::FeatureStatusChanged {
            feature_id: feature(),
            status: "completed".into(),
        })
        .unwrap();

    let rows = db.list_since("feat-1", 0).unwrap();
    assert_eq!(
        rows.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(),
        vec!["step_progress", "feature_status"]
    );

    let events = inner.events.lock().unwrap();
    // Interleaving: append-push precedes its own original event.
    let kinds: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            DomainEvent::RunEventAppended {
                run_id,
                event_kind,
                payload_json,
                ..
            } => {
                assert_eq!(run_id, "feat-1");
                Some((event_kind.clone(), payload_json.clone()))
            }
            _ => None,
        })
        .map(|(k, p)| {
            let stored = rows.iter().find(|r| r.kind == k).unwrap();
            assert_eq!(stored.payload_json.as_deref(), Some(p.as_str()));
            k
        })
        .collect();
    assert_eq!(kinds, vec!["step_progress", "feature_status"]);
    // The original events were forwarded unchanged (legacy path intact).
    assert!(events
        .iter()
        .any(|e| matches!(e, DomainEvent::StepProgress { .. })));
    assert!(events
        .iter()
        .any(|e| matches!(e, DomainEvent::FeatureStatusChanged { .. })));
}

/// A same-status telemetry refresh inside the throttle window is dropped
/// from the durable log (but still forwarded live); a status change
/// always lands.
#[test]
fn recorder_throttles_telemetry_refreshes() {
    let (recorder, inner, db) = wired_recorder();

    recorder.emit(&progress("running")).unwrap();
    recorder.emit(&progress("running")).unwrap(); // refresh, same status
    recorder.emit(&progress("completed")).unwrap(); // transition

    let rows = db.list_since("feat-1", 0).unwrap();
    assert_eq!(rows.len(), 2, "refresh must be coalesced");
    // All three originals were still forwarded to the UI.
    let forwarded = inner
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, DomainEvent::StepProgress { .. }))
        .count();
    assert_eq!(forwarded, 3);
}

/// `AgentStream` is never appended locally — the durable transcript
/// already lives in the `messages` table.
#[test]
fn recorder_skips_agent_stream() {
    let (recorder, _inner, db) = wired_recorder();
    recorder
        .emit(&DomainEvent::AgentStream {
            feature_id: feature(),
            step_execution_id: StepExecutionId::new("se-1"),
            content: "chunk".into(),
        })
        .unwrap();
    assert!(db.list_since("feat-1", 0).unwrap().is_empty());
}

/// An un-wired recorder forwards events live and records nothing — the
/// pre-P1.13 behavior, never an error.
#[test]
fn unwired_recorder_forwards_without_recording() {
    let inner = Arc::new(CapturingNotif::default());
    let recorder = RunEventRecorder::new(inner.clone());
    recorder.emit(&progress("running")).unwrap();
    let events = inner.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], DomainEvent::StepProgress { .. }));
}

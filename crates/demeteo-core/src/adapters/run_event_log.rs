//! Unified `run_events` vocabulary + the local recorder (task P1.13,
//! PRD §5.4 "Event log").
//!
//! `run_events` (V22) is the single append-only source of truth for a
//! run's narrative on **both** transports. The remote path has written it
//! since M3.3 (`demeteo-runner`'s `RunEventBridge` translates live
//! `DomainEvent`s, keyed by `run_id`); this module gives the *local*
//! path the same behavior: [`RunEventRecorder`] decorates the app's
//! `NotificationPort`, appends the translated row keyed by **feature id**
//! (a local run has no runner run row — the feature id *is* its run id),
//! and forwards a [`DomainEvent::RunEventAppended`] push so the UI
//! receives exactly the record shape the remote path polls via
//! `stream_events`.
//!
//! # Event-kind vocabulary
//!
//! One `kind` per row, payloads produced by [`run_event_record`] (shared
//! with the runner bridge so the shapes can never drift):
//!
//! | kind                 | payload fields | meaning |
//! |----------------------|----------------|---------|
//! | `step_progress`      | `step_id`, `status`, `cost_usd`, `tokens`, `wall_clock_secs`, `cache_read_input_tokens`, `cache_creation_input_tokens` | step state transition or (throttled) cost/token sample |
//! | `feature_status`     | `status` | feature lifecycle transition |
//! | `agent_spawned`      | `step_execution_id`, `agent_kind`, `model`, `effort` | what a step's agent was actually launched with (post-clamp effort) |
//! | `retry_decision`     | `step_id`, `error_class`, `rule_id`, `action`, `target_id`, `attempt`, `max`, `reason` | the retry-policy engine's answer to a failure (P1.10 rule id; `action` ∈ `redirect \| in_place \| exhausted \| fail`) |
//! | `retry_exhausted`    | `step_id`, `target_id`, `attempt`, `max`, `reason` | user-facing alarm when a redirect budget is spent |
//! | `env_not_ready`      | `step_id`, `reason` | harness triage verdict: the machine, not the code, is broken (C6) |
//! | `gate_required`      | `step_execution_id` | a gate is waiting on a human |
//! | `gate_decided`       | `step_execution_id`, `decision`, `feedback` | the human's answer (`approve`/`reject`/`redirect`) |
//! | `bootstrap_progress` | `phase`, `label`, `status`, `detail` | feature-start sub-steps |
//! | `step_output`        | `step_execution_id`, `text` | coalesced agent output — **remote only** (the runner bridge buffers `AgentStream` deltas; locally the durable transcript already lives in the `messages` table, so the recorder skips it) |
//!
//! Plus the coarse run-lifecycle kinds `run.rs` appends by hand on the
//! runner (`submitted`, `bootstrapped`, `pushed`, `pr_opened`, …), which
//! have no local equivalent.
//!
//! Deliberately **not** recorded: `AgentStream` (see `step_output`
//! above), `PermissionRequested`/`CommandExecuted` (high volume, not
//! run-progress), `MrMerged`/`ConflictDetected` (post-run concerns),
//! `TerminalAwaitingApproval` (not feature-scoped), and
//! `RunEventAppended` itself (would recurse).
//!
//! Secret scrubbing (M7.2) happens at the `RunEventsPort::append` sink;
//! the recorder additionally scrubs *before* append so the live-pushed
//! `payload_json` is byte-identical to the stored row.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::paths;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::ports::run_events::RunEventsPort;
use crate::shared::secret_scrub::scrub_secrets;

/// Minimum spacing between two *pure telemetry* `step_progress` appends
/// for the same step. `StepProgress` fires on genuine transitions **and**
/// on every mid-turn token/cost refresh; a transition always lands, a
/// same-status refresh inside this window is dropped so the log stays a
/// readable narrative (same policy and constant as the runner bridge).
pub const PROGRESS_THROTTLE_MS: i64 = 8_000;

/// A translated event, ready to append: which run-scoped id it belongs
/// to, the stored `kind`, and the JSON payload.
pub struct RunEventRecord {
    /// The owning feature. The **local** log keys rows by this id
    /// directly; the runner bridge resolves it to its `run_id` first.
    pub feature_id: String,
    pub kind: &'static str,
    pub payload: serde_json::Value,
}

/// Pure translation: which `DomainEvent`s become durable run-event rows,
/// and with what payload. This is the single source of truth for the
/// payload shapes in the module-level table — the local recorder and the
/// runner's `RunEventBridge` both call it, so local and remote rows for
/// the same event are structurally identical.
///
/// Returns `None` for events that are not part of the durable narrative
/// (see the module docs), **including** `AgentStream`: its treatment is
/// transport-specific (buffered remotely, skipped locally), so the
/// callers own that special case.
pub fn run_event_record(event: &DomainEvent) -> Option<RunEventRecord> {
    let rec = match event {
        DomainEvent::StepProgress {
            feature_id,
            step_id,
            status,
            cost_usd,
            tokens,
            wall_clock_secs,
            cache_read_input_tokens,
            cache_creation_input_tokens,
        } => RunEventRecord {
            feature_id: feature_id.0.clone(),
            kind: "step_progress",
            payload: serde_json::json!({
                "step_id": step_id,
                "status": status,
                "cost_usd": cost_usd,
                "tokens": tokens,
                "wall_clock_secs": wall_clock_secs,
                "cache_read_input_tokens": cache_read_input_tokens,
                "cache_creation_input_tokens": cache_creation_input_tokens,
            }),
        },
        DomainEvent::FeatureStatusChanged { feature_id, status } => RunEventRecord {
            feature_id: feature_id.0.clone(),
            kind: "feature_status",
            payload: serde_json::json!({ "status": status }),
        },
        DomainEvent::AgentSpawned {
            feature_id,
            step_execution_id,
            agent_kind,
            model,
            effort,
        } => RunEventRecord {
            feature_id: feature_id.0.clone(),
            kind: "agent_spawned",
            payload: serde_json::json!({
                "step_execution_id": step_execution_id.as_str(),
                "agent_kind": agent_kind,
                "model": model,
                "effort": effort,
            }),
        },
        DomainEvent::RetryDecision {
            feature_id,
            step_id,
            error_class,
            rule_id,
            action,
            target_id,
            attempt,
            max,
            reason,
        } => RunEventRecord {
            feature_id: feature_id.0.clone(),
            kind: "retry_decision",
            payload: serde_json::json!({
                "step_id": step_id,
                "error_class": error_class,
                "rule_id": rule_id,
                "action": action,
                "target_id": target_id,
                "attempt": attempt,
                "max": max,
                "reason": reason,
            }),
        },
        DomainEvent::RetryBudgetExhausted {
            feature_id,
            step_id,
            target_id,
            attempt,
            max,
            reason,
        } => RunEventRecord {
            feature_id: feature_id.0.clone(),
            kind: "retry_exhausted",
            payload: serde_json::json!({
                "step_id": step_id,
                "target_id": target_id,
                "attempt": attempt,
                "max": max,
                "reason": reason,
            }),
        },
        DomainEvent::EnvironmentNotReady {
            feature_id,
            step_id,
            reason,
        } => RunEventRecord {
            feature_id: feature_id.0.clone(),
            kind: "env_not_ready",
            payload: serde_json::json!({ "step_id": step_id, "reason": reason }),
        },
        DomainEvent::GateRequired {
            feature_id,
            step_execution_id,
        } => RunEventRecord {
            feature_id: feature_id.0.clone(),
            kind: "gate_required",
            payload: serde_json::json!({ "step_execution_id": step_execution_id.as_str() }),
        },
        DomainEvent::GateDecided {
            feature_id,
            step_execution_id,
            decision,
            feedback,
        } => RunEventRecord {
            feature_id: feature_id.0.clone(),
            kind: "gate_decided",
            payload: serde_json::json!({
                "step_execution_id": step_execution_id.as_str(),
                "decision": decision,
                "feedback": feedback,
            }),
        },
        DomainEvent::BootstrapProgress {
            feature_id,
            phase,
            label,
            status,
            detail,
        } => RunEventRecord {
            feature_id: feature_id.0.clone(),
            kind: "bootstrap_progress",
            payload: serde_json::json!({
                "phase": phase,
                "label": label,
                "status": status,
                "detail": detail,
            }),
        },
        _ => return None,
    };
    Some(rec)
}

/// Per-step throttle state: the last status appended and when, so a
/// telemetry-only refresh inside [`PROGRESS_THROTTLE_MS`] can be dropped
/// while a real status change always lands.
struct LastProgress {
    status: String,
    at_ms: i64,
}

/// `NotificationPort` decorator that makes the **local** transport write
/// the unified event log (see module docs). Wraps the real UI emitter:
/// every event is forwarded unchanged (the ad-hoc `step_progress` /
/// `agent_stream` Tauri events survive until P2.6 deletes the split
/// path); the append-worthy ones are *also* written to `run_events` and
/// pushed as [`DomainEvent::RunEventAppended`].
///
/// **Late binding**, same pattern as the runner's `RunEventBridge`: the
/// recorder is the `NotificationPort` handed *into* `build_core_context`,
/// but the `RunEventsPort` it writes is constructed *inside* that call —
/// so it starts un-wired and is [`wire`](Self::wire)d immediately after
/// the context is built. An event that fires before wiring (a startup
/// driver resume) is forwarded live but not recorded, exactly what the
/// pre-P1.13 local path did for every event.
pub struct RunEventRecorder {
    inner: Arc<dyn NotificationPort>,
    sink: OnceLock<Arc<dyn RunEventsPort>>,
    /// `(feature_id, step_id) -> last appended progress`, for the throttle.
    progress_state: Mutex<HashMap<(String, String), LastProgress>>,
}

impl RunEventRecorder {
    pub fn new(inner: Arc<dyn NotificationPort>) -> Self {
        Self {
            inner,
            sink: OnceLock::new(),
            progress_state: Mutex::new(HashMap::new()),
        }
    }

    /// Inject the live `run_events` port once the `AppContext` exists.
    /// A second call is ignored (`OnceLock`).
    pub fn wire(&self, run_events: Arc<dyn RunEventsPort>) {
        let _ = self.sink.set(run_events);
    }

    /// Throttle gate for `step_progress`: `true` (append) on a status
    /// change or once the per-step window elapsed, `false` (drop) for a
    /// telemetry-only refresh inside the window.
    fn should_record_progress(&self, feature_id: &str, step_id: &str, status: &str) -> bool {
        let now = paths::now_ms();
        let Ok(mut state) = self.progress_state.lock() else {
            return true; // never suppress on a poisoned lock
        };
        let key = (feature_id.to_string(), step_id.to_string());
        match state.get(&key) {
            Some(last) if last.status == status && now - last.at_ms < PROGRESS_THROTTLE_MS => false,
            _ => {
                state.insert(
                    key,
                    LastProgress {
                        status: status.to_string(),
                        at_ms: now,
                    },
                );
                true
            }
        }
    }

    /// Append one translated record and push the stored shape live.
    /// Best-effort: a sink error is logged and swallowed — recording the
    /// run must never break the run.
    fn record(&self, rec: RunEventRecord) {
        let Some(sink) = self.sink.get() else {
            return;
        };
        // Scrub *before* the append (which scrubs again, idempotently) so
        // the pushed payload is byte-identical to the stored row.
        let payload_json = scrub_secrets(&rec.payload.to_string()).into_owned();
        let now = paths::now_ms();
        match sink.append(&rec.feature_id, rec.kind, Some(&payload_json), now) {
            Ok(offset) => {
                let _ = self.inner.emit(&DomainEvent::RunEventAppended {
                    run_id: rec.feature_id,
                    offset,
                    event_kind: rec.kind.to_string(),
                    payload_json,
                    created_at: now,
                });
            }
            Err(e) => {
                tracing::warn!(
                    kind = rec.kind,
                    feature_id = %rec.feature_id,
                    error = %e,
                    "failed to append local run event"
                );
            }
        }
    }
}

impl NotificationPort for RunEventRecorder {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        if let Some(rec) = run_event_record(event) {
            let allowed = match event {
                DomainEvent::StepProgress {
                    feature_id,
                    step_id,
                    status,
                    ..
                } => self.should_record_progress(feature_id.as_str(), step_id, status),
                _ => true,
            };
            if allowed {
                self.record(rec);
            }
        }
        // Forward the original event unchanged, after the append — the
        // durable row precedes its own live echo, and the legacy ad-hoc
        // events keep flowing until P2.6.
        self.inner.emit(event)
    }
}

#[cfg(test)]
#[path = "../../tests/infrastructure/run_event_log.rs"]
mod tests;

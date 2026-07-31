//! Bridge the engine's live `DomainEvent` stream into the run event log
//! (proposal: "more visibility on remote unattended runs").
//!
//! The desktop app wires a `TauriNotificationAdapter` that forwards every
//! `DomainEvent` (step progress, retries, env-not-ready, gate-required, …)
//! to the frontend as a live Tauri event, so a local run animates in real
//! time. The headless runner historically wired
//! [`NoopNotificationAdapter`](demeteo_core::adapters::notification_noop),
//! throwing all of that away — so a *remote* run reported back only the
//! ~14 coarse lifecycle events `run.rs` appends by hand (submitted,
//! bootstrapped, pushed, pr_opened, …). The laptop had no per-step,
//! per-retry, or live-token visibility.
//!
//! This adapter closes that gap without inventing a new transport: it
//! translates each interesting `DomainEvent` into a [`RunEvent`](demeteo_core::ports::run_events::RunEvent) append on
//! the **same** append-only, offset-addressed log the laptop already tails
//! via the `stream_events` RPC (R9). Secret scrubbing (M7.2) happens at the
//! `RunEventsPort::append` sink, so every payload written here is scrubbed
//! for free — no separate scrub path.
//!
//! **Late binding.** The bridge is injected as the `NotificationPort` *into*
//! `build_core_context`, but the two ports it needs (`run_events` to write,
//! `runner_runs` to resolve `feature_id → run_id`) are constructed *inside*
//! that same call. So the bridge is created un-wired, passed in, and
//! [`wire`](RunEventBridge::wire)d immediately after the context is built.
//! An event that somehow fires before wiring (e.g. a startup driver resume)
//! is dropped exactly as the noop adapter would have dropped it — never an
//! error, never a panic.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use demeteo_core::adapters::run_event_log::run_event_record;
use demeteo_core::paths;
use demeteo_core::ports::notification::{DomainEvent, NotificationPort};
use demeteo_core::ports::run_events::RunEventsPort;
use demeteo_core::ports::runner_run::RunnerRunPort;
use std::sync::Arc;

/// Minimum spacing between two *pure telemetry* `step_progress` appends for
/// the same step. `StepProgress` fires both on genuine status transitions
/// **and** on every mid-turn token/cost refresh (opencode/hermes emit
/// `Usage` repeatedly per turn). A transition is always written; a refresh
/// that only moves the token/cost counters is coalesced to at most one
/// append per step per this window, so the event log stays a readable
/// narrative instead of a per-token firehose (and SQLite isn't hammered).
const PROGRESS_THROTTLE_MS: i64 = 8_000;

/// How long a step's streamed assistant text is allowed to accumulate
/// before it's flushed to the event log as one coalesced `step_output`
/// event. `AgentStream` arrives token-by-token; persisting each delta would
/// be a per-token firehose, so deltas are buffered and drained on this
/// cadence (plus a size trigger and an end-of-run drain).
const OUTPUT_FLUSH_INTERVAL_MS: i64 = 10_000;

/// Flush a step's buffered output early once it reaches this many bytes,
/// so a burst of text inside the time window still surfaces promptly
/// instead of waiting the full [`OUTPUT_FLUSH_INTERVAL_MS`].
const OUTPUT_FLUSH_BYTES: usize = 2_000;

/// Hard cap on a single flushed `step_output` payload. A flush larger than
/// this is truncated (head kept) with a marker — the event log is a
/// human-scannable narrative, not a full transcript store.
const OUTPUT_CHUNK_CAP: usize = 4_000;

/// The two ports the bridge writes/resolves through, injected after
/// `build_core_context` returns (see the module docs on late binding).
struct Wired {
    run_events: Arc<dyn RunEventsPort>,
    runner_runs: Arc<dyn RunnerRunPort>,
}

/// Per-step throttle state: the last status we appended and when, so a
/// telemetry-only refresh inside [`PROGRESS_THROTTLE_MS`] can be dropped
/// while a real status change always lands.
#[derive(Clone)]
struct LastProgress {
    status: String,
    at_ms: i64,
}

/// Per-step accumulator for coalesced `AgentStream` output, keyed by
/// `step_execution_id`. `feature_id` is retained so the pending text can be
/// routed to the owning run at flush time.
struct StreamBuf {
    feature_id: String,
    pending: String,
    last_flush_ms: i64,
}

pub struct RunEventBridge {
    inner: OnceLock<Wired>,
    /// `feature_id -> run_id`. Feature ids are globally unique, so a
    /// resolved mapping never changes and is cached permanently. Misses are
    /// *not* cached (the mapping may simply not exist yet — the run row's
    /// `feature_id` is populated right after `feature_start`), so a miss
    /// re-lists `runner_runs` and self-heals on the next event.
    resolve_cache: Mutex<HashMap<String, String>>,
    /// `(run_id, step_id) -> last appended progress`, for the throttle.
    progress_state: Mutex<HashMap<(String, String), LastProgress>>,
    /// `step_execution_id -> accumulated stream output`, for the coalesced
    /// `step_output` events (see [`StreamBuf`]).
    stream_bufs: Mutex<HashMap<String, StreamBuf>>,
}

impl RunEventBridge {
    pub fn new() -> Self {
        Self {
            inner: OnceLock::new(),
            resolve_cache: Mutex::new(HashMap::new()),
            progress_state: Mutex::new(HashMap::new()),
            stream_bufs: Mutex::new(HashMap::new()),
        }
    }

    /// Inject the live ports once the `AppContext` exists. Idempotent-safe:
    /// a second call is ignored (`OnceLock`), which never happens in
    /// practice — each binary wires exactly once after `build_core_context`.
    pub fn wire(&self, run_events: Arc<dyn RunEventsPort>, runner_runs: Arc<dyn RunnerRunPort>) {
        let _ = self.inner.set(Wired {
            run_events,
            runner_runs,
        });
    }

    /// Resolve the `run_id` that owns `feature_id`, or `None` if no run row
    /// claims it yet. Consults the permanent cache first; on a miss, lists
    /// `runner_runs` once and repopulates the cache from every row that has
    /// a `feature_id`, then retries the lookup.
    fn run_id_for_feature(&self, wired: &Wired, feature_id: &str) -> Option<String> {
        if let Some(run_id) = self
            .resolve_cache
            .lock()
            .ok()
            .and_then(|c| c.get(feature_id).cloned())
        {
            return Some(run_id);
        }
        let rows = wired.runner_runs.list().ok()?;
        let mut cache = self.resolve_cache.lock().ok()?;
        for row in rows {
            if let Some(fid) = row.feature_id {
                cache.insert(fid, row.run_id);
            }
        }
        cache.get(feature_id).cloned()
    }

    /// Append one bridged event for the run owning `feature_id`. Best-effort:
    /// an unresolved feature (no run row yet) or a sink error is logged and
    /// swallowed — event emission must never break the run it describes.
    fn emit_for_feature(&self, feature_id: &str, kind: &str, payload: serde_json::Value) {
        let Some(wired) = self.inner.get() else {
            return;
        };
        let Some(run_id) = self.run_id_for_feature(wired, feature_id) else {
            return;
        };
        let payload_json = payload.to_string();
        if let Err(e) = wired
            .run_events
            .append(&run_id, kind, Some(&payload_json), paths::now_ms())
        {
            eprintln!(
                "[demeteo-runner] warning: failed to bridge '{}' event for run {}: {}",
                kind, run_id, e
            );
        }
    }

    /// Throttle gate for `step_progress`: `true` (append) on a status change
    /// or once the per-step window has elapsed, `false` (drop) for a
    /// telemetry-only refresh inside the window.
    fn should_emit_progress(&self, feature_id: &str, step_id: &str, status: &str) -> bool {
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

    /// Accumulate one `AgentStream` delta for a step, flushing a coalesced
    /// `step_output` event when the per-step buffer crosses either the size
    /// or the time trigger. A first sighting seeds `last_flush_ms` to now so
    /// the time window is measured from when text started, not process start.
    fn buffer_stream(&self, feature_id: &str, step_execution_id: &str, content: &str) {
        let now = paths::now_ms();
        let ready = {
            let Ok(mut bufs) = self.stream_bufs.lock() else {
                return;
            };
            let buf = bufs
                .entry(step_execution_id.to_string())
                .or_insert_with(|| StreamBuf {
                    feature_id: feature_id.to_string(),
                    pending: String::new(),
                    last_flush_ms: now,
                });
            buf.pending.push_str(content);
            buf.pending.len() >= OUTPUT_FLUSH_BYTES
                || now - buf.last_flush_ms >= OUTPUT_FLUSH_INTERVAL_MS
        };
        if ready {
            self.flush_step_output(step_execution_id);
        }
    }

    /// Drain one step's buffered output to a single `step_output` event.
    /// A no-op when the buffer is empty or holds only whitespace (agent
    /// deltas are often bare newlines). Truncates to [`OUTPUT_CHUNK_CAP`].
    fn flush_step_output(&self, step_execution_id: &str) {
        let (feature_id, mut text) = {
            let Ok(mut bufs) = self.stream_bufs.lock() else {
                return;
            };
            let Some(buf) = bufs.get_mut(step_execution_id) else {
                return;
            };
            if buf.pending.trim().is_empty() {
                buf.pending.clear();
                return;
            }
            buf.last_flush_ms = paths::now_ms();
            (buf.feature_id.clone(), std::mem::take(&mut buf.pending))
        };
        text = text.trim().to_string();
        if text.len() > OUTPUT_CHUNK_CAP {
            // Keep the head on a char boundary, then mark the elision.
            let mut cut = OUTPUT_CHUNK_CAP;
            while cut > 0 && !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push_str(" …(truncated)");
        }
        self.emit_for_feature(
            &feature_id,
            "step_output",
            serde_json::json!({
                "step_execution_id": step_execution_id,
                "text": text,
            }),
        );
    }

    /// Flush every step buffer belonging to a feature — called on each
    /// `FeatureStatusChanged` so a step's trailing output isn't stranded in
    /// the buffer when the run reaches a terminal state (no more deltas will
    /// arrive to trip the size/time trigger).
    fn drain_feature_output(&self, feature_id: &str) {
        let step_ids: Vec<String> = {
            let Ok(bufs) = self.stream_bufs.lock() else {
                return;
            };
            bufs.iter()
                .filter(|(_, b)| b.feature_id == feature_id && !b.pending.trim().is_empty())
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in step_ids {
            self.flush_step_output(&id);
        }
    }
}

impl Default for RunEventBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationPort for RunEventBridge {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        // The DomainEvent → (kind, payload) translation is shared with
        // the local recorder (`demeteo_core::adapters::run_event_log`,
        // P1.13) so the two transports can never drift in shape; this
        // impl owns only the runner-specific concerns — feature→run_id
        // resolution, the progress throttle, and `AgentStream` buffering
        // into coalesced `step_output` events. Notes on individual
        // events:
        //   * `AgentSpawned`'s effective (post-clamp) effort matters in
        //     the durable log precisely *because* this is the detached
        //     path: a runner older than the submitting app silently
        //     drops `RunSpec::effort`, and this event is the only place
        //     the laptop can see what the run really used.
        //   * `BootstrapProgress` here covers the feature_start tail
        //     phases; the run's own pre-feature clone phases are
        //     appended separately in `run.rs`, keyed by run_id directly.
        //     The earliest phase(s) may predate the run row learning its
        //     feature_id and are simply dropped then.
        //   * Deliberately not bridged (the shared translation returns
        //     `None`): CommandExecuted / PermissionRequested (high
        //     volume, low value on the laptop timeline), MrMerged /
        //     ConflictDetected (not part of the run-progress narrative),
        //     and RunEventAppended (the local recorder's own echo).
        match event {
            DomainEvent::AgentStream {
                feature_id,
                step_execution_id,
                content,
            } => {
                self.buffer_stream(feature_id.as_str(), step_execution_id.as_str(), content);
                return Ok(());
            }
            DomainEvent::StepProgress {
                feature_id,
                step_id,
                status,
                ..
            } if !self.should_emit_progress(feature_id.as_str(), step_id, status) => {
                return Ok(());
            }
            DomainEvent::FeatureStatusChanged { feature_id, .. } => {
                // Drain any step's trailing streamed output before the
                // status line, so the timeline reads output-then-outcome
                // and nothing is stranded when the run goes terminal.
                self.drain_feature_output(feature_id.as_str());
            }
            _ => {}
        }
        if let Some(rec) = run_event_record(event) {
            self.emit_for_feature(&rec.feature_id, rec.kind, rec.payload);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "notify_bridge_tests.rs"]
mod tests;

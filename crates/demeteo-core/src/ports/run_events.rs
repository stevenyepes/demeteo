use serde::{Deserialize, Serialize};

/// One entry in a run's append-only event log
/// (docs/REMOTE_EXECUTION.md M3.3). `offset` is the log position a
/// resumed `stream_events(run_id, from_offset)` call pages from — it's
/// the SQLite rowid, so it's monotonic per table, not just per run.
///
/// Since P1.13 this log is written by **both** transports: the runner's
/// `RunEventBridge` keys rows by `run_id`, the local
/// [`RunEventRecorder`](crate::adapters::run_event_log::RunEventRecorder)
/// keys them by feature id. The `kind` vocabulary and payload shapes are
/// documented in [`crate::adapters::run_event_log`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEvent {
    pub offset: i64,
    pub run_id: String,
    pub kind: String,
    pub payload_json: Option<String>,
    pub created_at: i64,
}

pub trait RunEventsPort: Send + Sync {
    /// Append one event, returning its offset.
    fn append(
        &self,
        run_id: &str,
        kind: &str,
        payload_json: Option<&str>,
        now: i64,
    ) -> Result<i64, String>;
    /// Every event for `run_id` with `offset > from_offset`, oldest
    /// first. Never relies on a live connection having been open when
    /// the event was appended (R9) — a client that reconnects after any
    /// gap just asks for everything since its last-seen offset.
    fn list_since(&self, run_id: &str, from_offset: i64) -> Result<Vec<RunEvent>, String>;
}

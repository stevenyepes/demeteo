use serde::{Deserialize, Serialize};

/// The laptop's cached view of one remote run submitted to a
/// `demeteo-runner` (docs/REMOTE_EXECUTION.md M6.1/M6.2, design R9).
/// Keyed by `(machine_id, run_id)` — the laptop never owns this state,
/// it only mirrors what the runner's own `get_status`/`stream_events`
/// report, reconciling by `last_offset` so a dropped SSH tunnel never
/// loses an update, only delays seeing it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRunMirror {
    pub machine_id: String,
    pub run_id: String,
    /// The *local* project this run's spec was composed from — used to
    /// deep-link the inbox entry, not authoritative for anything on the
    /// runner side (the runner bootstraps its own project/feature rows
    /// from the embedded `RunSpec`).
    pub project_id: Option<String>,
    pub title: String,
    /// Mirrors `RunnerRun::status` (`pending`/`running`/`awaiting_mr`/
    /// `completed`/`failed`/`cancelled`/`interrupted`/`needs-credentials`)
    /// plus the laptop-only `unreachable` status (§7.1: set when the
    /// machine can't be reached at all, never conflated with `failed`).
    pub status: String,
    pub error: Option<String>,
    pub feature_id: Option<String>,
    pub pr_url: Option<String>,
    /// The feature branch pushed to origin, if any — set even when the
    /// run has no `pr_url` yet (failed/cancelled/parked after the push),
    /// so the inbox can offer a diff/branch deep link before a PR exists.
    pub pushed_branch: Option<String>,
    /// Highest `run_events` offset this mirror has consumed
    /// (`stream_events(run_id, from_offset)`, R9).
    pub last_offset: i64,
    pub created_at: i64,
    pub updated_at: i64,
    /// The status this run was in the last time the return-inbox
    /// notification diff (M6.3) fired for it — `None` until the first
    /// reconcile. Lets reopen-reconcile notify only on a *change* into an
    /// actionable status, not on every poll.
    pub last_notified_status: Option<String>,
}

pub trait RemoteRunMirrorPort: Send + Sync {
    /// Insert the row created at `submit_run` time. Idempotent by
    /// `(machine_id, run_id)` — a re-submit (same idempotency key) is a
    /// no-op here, matching the runner's own idempotent `submit_run`.
    /// `feature_id` is the laptop-chosen id shipped in the `RunSpec`
    /// (the eager shadow Feature), present from submit time so the run
    /// is navigable before the first reconcile.
    #[allow(clippy::too_many_arguments)]
    fn upsert_submitted(
        &self,
        machine_id: &str,
        run_id: &str,
        project_id: Option<&str>,
        feature_id: Option<&str>,
        title: &str,
        now: i64,
    ) -> Result<RemoteRunMirror, String>;
    /// Apply a reconciled status from the runner's `get_status`/
    /// `list_runs`/`stream_events`.
    #[allow(clippy::too_many_arguments)]
    fn update_status(
        &self,
        machine_id: &str,
        run_id: &str,
        status: &str,
        error: Option<&str>,
        feature_id: Option<&str>,
        pr_url: Option<&str>,
        pushed_branch: Option<&str>,
        last_offset: i64,
        now: i64,
    ) -> Result<(), String>;
    fn mark_notified(&self, machine_id: &str, run_id: &str, status: &str) -> Result<(), String>;
    /// Dismiss every laptop-local mirror associated with `feature_id`.
    ///
    /// This does not alter runner-owned records or events. The operation is
    /// idempotent: an unknown feature id succeeds without changing any rows.
    fn delete_for_feature(&self, feature_id: &str) -> Result<(), String>;
    fn get(&self, machine_id: &str, run_id: &str) -> Result<Option<RemoteRunMirror>, String>;
    fn list(&self) -> Result<Vec<RemoteRunMirror>, String>;
    /// The runs mirrored for `feature_ids`, most recently *submitted*
    /// (`created_at`) first — unlike [`Self::list`], whose `updated_at` order
    /// lets a reconcile of an older run overtake a newer submit. An empty
    /// slice yields no rows.
    ///
    /// Sized for one board: the SQLite adapter binds a parameter per id, so a
    /// slice past SQLite's bound-parameter limit (32 766 in the bundled build)
    /// is an `Err`, not a partial answer.
    fn list_for_features(&self, feature_ids: &[&str]) -> Result<Vec<RemoteRunMirror>, String>;
}

/// The `release_feature_cache` RPC of the `demeteo-runner` on `machine_id`,
/// for a run this client submitted there.
#[async_trait::async_trait]
pub trait RunnerCachePort: Send + Sync {
    async fn release_feature_cache(
        &self,
        machine_id: &str,
        run_id: &str,
        reason: crate::domain::runner_cache_release::CacheReleaseReason,
    ) -> Result<(), String>;
}

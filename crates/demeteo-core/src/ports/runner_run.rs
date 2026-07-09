use serde::{Deserialize, Serialize};

/// A run submitted to a headless runner (docs/REMOTE_EXECUTION_PLAN.md
/// M3.2). `run_id` is client-generated (a laptop-side UUID) — that's the
/// idempotency key: re-submitting the same `run_id` returns the existing
/// row instead of starting a duplicate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerRun {
    pub run_id: String,
    pub project_id: Option<String>,
    pub feature_id: Option<String>,
    pub spec_json: String,
    pub status: String,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// How many times this run has been auto-resumed after a runner
    /// restart (M2.3's bounded reboot-retry budget).
    pub resume_count: i64,
    /// The feature branch pushed to origin at completion (R3), if any —
    /// set even when the run then failed/was cancelled after the push,
    /// so the laptop can offer a diff/branch deep link before a PR
    /// exists (docs/REMOTE_EXECUTION_PLAN.md M6.2 follow-up).
    pub pushed_branch: Option<String>,
    /// Owning client's stable `install_id` (docs/MULTI_CLIENT_RUNNER.md
    /// MC-D2 / P0.2). Stamped at `submit_run` from the request's
    /// `client_id`; the runner's `require_owner` guard checks it on every
    /// run-scoped RPC so one client can't touch another's runs. `""` is
    /// the legacy/unknown tenant (old client that sent no id, or a
    /// pre-V26 row) — a single documented bucket, not a boundary.
    pub owner_client_id: String,
}

pub trait RunnerRunPort: Send + Sync {
    /// Insert a new run row. Returns the existing row unchanged (not an
    /// error) if `run_id` already exists — the caller uses this to decide
    /// whether to actually start the feature or just report the
    /// already-in-flight run (idempotent `submit_run`, R9/M3.2).
    /// `owner_client_id` stamps the run's owning client at creation
    /// (MC-D2); it is set only on the *insert* — re-submitting an existing
    /// `run_id` never re-homes an already-owned run to a new client.
    fn get_or_create(
        &self,
        run_id: &str,
        spec_json: &str,
        owner_client_id: &str,
        now: i64,
    ) -> Result<RunnerRun, String>;
    #[allow(clippy::too_many_arguments)]
    fn update_status(
        &self,
        run_id: &str,
        status: &str,
        project_id: Option<&str>,
        feature_id: Option<&str>,
        error: Option<&str>,
        pushed_branch: Option<&str>,
        now: i64,
    ) -> Result<(), String>;
    fn get(&self, run_id: &str) -> Result<Option<RunnerRun>, String>;
    fn list(&self) -> Result<Vec<RunnerRun>, String>;
    /// Mark every row currently `running` or `pending` as `interrupted`.
    /// Called on graceful shutdown (SIGTERM, M2.2) so `list_runs`/
    /// `get_status` reflect reality immediately instead of showing a
    /// stale `running` until the next restart's reconciliation.
    fn mark_all_running_interrupted(&self, now: i64) -> Result<(), String>;
    /// Increment and return the new `resume_count` for a run being
    /// auto-resumed after a restart (M2.3).
    fn bump_resume_count(&self, run_id: &str) -> Result<i64, String>;
    /// Atomically set `status = 'cancelled'` unless the row is already in
    /// a terminal state (`awaiting_mr`/`completed`/`failed`/`cancelled`).
    /// A single conditional `UPDATE`, not a read-then-write, so a
    /// `cancel_run` racing the run's own just-finished status update
    /// can't stomp a real outcome back to `cancelled` (M3.3).
    fn cancel_if_active(&self, run_id: &str, now: i64) -> Result<Option<RunnerRun>, String>;
}

//! Conflict resolver port (Phase R6).
//!
//! Cascade driver per `docs/DECISIONS.md` decision 20:
//!   1. **Auto-agent** (default `conflict_policy: "auto_agent"`):
//!      spawn a resolution subtask — a fresh agent session with a
//!      constrained prompt ("resolve these N conflicts; do not
//!      modify unrelated code"). Cost-capped (default 2 attempts,
//!      $0.50).
//!   2. **Manual** (on auto-agent failure or
//!      `conflict_policy: "auto_human"`): emit a gate so the user
//!      can open the Monaco 3-way merge UI.
//!   3. **Skip / Abort** (always available): the user can mark the
//!      subtask `skipped` or abort the feature.
//!
//! The trait is split by intent so each impl can be tested
//! independently:
//!
//! - [`ConflictResolver::resolve_via_agent`] does steps 1+2 itself
//!   and returns either `Ok(MergeOutcome)` or hands off to manual.
//! - [`ConflictResolver::request_manual_resolution`] emits a gate
//!   event so the UI can show the 3-way merge widget.

use crate::domain::ids::FeatureId;
use crate::domain::models::{ConflictReport, MergeOutcome};

pub trait ConflictResolver: Send + Sync {
    /// Spawn a resolution agent for `report`. On success returns
    /// the new merge outcome. On failure (cost cap, agent crash, …)
    /// the caller decides whether to cascade to manual based on
    /// the project's `ConflictPolicy`.
    ///
    /// The implementation must be **idempotent on re-entry**: if the
    /// executor is killed mid-resolution and restarts, calling this
    /// again must not double-bill the user.
    fn resolve_via_agent(
        &self,
        feature_id: &FeatureId,
        report: &ConflictReport,
        subtask_run_id: &str,
    ) -> Result<MergeOutcome, String>;

    /// Emit a `GateRequired` event so the existing `GateView` pops
    /// up. The user will see a "Conflict in <file list>" card and
    /// pick Auto / Manual / Skip / Abort.
    fn request_manual_resolution(&self, feature_id: &FeatureId, report: &ConflictReport);

    /// Hard cap on auto-agent attempts. Implementations should refuse
    /// to spawn more than this many resolution agents per conflict.
    fn max_auto_attempts(&self) -> u32;

    /// Hard cap on per-attempt cost (USD). Implementations should
    /// abort the resolution subtask once this is reached.
    fn max_attempt_cost_usd(&self) -> f64;
}

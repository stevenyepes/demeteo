//! `RunView` — the single read model for a run's rendered surface (C3,
//! `docs/EXECUTION_PARITY.md`).
//!
//! The UI renders a feature from four reads: the feature row, its step
//! executions (which carry per-step cost/tokens/artifact refs), the body of a
//! declared artifact, and the persisted agent stream. Today the Tauri commands
//! reach for `FeatureRepository`, `ThreadRepository`, and `ExecutionPort`
//! directly, so there is no single place that owns "how a run is read for
//! display". This type is that place.
//!
//! **Why it exists (D1).** Local and desktop-over-SSH runs share the laptop DB
//! and the machine filesystem, so every method here is a thin delegation to
//! those repos — this is a no-op refactor for those transports (the DoD for
//! C3.1 is byte-identical output). The payoff is C4: a runner-owned feature
//! lives in the runner's own DB/store, yet renders through this exact same
//! seam with no runner-specific branch. That works because C4.2's reconcile
//! (`commands/remote_runner::hydrate_shadow_feature`) hydrates a **read-only
//! shadow** of the runner's feature + steps into the laptop's *own*
//! `features`/`step_executions` tables and pulls each declared artifact into
//! the laptop `FsArtifactStore`, rewriting the shadow step's `artifact_path`
//! to that local cache path. So by the time the UI reads a runner feature,
//! `feature`/`steps`/`step`/`artifact_body` are all served from local state
//! identically to a native run — the `remote_run_mirror` row is the only thing
//! that marks it runner-owned. (The persisted `agent_stream` transcript is not
//! shadowed; the live agent stream is an at-execution-time push channel that no
//! completed run — local or remote — re-renders post-hoc.)
//!
//! Keep this layer **read-only**. Mutations (start/cancel/retry/gate-decide)
//! stay on the executor/presenter ports; folding them in here would blur the
//! read/write split that makes the runner mirror safe (the shadow is read-only
//! on the laptop, C4.2).

use std::sync::Arc;

use serde::Deserialize;

use crate::domain::ids::{FeatureId, StepExecutionId, ThreadId};
use crate::domain::models::sequence_view::assemble_tasks;
use crate::domain::models::{Feature, Message, SequenceState, StepAttempt, StepExecution};
use crate::ports::db::{FeatureRepository, ThreadRepository};
use crate::ports::execution::ExecutionPort;

/// Minimal projection of a persisted `TaskPlan` (`sequence_plan_cache`): just
/// the ordered id + title the drill-down needs. Parsing it here rather than
/// pulling in the adapter's full `TaskPlan` keeps the application layer free of
/// the execution-only skip/landed fields (which never serialize anyway).
#[derive(Deserialize)]
struct PlanRead {
    #[serde(alias = "subtasks")]
    tasks: Vec<PlanTaskRead>,
}

#[derive(Deserialize)]
struct PlanTaskRead {
    id: String,
    #[serde(default)]
    title: String,
}

/// Read model over a run's rendered surface. Cheap to clone (three `Arc`s);
/// construct one per `AppContext` and share it.
pub struct RunView {
    features: Arc<dyn FeatureRepository>,
    threads: Arc<dyn ThreadRepository>,
    exec: Arc<dyn ExecutionPort>,
}

impl RunView {
    pub fn new(
        features: Arc<dyn FeatureRepository>,
        threads: Arc<dyn ThreadRepository>,
        exec: Arc<dyn ExecutionPort>,
    ) -> Self {
        Self {
            features,
            threads,
            exec,
        }
    }

    /// The feature row (status, model, mr_url, aggregate cost, …). `None` when
    /// no such feature exists.
    pub fn feature(&self, id: &FeatureId) -> Result<Option<Feature>, String> {
        self.features.get(id)
    }

    /// Every step execution for the run, in creation order — each carrying its
    /// own `cost_usd`/`tokens`/`wall_clock_secs` and artifact path refs.
    pub fn steps(&self, feature_id: &FeatureId) -> Result<Vec<StepExecution>, String> {
        self.features.steps_for_feature(feature_id)
    }

    /// A single step execution by id. `None` when it doesn't exist.
    pub fn step(&self, id: &StepExecutionId) -> Result<Option<StepExecution>, String> {
        self.features.step_get(id)
    }

    /// Per-attempt history for a step (`step_attempts`, P1.8), ordered by
    /// `attempt_no`. Feeds the node drill-down panel's Overview tab (P2.3) —
    /// the row the timeline overwrites on retry, kept whole here so the UI can
    /// show class/cost/duration/applied-rule for every attempt.
    pub fn step_attempts(&self, id: &StepExecutionId) -> Result<Vec<StepAttempt>, String> {
        self.features.attempts_for_step(id)
    }

    /// A `sequence` node's task list, merged for the drill-down accordion
    /// (P2.5): the ordered plan (`sequence_plan_cache`), each task's landed
    /// flag (`sequence_checkpoints` — the committed Decision-13 prefix), and
    /// its per-task status/cost (`subtask_runs`). `node_id` is the graph node
    /// id (== v1 `step_id`), which keys plan + checkpoint; `execution_id` keys
    /// the subtask rows.
    ///
    /// Returns [`SequenceState::unplanned`] for a node that hasn't resolved a
    /// plan yet or for a non-sequence node (neither writes a plan-cache row),
    /// so the caller needs no node-type branch. Runner-owned features only
    /// populate this once their sequence state is mirrored locally; until then
    /// it reads unplanned, same as a not-yet-run node.
    pub fn sequence_state(
        &self,
        feature_id: &FeatureId,
        node_id: &str,
        execution_id: &StepExecutionId,
    ) -> Result<SequenceState, String> {
        let Some(plan_json) = self.features.plan_cache_get(feature_id, node_id)? else {
            return Ok(SequenceState::unplanned());
        };
        let plan: PlanRead = serde_json::from_str(&plan_json)
            .map_err(|e| format!("sequence plan cache is not valid TaskPlan JSON: {e}"))?;

        let landed: std::collections::HashSet<String> = self
            .features
            .sequence_checkpoint_get(feature_id, node_id)?
            .into_iter()
            .collect();

        let runs: std::collections::HashMap<String, _> = self
            .features
            .subtask_runs_for_step(execution_id)?
            .into_iter()
            .map(|r| (r.subtask_id.clone(), r))
            .collect();

        let plan_pairs: Vec<(String, String)> =
            plan.tasks.into_iter().map(|t| (t.id, t.title)).collect();

        Ok(SequenceState {
            planned: true,
            tasks: assemble_tasks(&plan_pairs, &landed, &runs),
        })
    }

    /// The persisted agent stream (canonical message history) for a step's
    /// thread. This is the *durable* transcript, not the live push events
    /// (`agent_stream`/`step_progress`), which stay a separate concern.
    pub fn agent_stream(&self, thread_id: &ThreadId) -> Result<Vec<Message>, String> {
        self.threads.get_messages(thread_id)
    }

    /// The UTF-8 body of a declared artifact at `path` on `machine_id`. For
    /// local/SSH this is a direct read of the machine filesystem via the
    /// execution port (a missing/unreadable path is an `Err`, never `Ok("")`,
    /// per the port contract). For a runner-owned feature no special-casing is
    /// needed here: C4.2 already cached the body into the laptop
    /// `FsArtifactStore` and rewrote the shadow step's path to that local
    /// cache path, so the caller passes `machine_id = "local"` with the cache
    /// path and this reads it like any native artifact.
    pub async fn artifact_body(&self, machine_id: &str, path: &str) -> Result<String, String> {
        self.exec.read_file(machine_id, path).await
    }
}

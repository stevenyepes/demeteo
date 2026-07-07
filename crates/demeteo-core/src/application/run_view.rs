//! `RunView` — the single read model for a run's rendered surface (C3,
//! `docs/EXECUTION_CONSISTENCY_PLAN.md`).
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
//! lives in the runner's own DB/store, and `RunView` becomes the one seam that
//! transparently sources it from a laptop-side shadow + lazy artifact cache, so
//! the UI renders a remote run with identical fidelity without knowing where it
//! ran.
//!
//! Keep this layer **read-only**. Mutations (start/cancel/retry/gate-decide)
//! stay on the executor/presenter ports; folding them in here would blur the
//! read/write split that makes the runner mirror safe (the shadow is read-only
//! on the laptop, C4.2).

use std::sync::Arc;

use crate::domain::ids::{FeatureId, StepExecutionId, ThreadId};
use crate::domain::models::{Feature, Message, StepExecution};
use crate::ports::db::{FeatureRepository, ThreadRepository};
use crate::ports::execution::ExecutionPort;

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

    /// The persisted agent stream (canonical message history) for a step's
    /// thread. This is the *durable* transcript, not the live push events
    /// (`agent_stream`/`step_progress`), which stay a separate concern.
    pub fn agent_stream(&self, thread_id: &ThreadId) -> Result<Vec<Message>, String> {
        self.threads.get_messages(thread_id)
    }

    /// The UTF-8 body of a declared artifact at `path` on `machine_id`. For
    /// local/SSH this is a direct read of the machine filesystem via the
    /// execution port (a missing/unreadable path is an `Err`, never `Ok("")`,
    /// per the port contract). C4 overrides this for runner-owned features to
    /// serve from the lazily-cached shadow copy.
    pub async fn artifact_body(&self, machine_id: &str, path: &str) -> Result<String, String> {
        self.exec.read_file(machine_id, path).await
    }
}

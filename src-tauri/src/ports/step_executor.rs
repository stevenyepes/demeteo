use crate::domain::models::{StepExecution, GateDecision, Feature};

pub trait StepExecutor: Send + Sync {
    /// Start a new feature run.
    ///
    /// - `title`: short human label for the feature (used as the
    ///   `features.title` row, the worktree branch slug, and the
    ///   ProjectHome header).
    /// - `description`: the rich prompt body. This is what gets
    ///   rendered into `{{feature_description}}` for every step.
    ///   Required — the executor refuses to start with an empty
    ///   description.
    fn feature_start(
        &self,
        project_id: &str,
        workflow_id: &str,
        title: &str,
        description: &str,
        agent_kind: Option<&str>,
        model: Option<&str>,
    ) -> Result<Feature, String>;

    fn feature_pause(&self, feature_id: &str) -> Result<(), String>;
    fn feature_resume(&self, feature_id: &str) -> Result<(), String>;
    fn feature_cancel(&self, feature_id: &str) -> Result<(), String>;

    fn step_get(&self, execution_id: &str) -> Result<StepExecution, String>;
    fn step_retry(&self, execution_id: &str, new_model: Option<&str>) -> Result<(), String>;
    /// Replay from the given step execution — reset the target step and
    /// all subsequent steps to `pending`, clear their artifacts and gate
    /// decisions, then restart the execution loop. Works for any step
    /// status (completed, failed, interrupted, awaiting_gate, running).
    fn replay_from_step(&self, execution_id: &str, new_model: Option<&str>) -> Result<(), String>;
    fn step_list_for_run(&self, feature_id: &str) -> Result<Vec<StepExecution>, String>;
}

pub trait GatePresenter: Send + Sync {
    fn gate_pending_for_run(&self, feature_id: &str) -> Result<Option<GateDecision>, String>;
    fn gate_decide(
        &self,
        step_execution_id: &str,
        decision: &str,
        feedback: Option<&str>,
    ) -> Result<(), String>;
}

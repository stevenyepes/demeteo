use crate::domain::models::{StepExecution, GateDecision, Feature};

pub trait StepExecutor: Send + Sync {
    fn feature_start(
        &self,
        project_id: &str,
        workflow_id: &str,
        description: &str,
    ) -> Result<Feature, String>;

    fn feature_pause(&self, feature_id: &str) -> Result<(), String>;
    fn feature_resume(&self, feature_id: &str) -> Result<(), String>;
    fn feature_cancel(&self, feature_id: &str) -> Result<(), String>;

    fn step_get(&self, execution_id: &str) -> Result<StepExecution, String>;
    fn step_retry(&self, execution_id: &str) -> Result<(), String>;
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

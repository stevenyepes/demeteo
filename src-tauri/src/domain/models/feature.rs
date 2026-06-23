use crate::domain::ids::{
    FeatureId, GateDecisionId, ProjectId, StepExecutionId, StepId, WorkflowId,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Feature {
    pub id: FeatureId,
    pub project_id: ProjectId,
    pub workflow_id: Option<WorkflowId>,
    pub title: String,
    pub status: String,
    pub total_cost: f64,
    pub duration: String,
    #[serde(default)]
    pub tokens: i64,
    pub created_at: i64,
    #[serde(default)]
    pub agent_kind: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub mr_url: Option<String>,
    #[serde(default)]
    pub mr_state: Option<String>,
    /// Per-feature override for the project's `commit_artifacts`
    /// setting. `None` = inherit from `ProjectSettings::commit_artifacts`.
    /// The StartFeatureModal exposes this as a toggle in the advanced
    /// section. See migration V12 and `commit_worktree_changes`.
    #[serde(default)]
    pub commit_artifacts: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StepExecution {
    pub id: StepExecutionId,
    pub feature_id: FeatureId,
    pub step_id: StepId,
    pub step_index: u32,
    pub step_kind: String,
    pub status: String,
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub tokens: Option<i64>,
    pub wall_clock_secs: Option<u64>,
    #[serde(default)]
    pub artifact_path: Option<String>,
    #[serde(default)]
    pub artifact_paths: Vec<String>,
    pub error_message: Option<String>,
    #[serde(default)]
    pub iteration_count: u32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubtaskRun {
    pub id: String,
    pub feature_id: FeatureId,
    pub step_execution_id: StepExecutionId,
    pub subtask_id: String,
    pub agent_id: Option<String>,
    pub worktree_path: String,
    pub branch: String,
    pub status: String,
    pub cost_usd: f64,
    #[serde(default)]
    pub tokens: i64,
    pub error_message: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GateDecision {
    pub id: GateDecisionId,
    pub step_execution_id: StepExecutionId,
    /// None = pending. "approve" | "redirect" | "cancel"
    pub decision: Option<String>,
    /// Feedback / redirect instructions provided by the user.
    pub feedback: Option<String>,
    pub created_at: i64,
}

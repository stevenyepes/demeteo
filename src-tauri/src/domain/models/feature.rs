use crate::domain::attachment::AttachedFile;
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
    /// Per-run override of the loop iteration budget. `None` = inherit the
    /// project default (`ProjectSettings::default_loop_iterations`) or the
    /// engine default (3). See migration V13.
    #[serde(default)]
    pub loop_iterations: Option<u32>,
    /// Per-step agent/model overrides chosen at launch, snapshotted on the
    /// feature so workflow/project edits don't affect an in-flight run.
    /// Empty = every step inherits the workflow/project defaults.
    #[serde(default)]
    pub step_overrides: Vec<StepOverride>,

    /// Per-feature user attachments (images, files) — owned by the
    /// feature run. Stored as a JSON column on the feature row
    /// (`features.attachments_json`, migration V19) rather than a
    /// separate table so feature cleanup (auto-delete branch)
    /// releases the attachment lifetime implicitly. The on-disk
    /// file content lives in `FsAttachmentStore` at
    /// `<app_local_data_dir>/attachments/<feature_id>/<sha256>.<ext>`
    /// and is dropped by `FsAttachmentStore::clear_feature` when the
    /// feature is purged.
    #[serde(default)]
    pub attachments: Vec<AttachedFile>,
}

/// A per-step agent/model override selected when launching a feature.
/// Either field may be `None`, meaning "inherit" for that dimension.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct StepOverride {
    pub step_id: String,
    #[serde(default)]
    pub agent_kind: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
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

use crate::domain::artifact::{ArtifactDecl, ArtifactMode};
use crate::domain::ids::{ProjectId, StepId, WorkflowId, WorkflowVersionId};
use crate::domain::verifier::VerifierConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowSchedule {
    pub cron: String,             // standard 5-field cron expression
    pub title_template: String,   // e.g. "Daily sweep {{date}}"
    pub project_id: ProjectId,    // which project to spawn features on
    pub next_run_at: Option<i64>, // unix ms; maintained by scheduler
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Workflow {
    pub id: WorkflowId,
    pub name: String,
    pub description: String,
    pub is_starter: bool,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub schedule: Option<WorkflowSchedule>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkflowVersion {
    pub id: WorkflowVersionId,
    pub workflow_id: WorkflowId,
    pub version: u32,
    pub steps_json: String,
    pub note: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct StepConfig {
    pub id: StepId,
    pub kind: String,
    pub title: String,
    pub agent_kind: Option<String>,
    pub prompt_template: Option<String>,
    pub artifact_mode: String,
    pub on_failure: Option<StepId>,
    pub max_iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<ArtifactDecl>>,
    #[serde(default)]
    pub verifier: Option<VerifierConfig>,
}

impl StepConfig {
    pub fn artifact_mode_typed(&self) -> ArtifactMode {
        ArtifactMode::from_str_loose(&self.artifact_mode)
    }
}

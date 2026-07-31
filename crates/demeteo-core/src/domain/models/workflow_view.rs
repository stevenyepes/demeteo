//! Read model: a workflow row joined to one of its versions.

use crate::domain::models::{StepConfig, Workflow, WorkflowSchedule, WorkflowVersion};
use serde::{Deserialize, Serialize};

/// What every workflow command hands the frontend — the `WorkflowWithSteps`
/// interface in `src/types.ts`.
///
/// A workflow always has a name and a description; the step list, version
/// number and version id belong to whichever version the caller asked about,
/// which is why they are flattened in here rather than nested: the library,
/// the builder and the version drawer all render the join, never the two rows.
#[derive(Serialize, Deserialize)]
pub struct WorkflowWithSteps {
    pub id: String,
    pub name: String,
    pub description: String,
    pub is_starter: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub steps: Vec<StepConfig>,
    pub version: u32,
    pub version_id: String,
    pub schedule: Option<WorkflowSchedule>,
}

impl WorkflowWithSteps {
    /// The workflow as stored, at the version given.
    ///
    /// `None` reports `version: 0` and an empty `version_id` — a row with no
    /// versions, which the library renders as an entry that can neither be
    /// opened nor exported. Nothing writes that state deliberately; it is what
    /// an interrupted create used to leave behind, so the reading exists to
    /// make it visible rather than to make it an error.
    pub fn joined(workflow: Workflow, version: Option<WorkflowVersion>) -> Self {
        let (steps, number, version_id) = match version {
            Some(v) => (
                serde_json::from_str::<Vec<StepConfig>>(&v.steps_json).unwrap_or_default(),
                v.version,
                v.id.0,
            ),
            None => (vec![], 0, String::new()),
        };
        Self {
            id: workflow.id.0,
            name: workflow.name,
            description: workflow.description,
            is_starter: workflow.is_starter,
            created_at: workflow.created_at,
            updated_at: workflow.updated_at,
            steps,
            version: number,
            version_id,
            schedule: workflow.schedule,
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/models/workflow_view.rs"]
mod tests;

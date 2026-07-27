use crate::domain::ids::{WorkflowId, WorkflowVersionId};
use crate::domain::models::{StepConfig, Workflow, WorkflowVersion};
use crate::paths;
use crate::ports::db::WorkflowRepository;
use std::sync::Arc;

/// Insert a single ad-hoc workflow (one `Workflow` row + its initial
/// `WorkflowVersion`) from a `{ "name", "description", "steps": [...] }`
/// JSON value. Shared by `commands::workflows::workflow_create` (Tauri) and
/// the headless runner's `RunSpec::workflow_json` ingestion (M1.2) — same
/// shape, same validation, one code path.
pub fn create_from_json(
    workflows: &Arc<dyn WorkflowRepository>,
    spec: &serde_json::Value,
) -> Result<WorkflowId, String> {
    let name = spec["name"]
        .as_str()
        .unwrap_or("Untitled workflow")
        .to_string();
    let description = spec["description"].as_str().unwrap_or("").to_string();
    let steps: Vec<StepConfig> = serde_json::from_value(spec["steps"].clone())
        .map_err(|e| format!("invalid workflow steps: {}", e))?;
    if steps.is_empty() {
        return Err("workflow must have at least one step".to_string());
    }

    let now = paths::now_ms();
    let id = WorkflowId::from(format!("wf-{}", paths::new_id()));
    let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;

    workflows.create(Workflow {
        id: id.clone(),
        name,
        description,
        is_starter: false,
        created_at: now,
        updated_at: now,
        schedule: None,
    })?;

    workflows.save_version(WorkflowVersion {
        id: WorkflowVersionId::from(format!("{}-v1", id.as_str())),
        workflow_id: id.clone(),
        version: 1,
        steps_json,
        // v1 ingestion path (the runner's `RunSpec::workflow_json`): no
        // authored v2 document exists, so readers migrate the step list on
        // the fly — the pre-V34 behavior, unchanged.
        definition_json: None,
        note: Some("Initial version".to_string()),
        created_at: now,
    })?;

    Ok(id)
}

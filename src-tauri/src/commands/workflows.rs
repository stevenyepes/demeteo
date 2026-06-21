use crate::domain::ids::{WorkflowId, WorkflowVersionId};
use crate::domain::models::{StepConfig, Workflow, WorkflowVersion};
use crate::paths;
use crate::ports::db::WorkflowRepository;
use crate::state::AppContext;
use std::sync::Arc;
use tauri::State;

/// Seed starter-pack workflows on first launch if the `workflows` table is empty.
pub fn seed_starter_workflows(workflows: &Arc<dyn WorkflowRepository>) {
    let starters: &[(&str, &str)] = &[
        (
            include_str!("../../workflows/standard-feature-pipeline.json"),
            "standard-feature-pipeline",
        ),
        (
            include_str!("../../workflows/bugfix-pipeline.json"),
            "bugfix-pipeline",
        ),
        (
            include_str!("../../workflows/docs-update.json"),
            "docs-update",
        ),
        (include_str!("../../workflows/refactor.json"), "refactor"),
        (
            include_str!("../../workflows/experiment.json"),
            "experiment",
        ),
        (include_str!("../../workflows/ci-fix.json"), "ci-fix"),
    ];

    for (json, _slug) in starters {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json) {
            let id = WorkflowId::from(v["id"].as_str().unwrap_or("").to_string());
            let name = v["name"].as_str().unwrap_or("").to_string();
            let description = v["description"].as_str().unwrap_or("").to_string();
            let is_starter = v["is_starter"].as_bool().unwrap_or(false);
            let steps: Vec<StepConfig> =
                serde_json::from_value(v["steps"].clone()).unwrap_or_default();
            let steps_json = serde_json::to_string(&steps).unwrap_or_default();
            let now = paths::now_ms();

            match workflows.get(&id) {
                Ok(Some(w)) => {
                    // Check if steps have changed compared to the latest DB version
                    if let Ok(Some(latest_ver)) = workflows.latest_version(&id) {
                        let db_steps: Vec<StepConfig> =
                            serde_json::from_str(&latest_ver.steps_json).unwrap_or_default();
                        if db_steps != steps {
                            let all_versions = workflows.versions(&id).unwrap_or_default();
                            let next_version = all_versions
                                .iter()
                                .map(|ver| ver.version)
                                .max()
                                .unwrap_or(0)
                                + 1;

                            if w.name != name || w.description != description {
                                let _ = workflows.update_meta(&id, &name, &description);
                            }

                            let version = WorkflowVersion {
                                id: WorkflowVersionId::from(format!(
                                    "{}-v{}",
                                    id.as_str(),
                                    next_version
                                )),
                                workflow_id: id.clone(),
                                version: next_version,
                                steps_json,
                                note: Some(
                                    "System auto-update to latest starter template".to_string(),
                                ),
                                created_at: now,
                            };
                            let _ = workflows.save_version(version);
                        }
                    }
                }
                Ok(None) => {
                    let workflow = Workflow {
                        id: id.clone(),
                        name,
                        description,
                        is_starter,
                        created_at: now,
                        updated_at: now,
                    };
                    let _ = workflows.create(workflow);
                    let version = WorkflowVersion {
                        id: WorkflowVersionId::from(format!("{}-v1", id.as_str())),
                        workflow_id: id,
                        version: 1,
                        steps_json,
                        note: Some("Initial version".to_string()),
                        created_at: now,
                    };
                    let _ = workflows.save_version(version);
                }
                Err(_) => {}
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri Commands
// ─────────────────────────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
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
}

#[tauri::command]
pub async fn workflow_list(ctx: State<'_, AppContext>) -> Result<Vec<WorkflowWithSteps>, String> {
    let workflows = &ctx.workflows;
    let ws = workflows.list()?;
    let mut result = Vec::new();
    for w in ws {
        let latest = workflows.latest_version(&w.id)?;
        let (steps, version, version_id) = if let Some(v) = latest {
            let steps = serde_json::from_str::<Vec<StepConfig>>(&v.steps_json).unwrap_or_default();
            (steps, v.version, v.id.0)
        } else {
            (vec![], 0, String::new())
        };
        result.push(WorkflowWithSteps {
            id: w.id.0,
            name: w.name,
            description: w.description,
            is_starter: w.is_starter,
            created_at: w.created_at,
            updated_at: w.updated_at,
            steps,
            version,
            version_id,
        });
    }
    Ok(result)
}

#[tauri::command]
pub async fn workflow_get(
    workflow_id: String,
    ctx: State<'_, AppContext>,
) -> Result<WorkflowWithSteps, String> {
    let workflows = &ctx.workflows;
    let wf_id = WorkflowId::from(workflow_id.clone());
    let w = workflows.get(&wf_id)?.ok_or("Workflow not found")?;
    let latest = workflows.latest_version(&wf_id)?;
    let (steps, version, version_id) = if let Some(v) = latest {
        let steps = serde_json::from_str::<Vec<StepConfig>>(&v.steps_json).unwrap_or_default();
        (steps, v.version, v.id.0)
    } else {
        (vec![], 0, String::new())
    };
    Ok(WorkflowWithSteps {
        id: w.id.0,
        name: w.name,
        description: w.description,
        is_starter: w.is_starter,
        created_at: w.created_at,
        updated_at: w.updated_at,
        steps,
        version,
        version_id,
    })
}

#[tauri::command]
pub async fn workflow_create(
    name: String,
    description: String,
    steps: Vec<StepConfig>,
    ctx: State<'_, AppContext>,
) -> Result<WorkflowWithSteps, String> {
    let workflows = &ctx.workflows;
    let now = paths::now_ms();
    let id = WorkflowId::from(format!("wf-{}", paths::new_id()));
    let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;

    let workflow = Workflow {
        id: id.clone(),
        name: name.clone(),
        description: description.clone(),
        is_starter: false,
        created_at: now,
        updated_at: now,
    };
    workflows.create(workflow)?;

    let version_id = WorkflowVersionId::from(format!("{}-v1", id.as_str()));
    let version = WorkflowVersion {
        id: version_id.clone(),
        workflow_id: id.clone(),
        version: 1,
        steps_json,
        note: Some("Initial version".to_string()),
        created_at: now,
    };
    workflows.save_version(version)?;

    Ok(WorkflowWithSteps {
        id: id.0,
        name,
        description,
        is_starter: false,
        created_at: now,
        updated_at: now,
        steps,
        version: 1,
        version_id: version_id.0,
    })
}

#[tauri::command]
pub async fn workflow_update(
    workflow_id: String,
    name: String,
    description: String,
    steps: Vec<StepConfig>,
    note: Option<String>,
    ctx: State<'_, AppContext>,
) -> Result<WorkflowWithSteps, String> {
    let workflows = &ctx.workflows;
    let now = paths::now_ms();
    let wf_id = WorkflowId::from(workflow_id.clone());
    workflows.update_meta(&wf_id, &name, &description)?;

    // Calculate next version number
    let existing_versions = workflows.versions(&wf_id)?;
    let next_version = existing_versions
        .iter()
        .map(|v| v.version)
        .max()
        .unwrap_or(0)
        + 1;
    let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;
    let version_id = WorkflowVersionId::from(format!("{}-v{}", workflow_id, next_version));
    let version = WorkflowVersion {
        id: version_id.clone(),
        workflow_id: wf_id.clone(),
        version: next_version,
        steps_json,
        note,
        created_at: now,
    };
    workflows.save_version(version)?;

    Ok(WorkflowWithSteps {
        id: workflow_id,
        name,
        description,
        is_starter: false,
        created_at: now,
        updated_at: now,
        steps,
        version: next_version,
        version_id: version_id.0,
    })
}

#[tauri::command]
pub async fn workflow_delete(
    workflow_id: String,
    ctx: State<'_, AppContext>,
) -> Result<(), String> {
    ctx.workflows.delete(&WorkflowId::from(workflow_id))
}

#[tauri::command]
pub async fn workflow_versions(
    workflow_id: String,
    ctx: State<'_, AppContext>,
) -> Result<Vec<WorkflowVersion>, String> {
    ctx.workflows.versions(&WorkflowId::from(workflow_id))
}

#[tauri::command]
pub async fn workflow_export(
    workflow_id: String,
    ctx: State<'_, AppContext>,
) -> Result<String, String> {
    let workflows = &ctx.workflows;
    let wf_id = WorkflowId::from(workflow_id);
    let w = workflows.get(&wf_id)?.ok_or("Workflow not found")?;
    let latest = workflows
        .latest_version(&wf_id)?
        .ok_or("No versions found")?;
    let steps: Vec<StepConfig> =
        serde_json::from_str(&latest.steps_json).map_err(|e| e.to_string())?;

    let export = serde_json::json!({
        "id": w.id,
        "name": w.name,
        "description": w.description,
        "is_starter": w.is_starter,
        "steps": steps
    });
    serde_json::to_string_pretty(&export).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_import(
    json: String,
    ctx: State<'_, AppContext>,
) -> Result<WorkflowWithSteps, String> {
    let v: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let name = v["name"]
        .as_str()
        .unwrap_or("Imported Workflow")
        .to_string();
    let description = v["description"].as_str().unwrap_or("").to_string();
    let steps: Vec<StepConfig> =
        serde_json::from_value(v["steps"].clone()).map_err(|e| format!("Invalid steps: {}", e))?;

    // Always create a new ID on import to avoid conflicts
    let workflows = &ctx.workflows;
    let now = paths::now_ms();
    let id = WorkflowId::from(format!("wf-imported-{}", paths::new_id()));
    let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;

    let workflow = Workflow {
        id: id.clone(),
        name: name.clone(),
        description: description.clone(),
        is_starter: false,
        created_at: now,
        updated_at: now,
    };
    workflows.create(workflow)?;
    let version_id = WorkflowVersionId::from(format!("{}-v1", id.as_str()));
    let version = WorkflowVersion {
        id: version_id.clone(),
        workflow_id: id.clone(),
        version: 1,
        steps_json,
        note: Some("Imported".to_string()),
        created_at: now,
    };
    workflows.save_version(version)?;

    Ok(WorkflowWithSteps {
        id: id.0,
        name,
        description,
        is_starter: false,
        created_at: now,
        updated_at: now,
        steps,
        version: 1,
        version_id: version_id.0,
    })
}

/// Revert a starter pack workflow to its bundled default version.
#[tauri::command]
pub async fn workflow_revert_to_default(
    workflow_id: String,
    ctx: State<'_, AppContext>,
) -> Result<WorkflowWithSteps, String> {
    let workflows = &ctx.workflows;
    let wf_id = WorkflowId::from(workflow_id.clone());
    let w = workflows.get(&wf_id)?.ok_or("Workflow not found")?;
    if !w.is_starter {
        return Err("Only starter pack workflows can be reverted to default.".to_string());
    }

    let starters: &[&str] = &[
        include_str!("../../workflows/standard-feature-pipeline.json"),
        include_str!("../../workflows/bugfix-pipeline.json"),
        include_str!("../../workflows/docs-update.json"),
        include_str!("../../workflows/refactor.json"),
        include_str!("../../workflows/experiment.json"),
        include_str!("../../workflows/ci-fix.json"),
    ];
    for json in starters {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json) {
            if v["id"].as_str().unwrap_or("") == workflow_id {
                let name = v["name"].as_str().unwrap_or("").to_string();
                let description = v["description"].as_str().unwrap_or("").to_string();
                let steps: Vec<StepConfig> =
                    serde_json::from_value(v["steps"].clone()).unwrap_or_default();
                let existing_versions = workflows.versions(&wf_id)?;
                let next_version = existing_versions
                    .iter()
                    .map(|v| v.version)
                    .max()
                    .unwrap_or(0)
                    + 1;
                let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;
                let now = paths::now_ms();
                let version_id =
                    WorkflowVersionId::from(format!("{}-v{}", workflow_id, next_version));
                workflows.update_meta(&wf_id, &name, &description)?;
                workflows.save_version(WorkflowVersion {
                    id: version_id.clone(),
                    workflow_id: wf_id.clone(),
                    version: next_version,
                    steps_json,
                    note: Some("Reverted to default".to_string()),
                    created_at: now,
                })?;
                return Ok(WorkflowWithSteps {
                    id: workflow_id,
                    name,
                    description,
                    is_starter: true,
                    created_at: w.created_at,
                    updated_at: now,
                    steps,
                    version: next_version,
                    version_id: version_id.0,
                });
            }
        }
    }
    Err("Starter pack source not found for this workflow id.".to_string())
}

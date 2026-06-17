use crate::domain::models::{Workflow, WorkflowVersion, StepConfig};
use crate::ports::db::DatabasePort;
use crate::state::DatabaseState;
use tauri::State;
use std::sync::Arc;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn new_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut h);
    std::thread::current().id().hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Seed starter-pack workflows on first launch if the `workflows` table is empty.
pub fn seed_starter_workflows(db: &Arc<dyn DatabasePort>) {
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
        (
            include_str!("../../workflows/refactor.json"),
            "refactor",
        ),
        (
            include_str!("../../workflows/experiment.json"),
            "experiment",
        ),
        (
            include_str!("../../workflows/ci-fix.json"),
            "ci-fix",
        ),
    ];

    for (json, _slug) in starters {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json) {
            let id = v["id"].as_str().unwrap_or("").to_string();
            let name = v["name"].as_str().unwrap_or("").to_string();
            let description = v["description"].as_str().unwrap_or("").to_string();
            let is_starter = v["is_starter"].as_bool().unwrap_or(false);
            let steps: Vec<StepConfig> = serde_json::from_value(v["steps"].clone()).unwrap_or_default();
            let steps_json = serde_json::to_string(&steps).unwrap_or_default();
            let now = now_ms();

            match db.workflow_get(&id) {
                Ok(Some(w)) => {
                    // Check if steps have changed compared to the latest DB version
                    if let Ok(Some(latest_ver)) = db.workflow_latest_version(&id) {
                        let db_steps: Vec<StepConfig> = serde_json::from_str(&latest_ver.steps_json).unwrap_or_default();
                        if db_steps != steps {
                            let all_versions = db.workflow_versions(&id).unwrap_or_default();
                            let next_version = all_versions.iter().map(|ver| ver.version).max().unwrap_or(0) + 1;

                            if w.name != name || w.description != description {
                                let _ = db.workflow_update_meta(&id, &name, &description);
                            }

                            let version = WorkflowVersion {
                                id: format!("{}-v{}", id, next_version),
                                workflow_id: id.clone(),
                                version: next_version,
                                steps_json,
                                note: Some("System auto-update to latest starter template".to_string()),
                                created_at: now,
                            };
                            let _ = db.workflow_save_version(version);
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
                    let _ = db.workflow_create(workflow);
                    let version = WorkflowVersion {
                        id: format!("{}-v1", id),
                        workflow_id: id,
                        version: 1,
                        steps_json,
                        note: Some("Initial version".to_string()),
                        created_at: now,
                    };
                    let _ = db.workflow_save_version(version);
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
pub async fn workflow_list(db_state: State<'_, DatabaseState>) -> Result<Vec<WorkflowWithSteps>, String> {
    let db = &db_state.db;
    let workflows = db.workflow_list()?;
    let mut result = Vec::new();
    for w in workflows {
        let latest = db.workflow_latest_version(&w.id)?;
        let (steps, version, version_id) = if let Some(v) = latest {
            let steps = serde_json::from_str::<Vec<StepConfig>>(&v.steps_json).unwrap_or_default();
            (steps, v.version, v.id)
        } else {
            (vec![], 0, String::new())
        };
        result.push(WorkflowWithSteps {
            id: w.id,
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
    db_state: State<'_, DatabaseState>,
) -> Result<WorkflowWithSteps, String> {
    let db = &db_state.db;
    let w = db.workflow_get(&workflow_id)?.ok_or("Workflow not found")?;
    let latest = db.workflow_latest_version(&workflow_id)?;
    let (steps, version, version_id) = if let Some(v) = latest {
        let steps = serde_json::from_str::<Vec<StepConfig>>(&v.steps_json).unwrap_or_default();
        (steps, v.version, v.id)
    } else {
        (vec![], 0, String::new())
    };
    Ok(WorkflowWithSteps {
        id: w.id,
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
    db_state: State<'_, DatabaseState>,
) -> Result<WorkflowWithSteps, String> {
    let db = &db_state.db;
    let now = now_ms();
    let id = format!("wf-{}", new_id());
    let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;

    let workflow = Workflow {
        id: id.clone(),
        name: name.clone(),
        description: description.clone(),
        is_starter: false,
        created_at: now,
        updated_at: now,
    };
    db.workflow_create(workflow)?;

    let version_id = format!("{}-v1", id);
    let version = WorkflowVersion {
        id: version_id.clone(),
        workflow_id: id.clone(),
        version: 1,
        steps_json,
        note: Some("Initial version".to_string()),
        created_at: now,
    };
    db.workflow_save_version(version)?;

    Ok(WorkflowWithSteps {
        id,
        name,
        description,
        is_starter: false,
        created_at: now,
        updated_at: now,
        steps,
        version: 1,
        version_id,
    })
}

#[tauri::command]
pub async fn workflow_update(
    workflow_id: String,
    name: String,
    description: String,
    steps: Vec<StepConfig>,
    note: Option<String>,
    db_state: State<'_, DatabaseState>,
) -> Result<WorkflowWithSteps, String> {
    let db = &db_state.db;
    let now = now_ms();
    db.workflow_update_meta(&workflow_id, &name, &description)?;

    // Calculate next version number
    let existing_versions = db.workflow_versions(&workflow_id)?;
    let next_version = existing_versions.iter().map(|v| v.version).max().unwrap_or(0) + 1;
    let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;
    let version_id = format!("{}-v{}", workflow_id, next_version);
    let version = WorkflowVersion {
        id: version_id.clone(),
        workflow_id: workflow_id.clone(),
        version: next_version,
        steps_json,
        note,
        created_at: now,
    };
    db.workflow_save_version(version)?;

    Ok(WorkflowWithSteps {
        id: workflow_id,
        name,
        description,
        is_starter: false,
        created_at: now,
        updated_at: now,
        steps,
        version: next_version,
        version_id,
    })
}

#[tauri::command]
pub async fn workflow_delete(
    workflow_id: String,
    db_state: State<'_, DatabaseState>,
) -> Result<(), String> {
    db_state.db.workflow_delete(&workflow_id)
}

#[tauri::command]
pub async fn workflow_versions(
    workflow_id: String,
    db_state: State<'_, DatabaseState>,
) -> Result<Vec<WorkflowVersion>, String> {
    db_state.db.workflow_versions(&workflow_id)
}

#[tauri::command]
pub async fn workflow_export(
    workflow_id: String,
    db_state: State<'_, DatabaseState>,
) -> Result<String, String> {
    let db = &db_state.db;
    let w = db.workflow_get(&workflow_id)?.ok_or("Workflow not found")?;
    let latest = db.workflow_latest_version(&workflow_id)?
        .ok_or("No versions found")?;
    let steps: Vec<StepConfig> = serde_json::from_str(&latest.steps_json)
        .map_err(|e| e.to_string())?;

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
    db_state: State<'_, DatabaseState>,
) -> Result<WorkflowWithSteps, String> {
    let v: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let name = v["name"].as_str().unwrap_or("Imported Workflow").to_string();
    let description = v["description"].as_str().unwrap_or("").to_string();
    let steps: Vec<StepConfig> = serde_json::from_value(v["steps"].clone())
        .map_err(|e| format!("Invalid steps: {}", e))?;

    // Always create a new ID on import to avoid conflicts
    let db = &db_state.db;
    let now = now_ms();
    let id = format!("wf-imported-{}", new_id());
    let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;

    let workflow = Workflow {
        id: id.clone(),
        name: name.clone(),
        description: description.clone(),
        is_starter: false,
        created_at: now,
        updated_at: now,
    };
    db.workflow_create(workflow)?;
    let version_id = format!("{}-v1", id);
    let version = WorkflowVersion {
        id: version_id.clone(),
        workflow_id: id.clone(),
        version: 1,
        steps_json,
        note: Some("Imported".to_string()),
        created_at: now,
    };
    db.workflow_save_version(version)?;

    Ok(WorkflowWithSteps {
        id,
        name,
        description,
        is_starter: false,
        created_at: now,
        updated_at: now,
        steps,
        version: 1,
        version_id,
    })
}

/// Revert a starter pack workflow to its bundled default version.
#[tauri::command]
pub async fn workflow_revert_to_default(
    workflow_id: String,
    db_state: State<'_, DatabaseState>,
) -> Result<WorkflowWithSteps, String> {
    let db = &db_state.db;
    let w = db.workflow_get(&workflow_id)?.ok_or("Workflow not found")?;
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
                let steps: Vec<StepConfig> = serde_json::from_value(v["steps"].clone()).unwrap_or_default();
                let existing_versions = db.workflow_versions(&workflow_id)?;
                let next_version = existing_versions.iter().map(|v| v.version).max().unwrap_or(0) + 1;
                let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;
                let now = now_ms();
                let version_id = format!("{}-v{}", workflow_id, next_version);
                db.workflow_update_meta(&workflow_id, &name, &description)?;
                db.workflow_save_version(WorkflowVersion {
                    id: version_id.clone(),
                    workflow_id: workflow_id.clone(),
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
                    version_id,
                });
            }
        }
    }
    Err("Starter pack source not found for this workflow id.".to_string())
}

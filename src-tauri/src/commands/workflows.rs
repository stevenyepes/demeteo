use crate::adapters::step_executor::node_catalog::{node_type_catalog, NodeTypeInfo};
use crate::adapters::step_executor::node_lint::lint_definition;
use crate::domain::ids::{FeatureId, WorkflowId, WorkflowVersionId};
use crate::domain::models::workflow_migrate::migrate_v1_to_v2;
use crate::domain::models::workflow_v2::{validate_workflow_v2, WorkflowDefinitionV2};
use crate::domain::models::{StepConfig, Workflow, WorkflowVersion};
use crate::domain::workflow_graph::{has_errors, LintFinding, LintSeverity};
use crate::error::AppError;
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
        (
            include_str!("../../workflows/simple-task.json"),
            "simple-task",
        ),
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
                        schedule: None,
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
    pub schedule: Option<crate::domain::models::WorkflowSchedule>,
}

#[tauri::command]
pub fn workflow_list(ctx: State<'_, AppContext>) -> Result<Vec<WorkflowWithSteps>, AppError> {
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
            schedule: w.schedule.clone(),
        });
    }
    Ok(result)
}

#[tauri::command]
pub fn workflow_get(
    workflow_id: String,
    ctx: State<'_, AppContext>,
) -> Result<WorkflowWithSteps, AppError> {
    let workflows = &ctx.workflows;
    let wf_id = WorkflowId::from(workflow_id.clone());
    let w = workflows
        .get(&wf_id)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found(format!("Workflow not found: {}", workflow_id)))?;
    let latest = workflows.latest_version(&wf_id).map_err(AppError::from)?;
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
        schedule: w.schedule.clone(),
    })
}

/// The **schema-v2 graph** for a feature's *pinned* workflow version
/// (P1.15), migrated on the fly from the stored v1 step list. This is the
/// definition the run-mode canvas (P2.2) renders: it must reflect the
/// version the run actually started with — not the workflow's latest edit
/// — so a historical run renders its own graph. Falls back to the
/// workflow's latest version for legacy features that predate the
/// `workflow_version_id` pin.
#[tauri::command]
pub fn feature_workflow_graph(
    feature_id: String,
    ctx: State<'_, AppContext>,
) -> Result<WorkflowDefinitionV2, AppError> {
    let feature = ctx
        .run_view
        .feature(&FeatureId::from(feature_id.clone()))
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found(format!("Feature not found: {feature_id}")))?;

    let workflow_id = feature
        .workflow_id
        .ok_or_else(|| AppError::not_found(format!("Feature {feature_id} has no workflow")))?;

    let workflows = &ctx.workflows;

    // Prefer the pinned version so a run always renders the graph it
    // started with; fall back to latest for pre-pin (legacy) features.
    let pinned = match feature.workflow_version_id {
        Some(vid) => workflows.version_get(&vid).map_err(AppError::from)?,
        None => None,
    };
    let version = match pinned {
        Some(v) => Some(v),
        None => workflows
            .latest_version(&workflow_id)
            .map_err(AppError::from)?,
    };

    let steps: Vec<StepConfig> = version
        .as_ref()
        .map(|v| serde_json::from_str(&v.steps_json).unwrap_or_default())
        .unwrap_or_default();

    let name = workflows
        .get(&workflow_id)
        .map_err(AppError::from)?
        .map(|w| w.name)
        .unwrap_or_default();

    Ok(migrate_v1_to_v2(workflow_id, name, &steps))
}

/// P1.3 boundary invariant: every definition accepted for storage must
/// have a schema-valid v2 projection (`migrate_v1_to_v2` is pure/total,
/// so this can only fire if the v2 model and its published JSON Schema
/// drift apart — surface that loudly at the write, not at run time).
///
/// P3.3 adds the second half: the projection must also pass the structural
/// lint at **error** severity. The builder disables Save while an error
/// finding stands, but a UI-only rule is a convention; enforcing it here is
/// what makes "an invalid definition cannot be stored" true of every write
/// path — including import of a hand-edited file. Warnings never block
/// (PRD §6.3).
fn ensure_valid_v2_projection(
    id: &WorkflowId,
    name: &str,
    steps: &[StepConfig],
) -> Result<(), AppError> {
    let projection = migrate_v1_to_v2(id.clone(), name, steps);
    let value = serde_json::to_value(&projection).map_err(|e| e.to_string())?;
    validate_workflow_v2(&value).map_err(|e| {
        AppError::validation(format!(
            "workflow definition failed schema-v2 validation:\n{e}"
        ))
    })?;

    let findings = lint_definition(&projection);
    if has_errors(&findings) {
        let detail = findings
            .iter()
            .filter(|f| f.severity == LintSeverity::Error)
            .map(|f| {
                let anchor = f
                    .node
                    .as_ref()
                    .map(|n| format!("{n}: "))
                    .or_else(|| f.edge.as_ref().map(|(a, b)| format!("{a} → {b}: ")))
                    .unwrap_or_default();
                format!("  - [{}] {anchor}{}", f.code, f.message)
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Err(AppError::validation(format!(
            "workflow definition has structural errors:\n{detail}"
        )));
    }
    Ok(())
}

/// Structural lint for a schema-v2 definition the builder holds in memory
/// (task P3.3, PRD §6.3) — the source of the canvas's per-node lint badges
/// and of the reason list shown when Save is blocked.
///
/// Takes the raw payload rather than a typed definition so a definition the
/// v2 model can't even read comes back as a renderable `schema-invalid`
/// finding instead of an opaque IPC deserialization error — the builder needs
/// something to *show*, and it is the same surface either way.
///
/// The rule set is the engine's own (`node_lint::lint_definition`): the
/// registry supplies the known node types, so this command never needs
/// editing when a node type is added.
#[tauri::command]
pub fn workflow_lint(definition: serde_json::Value) -> Vec<LintFinding> {
    let def: WorkflowDefinitionV2 = match serde_json::from_value(definition) {
        Ok(def) => def,
        Err(e) => {
            return vec![LintFinding::workflow_error(
                "schema-invalid",
                format!("definition is not a readable schema-v2 workflow: {e}"),
            )]
        }
    };
    lint_definition(&def)
}

#[tauri::command]
pub fn workflow_create(
    name: String,
    description: String,
    steps: Vec<StepConfig>,
    ctx: State<'_, AppContext>,
) -> Result<WorkflowWithSteps, AppError> {
    let workflows = &ctx.workflows;
    let now = paths::now_ms();
    let id = WorkflowId::from(format!("wf-{}", paths::new_id()));
    ensure_valid_v2_projection(&id, &name, &steps)?;
    let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;

    let workflow = Workflow {
        id: id.clone(),
        name: name.clone(),
        description: description.clone(),
        is_starter: false,
        created_at: now,
        updated_at: now,
        schedule: None,
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
        schedule: None,
    })
}

#[tauri::command]
pub fn workflow_update(
    workflow_id: String,
    name: String,
    description: String,
    steps: Vec<StepConfig>,
    note: Option<String>,
    ctx: State<'_, AppContext>,
) -> Result<WorkflowWithSteps, AppError> {
    let workflows = &ctx.workflows;
    let now = paths::now_ms();
    let wf_id = WorkflowId::from(workflow_id.clone());
    let w = workflows
        .get(&wf_id)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found(format!("Workflow not found: {}", workflow_id)))?;
    ensure_valid_v2_projection(&wf_id, &name, &steps)?;
    workflows
        .update_meta(&wf_id, &name, &description)
        .map_err(AppError::from)?;

    let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;
    let (next_version, version_id) = append_version(workflows, &wf_id, steps_json, note, now)?;

    Ok(WorkflowWithSteps {
        id: workflow_id,
        name,
        description,
        is_starter: false,
        created_at: w.created_at,
        updated_at: now,
        steps,
        version: next_version,
        version_id: version_id.0,
        schedule: w.schedule,
    })
}

#[tauri::command]
pub fn workflow_delete(workflow_id: String, ctx: State<'_, AppContext>) -> Result<(), AppError> {
    ctx.workflows
        .delete(&WorkflowId::from(workflow_id))
        .map_err(AppError::from)
}

#[tauri::command]
pub fn workflow_versions(
    workflow_id: String,
    ctx: State<'_, AppContext>,
) -> Result<Vec<WorkflowVersion>, AppError> {
    ctx.workflows
        .versions(&WorkflowId::from(workflow_id))
        .map_err(AppError::from)
}

/// The **schema-v2 graph** for one stored version — what the builder's version
/// drawer renders and diffs (P3.4).
///
/// The run-mode twin of this is `feature_workflow_graph`, which resolves the
/// version a *run* pinned. Design mode needs the same projection for a version
/// the author picked out of history instead, and migration is Rust-only, so the
/// drawer cannot derive it from the `steps_json` string `workflow_versions`
/// already hands it.
#[tauri::command]
pub fn workflow_version_graph(
    workflow_id: String,
    version_id: String,
    ctx: State<'_, AppContext>,
) -> Result<WorkflowDefinitionV2, AppError> {
    version_graph(
        &ctx.workflows,
        &WorkflowId::from(workflow_id),
        &WorkflowVersionId::from(version_id),
    )
}

/// Restore a stored version as a **new** version (P3.4): the row it copies is
/// left exactly where it was, so history only ever grows.
///
/// The copy is of `steps_json` **verbatim**, deliberately: the builder holds a
/// schema-v2 graph and storage is still the v1 step list, so routing a restore
/// through the editor's model would rewrite the restored version through a
/// lossy projection — an author asking for v3 back would get something that
/// merely migrates to the same graph. Content-preserving history operations
/// belong at the storage layer, below that seam. (The same reasoning applies to
/// `workflow_revert_to_default`, which has always copied the bundled starter's
/// steps directly.)
///
/// Name and description are not versioned, so they are left untouched.
#[tauri::command]
pub fn workflow_restore_version(
    workflow_id: String,
    version_id: String,
    ctx: State<'_, AppContext>,
) -> Result<WorkflowWithSteps, AppError> {
    restore_version(
        &ctx.workflows,
        &WorkflowId::from(workflow_id),
        &WorkflowVersionId::from(version_id),
    )
}

/// The command core, split out so the tests drive the same code the
/// `#[tauri::command]` wrapper does rather than a local mirror of it.
pub fn version_graph(
    workflows: &Arc<dyn WorkflowRepository>,
    workflow_id: &WorkflowId,
    version_id: &WorkflowVersionId,
) -> Result<WorkflowDefinitionV2, AppError> {
    let w = workflows
        .get(workflow_id)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found(format!("Workflow not found: {workflow_id}")))?;
    let version = version_of(workflows, workflow_id, version_id)?;
    let steps: Vec<StepConfig> = serde_json::from_str(&version.steps_json).unwrap_or_default();
    Ok(migrate_v1_to_v2(workflow_id.clone(), w.name, &steps))
}

/// See `workflow_restore_version`.
pub fn restore_version(
    workflows: &Arc<dyn WorkflowRepository>,
    workflow_id: &WorkflowId,
    version_id: &WorkflowVersionId,
) -> Result<WorkflowWithSteps, AppError> {
    let w = workflows
        .get(workflow_id)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found(format!("Workflow not found: {workflow_id}")))?;
    let source = version_of(workflows, workflow_id, version_id)?;
    let now = paths::now_ms();
    let (version, new_version_id) = append_version(
        workflows,
        workflow_id,
        source.steps_json.clone(),
        Some(format!("Restored from v{}", source.version)),
        now,
    )?;

    Ok(WorkflowWithSteps {
        id: workflow_id.0.clone(),
        name: w.name,
        description: w.description,
        is_starter: w.is_starter,
        created_at: w.created_at,
        updated_at: now,
        steps: serde_json::from_str(&source.steps_json).unwrap_or_default(),
        version,
        version_id: new_version_id.0,
        schedule: w.schedule,
    })
}

/// Load a version row and prove it belongs to the workflow the caller named.
/// Version ids are guessable by construction (`<workflow-id>-v3`), so the
/// pairing is checked rather than assumed — a mismatched pair would otherwise
/// let one workflow's history be restored onto another.
fn version_of(
    workflows: &Arc<dyn WorkflowRepository>,
    workflow_id: &WorkflowId,
    version_id: &WorkflowVersionId,
) -> Result<WorkflowVersion, AppError> {
    let version = workflows
        .version_get(version_id)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found(format!("Workflow version not found: {version_id}")))?;
    if &version.workflow_id != workflow_id {
        return Err(AppError::validation(format!(
            "Version {version_id} belongs to workflow {}, not {workflow_id}.",
            version.workflow_id
        )));
    }
    Ok(version)
}

/// One past the highest version that exists. Numbering never reuses a value,
/// so a version id derived from it is unique for the life of the workflow.
fn next_version_number(existing: &[WorkflowVersion]) -> u32 {
    existing.iter().map(|v| v.version).max().unwrap_or(0) + 1
}

/// Append an immutable version row and report what it became.
///
/// Every path that produces a version — an edit, a revert-to-default, a restore
/// from history — goes through here, so "saving is an append, never an edit"
/// stays one fact instead of three copies of the same numbering arithmetic.
fn append_version(
    workflows: &Arc<dyn WorkflowRepository>,
    workflow_id: &WorkflowId,
    steps_json: String,
    note: Option<String>,
    now: i64,
) -> Result<(u32, WorkflowVersionId), AppError> {
    let existing = workflows.versions(workflow_id).map_err(AppError::from)?;
    let version = next_version_number(&existing);
    let id = WorkflowVersionId::from(format!("{}-v{}", workflow_id.as_str(), version));
    workflows.save_version(WorkflowVersion {
        id: id.clone(),
        workflow_id: workflow_id.clone(),
        version,
        steps_json,
        note,
        created_at: now,
    })?;
    Ok((version, id))
}

#[tauri::command]
pub fn workflow_export(
    workflow_id: String,
    ctx: State<'_, AppContext>,
) -> Result<String, AppError> {
    let workflows = &ctx.workflows;
    let wf_id = WorkflowId::from(workflow_id);
    let w = workflows
        .get(&wf_id)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found("Workflow not found"))?;
    let latest = workflows
        .latest_version(&wf_id)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found("No versions found"))?;
    let steps: Vec<StepConfig> =
        serde_json::from_str(&latest.steps_json).map_err(|e| AppError::from(e.to_string()))?;

    let export = serde_json::json!({
        "id": w.id,
        "name": w.name,
        "description": w.description,
        "is_starter": w.is_starter,
        "steps": steps
    });
    serde_json::to_string_pretty(&export).map_err(|e| e.to_string().into())
}

#[tauri::command]
pub fn workflow_import(
    json: String,
    ctx: State<'_, AppContext>,
) -> Result<WorkflowWithSteps, AppError> {
    let v: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;

    // Schema-v2 documents are checked against the published JSON Schema
    // (docs-site/workflow-schema-v2.json) so hand-authored v2 files get
    // located, readable errors today. Storage + execution of v2 graphs
    // arrives with the DAG engine (P1.15/P3.6); until then even a valid
    // v2 document cannot be imported, and we say so instead of silently
    // storing something the engine can't run.
    if v.get("schema_version").and_then(|s| s.as_u64()) == Some(2) {
        validate_workflow_v2(&v).map_err(|e| {
            AppError::validation(format!("schema-v2 workflow failed validation:\n{e}"))
        })?;
        return Err(AppError::validation(
            "this is a valid schema-v2 workflow definition, but importing v2 graphs lands with \
             the DAG engine — export/import currently uses the v1 steps-list format",
        ));
    }

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
    ensure_valid_v2_projection(&id, &name, &steps)?;
    let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;

    let workflow = Workflow {
        id: id.clone(),
        name: name.clone(),
        description: description.clone(),
        is_starter: false,
        created_at: now,
        updated_at: now,
        schedule: None,
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
        schedule: None,
    })
}

/// Every node type this build can dispatch, with the display metadata,
/// config schema, and port declaration the builder palette needs (P3.1).
///
/// Derived entirely from the `NodeTypeRegistry`, so a newly registered
/// handler appears in the palette with no frontend change — the PRD §6.3
/// promise and P3.5's acceptance test. Static per build: the frontend
/// fetches it once and caches it.
#[tauri::command]
pub fn node_types_list() -> Vec<NodeTypeInfo> {
    node_type_catalog()
}

/// Revert a starter pack workflow to its bundled default version.
#[tauri::command]
pub fn workflow_revert_to_default(
    workflow_id: String,
    ctx: State<'_, AppContext>,
) -> Result<WorkflowWithSteps, AppError> {
    let workflows = &ctx.workflows;
    let wf_id = WorkflowId::from(workflow_id.clone());
    let w = workflows
        .get(&wf_id)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found(format!("Workflow not found: {}", workflow_id)))?;
    if !w.is_starter {
        return Err(AppError::validation(
            "Only starter pack workflows can be reverted to default.",
        ));
    }

    let starters: &[&str] = &[
        include_str!("../../workflows/standard-feature-pipeline.json"),
        include_str!("../../workflows/bugfix-pipeline.json"),
        include_str!("../../workflows/docs-update.json"),
        include_str!("../../workflows/refactor.json"),
        include_str!("../../workflows/experiment.json"),
        include_str!("../../workflows/ci-fix.json"),
        include_str!("../../workflows/simple-task.json"),
    ];
    for json in starters {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json) {
            if v["id"].as_str().unwrap_or("") == workflow_id {
                let name = v["name"].as_str().unwrap_or("").to_string();
                let description = v["description"].as_str().unwrap_or("").to_string();
                let steps: Vec<StepConfig> =
                    serde_json::from_value(v["steps"].clone()).unwrap_or_default();
                let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;
                let now = paths::now_ms();
                workflows.update_meta(&wf_id, &name, &description)?;
                let (next_version, version_id) = append_version(
                    workflows,
                    &wf_id,
                    steps_json,
                    Some("Reverted to default".to_string()),
                    now,
                )?;
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
                    schedule: w.schedule.clone(),
                });
            }
        }
    }
    Err(AppError::not_found(
        "Starter pack source not found for this workflow id.",
    ))
}

#[tauri::command]
pub fn workflow_save_schedule(
    workflow_id: String,
    schedule: Option<crate::domain::models::WorkflowSchedule>,
    ctx: State<'_, AppContext>,
) -> Result<(), AppError> {
    let wf_id = WorkflowId::from(workflow_id);
    let mut schedule_to_save = schedule;
    if let Some(ref mut s) = schedule_to_save {
        if s.next_run_at.is_none() {
            let now_secs = crate::paths::now_ms() / 1000;
            s.next_run_at = crate::adapters::scheduler::calculate_next_run(&s.cron, now_secs);
        }
    }
    ctx.workflows.update_schedule(&wf_id, schedule_to_save)?;
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/infrastructure/workflows.rs"]
mod starter_tests;

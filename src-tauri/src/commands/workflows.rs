use crate::adapters::step_executor::node_catalog::{node_type_catalog, NodeTypeInfo};
use crate::adapters::step_executor::node_lint::lint_definition;
use crate::domain::ids::{FeatureId, WorkflowId, WorkflowVersionId};
use crate::domain::models::workflow_migrate::{
    migrate_v1_to_v2, project_v2_to_v1, read_import, write_export,
};
use crate::domain::models::workflow_v2::{validate_workflow_v2, WorkflowDefinitionV2};
use crate::domain::models::{Workflow, WorkflowVersion, WorkflowWithSteps};
use crate::domain::workflow_graph::{error_summary, LintFinding};
use crate::domain::workflow_history::{ensure_owned_by, next_version_number, version_id};
use crate::domain::workflow_starters::{self, SeedAction, StarterDefinition};
use crate::error::AppError;
use crate::paths;
use crate::ports::db::WorkflowRepository;
use crate::state::AppContext;
use std::sync::Arc;
use tauri::State;

/// The starter pack this build ships, read by the first-launch seed below and
/// by `workflow_revert_to_default`.
///
/// `pub(crate)` so the starter suite asserts against *this* list rather than a
/// parallel copy of it. A second hand-written list gates nothing: deleting a
/// line here would leave the workflow unseeded and unreachable by
/// `workflow_revert_to_default`, with every test still green because each one
/// reached its JSON through its own `include_str!`.
pub(crate) const STARTER_FILES: &[&str] = &[
    include_str!("../../workflows/standard-feature-pipeline.json"),
    include_str!("../../workflows/bugfix-pipeline.json"),
    include_str!("../../workflows/docs-update.json"),
    include_str!("../../workflows/refactor.json"),
    include_str!("../../workflows/experiment.json"),
    include_str!("../../workflows/ci-fix.json"),
    include_str!("../../workflows/simple-task.json"),
    include_str!("../../workflows/code-review.json"),
    include_str!("../../workflows/address-review.json"),
];

/// Seed starter-pack workflows on first launch if the `workflows` table is empty.
pub fn seed_starter_workflows(workflows: &Arc<dyn WorkflowRepository>) {
    for json in STARTER_FILES {
        let Some(starter) = StarterDefinition::parse(json) else {
            continue;
        };
        let Ok(stored) = workflows.get(&starter.id) else {
            continue;
        };
        let latest = if stored.is_some() {
            workflows.latest_version(&starter.id).unwrap_or_default()
        } else {
            None
        };
        let now = paths::now_ms();

        match workflow_starters::plan_seed(&starter, stored.as_ref(), latest.as_ref()) {
            SeedAction::Skip => {}
            SeedAction::Create => {
                let _ = workflows.create(starter.workflow_row(now));
                let _ = workflows.save_version(starter.version_row(1, "Initial version", now));
            }
            SeedAction::Republish { rename } => {
                let version =
                    next_version_number(&workflows.versions(&starter.id).unwrap_or_default());
                if rename {
                    let _ = workflows.update_meta(&starter.id, &starter.name, &starter.description);
                }
                let _ = workflows.save_version(starter.version_row(
                    version,
                    "System auto-update to latest starter template",
                    now,
                ));
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri Commands
// ─────────────────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn workflow_list(ctx: State<'_, AppContext>) -> Result<Vec<WorkflowWithSteps>, AppError> {
    let workflows = &ctx.workflows;
    let mut result = Vec::new();
    for w in workflows.list()? {
        let latest = workflows.latest_version(&w.id)?;
        result.push(WorkflowWithSteps::joined(w, latest));
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
    Ok(WorkflowWithSteps::joined(w, latest))
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

    let name = workflows
        .get(&workflow_id)
        .map_err(AppError::from)?
        .map(|w| w.name)
        .unwrap_or_default();

    // `WorkflowVersion::definition` is the single seam: the stored v2 document
    // when the version has one (V34), the migration of its step list when it
    // doesn't — so a run launched before P3.6 still renders.
    Ok(match version {
        Some(v) => v.definition(&name),
        None => migrate_v1_to_v2(workflow_id, name, &[]),
    })
}

/// The boundary invariant every write path enforces.
///
/// **P1.3:** the definition must satisfy the published JSON Schema — surfaced
/// loudly at the write rather than at run time.
///
/// **P3.3:** it must also pass the structural lint at **error** severity. The
/// builder disables Save while an error finding stands, but a UI-only rule is
/// a convention; enforcing it here is what makes "an invalid definition cannot
/// be stored" true of *every* write, including the import of a hand-edited
/// file. Warnings never block (PRD §6.3).
///
/// P3.6 made this v2-native: with the builder authoring graphs directly, there
/// is no longer a v1 step list to project first — the definition being checked
/// is the definition being stored.
fn ensure_valid_definition(def: &WorkflowDefinitionV2) -> Result<(), AppError> {
    let value = serde_json::to_value(def).map_err(|e| e.to_string())?;
    validate_workflow_v2(&value).map_err(|e| {
        AppError::validation(format!(
            "workflow definition failed schema-v2 validation:\n{e}"
        ))
    })?;

    match error_summary(&lint_definition(def)) {
        Some(detail) => Err(AppError::validation(format!(
            "workflow definition has structural errors:\n{detail}"
        ))),
        None => Ok(()),
    }
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

/// Save a **schema-v2 graph** as a new version — the builder's only write
/// path (task P3.6), replacing the v1 `workflow_create` / `workflow_update`
/// pair the retired form editor used.
///
/// `workflow_id: None` creates the workflow first, so "new from a template"
/// and "edit an existing one" are the same call and the builder needs no
/// branch. Both mint a version through [`append_version`], because a save is
/// always an append.
///
/// The definition is stored **verbatim** in `definition_json` (V34) — layout,
/// joins, per-class retry, and edge guards intact — alongside its v1
/// projection in `steps_json`, which is what the runner and export still read.
/// The projection round-trips exactly for a chain (a test over all seven
/// starters pins that), and is a valid topological order for anything else.
#[tauri::command]
pub fn workflow_save(
    workflow_id: Option<String>,
    name: String,
    description: String,
    definition: WorkflowDefinitionV2,
    note: Option<String>,
    ctx: State<'_, AppContext>,
) -> Result<WorkflowWithSteps, AppError> {
    save_definition(
        &ctx.workflows,
        workflow_id.map(WorkflowId::from),
        &name,
        &description,
        definition,
        note,
    )
}

/// The command core, so tests drive the same code the wrapper does.
pub fn save_definition(
    workflows: &Arc<dyn WorkflowRepository>,
    workflow_id: Option<WorkflowId>,
    name: &str,
    description: &str,
    definition: WorkflowDefinitionV2,
    note: Option<String>,
) -> Result<WorkflowWithSteps, AppError> {
    let now = paths::now_ms();

    // Resolve the workflow row first: the definition's own `id` field is
    // normalized to it below, so a template's placeholder id or a graph copied
    // from another workflow can't travel into storage.
    let existing = match &workflow_id {
        Some(id) => Some(
            workflows
                .get(id)
                .map_err(AppError::from)?
                .ok_or_else(|| AppError::not_found(format!("Workflow not found: {id}")))?,
        ),
        None => None,
    };
    let wf_id = workflow_id.unwrap_or_else(|| WorkflowId::from(format!("wf-{}", paths::new_id())));

    let mut definition = definition;
    definition.id = wf_id.clone();
    definition.name = name.to_string();
    ensure_valid_definition(&definition)?;

    // Only now mint the row for a *new* workflow. Creating it before validation
    // left an orphan behind whenever a save or import was rejected: a row with
    // zero versions, which `workflow_list` reports as `version: 0` with an empty
    // version id — a library entry that can neither be opened nor exported.
    if existing.is_none() {
        workflows.create(Workflow {
            id: wf_id.clone(),
            name: name.to_string(),
            description: description.to_string(),
            is_starter: false,
            created_at: now,
            updated_at: now,
            schedule: None,
        })?;
    }
    let (created_at, is_starter, schedule) = match existing {
        Some(w) => (w.created_at, w.is_starter, w.schedule),
        None => (now, false, None),
    };

    let steps = project_v2_to_v1(&definition);
    let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;
    let definition_json = serde_json::to_string(&definition).map_err(|e| e.to_string())?;

    workflows
        .update_meta(&wf_id, name, description)
        .map_err(AppError::from)?;
    let appended = append_version(
        workflows,
        &wf_id,
        steps_json,
        Some(definition_json),
        note,
        now,
    )?;

    Ok(WorkflowWithSteps {
        id: wf_id.0,
        name: name.to_string(),
        description: description.to_string(),
        is_starter,
        created_at,
        updated_at: now,
        steps,
        version: appended.version,
        version_id: appended.id.0,
        schedule,
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
    Ok(version.definition(&w.name))
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
    // Both representations are copied verbatim, so a restore reproduces the
    // stored version exactly — layout included — rather than a graph that
    // merely migrates to the same shape.
    let appended = append_version(
        workflows,
        workflow_id,
        source.steps_json,
        source.definition_json,
        Some(format!("Restored from v{}", source.version)),
        now,
    )?;

    Ok(WorkflowWithSteps {
        updated_at: now,
        ..WorkflowWithSteps::joined(w, Some(appended))
    })
}

/// Load a version row and prove it belongs to the workflow the caller named.
fn version_of(
    workflows: &Arc<dyn WorkflowRepository>,
    workflow_id: &WorkflowId,
    version_id: &WorkflowVersionId,
) -> Result<WorkflowVersion, AppError> {
    let version = workflows
        .version_get(version_id)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found(format!("Workflow version not found: {version_id}")))?;
    ensure_owned_by(&version, workflow_id).map_err(AppError::validation)?;
    Ok(version)
}

/// Append an immutable version row and report what it became.
///
/// Every path that produces a version — an edit, a revert-to-default, a restore
/// from history — goes through here, so "saving is an append, never an edit"
/// stays one fact instead of three copies of the same numbering arithmetic.
/// `definition_json` is `None` for writes that have no authored v2 document —
/// a starter revert, a v1 import — and readers migrate `steps_json` for those,
/// exactly as they did before V34.
fn append_version(
    workflows: &Arc<dyn WorkflowRepository>,
    workflow_id: &WorkflowId,
    steps_json: String,
    definition_json: Option<String>,
    note: Option<String>,
    now: i64,
) -> Result<WorkflowVersion, AppError> {
    let existing = workflows.versions(workflow_id).map_err(AppError::from)?;
    let version = next_version_number(&existing);
    let row = WorkflowVersion {
        id: version_id(workflow_id, version),
        workflow_id: workflow_id.clone(),
        version,
        steps_json,
        definition_json,
        note,
        created_at: now,
    };
    workflows.save_version(row.clone())?;
    Ok(row)
}

/// Export the latest version as a **schema-v2 document, positions included**
/// (P3.6). Pre-P3.6 versions migrate on the way out, so every workflow
/// exports as v2 regardless of when it was saved.
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

    write_export(&latest.definition(&w.name), &w.description).map_err(AppError::from)
}

/// Import a workflow file of **either schema version** (P3.6).
///
/// A v2 document is stored as-is, positions and all — the other half of
/// `workflow_export` now emitting v2. A v1 steps-list file still imports: it
/// migrates on the way in, so files exported by older builds (and the
/// community's hand-written ones) keep working forever, which is the promise
/// PRD §10 makes about v1 documents.
///
/// The workflow always gets a **fresh id** so importing a file twice, or
/// importing one exported from this same install, can never overwrite an
/// existing workflow.
#[tauri::command]
pub fn workflow_import(
    json: String,
    ctx: State<'_, AppContext>,
) -> Result<WorkflowWithSteps, AppError> {
    let v: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let imported = read_import(&v).map_err(AppError::validation)?;

    save_definition(
        &ctx.workflows,
        None,
        &imported.name,
        &imported.description,
        imported.definition,
        Some("Imported".to_string()),
    )
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
    let starter = workflow_starters::find(STARTER_FILES, &wf_id).ok_or_else(|| {
        AppError::not_found("Starter pack source not found for this workflow id.")
    })?;

    let steps_json = starter.steps_json().map_err(|e| e.to_string())?;
    let now = paths::now_ms();
    workflows.update_meta(&wf_id, &starter.name, &starter.description)?;
    let appended = append_version(
        workflows,
        &wf_id,
        steps_json,
        None,
        Some("Reverted to default".to_string()),
        now,
    )?;
    Ok(WorkflowWithSteps {
        id: workflow_id,
        name: starter.name,
        description: starter.description,
        is_starter: true,
        created_at: w.created_at,
        updated_at: now,
        steps: starter.steps,
        version: appended.version,
        version_id: appended.id.0,
        schedule: w.schedule,
    })
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

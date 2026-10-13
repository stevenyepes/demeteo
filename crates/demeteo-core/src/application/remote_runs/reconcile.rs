use super::rpc::{json_str, remote_rpc};
use super::sequence_mirror::read_sequence_state_mirror;
use crate::adapters::artifact_store::fs::FsArtifactStore;
use crate::domain::artifact::{Artifact, ArtifactSource};
use crate::domain::ids::{FeatureId, ProjectId};
use crate::domain::models::{Feature, SequenceStateMirror, StepExecution};
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::db::{FeaturePatch, StepExecutionPatch};
use crate::ports::remote_run_mirror::RemoteRunMirror;
use crate::state::AppContext;

const HARD_TERMINAL: &[&str] = &["failed", "cancelled", "completed", "awaiting_mr"];
pub(super) const NOTIFY_ON: &[&str] = &[
    "awaiting_mr",
    "completed",
    "failed",
    "parked",
    "over-budget",
    "needs-credentials",
];

/// Which of a runner-side `Feature`'s columns the desktop's shadow row
/// mirrors on every poll.
///
/// Pure and separate from the `async fn` that does the RPC because *what
/// replicates* is a decision, and the way it goes wrong is invisible: `add`
/// serde-round-trips the whole `Feature`, so a field missing from this patch
/// still lands on the first poll and is simply never updated afterwards. The
/// symptom is therefore a stale value on a detached run only — no error, no
/// log line, and nothing a fixture that never hits the update branch would
/// catch. Being a free function over one value, it is assertable directly.
///
/// `effort`, `commit_artifacts`, `origin` and `diff_base_branch` are
/// deliberately absent: all four are launch inputs the desktop already holds,
/// and mirroring them back would let the runner's copy overwrite the local
/// pin. `resolved_branch` is not one of them — the runner cuts the branch, so
/// its name is an answer only the runner has.
///
/// `local_mr_state` is the shadow row's current value, for
/// [`shadow_mr_state`].
fn shadow_feature_patch(feature: &Feature, local_mr_state: Option<&str>) -> FeaturePatch {
    FeaturePatch {
        effort: None,
        status: Some(feature.status.clone()),
        total_cost: Some(Some(feature.total_cost)),
        duration: Some(Some(feature.duration.clone())),
        tokens: Some(Some(feature.tokens)),
        agent_kind: Some(feature.agent_kind.clone()),
        model: Some(feature.model.clone()),
        mr_url: Some(feature.mr_url.clone()),
        mr_state: Some(shadow_mr_state(local_mr_state, feature.mr_state.as_deref())),
        pr_title: Some(feature.pr_title.clone()),
        pr_body: Some(feature.pr_body.clone()),
        // The runner measured the baseline and the runner's own validate read
        // it, so the desktop needs it only to *show* the subtraction (V37,
        // decision 44) — but it is written mid-run and may be re-measured, so
        // the first poll's whole-`Feature` insert is not enough on its own.
        harness_baseline: Some(feature.harness_baseline.clone()),
        // The runner is authoritative for a detached run's assignment pins:
        // they are editable mid-run there, so the shadow must follow rather
        // than keep the list it was launched with.
        step_overrides: Some(feature.step_overrides.clone()),
        commit_artifacts: None,
        origin: None,
        diff_base_branch: None,
        resolved_branch: Some(feature.resolved_branch.clone()),
    }
}

/// The `mr_state` a detached run's shadow row keeps, given its current value
/// and the runner's.
///
/// The runner is not the only writer: the desktop `MrMonitor` polls the forge
/// and writes `merged` onto the shadow itself, and the runner's copy can lag
/// it by a poll or more. A landed ticket's lane, and every dependant it
/// released, derive from this column, so a lagging copy must never move it
/// back in flight. A merge is irreversible on every forge, so a local
/// `merged` is never overwritten. `closed` gets no such guard: a closed PR can be
/// reopened, and the runner seeing `open` again is the truth.
pub(super) fn shadow_mr_state(local: Option<&str>, runner: Option<&str>) -> Option<String> {
    match local {
        Some("merged") => local,
        _ => runner,
    }
    .map(str::to_string)
}

/// The status a detached run's shadow row shows, given the runner's own
/// `Feature.status` and the run-level status the mirror recorded.
///
/// The two disagree exactly when the run is blocked on the laptop. The
/// terminal credential park happens *after* the pipeline finished, so the
/// runner's feature already reads `completed` while the run waits for a PAT
/// to push — copied verbatim, the card banded a run that has published
/// nothing as done. A dangerous-gate park is reported through
/// `parked_gate_id` and likewise need not move the feature's own column.
/// Every surface that bands a run (`segmentFor`, the rail rollup) reads
/// `features.status`, so the blocking state has to be in that column, not
/// only in `remote_run_mirror`.
pub(super) fn shadow_feature_status(feature_status: &str, run_status: &str) -> String {
    match run_status {
        "needs-credentials" => "needs-credentials",
        "parked" if feature_status != "gated" => "awaiting_gate",
        _ => feature_status,
    }
    .to_string()
}

/// Puts the run-level block onto a shadow row the runner has no feature
/// for yet — the pre-clone credential park, which stops before the runner
/// bootstraps one, so `get_feature` has nothing to hydrate from.
fn mark_unhydrated_shadow(ctx: &AppContext, feature_id: &str, run_status: &str) {
    let feature_id = FeatureId::from(feature_id.to_string());
    let Ok(Some(local)) = ctx.features.get(&feature_id) else {
        return;
    };
    let status = shadow_feature_status(&local.status, run_status);
    if status == local.status {
        return;
    }
    let patch = FeaturePatch {
        status: Some(status),
        ..Default::default()
    };
    if let Err(error) = ctx.features.update(&feature_id, &patch) {
        eprintln!("shadow status write failed for feature {feature_id}: {error}");
    }
}

pub(super) async fn hydrate_shadow_feature(
    ctx: &AppContext,
    machine_id: &str,
    run_id: &str,
    local_project_id: &str,
    canonical_id: &str,
    run_status: &str,
) -> Result<(), String> {
    let feature_value = remote_rpc(
        ctx,
        machine_id,
        "get_feature",
        serde_json::json!({ "run_id": run_id }),
    )
    .await?;
    if feature_value.is_null() {
        mark_unhydrated_shadow(ctx, canonical_id, run_status);
        return Ok(());
    }
    let mut feature: Feature = serde_json::from_value(feature_value)
        .map_err(|error| format!("shadow feature decode: {error}"))?;
    let canonical = FeatureId::from(canonical_id.to_string());
    feature.project_id = ProjectId::new(local_project_id);
    feature.id = canonical;
    feature.status = shadow_feature_status(&feature.status, run_status);

    let steps_value = remote_rpc(
        ctx,
        machine_id,
        "list_steps",
        serde_json::json!({ "run_id": run_id }),
    )
    .await?;
    let steps: Vec<StepExecution> = serde_json::from_value(steps_value)
        .map_err(|error| format!("shadow steps decode: {error}"))?;
    let feature_id = feature.id.clone();
    match ctx.features.get(&feature_id)? {
        None => ctx.features.add(feature.clone())?,
        Some(local) => ctx.features.update(
            &feature_id,
            &shadow_feature_patch(&feature, local.mr_state.as_deref()),
        )?,
    }

    let store = FsArtifactStore::new(ctx.app_data_dir.clone());
    for step in steps {
        let force_refresh =
            shadow_step_artifacts_stale(ctx.features.step_get(&step.id)?.as_ref(), &step);
        let local_paths = cache_step_artifacts(
            ctx,
            &store,
            machine_id,
            run_id,
            feature_id.as_str(),
            &step,
            force_refresh,
        )
        .await;
        let existing_shadow = ctx.features.step_get(&step.id)?;
        let (single, local_paths) = if local_paths.is_empty() {
            match existing_shadow.as_ref() {
                Some(existing) => (
                    existing.artifact_path.clone(),
                    existing.artifact_paths.clone(),
                ),
                None => (None, local_paths),
            }
        } else {
            (local_paths.first().cloned(), local_paths)
        };
        if existing_shadow.is_none() {
            let mut shadow = step.clone();
            shadow.feature_id = feature_id.clone();
            shadow.artifact_path = single;
            shadow.artifact_paths = local_paths;
            ctx.features.step_create(shadow)?;
        } else {
            ctx.features.step_update(
                &step.id,
                &StepExecutionPatch {
                    status: Some(step.status.clone()),
                    cost_usd: Some(step.cost_usd),
                    tokens: Some(step.tokens),
                    wall_clock_secs: Some(step.wall_clock_secs),
                    error_message: Some(step.error_message.clone()),
                    artifact_path: Some(single),
                    artifact_paths: Some(local_paths),
                    ..Default::default()
                },
            )?;
        }

        if step.step_kind == "sequence" {
            hydrate_sequence_state(ctx, machine_id, run_id, &feature_id, &step, force_refresh)
                .await;
        }
    }
    Ok(())
}

/// C4.1/C4.2: mirror a detached run's `sequence`-step resume state — the
/// plan cache, the landed-task checkpoint, and the per-task run rows — onto
/// the laptop, so `RunView::sequence_state` finds real rows locally instead
/// of reading `unplanned` forever (the bug this closes: nothing ever wrote
/// these three tables for a runner-owned feature). When to ask is
/// [`sequence_state_needs_fetch`]; what crosses the wire is kept small by
/// sending the local copy's revision, so an unchanged node costs one tiny
/// round trip rather than the whole plan.
///
/// Never fails the caller: an older runner without `get_sequence_state`
/// (or any other RPC/decode failure) leaves the local tables untouched and
/// `sequence_state` keeps degrading to `unplanned`, exactly as it did
/// before this RPC existed.
async fn hydrate_sequence_state(
    ctx: &AppContext,
    machine_id: &str,
    run_id: &str,
    feature_id: &FeatureId,
    step: &StepExecution,
    force_refresh: bool,
) {
    let node_id = step.step_id.as_str();
    let local =
        read_sequence_state_mirror(&*ctx.features, &*ctx.sequence_resume, feature_id, node_id).ok();
    let plan_missing = local.as_ref().is_none_or(|l| l.plan_json.is_none());
    if !sequence_state_needs_fetch(plan_missing, force_refresh, &step.status) {
        return;
    }

    let value = match remote_rpc(
        ctx,
        machine_id,
        "get_sequence_state",
        serde_json::json!({
            "run_id": run_id,
            "node_id": node_id,
            "if_revision": local.as_ref().map(SequenceStateMirror::revision),
        }),
    )
    .await
    {
        Ok(value) if value.get("unchanged").and_then(|v| v.as_bool()) == Some(true) => return,
        Ok(value) => value,
        Err(error) => {
            if !error.starts_with("unknown method") {
                eprintln!(
                    "shadow sequence-state fetch failed for run {run_id} node {node_id}: {error}"
                );
            }
            return;
        }
    };
    let state: SequenceStateMirror = match serde_json::from_value(value) {
        Ok(state) => state,
        Err(error) => {
            eprintln!(
                "shadow sequence-state decode failed for run {run_id} node {node_id}: {error}"
            );
            return;
        }
    };
    let Some(plan_json) = state.plan_json else {
        return;
    };

    let now = crate::paths::now_ms();
    if let Err(error) = ctx
        .sequence_resume
        .plan_cache_put(feature_id, node_id, &plan_json, None, now)
    {
        eprintln!("shadow plan cache write failed for run {run_id} node {node_id}: {error}");
        return;
    }
    if let Err(error) = ctx.sequence_resume.sequence_checkpoint_set(
        feature_id,
        node_id,
        &state.checkpoint.landed_task_ids,
        state.checkpoint.anchor_sha.as_deref(),
        state.checkpoint.produced.as_ref(),
        now,
    ) {
        eprintln!("shadow checkpoint write failed for run {run_id} node {node_id}: {error}");
        return;
    }
    if let Err(error) =
        ctx.features
            .subtask_runs_replace_for_step(feature_id, &step.id, &state.subtask_runs)
    {
        eprintln!("shadow subtask runs write failed for run {run_id} node {node_id}: {error}");
    }
}

/// Whether a poll asks the runner for a `sequence` node's task list.
///
/// A `running` step is asked every time because its own counters cannot
/// say when to: the runner rolls cost, tokens and wall-clock into the step
/// row only when an attempt ends, so a sequence working through its tickets
/// looks identical to [`shadow_step_artifacts_stale`] for the whole attempt.
/// Gating on that alone froze the laptop's list at whatever the attempt
/// started with — an interrupted ticket 1 shown for an hour while the
/// runner landed tickets 1–4. Any other status changes only through a
/// status transition, which `force_refresh` already carries.
fn sequence_state_needs_fetch(plan_missing: bool, force_refresh: bool, step_status: &str) -> bool {
    plan_missing || force_refresh || step_status == "running"
}

fn shadow_step_artifacts_stale(existing: Option<&StepExecution>, fresh: &StepExecution) -> bool {
    let Some(existing) = existing else {
        return false;
    };
    existing.status != fresh.status
        || existing.tokens != fresh.tokens
        || existing.wall_clock_secs != fresh.wall_clock_secs
        || existing.cost_usd != fresh.cost_usd
}

pub(super) async fn cache_step_artifacts(
    ctx: &AppContext,
    store: &FsArtifactStore,
    machine_id: &str,
    run_id: &str,
    feature_id: &str,
    step: &StepExecution,
    force_refresh: bool,
) -> Vec<String> {
    let remote = declared_remote_paths(step.artifact_path.as_deref(), &step.artifact_paths);
    if remote.is_empty() {
        return Vec::new();
    }
    let existing = store
        .list_for_step(feature_id, step.id.as_str())
        .unwrap_or_default();
    if !force_refresh && !existing.is_empty() && existing.len() >= remote.len() {
        return existing;
    }

    let mut local = Vec::new();
    for path in remote {
        let fetched = match remote_rpc(
            ctx,
            machine_id,
            "read_artifact",
            serde_json::json!({ "run_id": run_id, "path": path }),
        )
        .await
        {
            Ok(value) => {
                let body = value.as_str().unwrap_or_default().to_string();
                let name = std::path::Path::new(&path)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("artifact")
                    .to_string();
                let artifact = Artifact {
                    name,
                    mime: mime_for_path(&path),
                    content: body,
                    source: ArtifactSource::ToolWrite { path: path.clone() },
                };
                match store.put(feature_id, step.id.as_str(), &artifact) {
                    Ok(local_ref) => Some(local_ref),
                    Err(error) => {
                        eprintln!("shadow artifact cache write failed for {path}: {error}");
                        None
                    }
                }
            }
            Err(error) => {
                eprintln!("shadow artifact fetch failed for {path}: {error}");
                None
            }
        };
        let resolved = fetched.or_else(|| backfill_local_path(&existing, &path));
        match resolved {
            Some(local_ref) if !local.contains(&local_ref) => local.push(local_ref),
            Some(_) | None => {}
        }
    }
    local
}

fn backfill_local_path(existing: &[String], remote_path: &str) -> Option<String> {
    let stem = std::path::Path::new(remote_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("artifact");
    let safe_stem: String = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    let prefix = format!("{safe_stem}.");
    existing
        .iter()
        .find(|path| {
            std::path::Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix))
        })
        .cloned()
}

fn declared_remote_paths(single: Option<&str>, many: &[String]) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(path) = single {
        paths.push(path.to_string());
    }
    for path in many {
        if !paths.iter().any(|existing| existing == path) {
            paths.push(path.clone());
        }
    }
    paths
}

fn mime_for_path(path: &str) -> String {
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    match extension {
        "md" | "markdown" => "text/markdown",
        "diff" | "patch" => "text/x-diff",
        "json" => "application/json",
        "html" => "text/html",
        _ => "text/plain",
    }
    .to_string()
}

pub(super) async fn reconcile_one_run(
    ctx: &AppContext,
    row: &RemoteRunMirror,
) -> Option<(String, Option<String>)> {
    // `reconcile_all_runs` intentionally lists before its runner awaits. A
    // cleanup can dismiss a row in that interval, so reclaim it while sharing
    // cleanup's guard before any runner status is persisted or hydrated.
    let _guard = ctx.remote_run_mirror_guard.lock().await;
    let row = match ctx.remote_run_mirror.get(&row.machine_id, &row.run_id) {
        Ok(Some(row)) => row,
        Ok(None) | Err(_) => return None,
    };
    let now = crate::paths::now_ms();
    let result = remote_rpc(
        ctx,
        &row.machine_id,
        "get_status",
        serde_json::json!({ "run_id": row.run_id }),
    )
    .await;
    match result {
        Ok(value) => {
            let status = if json_str(&value, "parked_gate_id").is_some() {
                "parked".to_string()
            } else {
                json_str(&value, "status").unwrap_or_else(|| row.status.clone())
            };
            let error = json_str(&value, "error");
            let remote_feature_id = json_str(&value, "feature_id");
            let mr_url = json_str(&value, "mr_url");
            let pushed_branch = json_str(&value, "pushed_branch");
            let canonical_feature_id =
                match (row.feature_id.as_deref(), remote_feature_id.as_deref()) {
                    (Some(local), Some(remote)) if !remote.is_empty() && local != remote => {
                        eprintln!(
                            "remote run {}: runner reports feature {remote} but the laptop \
                             expected {local} (runner predates RunSpec::feature_id?) — \
                             pinning the laptop's id and re-homing the shadow onto it",
                            row.run_id
                        );
                        Some(local.to_string())
                    }
                    _ => remote_feature_id.clone().or_else(|| row.feature_id.clone()),
                };
            let _ = ctx.remote_run_mirror.update_status(
                &row.machine_id,
                &row.run_id,
                &status,
                error.as_deref(),
                canonical_feature_id.as_deref(),
                mr_url.as_deref(),
                pushed_branch.as_deref(),
                0,
                now,
            );
            if let (Some(feature_id), Some(project_id)) = (&canonical_feature_id, &row.project_id) {
                if !feature_id.is_empty() {
                    if let Err(error) = hydrate_shadow_feature(
                        ctx,
                        &row.machine_id,
                        &row.run_id,
                        project_id,
                        feature_id,
                        &status,
                    )
                    .await
                    {
                        mark_unhydrated_shadow(ctx, feature_id, &status);
                        eprintln!(
                            "shadow hydrate failed for run {} (feature {feature_id}): {error}",
                            row.run_id
                        );
                    }
                }
            }
            Some((status, error))
        }
        Err(_) if HARD_TERMINAL.contains(&row.status.as_str()) => None,
        Err(_) => {
            let _ = ctx.remote_run_mirror.update_status(
                &row.machine_id,
                &row.run_id,
                "unreachable",
                None,
                None,
                None,
                None,
                0,
                now,
            );
            None
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/application/remote_runs/reconcile.rs"]
mod tests;

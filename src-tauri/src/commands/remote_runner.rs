//! Laptop-side control-channel commands (docs/REMOTE_EXECUTION_PLAN.md
//! M6.1). Everything here goes over `ExecutionPort::control_rpc`, which
//! reaches `demeteo-runner`'s Unix-socket RPC server via OpenSSH
//! Unix-socket forwarding (R4) — see
//! `crates/demeteo-core/src/adapters/ssh/client.rs`.

use crate::adapters::artifact_store::fs::FsArtifactStore;
use crate::domain::artifact::{Artifact, ArtifactSource};
use crate::domain::ids::{FeatureId, ProjectId, WorkflowId};
use crate::domain::models::feature::{Feature, StepExecution};
use crate::domain::run_spec::{RunBudget, RunSpec, RunSpecProvider};
use crate::error::AppError;
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::db::{FeaturePatch, StepExecutionPatch};
use crate::ports::remote_run_mirror::RemoteRunMirror;
use crate::state::AppContext;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_notification::NotificationExt;

#[derive(Serialize)]
pub struct RemoteRunHandle {
    pub run_id: String,
    pub machine_id: String,
    pub status: String,
    /// Id of the eager shadow Feature inserted at submit time — the
    /// frontend navigates straight to `FeatureDetail` with it, the same
    /// landing a local launch gets.
    pub feature_id: String,
}

/// Flip the eager placeholder Feature to `failed` when the submit RPC
/// itself fails — the run never started, and a perpetually-"pending"
/// ghost row would otherwise sit in the project's pipeline list.
fn mark_placeholder_failed(ctx: &AppContext, feature_id: &str) {
    let _ = ctx.features.update(
        &FeatureId::from(feature_id.to_string()),
        &FeaturePatch {
            status: Some("failed".to_string()),
            ..Default::default()
        },
    );
}

fn json_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|s| s.to_string())
}

/// Compose a [`RunSpec`] from the laptop's own DB state (project's first
/// repository + its provider instance, the chosen workflow's latest
/// version) and submit it to `machine_id`'s `demeteo-runner` over the
/// control channel: `submit_run` then `inject_credentials` (M4.2) with
/// the provider PAT the laptop already holds in its keyring — the
/// runner never gets a standing git secret (§6.2).
///
/// A machine missing the selected agent is rejected synchronously by the
/// runner's own `submit_run` (M4.1's agent-readiness precondition); that
/// comes back as `status: "failed"` with `error` set, which this command
/// turns into an `Err` instead of reporting a launched-but-doomed run.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn remote_submit_run(
    ctx: State<'_, AppContext>,
    machine_id: String,
    project_id: String,
    workflow_id: String,
    title: String,
    description: String,
    agent_kind: Option<String>,
    model: Option<String>,
    loop_iterations: Option<u32>,
    unattended: bool,
    max_cost_usd: Option<f64>,
    max_wall_clock_secs: Option<u64>,
) -> Result<RemoteRunHandle, AppError> {
    let pid = ProjectId::from(project_id.clone());
    let repos = ctx
        .projects
        .get_repositories_for(&pid)
        .map_err(AppError::from)?;
    let repo = repos.first().ok_or_else(|| {
        AppError::from("Project has no repository configured; remote runs need one".to_string())
    })?;

    let providers = ctx
        .app_settings
        .get_provider_instances()
        .map_err(AppError::from)?;
    let provider = providers
        .into_iter()
        .find(|p| p.id == repo.provider_id)
        .ok_or_else(|| {
            AppError::from("Repository's git provider instance is not configured".to_string())
        })?;

    let wf_id = WorkflowId::from(workflow_id.clone());
    let workflow = ctx
        .workflows
        .get(&wf_id)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found(format!("Workflow not found: {}", workflow_id)))?;
    let latest = ctx
        .workflows
        .latest_version(&wf_id)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::from("Workflow has no steps".to_string()))?;
    let steps: serde_json::Value =
        serde_json::from_str(&latest.steps_json).map_err(|e| AppError::from(e.to_string()))?;
    let workflow_json = serde_json::json!({
        "name": workflow.name,
        "description": workflow.description,
        "steps": steps,
    });

    // Same PAT source `GitOpsHelper::clone_repository` already uses for
    // laptop-driven clones (§6.2) — the runner gets it over the tunnel
    // via `inject_credentials`, never written to its own disk.
    let git_ops = crate::adapters::worktree::git_ops::GitOpsHelper::new(
        ctx.app_settings.clone(),
        ctx.exec.clone(),
    );
    let pat = git_ops
        .get_provider_pat(&provider.id.0)
        .map_err(AppError::from)?;

    let budget = if max_cost_usd.is_some() || max_wall_clock_secs.is_some() {
        Some(RunBudget {
            max_cost_usd,
            max_wall_clock_secs,
        })
    } else {
        None
    };

    // Eager shadow feature: the laptop chooses the feature id, inserts a
    // placeholder Feature it can navigate to immediately, and ships the
    // id in the spec so the runner's `feature_start` reuses it. From
    // second zero the run is one Feature on both databases; the first
    // reconcile's hydration (C4.2) updates this row in place instead of
    // creating a late twin.
    let now = crate::paths::now_ms();
    let feature_id = format!("f-{}", crate::paths::new_id());
    ctx.features
        .add(Feature {
            id: FeatureId::from(feature_id.clone()),
            project_id: pid.clone(),
            workflow_id: Some(wf_id.clone()),
            title: title.clone(),
            status: "pending".to_string(),
            total_cost: 0.0,
            duration: "0s".to_string(),
            tokens: 0,
            created_at: now,
            agent_kind: agent_kind.clone(),
            model: model.clone(),
            mr_url: None,
            mr_state: Some("none".to_string()),
            commit_artifacts: None,
            loop_iterations,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
        })
        .map_err(AppError::from)?;

    let spec = RunSpec {
        feature_id: Some(feature_id.clone()),
        title: title.clone(),
        description,
        provider: RunSpecProvider {
            kind: provider.kind.clone(),
            host: provider.host.clone(),
        },
        repo_path: repo.repo_path.clone(),
        workflow_json,
        agent_kind,
        model,
        loop_iterations,
        unattended,
        budget,
    };

    // Laptop-generated idempotency key (R9/M3.2): safe to retry
    // `remote_submit_run` with network hiccups mid-call without risking
    // a duplicate feature on the runner.
    let run_id = format!("laptop-{}", crate::paths::new_id());

    let submitted = match ctx
        .exec
        .control_rpc(
            &machine_id,
            "submit_run",
            serde_json::json!({ "run_id": run_id, "spec": spec }),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            mark_placeholder_failed(&ctx, &feature_id);
            return Err(AppError::from(e));
        }
    };
    let status = json_str(&submitted, "status").unwrap_or_else(|| "pending".to_string());
    if status == "failed" {
        let err = json_str(&submitted, "error")
            .unwrap_or_else(|| "the runner rejected this run".to_string());
        mark_placeholder_failed(&ctx, &feature_id);
        return Err(AppError::from(err));
    }

    // Mirror the run before injecting credentials: if the injection RPC
    // fails, the run already exists on the runner (it will park
    // `needs-credentials`), and an unmirrored run would be invisible to
    // reconcile forever.
    ctx.remote_run_mirror
        .upsert_submitted(
            &machine_id,
            &run_id,
            Some(&project_id),
            Some(&feature_id),
            &title,
            now,
        )
        .map_err(AppError::from)?;
    ctx.remote_run_mirror
        .update_status(
            &machine_id,
            &run_id,
            &status,
            None,
            None,
            None,
            None,
            0,
            now,
        )
        .map_err(AppError::from)?;

    ctx.exec
        .control_rpc(
            &machine_id,
            "inject_credentials",
            serde_json::json!({ "run_id": run_id, "git_pat": pat }),
        )
        .await
        .map_err(AppError::from)?;

    Ok(RemoteRunHandle {
        run_id,
        machine_id,
        status,
        feature_id,
    })
}

/// Cheap read of the laptop's own mirror — no network I/O. Used to
/// paint the return inbox (M6.2) instantly on app open; the caller
/// follows up with [`remote_reconcile_runs`] to refresh it.
#[tauri::command]
pub fn remote_list_mirrored_runs(
    ctx: State<'_, AppContext>,
) -> Result<Vec<RemoteRunMirror>, AppError> {
    ctx.remote_run_mirror.list().map_err(AppError::from)
}

/// Statuses that can never change again — a mirror row can't regress
/// out of these on its own account (a *new* run would get a new
/// `run_id`), so `unreachable` after a failed poll would only ever hide
/// an already-final outcome. Kept intentionally narrower than "not
/// running": `unreachable` must never overwrite them (§7.1 — a machine
/// that's merely off is *paused*, not *failed*).
const HARD_TERMINAL: &[&str] = &["failed", "cancelled"];

/// Statuses worth an OS-level desktop notification on reconcile (design
/// §8's taxonomy table — "Running"/"Unreachable" are silent, everything
/// else "raises"). Diffed against `last_notified_status` so a status
/// that hasn't changed since the last reconcile doesn't re-notify every
/// time the inbox polls.
const NOTIFY_ON: &[&str] = &[
    "awaiting_mr",
    "completed",
    "failed",
    "parked",
    "over-budget",
    "needs-credentials",
];

/// C4.2: hydrate a read-only *shadow* of a runner-owned feature into the
/// laptop DB + artifact cache, so `RunView` (C4.3) can render it with the
/// same fidelity as a native run (steps, cost, artifacts) without the UI
/// knowing it ran on a different machine.
///
/// **The mirror row is itself the runner-owned marker.** A feature whose
/// id appears in `remote_run_mirror` is a shadow the laptop only ever
/// *reads* — the engine never drives it (there is no local run behind
/// it), so no extra "read-only" column is needed on `features`.
///
/// Fetches the runner's own `Feature` + `StepExecution` rows over the
/// control channel, re-parents the feature to the **local** `project_id`
/// (so the `features.project_id` FK holds and the inbox deep-links into a
/// local project view — the runner's own project id doesn't exist on the
/// laptop), and upserts. Each step's declared artifacts are pulled **once**
/// into the laptop `FsArtifactStore` and the shadow step's paths rewritten
/// to those local references, so the artifact viewer reads them as ordinary
/// local files with no remote round-trip. Idempotent and best-effort: safe
/// to call on every reconcile; a captured artifact belongs to a finished
/// step and is never re-pulled (the offset-equivalent gate).
async fn hydrate_shadow_feature(
    ctx: &AppContext,
    machine_id: &str,
    run_id: &str,
    local_project_id: &str,
) -> Result<(), String> {
    let feat_val = ctx
        .exec
        .control_rpc(
            machine_id,
            "get_feature",
            serde_json::json!({ "run_id": run_id }),
        )
        .await?;
    // The run may not have bootstrapped a feature yet (early states), in
    // which case `get_feature` reports `null` — nothing to shadow.
    if feat_val.is_null() {
        return Ok(());
    }
    let mut feature: Feature =
        serde_json::from_value(feat_val).map_err(|e| format!("shadow feature decode: {e}"))?;
    // Re-home under the local project so the FK holds; keep the runner's
    // feature id (== the mirror's `feature_id`) so status/inbox deep-links
    // resolve to this same shadow.
    feature.project_id = ProjectId::new(local_project_id);

    let steps_val = ctx
        .exec
        .control_rpc(
            machine_id,
            "list_steps",
            serde_json::json!({ "run_id": run_id }),
        )
        .await?;
    let steps: Vec<StepExecution> =
        serde_json::from_value(steps_val).map_err(|e| format!("shadow steps decode: {e}"))?;

    let fid = feature.id.clone();
    if ctx.features.get(&fid)?.is_none() {
        ctx.features.add(feature.clone())?;
    } else {
        // Refresh the mutable surface a re-reconcile can change (status,
        // aggregate cost/tokens, PR url) — the shadow tracks the runner.
        ctx.features.update(
            &fid,
            &FeaturePatch {
                status: Some(feature.status.clone()),
                total_cost: Some(Some(feature.total_cost)),
                duration: Some(Some(feature.duration.clone())),
                tokens: Some(Some(feature.tokens)),
                agent_kind: Some(feature.agent_kind.clone()),
                model: Some(feature.model.clone()),
                mr_url: Some(feature.mr_url.clone()),
                mr_state: Some(feature.mr_state.clone()),
            },
        )?;
    }

    let store = FsArtifactStore::new(ctx.app_data_dir.clone());
    for step in steps {
        let local_paths =
            cache_step_artifacts(ctx, &store, machine_id, run_id, fid.as_str(), &step).await;
        let single = local_paths.first().cloned();
        if ctx.features.step_get(&step.id)?.is_none() {
            let mut shadow = step.clone();
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
    }
    Ok(())
}

/// Pull a shadow step's declared artifacts into the laptop artifact cache
/// once, returning the *local* references to store on the shadow step. If
/// the laptop already cached artifacts for this step, they are reused with
/// no remote read — a captured artifact belongs to a finished step and
/// won't change, which is the offset-equivalent "don't re-pull" gate C4.2
/// calls for. A per-artifact fetch failure is surfaced (logged) and
/// skipped rather than aborting the whole hydration.
async fn cache_step_artifacts(
    ctx: &AppContext,
    store: &FsArtifactStore,
    machine_id: &str,
    run_id: &str,
    feature_id: &str,
    step: &StepExecution,
) -> Vec<String> {
    let remote = declared_remote_paths(step.artifact_path.as_deref(), &step.artifact_paths);
    if remote.is_empty() {
        return Vec::new();
    }

    // Offset-equivalent gate: already-cached artifacts are not re-pulled.
    if let Ok(existing) = store.list_for_step(feature_id, step.id.as_str()) {
        if !existing.is_empty() && existing.len() >= remote.len() {
            return existing;
        }
    }

    let mut local = Vec::new();
    for path in remote {
        match ctx
            .exec
            .control_rpc(
                machine_id,
                "read_artifact",
                serde_json::json!({ "run_id": run_id, "path": path }),
            )
            .await
        {
            Ok(v) => {
                let body = v.as_str().unwrap_or_default().to_string();
                let name = std::path::Path::new(&path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("artifact")
                    .to_string();
                // `ToolWrite` makes the store infer the file extension from
                // the runner's path (preserving `.md`/`.diff`/…) and write
                // the fetched `body` verbatim — it never re-reads from disk.
                let artifact = Artifact {
                    name,
                    mime: mime_for_path(&path),
                    content: body,
                    source: ArtifactSource::ToolWrite { path: path.clone() },
                };
                match store.put(feature_id, step.id.as_str(), &artifact) {
                    Ok(local_ref) => local.push(local_ref),
                    Err(e) => {
                        eprintln!("shadow artifact cache write failed for {path}: {e}");
                    }
                }
            }
            Err(e) => {
                eprintln!("shadow artifact fetch failed for {path}: {e}");
            }
        }
    }
    local
}

/// A shadow step's declared artifact paths on the runner, de-duplicated
/// with the legacy single `artifact_path` first. These are the exact
/// paths the `read_artifact` RPC will be asked for (and which the runner
/// re-validates against its own step rows), so keeping the set minimal
/// keeps the number of remote reads minimal.
fn declared_remote_paths(single: Option<&str>, many: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(p) = single {
        out.push(p.to_string());
    }
    for p in many {
        if !out.iter().any(|e| e == p) {
            out.push(p.clone());
        }
    }
    out
}

/// Best-effort media type from a path's extension — only used to populate
/// the cached `Artifact`; the on-disk filename's extension comes from the
/// path itself via `ArtifactSource::ToolWrite`.
fn mime_for_path(path: &str) -> String {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    match ext {
        "md" | "markdown" => "text/markdown",
        "diff" | "patch" => "text/x-diff",
        "json" => "application/json",
        "html" => "text/html",
        _ => "text/plain",
    }
    .to_string()
}

/// Poll one run's runner for status, apply it to the mirror, and — once
/// the run has bootstrapped a feature — hydrate its read-only shadow
/// (C4.2). Shared by the whole-inbox reconcile (M6.2/M6.3) and the
/// single-run live refresh (M6.4, [`remote_refresh_run`]). On a *fresh*
/// poll returns `Some((status, error))` so the caller can run its own
/// notification diff — this helper itself never notifies. Returns `None`
/// when there was no fresh status from the runner (machine unreachable),
/// so the caller does not notify off stale data — matching the original
/// reconcile, which only ever notified on a successful poll. Mirrors the
/// `unreachable`-vs-known-terminal rule (§7.1): a machine we can't reach
/// flips a live row to `unreachable` but leaves an already-terminal row
/// untouched.
async fn reconcile_one_run(
    ctx: &AppContext,
    row: &RemoteRunMirror,
) -> Option<(String, Option<String>)> {
    let now = crate::paths::now_ms();
    let result = ctx
        .exec
        .control_rpc(
            &row.machine_id,
            "get_status",
            serde_json::json!({ "run_id": row.run_id }),
        )
        .await;
    match result {
        Ok(v) => {
            // A dangerous parked gate (M5.1) doesn't move the runner's own
            // coarse `status` column — the feature is still nominally
            // "running", just blocked on one decision — so `parked_gate_id`
            // (set only in that case) wins over whatever `status` says.
            let status = if json_str(&v, "parked_gate_id").is_some() {
                "parked".to_string()
            } else {
                json_str(&v, "status").unwrap_or_else(|| row.status.clone())
            };
            let error = json_str(&v, "error");
            let feature_id = json_str(&v, "feature_id");
            let mr_url = json_str(&v, "mr_url");
            let pushed_branch = json_str(&v, "pushed_branch");
            // Version skew: an old runner that ignored `RunSpec::feature_id`
            // reports its own generated id. The runner's id must win —
            // hydration keys the shadow off the runner's `get_feature` id,
            // so keeping the laptop's eager id would strand the shadow
            // forever. Retire the orphaned placeholder so it doesn't sit
            // "pending" in the pipeline list for eternity.
            if let (Some(local), Some(remote)) = (row.feature_id.as_deref(), feature_id.as_deref())
            {
                if !remote.is_empty() && local != remote {
                    eprintln!(
                        "remote run {}: runner reports feature {remote} but the laptop \
                         expected {local} (runner predates RunSpec::feature_id?) — \
                         adopting the runner's id",
                        row.run_id
                    );
                    mark_placeholder_failed(ctx, local);
                }
            }
            let _ = ctx.remote_run_mirror.update_status(
                &row.machine_id,
                &row.run_id,
                &status,
                error.as_deref(),
                feature_id.as_deref(),
                mr_url.as_deref(),
                pushed_branch.as_deref(),
                0,
                now,
            );

            // C4.2: once the run has bootstrapped a feature and we know the
            // local project it was composed from, hydrate a read-only shadow
            // of it (feature + steps + cached artifacts) so it renders on the
            // laptop with full fidelity. Best-effort: a hydration failure
            // never regresses the mirror status just applied.
            if let (Some(fid), Some(pid)) = (&feature_id, &row.project_id) {
                if !fid.is_empty() {
                    if let Err(e) =
                        hydrate_shadow_feature(ctx, &row.machine_id, &row.run_id, pid).await
                    {
                        eprintln!(
                            "shadow hydrate failed for run {} (feature {fid}): {e}",
                            row.run_id
                        );
                    }
                }
            }
            Some((status, error))
        }
        Err(_) if HARD_TERMINAL.contains(&row.status.as_str()) => {
            // Already know how this run ended; an unreachable machine
            // doesn't change that (and there's no fresh status to notify on).
            None
        }
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

/// Reconcile every mirrored run against its runner's live `get_status`
/// (docs/REMOTE_EXECUTION_PLAN.md M6.2, design R9). A machine that can't
/// be reached flips the mirror to `unreachable` — never `failed` (§7.1)
/// — unless the row is already known-terminal, in which case there's
/// nothing left to learn from a retry.
///
/// M6.3's "reconcile-on-reopen" channel: any row whose status just
/// transitioned into [`NOTIFY_ON`] raises a desktop notification here,
/// so "PR ready"/"failed"/"parked"/"needs-credentials" outcomes that
/// happened while the app was closed surface the moment it reopens and
/// calls this (not just silently updating the inbox list). Collected
/// across the whole pass and shown as a single notification — reopening
/// after several runs finished while away shouldn't spam one OS
/// notification per run.
#[tauri::command]
pub async fn remote_reconcile_runs(
    app: AppHandle,
    ctx: State<'_, AppContext>,
) -> Result<Vec<RemoteRunMirror>, AppError> {
    let rows = ctx.remote_run_mirror.list().map_err(AppError::from)?;
    let mut notify_bodies: Vec<String> = Vec::new();
    for row in &rows {
        let Some((status, error)) = reconcile_one_run(&ctx, row).await else {
            continue;
        };

        if NOTIFY_ON.contains(&status.as_str())
            && row.last_notified_status.as_deref() != Some(status.as_str())
        {
            let body = match status.as_str() {
                "awaiting_mr" | "completed" => format!("{} — PR ready", row.title),
                "failed" => format!(
                    "{} — failed{}",
                    row.title,
                    error
                        .as_deref()
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default()
                ),
                "parked" => format!("{} — parked, needs your decision", row.title),
                "over-budget" => format!("{} — hit its budget cap", row.title),
                "needs-credentials" => {
                    format!("{} — needs credentials re-injected", row.title)
                }
                _ => row.title.clone(),
            };
            notify_bodies.push(body);
            let _ = ctx
                .remote_run_mirror
                .mark_notified(&row.machine_id, &row.run_id, &status);
        }
    }

    if !notify_bodies.is_empty() {
        let (title, body) = if notify_bodies.len() == 1 {
            ("Demeteo — remote run".to_string(), notify_bodies[0].clone())
        } else {
            (
                format!(
                    "Demeteo — {} remote runs need attention",
                    notify_bodies.len()
                ),
                notify_bodies.join("\n"),
            )
        };
        let _ = app.notification().builder().title(title).body(body).show();
    }

    ctx.remote_run_mirror.list().map_err(AppError::from)
}

/// Single-run live refresh (docs/REMOTE_EXECUTION_PLAN.md M6.4). Polls
/// exactly one remote run's runner, applies its status, re-hydrates its
/// shadow feature, and returns the updated mirror row. Unlike
/// [`remote_reconcile_runs`] it touches a single run and **never** fires a
/// desktop notification — reopen-reconcile owns that channel; a run the
/// user is actively watching shouldn't double-notify. This is the poll
/// `FeatureDetail` drives while a remote run is open, so the mirrored
/// steps/status update live the way a *local* run's Tauri events drive its
/// timeline. `None` means the run isn't mirrored on this laptop.
#[tauri::command]
pub async fn remote_refresh_run(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
) -> Result<Option<RemoteRunMirror>, AppError> {
    let Some(row) = ctx
        .remote_run_mirror
        .get(&machine_id, &run_id)
        .map_err(AppError::from)?
    else {
        return Ok(None);
    };
    reconcile_one_run(&ctx, &row).await;
    ctx.remote_run_mirror
        .get(&machine_id, &run_id)
        .map_err(AppError::from)
}

/// Resolve the mirror row for a shadow feature (M6.4). `FeatureDetail`
/// calls this once per feature to learn whether it's a remote run and, if
/// so, which `(machine_id, run_id)` to live-refresh — a locally-run
/// feature simply isn't in the mirror and yields `None`.
#[tauri::command]
pub fn remote_run_for_feature(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<Option<RemoteRunMirror>, AppError> {
    let rows = ctx.remote_run_mirror.list().map_err(AppError::from)?;
    Ok(rows
        .into_iter()
        .find(|r| r.feature_id.as_deref() == Some(feature_id.as_str())))
}

/// Fresh (non-mirrored) `get_status`, including the `parked_gate_id`
/// the mirror collapses into a plain `"parked"` status string. The
/// inbox calls this right before showing the "clear gate" action so it
/// always has the live `gate_id` `remote_decide_gate` needs, without a
/// schema change to carry it through the mirror table.
#[tauri::command]
pub async fn remote_get_status(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
) -> Result<serde_json::Value, AppError> {
    ctx.exec
        .control_rpc(
            &machine_id,
            "get_status",
            serde_json::json!({ "run_id": run_id }),
        )
        .await
        .map_err(AppError::from)
}

/// Builds a browser-facing compare (or, lacking a known default branch,
/// plain branch-tree) URL for `branch`, so the inbox can offer a diff
/// deep link for a run that pushed a feature branch but has no PR yet
/// (failed/cancelled/parked). No web-URL builder existed anywhere in the
/// codebase before this — `provider_http.rs`/`mr_publisher.rs` only build
/// *API* endpoints — so this constructs the URL directly rather than
/// reusing an API-endpoint helper that doesn't apply to browser URLs.
fn build_diff_url(
    kind: &str,
    host: &str,
    repo_path: &str,
    default_branch: &str,
    branch: &str,
) -> String {
    let gitlab = kind.eq_ignore_ascii_case("gitlab");
    if default_branch.trim().is_empty() {
        // No default branch on record — a plain branch view is still
        // useful even without a diff.
        return if gitlab {
            format!("https://{host}/{repo_path}/-/tree/{branch}")
        } else {
            format!("https://{host}/{repo_path}/tree/{branch}")
        };
    }
    if gitlab {
        format!("https://{host}/{repo_path}/-/compare/{default_branch}...{branch}")
    } else {
        format!("https://{host}/{repo_path}/compare/{default_branch}...{branch}")
    }
}

/// Resolves `project_id`'s repository + provider (same lookup
/// `remote_submit_run` already does) and `branch` into a web URL. Never
/// a hard error for a missing repo/provider/settings — this is a
/// "nice to have" deep link, not something that should block the inbox.
#[tauri::command]
pub fn remote_run_diff_url(
    ctx: State<'_, AppContext>,
    project_id: String,
    branch: String,
) -> Result<Option<String>, AppError> {
    let pid = ProjectId::from(project_id);
    let Ok(repos) = ctx.projects.get_repositories_for(&pid) else {
        return Ok(None);
    };
    let Some(repo) = repos.first() else {
        return Ok(None);
    };
    let Ok(providers) = ctx.app_settings.get_provider_instances() else {
        return Ok(None);
    };
    let Some(provider) = providers.into_iter().find(|p| p.id == repo.provider_id) else {
        return Ok(None);
    };
    let settings = ctx
        .projects
        .get_settings(&pid)
        .ok()
        .flatten()
        .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);
    Ok(Some(build_diff_url(
        &provider.kind,
        &provider.host,
        &repo.repo_path,
        &settings.worktree_strategy.default_branch,
        &branch,
    )))
}

/// Tail a remote run's append-only event log (M3.3/M6.4): "identical to
/// a local run" for what the control channel actually carries, which is
/// the run's coarse milestones (submitted/bootstrapped/gate decisions/
/// terminal state/PR), not a per-token agent transcript — the runner
/// doesn't stream raw agent stdout over the control channel today.
/// Callers page forward by passing the highest `offset` they've already
/// seen; a dropped tunnel just means the next call's `from_offset` is a
/// little further behind, never a gap (R9).
#[tauri::command]
pub async fn remote_stream_events(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
    from_offset: i64,
) -> Result<serde_json::Value, AppError> {
    ctx.exec
        .control_rpc(
            &machine_id,
            "stream_events",
            serde_json::json!({ "run_id": run_id, "from_offset": from_offset }),
        )
        .await
        .map_err(AppError::from)
}

/// C4.1 read-model RPCs. These reach the runner's `get_feature`/
/// `list_steps`/`read_artifact`/`list_messages` over the same control
/// socket, and are the raw fetch primitives the C4.2 reconcile path uses
/// to hydrate a read-only shadow of a runner-owned feature into the
/// laptop DB + artifact cache. They return the runner's JSON verbatim (a
/// `Feature`/`Vec<StepExecution>`/artifact body string/`Vec<Message>`)
/// so the shadow-hydration layer owns the deserialization + local
/// rewrite, not this thin transport wrapper.
#[tauri::command]
pub async fn remote_get_feature(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
) -> Result<serde_json::Value, AppError> {
    ctx.exec
        .control_rpc(
            &machine_id,
            "get_feature",
            serde_json::json!({ "run_id": run_id }),
        )
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn remote_list_steps(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
) -> Result<serde_json::Value, AppError> {
    ctx.exec
        .control_rpc(
            &machine_id,
            "list_steps",
            serde_json::json!({ "run_id": run_id }),
        )
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn remote_read_artifact(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
    path: String,
) -> Result<serde_json::Value, AppError> {
    ctx.exec
        .control_rpc(
            &machine_id,
            "read_artifact",
            serde_json::json!({ "run_id": run_id, "path": path }),
        )
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn remote_list_messages(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
    thread_id: String,
) -> Result<serde_json::Value, AppError> {
    ctx.exec
        .control_rpc(
            &machine_id,
            "list_messages",
            serde_json::json!({ "run_id": run_id, "thread_id": thread_id }),
        )
        .await
        .map_err(AppError::from)
}

/// Clear a parked gate on a remote run from the laptop (M5.3's
/// `decide_gate` RPC, exposed here so the return inbox's "Parked (needs
/// you)" bucket can act on it).
#[tauri::command]
pub async fn remote_decide_gate(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
    gate_id: String,
    decision: String,
    feedback: Option<String>,
) -> Result<(), AppError> {
    ctx.exec
        .control_rpc(
            &machine_id,
            "decide_gate",
            serde_json::json!({
                "run_id": run_id,
                "gate_id": gate_id,
                "decision": decision,
                "feedback": feedback,
            }),
        )
        .await
        .map(|_| ())
        .map_err(AppError::from)
}

/// R8: the only way to stop a remote run — closing the app or losing
/// the tunnel never cancels it.
#[tauri::command]
pub async fn remote_cancel_run(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
) -> Result<(), AppError> {
    let result = ctx
        .exec
        .control_rpc(
            &machine_id,
            "cancel_run",
            serde_json::json!({ "run_id": run_id }),
        )
        .await
        .map_err(AppError::from)?;
    let status = json_str(&result, "status").unwrap_or_else(|| "cancelled".to_string());
    let now = crate::paths::now_ms();
    ctx.remote_run_mirror
        .update_status(
            &machine_id,
            &run_id,
            &status,
            None,
            None,
            None,
            None,
            0,
            now,
        )
        .map_err(AppError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{declared_remote_paths, mime_for_path};

    #[test]
    fn declared_paths_single_first_and_deduped() {
        let out = declared_remote_paths(
            Some("/w/report.md"),
            &["/w/report.md".to_string(), "/w/diff.patch".to_string()],
        );
        // The legacy single path leads, and it is not repeated even though
        // it also appears in the list.
        assert_eq!(out, vec!["/w/report.md", "/w/diff.patch"]);
    }

    #[test]
    fn declared_paths_none_single_uses_list_only() {
        let out = declared_remote_paths(None, &["/w/a.txt".to_string(), "/w/b.txt".to_string()]);
        assert_eq!(out, vec!["/w/a.txt", "/w/b.txt"]);
    }

    #[test]
    fn declared_paths_empty_when_nothing_declared() {
        assert!(declared_remote_paths(None, &[]).is_empty());
    }

    #[test]
    fn mime_inferred_from_extension() {
        assert_eq!(mime_for_path("/w/report.md"), "text/markdown");
        assert_eq!(mime_for_path("/w/change.diff"), "text/x-diff");
        assert_eq!(mime_for_path("/w/manifest.json"), "application/json");
        // Unknown / extensionless falls back to plain text.
        assert_eq!(mime_for_path("/w/LICENSE"), "text/plain");
    }
}

//! Laptop-side control-channel commands (docs/REMOTE_EXECUTION_PLAN.md
//! M6.1). Everything here goes over `ExecutionPort::control_rpc`, which
//! reaches `demeteo-runner`'s Unix-socket RPC server via OpenSSH
//! Unix-socket forwarding (R4) — see
//! `crates/demeteo-core/src/adapters/ssh/client.rs`.

use crate::adapters::artifact_store::fs::FsArtifactStore;
use crate::domain::artifact::{Artifact, ArtifactSource};
use crate::domain::ids::{FeatureId, ProjectId, WorkflowId};
use crate::domain::models::feature::{Feature, StepExecution};
use crate::domain::run_spec::{RunBudget, RunSpec, RunSpecAttachment, RunSpecProvider};
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

/// Hard cap per attachment on the detached path. The local path accepts
/// up to 100 MB per file; SFTP-spooling multi-hundred-MB blobs through
/// the submit call would stall it for minutes, so the detached path is
/// deliberately tighter (mirrored in the Start Feature modal copy).
const MAX_DETACHED_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

/// The runner-side directory a run's attachments are spooled into. Same
/// data-dir convention `enable_remote_runs` provisions the runner with
/// (`{home}/.local/share/demeteo-runner`).
fn attachment_spool_dir(home: &str, run_id: &str) -> String {
    format!("{home}/.local/share/demeteo-runner/attachment-spool/{run_id}")
}

/// Best-effort removal of a run's attachment spool for a run the runner
/// never accepted (spool/submit failure). The runner only cleans up
/// spools of runs it actually executed; without this, a rejected submit
/// would orphan the directory on the remote host forever.
async fn cleanup_attachment_spool(ctx: &AppContext, machine_id: &str, run_id: &str) {
    let Ok(home) = ctx.exec.resolve_home(machine_id).await else {
        return;
    };
    let spool_dir = attachment_spool_dir(&home, run_id);
    let _ = ctx
        .exec
        .run_command(
            machine_id,
            &format!("rm -rf {}", crate::paths::shell_escape_posix(&spool_dir)),
        )
        .await;
}

/// Spool pre-launch attachments onto the runner host over SFTP before
/// `submit_run`, returning the [`RunSpecAttachment`] references the spec
/// carries — raw bytes never ride the line-JSON control RPC. Keyed by
/// `run_id`; the runner deletes the whole spool directory when the run
/// reaches a terminal state (with the credential teardown). On `Err` the
/// spool may be partially written — the caller cleans it up.
async fn spool_attachments(
    ctx: &AppContext,
    machine_id: &str,
    run_id: &str,
    staged: Vec<crate::commands::attachments::StagedAttachmentInput>,
) -> Result<Vec<RunSpecAttachment>, String> {
    if staged.is_empty() {
        return Ok(Vec::new());
    }
    let home = ctx.exec.resolve_home(machine_id).await?;
    let spool_dir = attachment_spool_dir(&home, run_id);
    ctx.exec
        .run_command(
            machine_id,
            &format!("mkdir -p {}", crate::paths::shell_escape_posix(&spool_dir)),
        )
        .await?;

    let mut out = Vec::new();
    for (i, a) in staged.into_iter().enumerate() {
        let display_name = a
            .source_filename
            .clone()
            .unwrap_or_else(|| a.source_path.clone());
        let too_big = |actual_bytes: u64| {
            format!(
                "attachment {display_name} is {} MB — detached runs cap attachments at {} MB",
                actual_bytes / (1024 * 1024),
                MAX_DETACHED_ATTACHMENT_BYTES / (1024 * 1024),
            )
        };
        let bytes = match a.bytes {
            Some(b) => b,
            None => {
                // Size check from metadata first, so an oversized file is
                // rejected without pulling its whole body into memory.
                let meta = tokio::fs::metadata(&a.source_path)
                    .await
                    .map_err(|e| format!("failed to read attachment {}: {e}", a.source_path))?;
                if meta.len() > MAX_DETACHED_ATTACHMENT_BYTES as u64 {
                    return Err(too_big(meta.len()));
                }
                tokio::fs::read(&a.source_path)
                    .await
                    .map_err(|e| format!("failed to read attachment {}: {e}", a.source_path))?
            }
        };
        if bytes.len() > MAX_DETACHED_ATTACHMENT_BYTES {
            return Err(too_big(bytes.len() as u64));
        }
        // Keep only the final path component of whatever name we have —
        // a spool entry must never escape its run directory.
        let name = a
            .source_filename
            .clone()
            .or_else(|| {
                std::path::Path::new(&a.source_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| format!("attachment-{i}"));
        let safe_name = std::path::Path::new(&name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment")
            .to_string();
        let staged_path = format!("{spool_dir}/{i}-{safe_name}");
        ctx.exec
            .write_file_bytes(machine_id, &staged_path, &bytes)
            .await
            .map_err(|e| format!("failed to spool attachment {safe_name} to the runner: {e}"))?;
        out.push(RunSpecAttachment {
            staged_path,
            mime: a.mime,
            source_filename: a.source_filename,
        });
    }
    Ok(out)
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

/// App-settings key holding this install's stable `client_id` (MC-D1).
const INSTALL_ID_KEY: &str = "install_id";

/// This install's stable `client_id` (docs/MULTI_CLIENT_RUNNER.md MC-D1):
/// a UUID generated once and persisted in app-settings, so every remote
/// RPC from this machine carries the *same* owner identity across app
/// restarts. Generated lazily on first remote use and cached in the DB —
/// a single indexed KV read thereafter.
fn client_install_id(ctx: &AppContext) -> Result<String, String> {
    if let Some(id) = ctx.app_settings.app_setting_get(INSTALL_ID_KEY)? {
        if !id.is_empty() {
            return Ok(id);
        }
    }
    let id = format!("client-{}", crate::paths::new_id());
    ctx.app_settings.app_setting_set(INSTALL_ID_KEY, &id)?;
    Ok(id)
}

/// MC-D3 single stamping site: the one wrapper every remote control RPC
/// funnels through, injecting this install's `client_id` into `params`
/// **without** touching the generic `ExecutionPort::control_rpc`
/// signature (the local-vs-remote port seam stays transport-agnostic).
/// Existing param keys are preserved; only `client_id` is added — so
/// there is no per-call-site drift and no way to forget it. The runner
/// extracts it in `dispatch` and enforces ownership (`require_owner`).
async fn remote_rpc(
    ctx: &AppContext,
    machine_id: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client_id = client_install_id(ctx)?;
    let params = stamp_client_id(params, &client_id);
    ctx.exec.control_rpc(machine_id, method, params).await
}

/// The pure param-stamping step of [`remote_rpc`], split out so the
/// injection is unit-testable: add `client_id` to an object payload while
/// preserving every existing key. `submit_run`/`get_status`/… all send an
/// object; a non-object payload is returned unchanged (rather than
/// corrupted), and the runner defaults such a caller to the `""` legacy
/// tenant — old-client parity.
fn stamp_client_id(mut params: serde_json::Value, client_id: &str) -> serde_json::Value {
    if let Some(obj) = params.as_object_mut() {
        obj.insert(
            "client_id".to_string(),
            serde_json::Value::String(client_id.to_string()),
        );
    }
    params
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
    // Feature-wide effort, shipped in the `RunSpec` (`#[serde(default)]`, so a
    // runner older than this app ignores it and runs at the agent's own
    // default — the accepted version-skew risk, AGENTS.md §9.1).
    effort: Option<crate::domain::models::EffortLevel>,
    commit_artifacts: Option<bool>,
    loop_iterations: Option<u32>,
    step_overrides: Option<Vec<crate::domain::models::StepOverride>>,
    staged_attachments: Option<Vec<crate::commands::attachments::StagedAttachmentInput>>,
    target_repo_id: Option<String>,
    unattended: bool,
    max_cost_usd: Option<f64>,
    max_wall_clock_secs: Option<u64>,
) -> Result<RemoteRunHandle, AppError> {
    let pid = ProjectId::from(project_id.clone());
    let repos = ctx
        .projects
        .get_repositories_for(&pid)
        .map_err(AppError::from)?;
    // A detached run clones exactly one repository (RunSpec carries one
    // `repo_path`). The modal sends the first repo the user selected;
    // launches that don't pick keep the historical first-repo default.
    let repo = match &target_repo_id {
        Some(id) => repos.iter().find(|r| r.id.0 == *id).ok_or_else(|| {
            AppError::from(format!(
                "Selected repository {id} is not attached to this project"
            ))
        })?,
        None => repos.first().ok_or_else(|| {
            AppError::from("Project has no repository configured; remote runs need one".to_string())
        })?,
    };

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

    // Laptop-generated idempotency key (R9/M3.2): safe to retry
    // `remote_submit_run` with network hiccups mid-call without risking
    // a duplicate feature on the runner. Generated before the attachment
    // spool below, which is keyed by it.
    let run_id = format!("laptop-{}", crate::paths::new_id());

    // Spool the attachments before inserting the placeholder Feature:
    // an oversized/unreadable attachment fails the launch outright, and
    // failing before the insert means there is no ghost row to retire.
    // On failure the partially-written spool is removed so a rejected
    // launch leaves nothing behind on the runner host.
    let staged = staged_attachments.unwrap_or_default();
    let had_attachments = !staged.is_empty();
    let attachments = match spool_attachments(&ctx, &machine_id, &run_id, staged).await {
        Ok(list) => list,
        Err(e) => {
            if had_attachments {
                cleanup_attachment_spool(&ctx, &machine_id, &run_id).await;
            }
            return Err(AppError::from(e));
        }
    };

    // Eager shadow feature: the laptop chooses the feature id, inserts a
    // placeholder Feature it can navigate to immediately, and ships the
    // id in the spec so the runner's `feature_start` reuses it. From
    // second zero the run is one Feature on both databases; the first
    // reconcile's hydration (C4.2) updates this row in place instead of
    // creating a late twin.
    let now = crate::paths::now_ms();
    let feature_id = format!("f-{}", crate::paths::new_id());
    let step_overrides = step_overrides.unwrap_or_default();
    if let Err(e) = ctx.features.add(Feature {
        // The shadow row carries the same effort the runner is about to run
        // with, so the laptop's Feature view isn't lying about the run.
        effort,
        id: FeatureId::from(feature_id.clone()),
        project_id: pid.clone(),
        workflow_id: Some(wf_id.clone()),
        title: title.clone(),
        description: description.clone(),
        status: "pending".to_string(),
        total_cost: 0.0,
        duration: "0s".to_string(),
        tokens: 0,
        created_at: now,
        agent_kind: agent_kind.clone(),
        model: model.clone(),
        mr_url: None,
        mr_state: Some("none".to_string()),
        pr_title: None,
        pr_body: None,
        commit_artifacts,
        loop_iterations,
        step_overrides: step_overrides.clone(),
        attachments: Vec::new(),
    }) {
        if had_attachments {
            cleanup_attachment_spool(&ctx, &machine_id, &run_id).await;
        }
        return Err(AppError::from(e));
    }

    // MC-D4 / P0.5: ship the launching client's project settings so the
    // detached run honors *this* client's harnesses/prepare-command/
    // test-command/extra-writable-paths/lifecycle instead of runner-side
    // re-detected defaults. Best-effort: no settings row (or a read error)
    // → `None`, which reproduces today's runner-detection behavior exactly.
    let project_settings = ctx.projects.get_settings(&pid).ok().flatten();

    let spec = RunSpec {
        effort,
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
        step_overrides,
        commit_artifacts,
        attachments,
        unattended,
        budget,
        project_settings,
    };

    let submitted = match remote_rpc(
        &ctx,
        &machine_id,
        "submit_run",
        serde_json::json!({ "run_id": run_id, "spec": spec }),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            // The runner never accepted this run: retire the placeholder
            // and reclaim the spool it will never read (a run it *did*
            // accept has its spool torn down by the runner itself).
            mark_placeholder_failed(&ctx, &feature_id);
            if had_attachments {
                cleanup_attachment_spool(&ctx, &machine_id, &run_id).await;
            }
            return Err(AppError::from(e));
        }
    };
    let status = json_str(&submitted, "status").unwrap_or_else(|| "pending".to_string());
    if status == "failed" {
        let err = json_str(&submitted, "error")
            .unwrap_or_else(|| "the runner rejected this run".to_string());
        mark_placeholder_failed(&ctx, &feature_id);
        if had_attachments {
            cleanup_attachment_spool(&ctx, &machine_id, &run_id).await;
        }
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

    remote_rpc(
        &ctx,
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

/// Re-inject the git-provider PAT for a run parked at `needs-credentials`
/// (§6.2/§7.1). The runner holds the PAT in memory only, so it's lost when
/// the runner restarts mid-run — and the one-shot injection inside
/// [`remote_submit_run`] can also fail *after* `submit_run` already
/// accepted the run, stranding it the same way. Either way the runner's
/// `wait_for_pat` parks the run and waits for the PAT to be re-supplied
/// "however much later" (its resume-from-parked path, M4.2); this is the
/// laptop trigger that supplies it. Resolves the PAT from the run's own
/// project exactly as the original submit did, pushes it over the control
/// channel, then re-polls so the caller immediately sees the run leave
/// `needs-credentials`.
#[tauri::command]
pub async fn remote_reinject_credentials(
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
    inject_pat_for_run(&ctx, &machine_id, &run_id, &row).await?;

    // The runner resumes on injection; re-poll so the returned row already
    // reflects the run leaving `needs-credentials` rather than lagging a tick.
    reconcile_one_run(&ctx, &row).await;
    ctx.remote_run_mirror
        .get(&machine_id, &run_id)
        .map_err(AppError::from)
}

/// Resolve the run's git-provider PAT from the laptop's keyring — exactly
/// as the original submit did, via the run's own project — and push it
/// over the control channel. Shared by [`remote_reinject_credentials`] and
/// [`remote_retry_step`]: the runner wipes a run's PAT the moment the run
/// reaches *any* end state (success, failure, or cancel — §6.2), so a
/// retry of a failed run is re-animating a run with no credential, and
/// would otherwise reach its terminal push with nothing to authenticate
/// with.
async fn inject_pat_for_run(
    ctx: &AppContext,
    machine_id: &str,
    run_id: &str,
    row: &RemoteRunMirror,
) -> Result<(), AppError> {
    let project_id = row.project_id.clone().ok_or_else(|| {
        AppError::from("Run has no project on record; cannot resolve its git provider".to_string())
    })?;
    let pid = ProjectId::from(project_id);
    let repos = ctx
        .projects
        .get_repositories_for(&pid)
        .map_err(AppError::from)?;
    // Same default as `remote_submit_run`'s no-target case: a detached run
    // clones one repository, and its PAT comes from that repo's provider.
    let repo = repos.first().ok_or_else(|| {
        AppError::from("Project has no repository configured; cannot resolve a PAT".to_string())
    })?;
    let provider = ctx
        .app_settings
        .get_provider_instances()
        .map_err(AppError::from)?
        .into_iter()
        .find(|p| p.id == repo.provider_id)
        .ok_or_else(|| {
            AppError::from("Repository's git provider instance is not configured".to_string())
        })?;
    let git_ops = crate::adapters::worktree::git_ops::GitOpsHelper::new(
        ctx.app_settings.clone(),
        ctx.exec.clone(),
    );
    let pat = git_ops
        .get_provider_pat(&provider.id.0)
        .map_err(AppError::from)?;

    remote_rpc(
        ctx,
        machine_id,
        "inject_credentials",
        serde_json::json!({ "run_id": run_id, "git_pat": pat }),
    )
    .await
    .map_err(AppError::from)?;
    Ok(())
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
/// an already-final outcome (§7.1 — a machine that's merely off is
/// *paused*, not *failed*, and must never overwrite a settled result).
/// Includes the *success* terminals (`completed`/`awaiting_mr`, i.e. "PR
/// ready") as well as the failure ones: once a run has finished, a runner
/// that later goes to sleep must not flip a finished PR to "Unreachable".
const HARD_TERMINAL: &[&str] = &["failed", "cancelled", "completed", "awaiting_mr"];

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
    canonical_id: &str,
) -> Result<(), String> {
    let feat_val = remote_rpc(
        ctx,
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
    // Re-home under the local project so the FK holds. Pin the id to
    // `canonical_id` (== the mirror's `feature_id`) rather than the runner's
    // reported id: under version skew the two differ, and the caller has
    // chosen the laptop's eager-placeholder id as canonical so the shadow
    // *is* the row the user already has open (adopt-in-place). With a
    // feature_id-aware runner the two ids coincide and this is a no-op.
    let canonical = FeatureId::from(canonical_id.to_string());
    feature.project_id = ProjectId::new(local_project_id);
    feature.id = canonical.clone();

    let steps_val = remote_rpc(
        ctx,
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
                effort: None,
                status: Some(feature.status.clone()),
                total_cost: Some(Some(feature.total_cost)),
                duration: Some(Some(feature.duration.clone())),
                tokens: Some(Some(feature.tokens)),
                agent_kind: Some(feature.agent_kind.clone()),
                model: Some(feature.model.clone()),
                mr_url: Some(feature.mr_url.clone()),
                mr_state: Some(feature.mr_state.clone()),
                // Mirror the summary the runner's finalize step authored, so
                // the desktop shadow shows the same PR title/body the runner
                // published with.
                pr_title: Some(feature.pr_title.clone()),
                pr_body: Some(feature.pr_body.clone()),
                // The shadow inherits the runner's snapshot on the initial
                // insert; a re-reconcile doesn't change the commit flag.
                commit_artifacts: None,
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
            // Keep the step's FK pointing at the pinned feature id, not the
            // runner's — otherwise `steps_for_feature(canonical)` wouldn't
            // find them under version skew.
            shadow.feature_id = fid.clone();
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
        match remote_rpc(
            ctx,
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
                    // Two distinct runner paths that share a basename (e.g. the
                    // same file declared both as `artifact_path` and inside
                    // `artifact_paths`, or captured under two absolute paths)
                    // resolve to the *same* local cache file — `put` is
                    // idempotent on `{name}.{ext}`. Dedupe so the shadow step's
                    // `artifact_paths` never carries the identical local ref
                    // twice (which surfaced as a duplicate-React-key crash in
                    // the per-step artifact list).
                    Ok(local_ref) if !local.contains(&local_ref) => local.push(local_ref),
                    Ok(_) => {}
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
    let result = remote_rpc(
        ctx,
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
            let remote_feature_id = json_str(&v, "feature_id");
            let mr_url = json_str(&v, "mr_url");
            let pushed_branch = json_str(&v, "pushed_branch");
            // Version skew: an old runner that ignored `RunSpec::feature_id`
            // reports its own generated id, different from the laptop's eager
            // placeholder id. Rather than orphan+fail the placeholder (the
            // run is healthy, and the user may already have that row open),
            // keep the laptop's id *canonical* and re-home the runner's
            // shadow onto it below (adopt-in-place). A feature_id-aware
            // runner reports the same id, so `canonical` == both and this is
            // a no-op. When only one side has an id, take whichever exists.
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

            // C4.2: once the run has bootstrapped a feature and we know the
            // local project it was composed from, hydrate a read-only shadow
            // of it (feature + steps + cached artifacts) so it renders on the
            // laptop with full fidelity. Best-effort: a hydration failure
            // never regresses the mirror status just applied.
            if let (Some(fid), Some(pid)) = (&canonical_feature_id, &row.project_id) {
                if !fid.is_empty() {
                    if let Err(e) =
                        hydrate_shadow_feature(ctx, &row.machine_id, &row.run_id, pid, fid).await
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
    remote_rpc(
        &ctx,
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
    remote_rpc(
        &ctx,
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
    remote_rpc(
        &ctx,
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
    remote_rpc(
        &ctx,
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
    remote_rpc(
        &ctx,
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
    remote_rpc(
        &ctx,
        &machine_id,
        "list_messages",
        serde_json::json!({ "run_id": run_id, "thread_id": thread_id }),
    )
    .await
    .map_err(AppError::from)
}

/// Variant A of the detached-run "Browse Code" fix: resolve the *runner's*
/// real worktree path + branch for a run, then re-home `machine_id` onto the
/// mirror's box so the laptop's existing SFTP `CodeEditorView` browses the
/// actual checkout. The runner reports `machine_id: "local"` (it is the
/// machine it ran on); we overwrite it with the mirror's SSH machine — the
/// path is a path *on that box*, reachable over the SSH the laptop already
/// has. This is the only field we override; `worktree_path`/`branch`/
/// `default_branch` are the runner's own, so the browse targets exactly
/// where the run worked.
#[tauri::command]
pub async fn remote_get_worktree(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
) -> Result<serde_json::Value, AppError> {
    let mut info = remote_rpc(
        &ctx,
        &machine_id,
        "get_worktree",
        serde_json::json!({ "run_id": run_id }),
    )
    .await
    .map_err(AppError::from)?;
    if let Some(obj) = info.as_object_mut() {
        obj.insert("machine_id".to_string(), serde_json::json!(machine_id));
    }
    Ok(info)
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
    remote_rpc(
        &ctx,
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
    let result = remote_rpc(
        &ctx,
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

/// Retry a failed / interrupted step of a detached run — the remote twin of
/// the `step_retry` command, which only ever drives runs this machine owns
/// (the local executor refuses a runner-owned shadow: it has neither the
/// driver nor the worktree).
///
/// Order matters. The PAT goes first: the runner wipes a run's credential
/// when the run ends, so the failed run we're re-animating has none, and the
/// retried pipeline would run all the way to its terminal push before
/// discovering that. Then the retry itself, which re-opens the run on the
/// runner (back to `running`, with a fresh await/push tail). Finally the
/// laptop's mirror is dragged off its terminal status too — otherwise
/// `FeatureDetail`'s poll and the reconcile loop, both of which stop at a
/// terminal status, would never notice the run came back to life.
#[tauri::command]
pub async fn remote_retry_step(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
    step_execution_id: String,
    model: Option<String>,
    agent_kind: Option<String>,
    // Re-pin the effort on the retry, the remote twin of `step_retry`'s
    // `new_effort`. A runner too old to know the field ignores it.
    effort: Option<crate::domain::models::EffortLevel>,
) -> Result<(), AppError> {
    let Some(row) = ctx
        .remote_run_mirror
        .get(&machine_id, &run_id)
        .map_err(AppError::from)?
    else {
        return Err(AppError::not_found(format!(
            "No detached run {} on machine {}",
            run_id, machine_id
        )));
    };
    inject_pat_for_run(&ctx, &machine_id, &run_id, &row).await?;

    let result = remote_rpc(
        &ctx,
        &machine_id,
        "retry_step",
        serde_json::json!({
            "run_id": run_id,
            "step_execution_id": step_execution_id,
            "model": model,
            "agent_kind": agent_kind,
            "effort": effort,
        }),
    )
    .await
    .map_err(AppError::from)?;

    let status = json_str(&result, "status").unwrap_or_else(|| "running".to_string());
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
    // Pull the runner's freshly-rewound steps into the shadow now, so the
    // retried step doesn't sit on screen in its old `failed` state until
    // the next poll tick.
    reconcile_one_run(&ctx, &row).await;
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/infrastructure/remote_runner.rs"]
mod tests;

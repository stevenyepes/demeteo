//! Laptop-side control-channel commands (docs/REMOTE_EXECUTION_PLAN.md
//! M6.1). Everything here goes over `ExecutionPort::control_rpc`, which
//! reaches `demeteo-runner`'s Unix-socket RPC server via OpenSSH
//! Unix-socket forwarding (R4) — see
//! `crates/demeteo-core/src/adapters/ssh/client.rs`.

use crate::domain::ids::{ProjectId, WorkflowId};
use crate::domain::run_spec::{RunBudget, RunSpec, RunSpecProvider};
use crate::error::AppError;
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

    let spec = RunSpec {
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

    let submitted = ctx
        .exec
        .control_rpc(
            &machine_id,
            "submit_run",
            serde_json::json!({ "run_id": run_id, "spec": spec }),
        )
        .await
        .map_err(AppError::from)?;
    let status = json_str(&submitted, "status").unwrap_or_else(|| "pending".to_string());
    if status == "failed" {
        let err = json_str(&submitted, "error")
            .unwrap_or_else(|| "the runner rejected this run".to_string());
        return Err(AppError::from(err));
    }

    ctx.exec
        .control_rpc(
            &machine_id,
            "inject_credentials",
            serde_json::json!({ "run_id": run_id, "git_pat": pat }),
        )
        .await
        .map_err(AppError::from)?;

    let now = crate::paths::now_ms();
    ctx.remote_run_mirror
        .upsert_submitted(&machine_id, &run_id, Some(&project_id), &title, now)
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

    Ok(RemoteRunHandle {
        run_id,
        machine_id,
        status,
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
                // A dangerous parked gate (M5.1) doesn't move the
                // runner's own coarse `status` column — the feature is
                // still nominally "running", just blocked on one
                // decision — so `parked_gate_id` (set only in that
                // case) wins over whatever `status` says, giving the
                // inbox a distinct "parked" bucket to group on.
                let status = if json_str(&v, "parked_gate_id").is_some() {
                    "parked".to_string()
                } else {
                    json_str(&v, "status").unwrap_or_else(|| row.status.clone())
                };
                let error = json_str(&v, "error");
                let feature_id = json_str(&v, "feature_id");
                let mr_url = json_str(&v, "mr_url");
                let pushed_branch = json_str(&v, "pushed_branch");
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
                    let _ =
                        ctx.remote_run_mirror
                            .mark_notified(&row.machine_id, &row.run_id, &status);
                }
            }
            Err(_) if HARD_TERMINAL.contains(&row.status.as_str()) => {
                // Already know how this run ended; an unreachable
                // machine doesn't change that.
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
            }
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

#[derive(Debug, Serialize)]
pub struct RemoteAgentReadiness {
    pub kind: String,
    pub available: bool,
}

/// Upfront agent-readiness check (M4.1's `probe_agent` RPC, already
/// implemented runner-side but never exposed to the laptop UI until
/// now): lets `StartFeatureModal` warn *before* Launch is clicked that
/// the selected machine/agent combination will fail, instead of only
/// finding out after `remote_submit_run` rejects it synchronously. A
/// `control_rpc` failure here (machine unreachable, or `demeteo-runner`
/// never installed) is surfaced as an `Err` — distinct from a
/// successful probe reporting `available: false` — so the UI can tell
/// "this machine isn't set up for remote runs" apart from "the agent
/// itself isn't ready on it".
#[tauri::command]
pub async fn remote_probe_agent(
    ctx: State<'_, AppContext>,
    machine_id: String,
    agent_kind: String,
) -> Result<RemoteAgentReadiness, AppError> {
    let v = ctx
        .exec
        .control_rpc(
            &machine_id,
            "probe_agent",
            serde_json::json!({ "kind": agent_kind }),
        )
        .await
        .map_err(AppError::from)?;
    Ok(RemoteAgentReadiness {
        kind: json_str(&v, "kind").unwrap_or(agent_kind),
        available: v
            .get("available")
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
    })
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

//! The actual "do a run" pipeline (docs/REMOTE_EXECUTION.md M1.2,
//! M4, M5): register the provider, clone via per-run askpass, bootstrap a
//! project, ingest the workflow, drive a feature to a terminal state
//! (applying the unattended gate policy + budget caps along the way),
//! push the branch, and auto-open the PR. Shared by the one-shot `submit`
//! CLI command (M1) and the `submit_run` RPC method (M3.2) so there is
//! exactly one code path, not two copies that drift.

use crate::git_askpass;
use crate::services::RunnerServices;
use demeteo_core::adapters::step_executor::setup::fetch_default_settings;
use demeteo_core::application::attachments::StagedAttachmentInput;
use demeteo_core::application::{bootstrap, projects, workflows};
use demeteo_core::domain::ids::{FeatureId, ProjectId, ProviderId};
use demeteo_core::domain::models::{
    Feature, GateDecision, MrInfo, ProjectSettings, ProviderInstance, PublishOptions, StepConfig,
    WorktreeStrategy,
};
use demeteo_core::domain::run_spec::RunSpec;
use demeteo_core::paths;
use demeteo_core::state::AppContext;
use serde::Serialize;
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// The one Provider Instance a run's repo is registered under. M1/M3 have
/// no multi-provider config yet — every run gets a fresh instance derived
/// from `RunSpec::provider`.
const RUN_PROVIDER_ID: &str = "runner-provider";

/// Env var carrying the git-provider PAT for the one-shot CLI `submit`
/// entry point, which has no laptop RPC to call `inject_credentials`
/// (M4.2). `main.rs` bridges this into the run's in-memory
/// `CredentialStore` entry before calling [`execute_run`] — from there on
/// the exact same askpass path (M4.3) serves both entry points.
pub const GIT_PAT_ENV: &str = "DEMETEO_RUNNER_GIT_PAT";

/// How long to wait for `inject_credentials` before parking a run as
/// `needs-credentials` (§6.2/§7.1). The common case is the laptop calling
/// `submit_run` immediately followed by `inject_credentials` — this
/// window absorbs that ordinary race without needing the full park+resume
/// path. A genuinely missing PAT (runner restarted mid-run and lost it,
/// laptop never reconnected) parks after this and resumes whenever
/// `inject_credentials` arrives, however much later that is.
const PAT_WAIT_TIMEOUT: Duration = Duration::from_secs(20);
const PAT_WAIT_POLL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Serialize)]
pub struct RunOutcome {
    /// `None` until the project has actually been created — distinct from
    /// `Some("")`, so a park that happens before `feature_start` doesn't
    /// get coalesced into the runner_runs row as an empty-string id.
    pub project_id: Option<String>,
    pub feature_id: Option<String>,
    /// The run's terminal (or parked) status: `pr_ready` (success, PR
    /// opened), `awaiting_mr`/`completed` (success, PR not opened — the
    /// publish call failed; see the `pr_open_failed` event), `failed`,
    /// `interrupted`, `needs-credentials` (parked, §6.2/§7.1), or
    /// `over-budget` (parked/stopped, M5.2).
    pub status: String,
    /// Set when the branch push succeeded.
    pub pushed_branch: Option<String>,
    /// Set when the terminal PR was opened (M5.3/R10).
    pub pr_url: Option<String>,
}

/// Append an event to `run_id`'s log (M3.3). Best-effort: a failure to
/// write the event log must never abort the run itself, so this only
/// logs to stderr on error rather than propagating one.
///
/// **Secret scrubbing (M7.2, §6)** happens at the sink, not here: the
/// `RunEventsPort::append` adapter scrubs every payload before it's
/// persisted, so the *direct* failure-path appends in `rpc/` (which
/// bypass this helper) are covered by the same guarantee.
pub(crate) fn emit(ctx: &AppContext, run_id: &str, kind: &str, payload: impl Serialize) {
    let payload_json = serde_json::to_string(&payload).ok();
    if let Err(e) = ctx
        .run_events
        .append(run_id, kind, payload_json.as_deref(), paths::now_ms())
    {
        eprintln!(
            "[demeteo-runner] warning: failed to append '{}' event for run {}: {}",
            kind, run_id, e
        );
    }
}

/// Poll the in-memory credential store for up to [`PAT_WAIT_TIMEOUT`].
/// `None` means the laptop hasn't (yet) called `inject_credentials` for
/// this run — the caller parks as `needs-credentials` rather than
/// guessing or falling back to a standing secret.
async fn wait_for_pat(svc: &RunnerServices, run_id: &str) -> Option<String> {
    if let Some(p) = svc.creds.get(run_id) {
        return Some(p);
    }
    let started = Instant::now();
    while started.elapsed() < PAT_WAIT_TIMEOUT {
        tokio::time::sleep(PAT_WAIT_POLL).await;
        if let Some(p) = svc.creds.get(run_id) {
            return Some(p);
        }
    }
    None
}

/// The clone/remote URL for this run's repo — deliberately carries no
/// credential (M4.3): a bare `x-access-token@`/`oauth2@` username with no
/// password segment. Git prompts for the password via `GIT_ASKPASS`
/// instead of embedding the PAT in the URL, keeping it out of `ps aux`,
/// shell history, and `git remote -v`.
fn remote_url(spec: &RunSpec) -> String {
    if spec.provider.kind.eq_ignore_ascii_case("github") {
        format!(
            "https://x-access-token@{}/{}",
            spec.provider.host, spec.repo_path
        )
    } else {
        format!("https://oauth2@{}/{}", spec.provider.host, spec.repo_path)
    }
}

/// Clone the repo directly via askpass (M4.3), *before* handing off to
/// the shared `bootstrap::bootstrap_project` — which detects the
/// already-cloned directory and skips its own (keyring-backed, PAT-in-URL)
/// clone path entirely. A no-op if the directory is already a git
/// working tree (idempotent across retries/resumes).
async fn pre_clone_with_askpass(
    svc: &RunnerServices,
    project_id: &ProjectId,
    spec: &RunSpec,
    pat: &str,
) -> Result<(), String> {
    let target_dir =
        paths::repo_target_dir_local(&svc.ctx.workspace_dir, project_id.as_str(), &spec.repo_path);
    if target_dir.join(".git").exists() {
        return Ok(());
    }
    if let Some(parent) = target_dir.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create repo parent dir: {}", e))?;
    }
    let args = vec![
        "clone".to_string(),
        remote_url(spec),
        target_dir.to_string_lossy().to_string(),
    ];
    git_askpass::run_git(&svc.askpass_path, &args, Some(pat)).await?;
    Ok(())
}

/// Fresh submission: register the provider, clone the repo, bootstrap a
/// project, ingest the workflow, start the feature, then hand off to
/// [`await_terminal_and_push`]. This is the *only* path that creates a
/// new project/feature — a resume (M2.3/M4.3) must never call this again
/// for a run that already has a `feature_id`, or it would create a second
/// project from scratch instead of resuming the first. Use
/// [`resume_or_run`] to pick the right path.
pub async fn execute_run(
    svc: &RunnerServices,
    run_id: &str,
    spec: &RunSpec,
) -> Result<RunOutcome, String> {
    emit(&svc.ctx, run_id, "submitted", &spec.title);

    // M4.2/§6.2: nothing that touches `origin` happens without an
    // in-memory PAT for this run. Park rather than fall back to any
    // standing secret.
    let Some(pat) = wait_for_pat(svc, run_id).await else {
        let msg = "no PAT injected before the pre-clone wait timed out";
        emit(&svc.ctx, run_id, "needs_credentials", msg);
        svc.away_notifier
            .notify("Run needs credentials", &format!("{}: {}", run_id, msg))
            .await;
        return Ok(RunOutcome {
            project_id: None,
            feature_id: None,
            status: "needs-credentials".to_string(),
            pushed_branch: None,
            pr_url: None,
        });
    };

    svc.ctx
        .app_settings
        .add_provider_instance(ProviderInstance {
            id: ProviderId::from(RUN_PROVIDER_ID),
            kind: spec.provider.kind.clone(),
            host: spec.provider.host.clone(),
            username: String::new(),
            avatar_url: String::new(),
            created_at: paths::now_ms(),
        })
        .map_err(|e| format!("failed to register provider instance: {}", e))?;

    let project = projects::create(
        &svc.ctx,
        projects::ProjectConfig {
            name: spec.title.clone(),
            compute_type: "local".to_string(),
            remote_host: None,
            repos: vec![projects::RepositoryConfig {
                repo_path: spec.repo_path.clone(),
                provider_id: RUN_PROVIDER_ID.to_string(),
            }],
        },
    )
    .map_err(|e| format!("failed to create project: {}", e))?;
    eprintln!("[demeteo-runner] project {} created", project.id.as_str());
    emit(&svc.ctx, run_id, "project_created", &project.id.0);

    // Phase-A bootstrap sub-steps for the laptop's inline stepper. Emitted
    // directly against `run_id` (the feature doesn't exist yet), matching the
    // `bootstrap_progress` payload the feature-start tail emits later so the
    // frontend normalizes both into one stepper. `bootstrap_run` is the id the
    // laptop tails; the feature-start phases carry their own ids.
    let bstep = |phase: &str, label: &str, status: &str, detail: Option<&str>| {
        emit(
            &svc.ctx,
            run_id,
            "bootstrap_progress",
            serde_json::json!({ "phase": phase, "label": label, "status": status, "detail": detail }),
        );
    };

    bstep("cloning", "Cloning repository", "running", None);
    if let Err(e) = pre_clone_with_askpass(svc, &project.id, spec, &pat).await {
        bstep("cloning", "Cloning repository", "failed", Some(&e));
        return Err(e);
    }
    bstep("cloning", "Cloning repository", "completed", None);

    bstep(
        "detecting_strategy",
        "Detecting project layout",
        "running",
        None,
    );
    let strategy = match bootstrap::bootstrap_project(&svc.ctx, project.id.0.clone()).await {
        Ok(s) => s,
        Err(e) => {
            let e = format!("bootstrap failed: {}", e);
            bstep(
                "detecting_strategy",
                "Detecting project layout",
                "failed",
                Some(&e),
            );
            return Err(e);
        }
    };
    bstep(
        "detecting_strategy",
        "Detecting project layout",
        "completed",
        None,
    );

    // Persist the settings the run will execute under. Two inputs merge
    // here (MC-D4 / P0.5): the bootstrap-*detected* worktree strategy (it
    // read the true `default_branch` from `origin/HEAD` on this clone —
    // ground truth) and, when the launching client sent them, that
    // client's own project settings (harnesses, prepare/test commands,
    // extra writable paths, lifecycle, …). The client wins on every
    // tunable; the detected `default_branch` wins over the client's stale
    // copy. Without persisting *something*, `get_settings` returns None at
    // feature_start and `fetch_default_settings` would supply
    // `default_branch = "main"`, so `create_feature_branch` would run
    // `git branch -f <feature> main` and fail on a `master`-default repo.
    // `None` client settings reproduce the pre-multi-client behavior
    // exactly (detected strategy + engine defaults).
    let settings =
        merge_project_settings(strategy, spec.project_settings.clone(), project.id.clone());
    svc.ctx
        .projects
        .save_settings(settings)
        .map_err(|e| format!("failed to persist project settings: {}", e))?;

    eprintln!(
        "[demeteo-runner] project bootstrapped (cloned {})",
        spec.repo_path
    );
    emit(&svc.ctx, run_id, "bootstrapped", &spec.repo_path);

    let workflow_id = workflows::create_from_json(&svc.ctx.workflows, &spec.workflow_json)
        .map_err(|e| format!("failed to ingest workflow: {}", e))?;

    // Attachments arrive pre-spooled on this host (SFTP'd by the laptop
    // before submit_run); hand them to feature_start as ordinary
    // path-based staged attachments. The spool is deleted at run end —
    // see `cleanup_attachment_spool`.
    let staged: Vec<StagedAttachmentInput> = spec
        .attachments
        .iter()
        .map(|a| StagedAttachmentInput {
            source_path: a.staged_path.clone(),
            mime: a.mime.clone(),
            source_filename: a.source_filename.clone(),
            bytes: None,
        })
        .collect();

    let feature = svc
        .ctx
        .executor
        .feature_start(
            // Reuse the laptop-chosen id (if the submitting app is new
            // enough to send one) so the eager shadow Feature on the
            // laptop and this runner-owned row are the same feature.
            spec.feature_id.clone(),
            project.id.as_str(),
            workflow_id.as_str(),
            &spec.title,
            &spec.description,
            spec.agent_kind.as_deref(),
            spec.model.as_deref(),
            spec.effort,
            spec.commit_artifacts,
            spec.loop_iterations,
            spec.max_budget_usd,
            spec.step_overrides.clone(),
            staged,
        )
        .await
        .map_err(|e| format!("feature_start failed: {}", e))?;
    eprintln!("[demeteo-runner] feature {} started", feature.id.as_str());
    emit(&svc.ctx, run_id, "feature_started", &feature.id.0);

    // Record the project/feature ids on the run row *now*, not just at
    // terminal state. Two things depend on it mid-run: (1) the
    // `RunEventBridge` resolves `feature_id -> run_id` from this row to tag
    // per-step progress events, and (2) `get_status` reports `feature_id`,
    // which is what lets the laptop hydrate the run's read-only shadow
    // (steps/tokens/cost) *while it runs* instead of only once it finishes.
    // Best-effort: a failure here only costs early visibility, never the run.
    if let Err(e) = svc.ctx.runner_runs.update_status(
        run_id,
        "running",
        Some(project.id.as_str()),
        Some(feature.id.as_str()),
        None,
        None,
        paths::now_ms(),
    ) {
        eprintln!(
            "[demeteo-runner] warning: failed to record feature id on run {}: {}",
            run_id, e
        );
    }

    await_terminal_and_push(svc, run_id, &project.id, &feature.id, spec).await
}

/// MC-D4 merge (P0.5): compose the `ProjectSettings` row the runner
/// persists for a run's own project from the two sources of truth. The
/// launching client's settings win on **every tunable** (`branch_prefix`,
/// `test_command`, `build_command`, `coverage_command`, `conventions_file`,
/// `pr_template`, `harnesses`, `prepare_command`, `extra_writable_paths`,
/// `conflict_policy`, `feature_lifecycle`, `default_*`, `artifact_subdir`,
/// `commit_artifacts`). The bootstrap-*detected* `default_branch` wins over
/// the client's, because it was read from `origin/HEAD` on the *actual*
/// clone — ground truth for this checkout — falling back to the client's
/// value, then `"main"`. `project_id` is always the run's own project (the
/// client's is meaningless on the runner). `None` client settings
/// reproduce the pre-multi-client behavior exactly: detected strategy +
/// engine defaults. Pure over its inputs so the merge is unit-testable.
fn merge_project_settings(
    detected: WorktreeStrategy,
    client: Option<ProjectSettings>,
    project_id: ProjectId,
) -> ProjectSettings {
    match client {
        None => {
            let mut settings = fetch_default_settings();
            settings.project_id = project_id;
            settings.worktree_strategy = detected;
            settings
        }
        Some(mut settings) => {
            settings.project_id = project_id;
            // Detected `default_branch` is ground truth for this clone;
            // every other strategy tunable stays the client's.
            if !detected.default_branch.trim().is_empty() {
                settings.worktree_strategy.default_branch = detected.default_branch;
            } else if settings.worktree_strategy.default_branch.trim().is_empty() {
                settings.worktree_strategy.default_branch = "main".to_string();
            }
            settings
        }
    }
}

/// Dispatch to [`execute_run`] (nothing created yet) or
/// [`await_terminal_and_push`] (project/feature already exist — the
/// engine's own restart reconciliation already re-armed the driver) based
/// on what the `runner_runs` row remembers. Shared by the M2.3 restart
/// path and the M4.3 `inject_credentials`-triggered resume-from-park
/// path so there is exactly one "how do I pick up a run" decision.
pub async fn resume_or_run(
    svc: &RunnerServices,
    run_id: &str,
    spec: &RunSpec,
    project_id: Option<String>,
    feature_id: Option<String>,
) -> Result<RunOutcome, String> {
    match (project_id, feature_id) {
        (Some(pid), Some(fid)) => {
            await_terminal_and_push(
                svc,
                run_id,
                &ProjectId::from(pid),
                &FeatureId::from(fid),
                spec,
            )
            .await
        }
        _ => execute_run(svc, run_id, spec).await,
    }
}

/// Resolve whether the currently-pending gate for `feature` is classified
/// `dangerous` (M5.1). Best-effort: any lookup failure (feature has no
/// workflow, version vanished, steps_json unparseable) is treated as
/// `false` — the conservative choice would be to always park on lookup
/// failure, but a workflow authored before `gate_class` existed has no
/// way to express "dangerous" at all, and defaulting those to a permanent
/// park would silently wedge every unattended run on a pre-M5 workflow.
pub(crate) fn gate_is_dangerous(
    ctx: &AppContext,
    feature: &Feature,
    gate_dec: &GateDecision,
) -> bool {
    let Ok(Some(step_exec)) = ctx.features.step_get(&gate_dec.step_execution_id) else {
        return false;
    };
    let Some(workflow_id) = feature.workflow_id.as_ref() else {
        return false;
    };
    let Ok(Some(version)) = ctx.workflows.latest_version(workflow_id) else {
        return false;
    };
    let Ok(steps) = serde_json::from_str::<Vec<StepConfig>>(&version.steps_json) else {
        return false;
    };
    steps
        .iter()
        .find(|s| s.id == step_exec.step_id)
        .map(|s| s.is_dangerous_gate())
        .unwrap_or(false)
}

/// M5.1: apply the unattended gate policy to the feature's currently
/// pending gate (if any) — auto-approve `safe`, leave `dangerous` parked
/// for a human (cleared later via the `decide_gate` RPC). `parked_seen`
/// dedupes the `parked` event and the approve attempt across poll ticks
/// so a long-parked dangerous gate doesn't spam the event log every
/// `POLL_INTERVAL`.
async fn apply_gate_policy(
    svc: &RunnerServices,
    run_id: &str,
    feature: &Feature,
    parked_seen: &mut HashSet<String>,
) {
    let Ok(Some(gate_dec)) = svc
        .ctx
        .presenter
        .gate_pending_for_run(feature.id.as_str())
        .await
    else {
        return;
    };
    let step_exec_id = gate_dec.step_execution_id.as_str().to_string();
    if parked_seen.contains(&step_exec_id) {
        return;
    }

    if gate_is_dangerous(&svc.ctx, feature, &gate_dec) {
        parked_seen.insert(step_exec_id.clone());
        let msg = format!(
            "dangerous gate {} awaiting a human decision (unattended run) — clear it via decide_gate",
            step_exec_id
        );
        emit(&svc.ctx, run_id, "parked", &msg);
        svc.away_notifier
            .notify("Run needs you", &format!("{}: {}", run_id, msg))
            .await;
        return;
    }

    match svc
        .ctx
        .presenter
        .gate_decide(&step_exec_id, "approve", None)
        .await
    {
        Ok(()) => {
            parked_seen.insert(step_exec_id.clone());
            emit(&svc.ctx, run_id, "gate_auto_approved", &step_exec_id);
        }
        Err(e) => eprintln!(
            "[demeteo-runner] failed to auto-approve gate {}: {}",
            step_exec_id, e
        ),
    }
}

/// Resume path (M2.3/M4.3): the feature already exists (created by a
/// prior, interrupted or credential-parked `execute_run`) and the
/// engine's own restart reconciliation has already re-armed its driver.
/// Polls to a terminal state — applying the unattended gate policy
/// (M5.1) and budget caps (M5.2) along the way — then pushes and
/// auto-opens the PR (M5.3). Never touches project/workflow creation.
pub async fn await_terminal_and_push(
    svc: &RunnerServices,
    run_id: &str,
    project_id: &ProjectId,
    feature_id: &FeatureId,
    spec: &RunSpec,
) -> Result<RunOutcome, String> {
    let result = await_terminal_and_push_inner(svc, run_id, project_id, feature_id, spec).await;
    // Wipe the run's credential the moment we reach any outcome that
    // isn't itself "still waiting on one" — `needs-credentials` means
    // there was nothing to wipe yet (§6.2: wiped at run end — success,
    // failure, or cancel). The attachment spool shares the credential's
    // lifetime: staged for the run, deleted with it.
    let still_needs_pat = matches!(&result, Ok(o) if o.status == "needs-credentials");
    if !still_needs_pat {
        svc.creds.remove(run_id);
        cleanup_attachment_spool(spec, run_id);
    }
    result
}

/// Delete the per-run attachment spool the laptop SFTP'd before
/// `submit_run`. The directory is derived from the staged paths in the
/// spec itself (not from a re-computed data dir, which could disagree
/// with what the submitting laptop chose), and only removed when its
/// name is exactly this `run_id` — a mis-addressed spec must never
/// delete another run's spool or an arbitrary directory.
fn cleanup_attachment_spool(spec: &RunSpec, run_id: &str) {
    let mut dirs = std::collections::HashSet::new();
    for a in &spec.attachments {
        if let Some(parent) = std::path::Path::new(&a.staged_path).parent() {
            if parent.file_name().and_then(|n| n.to_str()) == Some(run_id) {
                dirs.insert(parent.to_path_buf());
            }
        }
    }
    for dir in dirs {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            eprintln!(
                "[demeteo-runner] failed to delete attachment spool {}: {}",
                dir.display(),
                e
            );
        }
    }
}

async fn await_terminal_and_push_inner(
    svc: &RunnerServices,
    run_id: &str,
    project_id: &ProjectId,
    feature_id: &FeatureId,
    spec: &RunSpec,
) -> Result<RunOutcome, String> {
    const POLL_INTERVAL: Duration = Duration::from_secs(2);
    // Backstop only — a detached run is "fire and forget" and may legitimately
    // run for hours, so this must not guillotine a healthy long run before it
    // reaches a terminal state (the terminal push/PR-open lives *after* this
    // loop). Callers that want a real bound set a `budget.max_wall_clock_secs`
    // cap (M5.2), which cancels cleanly above; this only catches a feature
    // wedged forever with no cap at all.
    const MAX_WAIT: Duration = Duration::from_secs(24 * 60 * 60);
    let started = Instant::now();
    let mut parked_gates: HashSet<String> = HashSet::new();

    let final_status = loop {
        let feature = match svc.ctx.features.get(feature_id) {
            Ok(Some(f)) => f,
            Ok(None) => return Err("feature disappeared mid-run".to_string()),
            Err(e) => return Err(format!("error polling feature: {}", e)),
        };

        if matches!(
            feature.status.as_str(),
            "completed" | "awaiting_mr" | "failed" | "interrupted"
        ) {
            break feature.status;
        }

        // M5.2: hard budget caps. Unattended never auto-approves more
        // spend — exceeding either cap cancels the run outright.
        //
        // `feature.total_cost` is only rolled up when the feature reaches
        // a terminal status (`finish_feature` in
        // `step_executor/updates.rs`), i.e. exactly the iterations this
        // loop already exits on above — so it reads 0.0 on every
        // iteration this check actually runs on. Sum the live per-step
        // costs instead, which `update_step_status` writes incrementally
        // as each step finishes.
        if let Some(budget) = spec.budget.as_ref() {
            let live_cost = svc
                .ctx
                .features
                .steps_for_feature(feature_id)
                .map(|steps| steps.iter().map(|s| s.cost_usd.unwrap_or(0.0)).sum::<f64>())
                .unwrap_or(0.0);
            let over_cost = budget.max_cost_usd.is_some_and(|cap| live_cost > cap);
            let over_wall = budget
                .max_wall_clock_secs
                .is_some_and(|cap| started.elapsed().as_secs() > cap);
            if over_cost || over_wall {
                let reason = if over_cost {
                    format!(
                        "cost ${:.4} exceeded cap ${:.4}",
                        live_cost,
                        budget.max_cost_usd.unwrap_or_default()
                    )
                } else {
                    format!(
                        "wall clock {}s exceeded cap {}s",
                        started.elapsed().as_secs(),
                        budget.max_wall_clock_secs.unwrap_or_default()
                    )
                };
                eprintln!("[demeteo-runner] run {} over budget: {}", run_id, reason);
                emit(&svc.ctx, run_id, "over_budget", &reason);
                svc.away_notifier
                    .notify("Run over budget", &format!("{}: {}", run_id, reason))
                    .await;
                let _ = svc.ctx.executor.feature_cancel(feature_id.as_str()).await;
                return Ok(RunOutcome {
                    project_id: Some(project_id.0.clone()),
                    feature_id: Some(feature_id.0.clone()),
                    status: "over-budget".to_string(),
                    pushed_branch: None,
                    pr_url: None,
                });
            }
        }

        // M5.1: unattended relaxes gates only — the per-command
        // permission/intercept layer and worktree fence are untouched.
        //
        // Keyed off the *pending gate row*, not the feature status: an
        // open gate only flips the gate step to `awaiting_gate` (see
        // `steps/gate/`), while the feature it belongs to stays
        // `running`. The one writer of `awaiting_gate` onto a feature is
        // the startup watchdog's restart reconciliation — so gating this
        // call on the feature status meant a live unattended run never
        // auto-approved anything and parked on every gate.
        // `apply_gate_policy` no-ops when there is no pending gate.
        if spec.unattended {
            apply_gate_policy(svc, run_id, &feature, &mut parked_gates).await;
        }

        if started.elapsed() > MAX_WAIT {
            // The poll window elapsed while the run is still in flight —
            // this is NOT a terminal state. Return the live (non-terminal)
            // status directly, mirroring the budget-cap early return above,
            // so the code below never emits `terminal_state` for a status
            // like "running" (which the laptop would read as a real
            // terminal and the away-notifier as a failure). The run keeps
            // executing on the engine; we've simply stopped waiting on it.
            eprintln!(
                "[demeteo-runner] poll timed out — run still in flight (last status: {})",
                feature.status
            );
            return Ok(RunOutcome {
                project_id: Some(project_id.0.clone()),
                feature_id: Some(feature_id.0.clone()),
                status: feature.status,
                pushed_branch: None,
                pr_url: None,
            });
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    };
    eprintln!(
        "[demeteo-runner] run reached terminal state: {}",
        final_status
    );

    // M7.2 audit: record the run's total spend as its own event so the
    // audit trail is complete on the *cost* dimension too, not just gate
    // decisions and lifecycle. Summed from the live per-step costs (same
    // source the budget cap reads) because `feature.total_cost` is only
    // rolled up at terminal state and the runner's own feature row may
    // not reflect it yet on the first poll after completion.
    if let Ok(steps) = svc.ctx.features.steps_for_feature(feature_id) {
        let total_cost: f64 = steps.iter().map(|s| s.cost_usd.unwrap_or(0.0)).sum();
        emit(
            &svc.ctx,
            run_id,
            "cost",
            format!("${:.4} across {} steps", total_cost, steps.len()),
        );
    }

    emit(&svc.ctx, run_id, "terminal_state", &final_status);

    if !matches!(final_status.as_str(), "completed" | "awaiting_mr") {
        // The success path notifies separately once `pr_opened` fires
        // below (it has a URL, which is the actionable part) — only a
        // failure/interruption is worth an away-notification here.
        svc.away_notifier
            .notify("Run failed", &format!("{}: {}", run_id, final_status))
            .await;
        return Ok(RunOutcome {
            project_id: Some(project_id.0.clone()),
            feature_id: Some(feature_id.0.clone()),
            status: final_status,
            pushed_branch: None,
            pr_url: None,
        });
    }

    // Push is the run-end operation that actually needs the PAT (§6.2) —
    // most of a run executes with no git secret resident at all. A
    // reboot between the poll loop above and here loses the memory-only
    // credential, so re-wait/park here too rather than assuming the
    // pre-clone's PAT is still around.
    let Some(pat) = wait_for_pat(svc, run_id).await else {
        let msg = "no PAT available for the terminal push";
        emit(&svc.ctx, run_id, "needs_credentials", msg);
        svc.away_notifier
            .notify("Run needs credentials", &format!("{}: {}", run_id, msg))
            .await;
        return Ok(RunOutcome {
            project_id: Some(project_id.0.clone()),
            feature_id: Some(feature_id.0.clone()),
            status: "needs-credentials".to_string(),
            pushed_branch: None,
            pr_url: None,
        });
    };

    let branch = push_feature_branch(svc, project_id, feature_id, spec, &pat).await?;
    emit(&svc.ctx, run_id, "pushed", &branch);

    // M5.3/R10: "PR ready" is the success terminal state. Uses the same
    // in-memory PAT (never a keyring lookup) via `publish_mr_with_pat` —
    // opening the PR is an HTTP API call, not a git-credential-store
    // operation, but it must still never touch a standing secret.
    //
    // `title`/`body` stay `None`: the publisher picks up whatever the
    // `finalize` step's agent authored onto the feature row after squashing
    // the branch, and falls back to its own defaults for a workflow that has
    // no finalize step. This is why finalize does not publish for itself — at
    // the time it runs, no PAT is resident (§6.2); the credential only arrives
    // here, at the push.
    let pr_url = match svc
        .ctx
        .mr_publisher
        .publish_mr_with_pat(
            project_id.as_str(),
            feature_id,
            PublishOptions {
                draft: false,
                title: None,
                body: None,
                target_branch: None,
            },
            Some(pat.as_str()),
        )
        .await
    {
        Ok(MrInfo { url, .. }) => {
            emit(&svc.ctx, run_id, "pr_opened", &url);
            svc.away_notifier
                .notify("PR ready", &format!("{}: {}", run_id, url))
                .await;
            Some(url)
        }
        Err(e) => {
            eprintln!("[demeteo-runner] failed to open PR: {}", e);
            emit(&svc.ctx, run_id, "pr_open_failed", &e);
            svc.away_notifier
                .notify(
                    "Run finished but PR failed to open",
                    &format!("{}: {}", run_id, e),
                )
                .await;
            None
        }
    };

    let status = if pr_url.is_some() {
        "pr_ready".to_string()
    } else {
        final_status
    };

    Ok(RunOutcome {
        project_id: Some(project_id.0.clone()),
        feature_id: Some(feature_id.0.clone()),
        status,
        pushed_branch: Some(branch),
        pr_url,
    })
}

/// Push the completed feature's branch to `origin` (R3: results ride
/// git) via per-run askpass (M4.3) — no PAT embedded in the URL or
/// command line.
async fn push_feature_branch(
    svc: &RunnerServices,
    project_id: &ProjectId,
    feature_id: &FeatureId,
    spec: &RunSpec,
    pat: &str,
) -> Result<String, String> {
    let repos = svc.ctx.projects.get_repositories_for(project_id)?;
    let repo = repos
        .first()
        .ok_or_else(|| "project has no repositories configured".to_string())?;

    let settings = svc
        .ctx
        .projects
        .get_settings(project_id)?
        .unwrap_or_else(fetch_default_settings);

    let target_dir =
        paths::repo_target_dir_local(&svc.ctx.workspace_dir, project_id.as_str(), &repo.repo_path);
    let target_dir_str = target_dir.to_string_lossy().to_string();
    let branch = format!(
        "{}{}",
        settings.worktree_strategy.branch_prefix,
        feature_id.as_str()
    );

    git_askpass::run_git(
        &svc.askpass_path,
        &[
            "-C".to_string(),
            target_dir_str.clone(),
            "remote".to_string(),
            "set-url".to_string(),
            "origin".to_string(),
            remote_url(spec),
        ],
        None,
    )
    .await
    .map_err(|e| format!("failed to update remote origin URL: {}", e))?;

    git_askpass::run_git(
        &svc.askpass_path,
        &[
            "-C".to_string(),
            target_dir_str,
            "push".to_string(),
            "-f".to_string(),
            "origin".to_string(),
            branch.clone(),
        ],
        Some(pat),
    )
    .await
    .map_err(|e| format!("failed to push feature branch to origin: {}", e))?;

    eprintln!(
        "[demeteo-runner] pushed feature branch {} to origin",
        branch
    );
    Ok(branch)
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;

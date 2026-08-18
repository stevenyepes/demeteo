//! One conflict-resolution turn, for whoever asked for it.
//!
//! The workflow `sync` node and the "Resolve with agent" button are the same
//! operation on the same worktree, and while each owned its own copy they
//! diverged in the ways that matter: only one of them told the session a turn
//! had started, only one discarded the tree afterwards, and the other left a
//! row reading `conflicted` beside a branch that had been merged. That is the
//! divergence the [`ExecutionPort`] invariant (AGENTS.md §2) forbids one level
//! down, for the same reason — a feature must not behave differently because of
//! who started it.
//!
//! So the turn is a free function over a borrowed bundle rather than a method,
//! and the caller-specific half is only what genuinely differs: which
//! agent/model/effort chain was resolved, which row the stream is keyed to, and
//! what the answer is rendered as. Everything the *sync* is — the preflight, the
//! turn, the marker check, staging, the commit, the push, the session verdict,
//! the teardown — is here once.

use std::sync::Arc;

use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::step_executor::steps::list_unmerged::list_unmerged_files;
use crate::adapters::step_executor::sync_worktree::discard_sync_worktree;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::agent_event::AgentEvent;
use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::sync_session::SyncResolution;
use crate::paths;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ExecutionPort;
use crate::ports::merge::MergeExecutor;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::ports::pricing::PricingTable;

/// The thread-id suffix for the conflict-resolution agent. We use a
/// fresh id (not `feature_id`) so the resolution session is fully
/// independent from the step-execution agent session that drove the
/// implementation: the resolver gets a clean prompt and its own
/// `OPENCODE_PERMISSION` scope.
pub(crate) const SYNC_RESOLVER_THREAD_PREFIX: &str = "sync-resolver";

/// Everything one resolution turn needs, borrowed for the length of it.
///
/// A bundle rather than twenty arguments, and free rather than a method on
/// [`ExecutionDriver`](crate::adapters::step_executor::driver::ExecutionDriver),
/// because the button's caller has no driver and must not make one:
/// `start_execution_with_ctx` answers a second driver for one feature with a
/// silent `Ok(())`, so a run parked at a gate would swallow the request whole.
/// Borrowed ports are also what makes the turn reachable from a test with
/// doubles alone (AGENTS.md §3).
pub(crate) struct ResolveSyncContext<'a> {
    pub exec: &'a Arc<dyn ExecutionPort>,
    pub registry: &'a Arc<AgentRegistry>,
    pub notif: &'a Arc<dyn NotificationPort>,
    pub agent_exec: &'a Arc<dyn AgentExecutionPort>,
    pub app_settings: &'a Arc<dyn AppSettingsRepository>,
    pub git_ops: &'a GitOpsHelper,
    /// Writes the verdict to the feature's sync session — the row every reader
    /// of "is this feature conflicted?" answers from.
    pub merge_executor: &'a Arc<dyn MergeExecutor>,
    pub feature_id: &'a FeatureId,
    /// The clone `resolved_cwd` was cut from; what the teardown runs `git -C`
    /// against, and the one path it refuses to delete.
    pub repo_dir: &'a str,
    pub resolved_cwd: &'a str,
    pub machine_str: &'a str,
    pub feature_branch: &'a str,
    pub base_branch: &'a str,
    pub conflict_files: &'a [String],
    pub step_execution_id: &'a StepExecutionId,
    pub thread_id_prefix: &'a str,
    pub agent_kind: &'a str,
    pub override_model: Option<&'a str>,
    /// The run's resolved effort. Resolving a merge conflict is real
    /// reasoning work, so it inherits rather than being pinned like the
    /// verifier / triage / finalize turns.
    pub effort: crate::domain::models::EffortLevel,
    pub pricing: &'a Arc<dyn PricingTable>,
}

/// Resolve the conflicts in `resolved_cwd` with an agent: the merge commit's
/// sha, or why the resolution did not land.
///
/// The session is moved to `resolving` before the turn and to its verdict
/// afterwards, which is what stops a conflict the run is already resolving from
/// being offered to the user as theirs to act on
/// ([`user_may_intervene`](crate::domain::sync_session::user_may_intervene)).
pub(crate) async fn resolve_sync_conflicts(ctx: ResolveSyncContext<'_>) -> Result<String, String> {
    let merge_executor = ctx.merge_executor;
    let feature_id = ctx.feature_id;
    let exec = ctx.exec;
    let machine_str = ctx.machine_str;
    let repo_dir = ctx.repo_dir;
    let resolved_cwd = ctx.resolved_cwd;

    let _ = merge_executor
        .record_sync_resolution(feature_id, &SyncResolution::Started)
        .await;

    let outcome = run_resolver_turn(ctx).await;

    let verdict = match &outcome {
        Ok(merge_commit_sha) => SyncResolution::Succeeded {
            merge_commit_sha: merge_commit_sha.clone(),
        },
        Err(reason) => SyncResolution::Failed {
            reason: reason.clone(),
        },
    };
    if outcome.is_ok() {
        discard_sync_worktree(&**exec, machine_str, repo_dir, resolved_cwd).await;
    }
    let _ = merge_executor
        .record_sync_resolution(feature_id, &verdict)
        .await;

    outcome
}

async fn run_resolver_turn(sync_ctx: ResolveSyncContext<'_>) -> Result<String, String> {
    let ResolveSyncContext {
        exec,
        registry,
        notif,
        agent_exec,
        app_settings,
        git_ops,
        feature_id,
        resolved_cwd,
        machine_str,
        feature_branch,
        base_branch,
        conflict_files,
        step_execution_id,
        thread_id_prefix,
        agent_kind,
        override_model,
        effort,
        pricing,
        ..
    } = sync_ctx;

    let fid = feature_id;

    // Safety check: is a merge actually active?
    let pre_unmerged = list_unmerged_files(&**exec, machine_str, resolved_cwd).await;
    let merge_in_progress = exec
        .run_command(
            machine_str,
            &format!(
                "git -C {} rev-parse --verify MERGE_HEAD",
                paths::shell_escape_posix(resolved_cwd)
            ),
        )
        .await
        .is_ok();

    if pre_unmerged.is_empty() && !merge_in_progress {
        return Err("No active merge in progress. Please run 'Sync with main' first.".to_string());
    }

    // Spawn a fresh agent session.
    let resolver_thread_id = format!("{}-{}", thread_id_prefix, paths::now_ms());
    // Every supported agent is a CLI runtime that takes its model via the
    // `--model` flag in `build_args` from `ctx.model` below.
    let agent_env = crate::ports::agent_runtime::agent_base_env(exec.as_ref(), machine_str).await;
    let platform =
        crate::ports::agent_runtime::resolve_agent_platform(exec.as_ref(), machine_str).await;

    let binary = registry
        .runtime_for(agent_kind)
        .map(|r| r.binary().to_string())
        .unwrap_or_else(|| agent_kind.to_string());
    let ctx = AgentContext {
        thread_id: resolver_thread_id.clone(),
        machine_id: machine_str.to_string(),
        binary,
        args: vec![],
        env: agent_env,
        cwd: resolved_cwd.to_string(),
        model: override_model.map(str::to_string),
        effort: Some(effort),
        title: Some("Sync conflict resolver".to_string()),
        platform,
        agent_exec: agent_exec.clone(),
        exec: exec.clone(),
        permissions: crate::domain::permission::PermissionProfile::all_allow(),
        bare_mode: true,
        keep_harness_personalization: crate::domain::turn_role::TurnRole::Orchestrator
            .keeps_harness_personalization(),
        tool_allowlist: None,
        max_turns: None,
        // Standalone resolver path (no driver budget in scope); uncapped like
        // its turn count.
        max_budget_usd: None,
    };

    let session = registry
        .get_or_spawn(&resolver_thread_id, agent_kind, ctx)
        .await
        .map_err(|e| format!("Failed to spawn resolver agent: {}", e))?;

    let prompt = build_resolver_prompt(feature_branch, base_branch, conflict_files);

    let timeouts = crate::application::timeouts::resolve_effective(app_settings.as_ref());

    let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
        &*session,
        &prompt,
        timeouts,
        None, // No cancel watch for resolver agent
        machine_str,
        &**exec,
        override_model.map(str::to_string),
        pricing.clone(),
        |event| {
            if let AgentEvent::Text { delta } = event {
                let _ = notif.emit(&DomainEvent::AgentStream {
                    feature_id: fid.clone(),
                    step_execution_id: step_execution_id.clone(),
                    content: delta.clone(),
                });
            }
        },
    )
    .await;

    match turn_res {
        crate::adapters::agent::event_stream::TurnResult::Interrupted => {
            let _ = registry.kill(&resolver_thread_id).await;
            return Err("Resolver execution interrupted".to_string());
        }
        crate::adapters::agent::event_stream::TurnResult::Failed(descriptive)
        | crate::adapters::agent::event_stream::TurnResult::Environmental(descriptive) => {
            let _ = registry.kill(&resolver_thread_id).await;
            return Err(descriptive);
        }
        crate::adapters::agent::event_stream::TurnResult::Success(_) => {}
    }

    // The agent's worktree fence deliberately excludes the linked-worktree
    // index. Demeteo owns staging and committing after the agent resolves
    // the conflicted content.
    if let Err(reason) =
        ensure_conflict_markers_removed(&**exec, machine_str, resolved_cwd, &pre_unmerged).await
    {
        let _ = registry.kill(&resolver_thread_id).await;
        return Err(reason);
    }

    let conflict_paths = pre_unmerged
        .iter()
        .map(|file| paths::shell_escape_posix(&file.path))
        .collect::<Vec<_>>()
        .join(" ");
    if let Err(e) = exec
        .run_command(
            machine_str,
            &format!(
                "git -C {} add -- {}",
                paths::shell_escape_posix(resolved_cwd),
                conflict_paths,
            ),
        )
        .await
    {
        let _ = registry.kill(&resolver_thread_id).await;
        return Err(format!("Failed to stage conflict resolution: {}", e));
    }

    // Staging turns Git's unmerged index entries into the resolved files;
    // this is the authoritative completion check, independent of agent kind.
    let still_unmerged = list_unmerged_files(&**exec, machine_str, resolved_cwd).await;
    if !still_unmerged.is_empty() {
        let _ = registry.kill(&resolver_thread_id).await;
        return Err("Resolver did not resolve every conflicted file.".to_string());
    }

    let message = format!("chore: resolve sync conflicts with origin/{}", base_branch);
    if let Err(rejection) = git_ops
        .validate_commit_message(
            if machine_str == crate::domain::ids::LOCAL_MACHINE {
                None
            } else {
                Some(machine_str)
            },
            resolved_cwd,
            &message,
        )
        .await
    {
        let _ = registry.kill(&resolver_thread_id).await;
        return Err(format!(
            "The repository's commit-msg hook rejected the sync-resolution commit: {}",
            rejection.hook_output
        ));
    }

    // The hook has already accepted this exact message above, matching the
    // finalize flow's validate-then-commit split. Avoid rerunning arbitrary
    // repository hooks after the merge has been staged.
    let commit_resolved = exec
        .run_command(
            machine_str,
            &format!(
                "{} -c user.email=demeteo@local -c user.name=demeteo commit -m {}",
                paths::git_no_hooks(resolved_cwd),
                paths::shell_escape_posix(&message),
            ),
        )
        .await;
    if let Err(e) = commit_resolved {
        let _ = registry.kill(&resolver_thread_id).await;
        return Err(format!("Failed to commit resolution: {}", e));
    }

    // Push the resolution to origin remote.
    exec.run_command(
        machine_str,
        &format!(
            "git -C {} push origin {}",
            paths::shell_escape_posix(resolved_cwd),
            paths::shell_escape_posix(feature_branch),
        ),
    )
    .await
    .map_err(|e| {
        format!(
            "Resolution committed locally but push to origin/{} failed: {}. Push the feature branch manually.",
            feature_branch, e
        )
    })?;

    let _ = registry.kill(&resolver_thread_id).await;

    // Capture the new HEAD sha.
    let head_sha = exec
        .run_command(
            machine_str,
            &format!(
                "git -C {} rev-parse HEAD",
                paths::shell_escape_posix(resolved_cwd)
            ),
        )
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    Ok(head_sha)
}

async fn ensure_conflict_markers_removed(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    worktree: &str,
    conflict_files: &[crate::domain::models::ConflictFile],
) -> Result<(), String> {
    for file in conflict_files {
        let path = paths::join_on(
            worktree,
            [file.path.as_str()],
            paths::targets_windows_host(machine_str),
        );
        let content = exec
            .read_file(machine_str, &path)
            .await
            .map_err(|e| format!("Failed to read resolved conflict file {}: {}", file.path, e))?;
        if has_conflict_marker(&content) {
            return Err(format!(
                "Resolver left merge conflict markers in {}.",
                file.path
            ));
        }
    }
    Ok(())
}

fn has_conflict_marker(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("<<<<<<<")
            || trimmed.starts_with("=======")
            || trimmed.starts_with(">>>>>>>")
            || trimmed.starts_with("|||||||")
    })
}

/// Build the constrained prompt for the conflict-resolution agent.
/// The agent is told exactly which files to edit and explicitly
/// forbidden from touching anything else — keeps the cost low and
/// the resolution deterministic.
fn build_resolver_prompt(
    feature_branch: &str,
    base_branch: &str,
    conflict_files: &[String],
) -> String {
    let files_list = conflict_files
        .iter()
        .map(|f| format!("- {}", f))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "We just merged origin/{base} into {feature}. A merge conflict was detected.\n\
         Please resolve the conflicts in the following files:\n\
         {files}\n\n\
         For each file:\n\
         - Read the conflict markers (<<<<<<<, =======, >>>>>>>).\n\
         - Integrate the changes from both sides correctly.\n\
         - Remove all conflict markers.\n\
         - Do NOT modify any other file or any other part of the listed files.\n\
         - When done, run the project's build / test suite to confirm nothing is broken.\n\
         - Do NOT stage or commit — Demeteo validates, stages, and commits the resolution.\n\
         - Report back with a one-line summary when you're done.",
        base = base_branch,
        feature = feature_branch,
        files = files_list,
    )
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/step_executor/sync_resolve.rs"]
mod tests;

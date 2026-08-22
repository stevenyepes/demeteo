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

use tokio::sync::watch;

use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::step_executor::spend::RunningSpend;
use crate::adapters::step_executor::step_status::CacheTokens;
use crate::adapters::step_executor::steps::list_unmerged::try_list_unmerged_files;
use crate::adapters::step_executor::steps::pending_commit::{self, PendingCommit};
use crate::adapters::step_executor::sync_worktree::discard_sync_worktree;
use crate::adapters::worktree::git_ops::sync_verify::GateVerdict;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::agent_event::AgentEvent;
use crate::domain::ids::FeatureId;
use crate::domain::models::StepExecution;
use crate::domain::sync_session::{
    publish_policy, resolution_is_reviewable, resolution_refusal, ResolutionPublish, SyncResolution,
};
use crate::paths;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::{ask, Answer, ExecutionPort};
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
    /// What this project asks of a resolved tree before it is committed, and
    /// the command the prompt names.
    ///
    /// The same gate a clean merge is held to, derived once by
    /// [`sync_gate`](crate::adapters::step_executor::sync::sync_gate). Why it
    /// has to be the same one is
    /// [`verify_failure_stage`](crate::domain::sync_failure::verify_failure_stage)'s
    /// to say.
    pub gate: crate::ports::worktree_ops::MergeGate<'a>,
    /// The persisted row this turn reports through.
    ///
    /// `AgentStream` is keyed to its execution id and `StepProgress` to its
    /// step id, and the inspector only ever subscribes to an id it found in
    /// `step_list_for_run`. A synthesised id therefore streams to a buffer
    /// nothing renders, which is what the button did until the row existed.
    pub step_exec: &'a StepExecution,
    pub thread_id_prefix: &'a str,
    pub agent_kind: &'a str,
    pub override_model: Option<&'a str>,
    /// The run's resolved effort. Resolving a merge conflict is real
    /// reasoning work, so it inherits rather than being pinned like the
    /// verifier / triage / finalize turns.
    pub effort: crate::domain::models::EffortLevel,
    /// The per-turn dollar ceiling, resolved by whichever caller owns the
    /// run's override chain through [`crate::domain::agent_session::budget`].
    /// One arithmetic, two callers — a second precedence chain here is how the
    /// button and the node would come to spend different money.
    pub max_budget_usd: Option<f64>,
    /// The project's `sync_review_before_push`, and the feature's own status.
    ///
    /// The two facts [`publish_policy`](crate::domain::sync_session::publish_policy)
    /// decides publication from, carried raw so the decision itself is made
    /// here — once, for both callers. Handing this module the *verdict* is how
    /// the two callers came to hold two copies of one policy, each reachable
    /// only through a driver and so pinned by nothing: a call site rewritten to
    /// a constant turned the feature off with the suite green. A status is data
    /// the caller reports, not an answer to "who called"; that a workflow node
    /// always arrives at `Push` is a consequence of the run still holding the
    /// feature, which is exactly what the status says.
    pub review_before_push: Option<bool>,
    pub feature_status: &'a str,
    /// Stop, as whoever owns this turn hears it: the driver's own watch on the
    /// workflow path, the executor's out-of-band sender on the button's.
    pub cancel: Option<watch::Receiver<bool>>,
    /// The totals this turn spends against, advanced in place. Discarding the
    /// `TurnOutcome` is what billed a $10 resolution to nothing.
    pub spend: RunningSpend<'a>,
    pub pricing: &'a Arc<dyn PricingTable>,
}

/// Anti-runaway cap on the resolver's agentic turns, the verifier's number and
/// its reasoning: a tripped cap ends the turn through the normal error path,
/// and the caller's own retry ladder owns recovery.
///
/// It is not a cap on the *resolution*. Tripping it after the conflicted files
/// were already correct still lands the resolution, because the tree is read
/// afterwards either way — the reasoning is on
/// [`resolution_refusal`](crate::domain::sync_session::resolution_refusal).
/// That is what keeps this number free to be conservative: the cost of it
/// being too low is a turn that stops early, not work thrown away.
const RESOLVER_MAX_TURNS: u32 = 25;

/// What a stopped resolution reads as, wherever the stop was noticed.
const CANCELLED_REASON: &str = "Resolver interrupted by Stop.";

/// Why a resolution did not land.
///
/// Two arms because the row they close reads differently: a turn the user
/// stopped is `interrupted`, and only a turn that ran and failed is `failed`.
/// Spelled as a type rather than a marker inside the message because reading a
/// class back out of an error string is the bug this feature keeps
/// reintroducing ([`crate::domain::harness_failure`]).
#[derive(Debug)]
pub(crate) enum ResolveSyncError {
    Cancelled(String),
    Failed(String),
}

impl ResolveSyncError {
    /// What the user reads, whichever arm it is.
    pub(crate) fn reason(self) -> String {
        match self {
            Self::Cancelled(reason) | Self::Failed(reason) => reason,
        }
    }

    fn message(&self) -> &str {
        match self {
            Self::Cancelled(reason) | Self::Failed(reason) => reason,
        }
    }
}

/// A resolution that landed: the commit that proves it, and what the turn read
/// from and wrote into the prompt cache.
///
/// The cache pair rides back rather than being folded into
/// [`RunningSpend`] because the row reports it beside the dollars in one
/// transition, and a caller holding one without the other is how the live
/// cache chip goes blank — the shape `steps::conflict_pass` already returns
/// for the same reason.
#[derive(Debug)]
pub(crate) struct ResolvedSync {
    pub merge_commit_sha: String,
    /// Whether the commit was *confirmed* to have reached origin. `false` is
    /// not a failure — it is a resolution waiting for a look, whether it was
    /// held deliberately or pushed without the remote-tracking ref agreeing —
    /// and the session records it as `resolved` with no `pushed_at`.
    pub published: bool,
    pub cache: CacheTokens,
}

/// Resolve the conflicts in `resolved_cwd` with an agent: the merge commit's
/// sha, or why the resolution did not land.
///
/// The session is moved to `resolving` before the turn and to its verdict
/// afterwards, which is what stops a conflict the run is already resolving from
/// being offered to the user as theirs to act on
/// ([`user_may_intervene`](crate::domain::sync_session::user_may_intervene)).
pub(crate) async fn resolve_sync_conflicts(
    ctx: ResolveSyncContext<'_>,
) -> Result<ResolvedSync, ResolveSyncError> {
    let merge_executor = ctx.merge_executor;
    let feature_id = ctx.feature_id;
    let exec = ctx.exec;
    let machine_str = ctx.machine_str;
    let repo_dir = ctx.repo_dir;
    let resolved_cwd = ctx.resolved_cwd;

    let pre_unmerged = match preflight(&**exec, machine_str, resolved_cwd).await {
        Ok(files) => files,
        // A verdict closes the session; an unreadable tree leaves it exactly as
        // it was, still naming the worktree, for whoever can reach the machine.
        Err(PreflightRefusal::Unreadable(why)) => return Err(ResolveSyncError::Failed(why)),
        Err(PreflightRefusal::NothingToResolve(why)) => {
            let _ = merge_executor
                .record_sync_resolution(
                    feature_id,
                    &SyncResolution::Failed {
                        reason: why.clone(),
                    },
                )
                .await;
            return Err(ResolveSyncError::Failed(why));
        }
    };

    let _ = merge_executor
        .record_sync_resolution(feature_id, &SyncResolution::Started)
        .await;

    let outcome = run_resolver_turn(ctx, &pre_unmerged).await;

    let verdict = match &outcome {
        // A published resolution has nothing left in the tree it was made in.
        // An unpublished one is the opposite: the branch it is committed on is
        // checked out there, so that tree is where the review's `Discard` puts
        // the branch back, and tearing it down here would leave the only way
        // out of the state a `reset` in the user's own clone against whatever
        // it happens to have checked out.
        Ok(resolved) if resolved.published => {
            discard_sync_worktree(&**exec, machine_str, repo_dir, resolved_cwd).await;
            SyncResolution::Succeeded {
                merge_commit_sha: resolved.merge_commit_sha.clone(),
                published: true,
                worktree_discarded: resolved_cwd == repo_dir
                    || crate::application::sync_session::worktree_confirmed_gone(
                        &**exec,
                        machine_str,
                        resolved_cwd,
                    )
                    .await,
            }
        }
        Ok(resolved) => SyncResolution::Succeeded {
            merge_commit_sha: resolved.merge_commit_sha.clone(),
            published: false,
            worktree_discarded: false,
        },
        Err(failure) => SyncResolution::Failed {
            reason: failure.message().to_string(),
        },
    };
    let _ = merge_executor
        .record_sync_resolution(feature_id, &verdict)
        .await;

    outcome
}

/// Why a turn stopped before it started — and, decisively, whether the machine
/// answered.
///
/// The two arms differ only in what the caller is then allowed to write. A
/// preflight that answered from an *unreachable* host used to rewrite a
/// `conflicted` row to `resolution_failed` and replace `raw_error` with "no
/// merge in progress": a diagnosis of a tree nobody looked at, telling the user
/// to re-run Sync, whose force-remove then takes the still-live conflicted
/// worktree with it.
enum PreflightRefusal {
    /// git answered, and there is no merge here for an agent to resolve.
    NothingToResolve(String),
    /// The worktree could not be read. Nothing may be concluded from that in
    /// either direction, so nothing may be recorded from it either.
    Unreadable(String),
}

/// Is there a merge in `worktree` for an agent to resolve, and which files did
/// it leave unmerged?
async fn preflight(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    worktree: &str,
) -> Result<Vec<crate::domain::models::ConflictFile>, PreflightRefusal> {
    let unmerged = try_list_unmerged_files(exec, machine_str, worktree)
        .await
        .map_err(|why| {
            PreflightRefusal::Unreadable(format!(
                "Could not read the sync worktree at {} to see what the merge left, \
                 so the sync was left as it was: {}",
                worktree, why
            ))
        })?;
    if !unmerged.is_empty() {
        return Ok(unmerged);
    }
    match ask(
        exec,
        machine_str,
        &format!(
            "git -C {} rev-parse --verify --quiet MERGE_HEAD",
            paths::shell_escape_posix(worktree)
        ),
    )
    .await
    {
        Answer::Said(out) if !out.trim().is_empty() => Ok(unmerged),
        Answer::Said(_) | Answer::Refused => Err(PreflightRefusal::NothingToResolve(
            "No active merge in progress. Please run 'Sync with main' first.".to_string(),
        )),
        Answer::Unreadable(e) => Err(PreflightRefusal::Unreadable(format!(
            "Could not check {} for an open merge, so the sync was left as it was: {}",
            worktree, e
        ))),
    }
}

/// Has a stop arrived? Read before the spawn and again after the turn, because
/// a cancel that lands while git is mid-flight otherwise surfaces as an
/// ordinary failure and the row records the wrong thing about who ended it.
fn cancelled(cancel: &Option<watch::Receiver<bool>>) -> bool {
    cancel.as_ref().is_some_and(|rx| *rx.borrow())
}

async fn run_resolver_turn(
    sync_ctx: ResolveSyncContext<'_>,
    pre_unmerged: &[crate::domain::models::ConflictFile],
) -> Result<ResolvedSync, ResolveSyncError> {
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
        gate,
        step_exec,
        thread_id_prefix,
        agent_kind,
        override_model,
        effort,
        max_budget_usd,
        review_before_push,
        feature_status,
        cancel,
        spend,
        pricing,
        ..
    } = sync_ctx;

    let publish = publish_policy(review_before_push, resolution_is_reviewable(feature_status));

    let RunningSpend {
        cost: accumulated_cost,
        tokens: accumulated_tokens,
        start: step_start,
    } = spend;
    let fid = feature_id;

    if cancelled(&cancel) {
        return Err(ResolveSyncError::Cancelled(CANCELLED_REASON.to_string()));
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
        // `all_allow`, and not a `StepCapability`, because the resolver edits
        // conflicted *source* and then runs the project's build: every
        // capability but `Implement` resolves `write_scope()` to `None` or
        // `ArtifactsOnly`, whose chmod fence would take write off exactly the
        // files this turn exists to change. Against `Implement` — whose fence
        // is a documented no-op — the one dimension that differs is `network`,
        // deliberately: a resolution may need to read a changelog.
        //
        // How tightly `cwd` then confines the turn is the harness's answer and
        // not this profile's — `PathContainment` in
        // `domain/models/sandbox.rs`. Narrowing the profile is not the lever
        // for it either: a profile that cannot write source cannot resolve a
        // conflict.
        permissions: crate::domain::permission::PermissionProfile::all_allow(),
        bare_mode: true,
        keep_harness_personalization: crate::domain::turn_role::TurnRole::Orchestrator
            .keeps_harness_personalization(),
        tool_allowlist: None,
        max_turns: Some(RESOLVER_MAX_TURNS),
        max_budget_usd,
    };

    let session = registry
        .get_or_spawn(&resolver_thread_id, agent_kind, ctx)
        .await
        .map_err(|e| ResolveSyncError::Failed(format!("Failed to spawn resolver agent: {}", e)))?;

    let base_moves = base_side_moves(&**exec, machine_str, resolved_cwd).await;
    let prompt = build_resolver_prompt(
        feature_branch,
        base_branch,
        conflict_files,
        gate.harness,
        &base_moves,
    );

    let timeouts = crate::application::timeouts::resolve_effective(app_settings.as_ref());
    let base_cost = *accumulated_cost;
    let base_tokens = *accumulated_tokens;

    let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
        &*session,
        &prompt,
        timeouts,
        cancel.clone(),
        machine_str,
        &**exec,
        override_model.map(str::to_string),
        pricing.clone(),
        |event| {
            if let AgentEvent::Text { delta } = event {
                let _ = notif.emit(&DomainEvent::AgentStream {
                    feature_id: fid.clone(),
                    step_execution_id: step_exec.id.clone(),
                    content: delta.clone(),
                });
                let _ = notif.emit(&DomainEvent::StepProgress {
                    feature_id: fid.clone(),
                    step_id: step_exec.step_id.0.clone(),
                    status: "running".into(),
                    cost_usd: Some(base_cost),
                    tokens: Some(base_tokens),
                    wall_clock_secs: Some(step_start.elapsed().as_secs()),
                    cache_read_input_tokens: None,
                    cache_creation_input_tokens: None,
                });
            }
        },
    )
    .await;

    // A turn that ended badly is *not* a reading of the tree, and only the tree
    // decides here — `domain::sync_session::resolution_refusal` carries the
    // whole of why. So every
    // ending but a stop falls through to the marker check, the staging and the
    // index re-check below, which are the same evidence a clean exit is judged
    // on. What the ending is still good for is explaining a tree that really is
    // unresolved, and it is kept for exactly that.
    let (cache, turn_stop) = match turn_res {
        crate::adapters::agent::event_stream::TurnResult::Interrupted => {
            let _ = registry.kill(&resolver_thread_id).await;
            return Err(ResolveSyncError::Cancelled(CANCELLED_REASON.to_string()));
        }
        crate::adapters::agent::event_stream::TurnResult::Failed { reason, spent }
        | crate::adapters::agent::event_stream::TurnResult::Environmental { reason, spent } => {
            *accumulated_cost += spent.cost_usd;
            *accumulated_tokens += spent.tokens;
            (
                CacheTokens {
                    read: Some(spent.cache_read_input_tokens),
                    creation: Some(spent.cache_creation_input_tokens),
                },
                Some(reason),
            )
        }
        crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
            *accumulated_cost += outcome.cost_usd;
            *accumulated_tokens += outcome.tokens;
            (
                CacheTokens {
                    read: Some(outcome.cache_read_input_tokens),
                    creation: Some(outcome.cache_creation_input_tokens),
                },
                None,
            )
        }
    };
    let turn_stop = turn_stop.as_deref();

    if cancelled(&cancel) {
        let _ = registry.kill(&resolver_thread_id).await;
        return Err(ResolveSyncError::Cancelled(CANCELLED_REASON.to_string()));
    }

    // The agent's worktree fence deliberately excludes the linked-worktree
    // index. Demeteo owns staging and committing after the agent resolves
    // the conflicted content.
    if let Err(reason) =
        ensure_conflict_markers_removed(&**exec, machine_str, resolved_cwd, pre_unmerged).await
    {
        let _ = registry.kill(&resolver_thread_id).await;
        return Err(ResolveSyncError::Failed(resolution_refusal(
            turn_stop, &reason,
        )));
    }

    // Reaped here rather than on each exit below: the turn is over, and holding
    // a live process — over SSH, its channel too — across the multi-minute
    // build the gate is about to run is waste.
    let _ = registry.kill(&resolver_thread_id).await;

    // Before `git add -A`, not after. A gate run afterwards stages its own
    // build output through the same `-A`, which `pending_commit::probe` then
    // reads as work an agent left uncommitted; and a red gate would leave a
    // staged index, so the next attempt's marker check would iterate an empty
    // unmerged list and pass vacuously. The tree on disk is the same either
    // way — unmerged *index* entries are not file contents.
    match crate::adapters::worktree::git_ops::sync_verify::run_merge_gate(
        &**exec,
        machine_str,
        gate,
        crate::adapters::step_executor::harness_shell::harness_shell_options(
            app_settings.as_ref(),
            resolved_cwd,
        ),
        cancel.clone(),
    )
    .await
    {
        GateVerdict::Clear => {}
        GateVerdict::Stopped => {
            return Err(ResolveSyncError::Cancelled(CANCELLED_REASON.to_string()))
        }
        GateVerdict::Failed { command, error } => {
            if let Some(refusal) =
                crate::domain::sync_session::resolution_verification_refusal(&command, Err(&error))
            {
                return Err(ResolveSyncError::Failed(resolution_refusal(
                    turn_stop, &refusal,
                )));
            }
        }
    }

    // `-A`, not the conflicted paths the merge reported. The sync worktree is a
    // throwaway checkout that exists only for this resolution, and it is deleted
    // the moment the resolution lands — so a file the agent had to add, or a
    // fourth file it had to fix for the tree to build, is not "extra": staging
    // only the reported paths committed a tree that does not compile and then
    // removed the rest with the directory. `-A` still honours `.gitignore`, so a
    // resolver that ran the project's tests does not stage `node_modules` or
    // `target`.
    if let Err(e) = exec
        .run_command(
            machine_str,
            &format!("git -C {} add -A", paths::shell_escape_posix(resolved_cwd)),
        )
        .await
    {
        return Err(ResolveSyncError::Failed(format!(
            "Failed to stage conflict resolution: {}",
            e
        )));
    }

    // Staging turns Git's unmerged index entries into the resolved files;
    // this is the authoritative completion check, independent of agent kind.
    let still_unmerged = match try_list_unmerged_files(&**exec, machine_str, resolved_cwd).await {
        Ok(files) => files,
        Err(why) => {
            return Err(ResolveSyncError::Failed(format!(
                "Could not read {} back to confirm the resolution: {}",
                resolved_cwd, why
            )));
        }
    };
    if !still_unmerged.is_empty() {
        return Err(ResolveSyncError::Failed(resolution_refusal(
            turn_stop,
            "Resolver did not resolve every conflicted file.",
        )));
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
        return Err(ResolveSyncError::Failed(format!(
            "The repository's commit-msg hook rejected the sync-resolution commit: {}",
            rejection.hook_output
        )));
    }

    // The hook has already accepted this exact message above, matching the
    // finalize flow's validate-then-commit split. Avoid rerunning arbitrary
    // repository hooks after the merge has been staged.
    //
    // Guarded because an agent that committed on its own leaves nothing to
    // record — see `steps::pending_commit`.
    match pending_commit::probe(&**exec, machine_str, resolved_cwd).await {
        PendingCommit::Nothing => {}
        // The one arm with data loss behind it. A skipped commit still pushes
        // (a no-op), still reads a sha back (the pre-merge one), still files
        // the session `Resolved` — and the teardown then force-removes the
        // worktree the agent's work is sitting in, unpublished.
        PendingCommit::Unreadable(why) => {
            return Err(ResolveSyncError::Failed(format!(
                "Could not tell whether the resolution still needs committing, so it was left in {}: {}",
                resolved_cwd, why
            )));
        }
        PendingCommit::Pending => {
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
                return Err(ResolveSyncError::Failed(format!(
                    "Failed to commit resolution: {}",
                    e
                )));
            }
        }
    }

    // Read before the push rather than after it. `push` does not move `HEAD`,
    // so the two orders name the same commit — but this one is still on the
    // failing side of the publish, so an unreadable answer can be refused
    // outright instead of becoming the empty sha a `Succeeded` verdict then
    // carries as its evidence.
    let head_sha = match ask(
        &**exec,
        machine_str,
        &format!(
            "git -C {} rev-parse HEAD",
            paths::shell_escape_posix(resolved_cwd)
        ),
    )
    .await
    {
        Answer::Said(out) => out.trim().to_string(),
        Answer::Refused | Answer::Unreadable(_) => {
            return Err(ResolveSyncError::Failed(format!(
                "The resolution was committed in {} but its commit could not be read back, so it was not published.",
                resolved_cwd
            )));
        }
    };

    // The row's `pushed_at` is written from this bool, and the button's
    // `publish` refuses to write it on the strength of an exit code
    // ([`push_landed`](crate::application::sync_session::push_landed)). Two
    // paths writing one column on opposite evidence rules is how a merge origin
    // never received suppresses its own review card forever, so this one asks
    // origin too. An unconfirmed push is not a failed resolution — the commit
    // is on the branch either way — so it lands as a resolution still waiting,
    // which is the state that keeps a surface pointing at it.
    let published = if publish == ResolutionPublish::Push {
        // The credential the remote needs, read from the remote itself. A
        // resolution that is committed and unpushed is recoverable from the
        // banner; one that cannot authenticate is recoverable from nowhere
        // until the provider is reconnected, which is why the failure says
        // which of the two it is.
        let credential = crate::adapters::git_push::credential_for_repo(
            &**exec,
            app_settings.as_ref(),
            machine_str,
            resolved_cwd,
        )
        .await;
        if let Err(e) = exec
            .run_program(
                machine_str,
                crate::adapters::git_push::push_request(
                    resolved_cwd,
                    feature_branch,
                    false,
                    credential.as_ref(),
                ),
            )
            .await
        {
            return Err(ResolveSyncError::Failed(format!(
                "Resolution committed locally but push to origin/{} failed: {}. Publish it from the sync banner once the push can go through.",
                feature_branch,
                crate::adapters::git_push::push_failure(&e, credential.as_ref())
            )));
        }
        matches!(
            crate::application::sync_session::push_landed(
                &**exec,
                machine_str,
                resolved_cwd,
                feature_branch,
                &head_sha,
            )
            .await,
            Answer::Said(_)
        )
    } else {
        false
    };

    Ok(ResolvedSync {
        merge_commit_sha: head_sha,
        published,
        cache,
    })
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

/// What `origin/<base>` added, moved or deleted since this branch left it.
///
/// The conflicted paths are the files git could not merge; these are the files
/// it merged *without asking*, and that is the pair the resolution has to be
/// correct over. A test the base side moved out from under a signature this
/// branch was changing carries no marker and appears in no conflict list, and
/// that is exactly what ended one resolution as a tree that does not build.
///
/// Best-effort by construction: an unreadable answer leaves the prompt without
/// the section rather than failing a turn over a hint. The gate in
/// [`run_resolver_turn`] is what refuses to publish when the aim was off, so
/// neither half is asked to carry the failure alone.
async fn base_side_moves(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    worktree: &str,
) -> Vec<String> {
    match ask(
        exec,
        machine_str,
        &format!(
            "git -C {} diff --name-status -M --diff-filter=ADR HEAD...MERGE_HEAD",
            paths::shell_escape_posix(worktree)
        ),
    )
    .await
    {
        Answer::Said(out) => out
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
        Answer::Refused | Answer::Unreadable(_) => Vec::new(),
    }
}

/// The base side's moves, the ones most likely to matter first.
///
/// Git answers in path order, and path order is not relevance order. On the
/// merge that occasioned this the file that broke the build —
/// `tests/application/run_view.rs`, moved out from under a signature the branch
/// was changing — was the 69th of 252 entries, so a path-ordered cap would have
/// dropped exactly what the section exists to surface. Sharing a filename with
/// a file the resolver is already editing is the one relationship a path alone
/// can carry, so those lead and the rest follow in git's own order.
fn aimed_first<'a>(conflict_files: &[String], base_moves: &'a [String]) -> Vec<&'a str> {
    fn file_name(path: &str) -> &str {
        path.rsplit('/').next().unwrap_or(path)
    }
    let conflicted: std::collections::HashSet<&str> =
        conflict_files.iter().map(|f| file_name(f)).collect();
    let aims_at_a_conflict = |line: &str| {
        line.split_whitespace()
            .any(|field| conflicted.contains(file_name(field)))
    };
    let (aimed, rest): (Vec<&str>, Vec<&str>) = base_moves
        .iter()
        .map(String::as_str)
        .partition(|line| aims_at_a_conflict(line));
    aimed.into_iter().chain(rest).collect()
}

/// How many base-side moves the prompt is willing to spend context on before it
/// hands the resolver the command instead. A hint that drowns the conflict it
/// was meant to aim at is worse than no hint, and the tail is reachable in one
/// call by an agent that has a reason to want it.
const RESOLVER_BASE_MOVE_CAP: usize = 40;

/// Build the prompt for the conflict-resolution agent.
///
/// The scope clause is bounded by what the merge broke, not by what git could
/// not merge. Git reports the files it failed to reconcile *textually*, and
/// that list is not the set a resolution has to be correct over: a file only
/// one side touched merges silently and can still call a signature the other
/// side changed. A resolver told to touch nothing else did exactly as asked,
/// and the merge commit it produced turned every check on the pull request
/// red. So the bound is the merge's own damage — the same bound the `git add
/// -A` in [`run_resolver_turn`] already stages to — and it stays a bound
/// rather than an opening because an agent invited to fix whatever it finds
/// returns a refactor nobody merged.
///
/// The verification line is the one that has to be exact. "Run the project's
/// build / test suite" reads as a complete instruction and is not one: the
/// agent has to *find* the command first, and a search is a turn each against
/// a cap of [`RESOLVER_MAX_TURNS`]. Naming the project's own command turns
/// that search into a single call, and a project with no command configured
/// gets no verification line at all rather than a vague one — an unanswerable
/// instruction is more expensive than a missing one.
fn build_resolver_prompt(
    feature_branch: &str,
    base_branch: &str,
    conflict_files: &[String],
    test_command: Option<&str>,
    base_moves: &[String],
) -> String {
    let files_list = conflict_files
        .iter()
        .map(|f| format!("- {}", f))
        .collect::<Vec<_>>()
        .join("\n");
    let merged_silently = if base_moves.is_empty() {
        String::new()
    } else {
        let mut lines = aimed_first(conflict_files, base_moves)
            .into_iter()
            .take(RESOLVER_BASE_MOVE_CAP)
            .map(|m| format!("- {}", m))
            .collect::<Vec<_>>();
        let rest = base_moves.len().saturating_sub(RESOLVER_BASE_MOVE_CAP);
        if rest > 0 {
            lines.push(format!(
                "- …and {} more. Run `git diff --name-status -M --diff-filter=ADR \
                 HEAD...MERGE_HEAD` for the full list.",
                rest
            ));
        }
        format!(
            "origin/{base} also added, moved or deleted these files since this branch \
             left it. Git merged them without asking, so they carry no markers — and \
             they are where a resolution that only fixes the listed files goes wrong:\n\
             {moves}\n\n",
            base = base_branch,
            moves = lines.join("\n"),
        )
    };
    let verification = match test_command.map(str::trim).filter(|c| !c.is_empty()) {
        Some(cmd) => format!(
            "- When done, verify with this project's own command, exactly as written: `{}`.\n\
             - Do NOT go looking for another command if that one does not work here — \
             say so in your summary and stop.\n\
             - Demeteo runs that same command itself before it commits anything. A tree \
             that does not build is not a resolved conflict, so fix what it reports here \
             rather than leaving it.\n",
            cmd
        ),
        None => String::new(),
    };
    format!(
        "We just merged origin/{base} into {feature}. A merge conflict was detected.\n\
         Please resolve the conflicts in the following files:\n\
         {files}\n\n\
         {merged_silently}\
         For each file:\n\
         - Read the conflict markers (<<<<<<<, =======, >>>>>>>).\n\
         - Integrate the changes from both sides correctly.\n\
         - Remove all conflict markers.\n\
         - Fix a file outside this list only where the merge itself broke it — a \
         caller of a signature one side changed, a test the other side moved. Do \
         not refactor, reformat, or fix anything the merge did not break.\n\
         {verification}\
         - Do NOT stage or commit — Demeteo validates, stages, and commits the resolution.\n\
         - Report back with a one-line summary when you're done.",
        base = base_branch,
        feature = feature_branch,
        files = files_list,
        merged_silently = merged_silently,
        verification = verification,
    )
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/step_executor/sync_resolve.rs"]
mod tests;

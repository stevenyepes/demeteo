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

mod land;
mod preflight;
mod prompt;
mod turn;

use std::sync::Arc;

use tokio::sync::watch;

use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::step_executor::spend::RunningSpend;
use crate::adapters::step_executor::step_status::CacheTokens;
use crate::adapters::step_executor::sync_worktree::discard_sync_worktree;
use crate::adapters::worktree::git_ops::sync_verify::{run_gate_harness, GateVerdict};
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::agent_event::AgentEvent;
use crate::domain::ids::FeatureId;
use crate::domain::models::StepExecution;
use crate::domain::sync_session::{
    gate_follow_up, publish_policy, resolution_is_reviewable, resolution_refusal, GateFollowUp,
    SyncResolution,
};
use crate::paths;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ExecutionPort;
use crate::ports::merge::MergeExecutor;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::ports::pricing::PricingTable;

use self::land::{land, Landing};
/// The two stages the module's own tests reach directly. `super::*` in
/// `tests/infrastructure/step_executor/sync_resolve.rs` resolves through here,
/// so the split above did not move them out of that file's reach.
#[cfg(test)]
use self::preflight::has_conflict_marker;
use self::preflight::{ensure_conflict_markers_removed, preflight, PreflightRefusal};
#[cfg(test)]
use self::prompt::{aimed_first, tracking_tip_at_merge_head, IncomingSide};
use self::prompt::{
    base_side_moves, build_repair_prompt, build_resolver_prompt, incoming_side,
    prepared_verification, Verification,
};
use self::turn::ResolverAgent;

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
    /// the command the prompt names — the same gate a clean merge is held to,
    /// derived once by
    /// [`sync_gate`](crate::adapters::step_executor::sync::sync_gate), for
    /// [`verify_failure_stage`](crate::domain::sync_failure::verify_failure_stage)'s
    /// reasons.
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
    /// The dollar ceiling for the whole resolution, resolved by whichever
    /// caller owns the run's override chain through
    /// [`crate::domain::agent_session::budget`]. One arithmetic, two callers —
    /// a second precedence chain here is how the button and the node would come
    /// to spend different money. A repair round is handed what is left of it,
    /// not a second copy of it.
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

    // Above the spawn, not beside the gate below it: the agent is asked to run
    // this project's harness during its own turn, and in a tree `prepare` never
    // ran in that harness answers about the missing install. Preparing here
    // also means a stop landing during it costs no agent.
    let gate_opts = crate::adapters::step_executor::harness_shell::harness_shell_options(
        app_settings.as_ref(),
        resolved_cwd,
    );
    let Some(verification) = prepared_verification(
        &**exec,
        machine_str,
        gate,
        gate_opts.clone(),
        cancel.clone(),
    )
    .await
    else {
        return Err(ResolveSyncError::Cancelled(CANCELLED_REASON.to_string()));
    };

    let agent = ResolverAgent {
        binary: registry
            .runtime_for(agent_kind)
            .map(|r| r.binary().to_string())
            .unwrap_or_else(|| agent_kind.to_string()),
        env: crate::ports::agent_runtime::agent_base_env(exec.as_ref(), machine_str).await,
        platform: crate::ports::agent_runtime::resolve_agent_platform(exec.as_ref(), machine_str)
            .await,
        machine_str,
        cwd: resolved_cwd,
        model: override_model,
        effort,
        max_budget_usd,
        agent_exec,
        exec,
    };
    let timeouts = crate::application::timeouts::resolve_effective(app_settings.as_ref());

    let mut cache = CacheTokens::default();
    let mut rounds_spent = 0u32;
    let resolution_start_cost = *accumulated_cost;
    // What a red gate is worth is `domain::sync_session::gate_follow_up`. Held
    // as the built prompt rather than the words it was built from, so the
    // harness's output cannot outlive the loop that read it.
    let mut repair_prompt: Option<String> = None;
    let turn_stop = loop {
        if cancelled(&cancel) {
            return Err(ResolveSyncError::Cancelled(CANCELLED_REASON.to_string()));
        }

        // A fresh session per round: the one before it was reaped ahead of the
        // gate, and §2's one-shot-CLI rule leaves no resume contract to lean on.
        let resolver_thread_id = format!("{}-{}", thread_id_prefix, paths::now_ms());
        let ctx = agent.context(
            &resolver_thread_id,
            *accumulated_cost - resolution_start_cost,
        );

        let session = registry
            .get_or_spawn(&resolver_thread_id, agent_kind, ctx)
            .await
            .map_err(|e| {
                ResolveSyncError::Failed(format!("Failed to spawn resolver agent: {}", e))
            })?;

        let prompt = match repair_prompt.take() {
            Some(prompt) => prompt,
            None => {
                // `fast_timeout_s` is the user's own "how long may something
                // be silent"; the wall cap would bound this at half an hour.
                let within = std::time::Duration::from_secs(timeouts.fast_timeout_s);
                let incoming = incoming_side(
                    &**exec,
                    machine_str,
                    resolved_cwd,
                    base_branch,
                    feature_branch,
                    within,
                )
                .await;
                let base_moves = base_side_moves(&**exec, machine_str, resolved_cwd, within).await;
                build_resolver_prompt(
                    feature_branch,
                    incoming,
                    conflict_files,
                    verification,
                    &base_moves,
                )
            }
        };

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

        // A turn that ended badly is *not* a reading of the tree, and only the
        // tree decides here — `domain::sync_session::resolution_refusal` carries
        // the whole of why. So every ending but a stop falls through to the
        // marker check, the staging and the index re-check below, which are the
        // same evidence a clean exit is judged on. What the ending is still good
        // for is explaining a tree that really is unresolved, and it is kept for
        // exactly that.
        let turn_stop = match turn_res {
            crate::adapters::agent::event_stream::TurnResult::Interrupted => {
                let _ = registry.kill(&resolver_thread_id).await;
                return Err(ResolveSyncError::Cancelled(CANCELLED_REASON.to_string()));
            }
            crate::adapters::agent::event_stream::TurnResult::Failed { reason, spent }
            | crate::adapters::agent::event_stream::TurnResult::Environmental { reason, spent } => {
                *accumulated_cost += spent.cost_usd;
                *accumulated_tokens += spent.tokens;
                cache = billed(
                    cache,
                    spent.cache_read_input_tokens,
                    spent.cache_creation_input_tokens,
                );
                Some(reason)
            }
            crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                *accumulated_cost += outcome.cost_usd;
                *accumulated_tokens += outcome.tokens;
                cache = billed(
                    cache,
                    outcome.cache_read_input_tokens,
                    outcome.cache_creation_input_tokens,
                );
                None
            }
        };

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
                turn_stop.as_deref(),
                &reason,
            )));
        }

        // Reaped here rather than on each exit below: the turn is over, and
        // holding a live process — over SSH, its channel too — across the
        // multi-minute build the gate is about to run is waste.
        let _ = registry.kill(&resolver_thread_id).await;

        // Before `git add -A`, not after: a red gate leaves a staged index, and
        // the next attempt's marker check would then iterate an empty unmerged
        // list and pass vacuously. The tree on disk is the same either way —
        // unmerged *index* entries are not file contents.
        let Verification::Gated { command } = verification else {
            break turn_stop;
        };
        match run_gate_harness(
            &**exec,
            machine_str,
            Some(command),
            gate_opts.clone(),
            cancel.clone(),
        )
        .await
        {
            GateVerdict::NotGated | GateVerdict::Passed | GateVerdict::Unprepared { .. } => {
                break turn_stop
            }
            GateVerdict::Stopped => {
                return Err(ResolveSyncError::Cancelled(CANCELLED_REASON.to_string()))
            }
            GateVerdict::Failed { error } => {
                match gate_follow_up(command, &error, rounds_spent) {
                    GateFollowUp::Repair { excerpt } => {
                        repair_prompt =
                            Some(build_repair_prompt(feature_branch, command, &excerpt));
                        rounds_spent += 1;
                    }
                    GateFollowUp::Refuse(refusal) => {
                        return Err(ResolveSyncError::Failed(resolution_refusal(
                            turn_stop.as_deref(),
                            &refusal,
                        )))
                    }
                    // The gate failing open is the incident it exists to
                    // prevent, and without this line its only trace is a wall
                    // clock sitting at the deadline.
                    GateFollowUp::LandUnverified => {
                        tracing::warn!(
                            machine = %machine_str,
                            worktree = %resolved_cwd,
                            command = %command,
                            error = %error,
                            "sync gate reached no verdict: the resolution lands unverified",
                        );
                        break turn_stop;
                    }
                }
            }
        }
    };

    let (merge_commit_sha, published) = land(
        Landing {
            exec,
            git_ops,
            app_settings,
            machine_str,
            resolved_cwd,
            base_branch,
            feature_branch,
            publish,
        },
        turn_stop.as_deref(),
    )
    .await?;

    Ok(ResolvedSync {
        merge_commit_sha,
        published,
        cache,
    })
}
/// One turn's cache counters into the running totals a repair round adds to
/// rather than replaces.
fn billed(cache: CacheTokens, read: u64, creation: u64) -> CacheTokens {
    CacheTokens {
        read: Some(cache.read.unwrap_or(0) + read),
        creation: Some(cache.creation.unwrap_or(0) + creation),
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/sync_resolve.rs"]
mod tests;

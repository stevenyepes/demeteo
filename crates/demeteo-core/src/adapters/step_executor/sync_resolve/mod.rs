//! Resolving the conflict a sync left, for whoever is doing it.
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
//!
//! A third caller has no turn at all: [`continue_sync_resolution`] is the person
//! who resolved it themselves pressing "I've resolved it". It shares this
//! module for the same reason the other two do — a conflict a person finished
//! and one an agent finished must reach origin as the same commit, gated the
//! same way — so what it skips is the agent and nothing else.

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
    gate_follow_up, publish_policy, remaining_conflicts_refusal, resolution_follow_up,
    resolution_is_reviewable, resolution_refusal, resolution_verification_refusal, GateFollowUp,
    ResolutionFollowUp, ResolutionPublish, SyncResolution,
};
use crate::paths;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ExecutionPort;
use crate::ports::merge::MergeExecutor;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::ports::pricing::PricingTable;

use self::land::{land, Landing};
pub(crate) use self::preflight::unresolved_files;
/// The two stages the module's own tests reach directly. `super::*` in
/// `tests/infrastructure/step_executor/sync_resolve.rs` resolves through here,
/// so the split above did not move them out of that file's reach.
#[cfg(test)]
use self::preflight::{conflict_hunks, has_conflict_marker};
use self::preflight::{preflight, PreflightRefusal};
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
    /// What this resolution must prove marker-free before any of it lands.
    ///
    /// The conflict as the *session row* recorded it, and deliberately not the
    /// index's live answer. Clearing a marker in the working tree leaves the
    /// path `UU` until something stages it, and `git add -A` collapses every
    /// stage whatever the file still holds — so an index-derived list can
    /// neither see that a file was finished nor that one was not. Both checks
    /// standing between a conflict and a commit used to derive their files from
    /// `git status`, and an index with nothing unmerged left in it makes both
    /// iterate an empty list and pass without reading a byte.
    ///
    /// Carried by the caller rather than read back here because both callers
    /// already hold it, and `sync_session` reconciles the row against the
    /// worktree — several round trips, on a path that may be an SSH one.
    pub declared_conflicts: &'a [crate::domain::models::ConflictFile],
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

/// Anti-runaway cap on one *round's* agentic turns — the verifier's number, for
/// a job that is nothing like the verifier's.
///
/// It is not a cap on the resolution. Tripping it after the conflicted files
/// were already correct still lands the resolution, because the tree is read
/// afterwards either way
/// ([`resolution_refusal`](crate::domain::sync_session::resolution_refusal)) —
/// and tripping it *part* way now costs a round rather than the resolution,
/// because the loop reads what is left and hands the next round only that
/// ([`resolution_follow_up`](crate::domain::sync_session::resolution_follow_up)).
///
/// This comment used to say the caller's retry ladder owned recovery from a
/// tripped cap. Only the workflow node had one. On the button's path a stopped
/// turn was the whole resolution's verdict, and the press that produced it had
/// to be repeated by hand — each repetition spending its first two-thirds of
/// this cap re-reading the files the last one had already finished, because the
/// prompt was rebuilt from a list that could not shrink. Eight files was enough
/// for that to never terminate.
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

/// Where a resolution is written down, and where it is announced.
///
/// One parameter rather than two, for
/// [`StatusWriters`](crate::adapters::step_executor::step_status)' reason: the
/// row is the truth and the event is its push, and a caller that can reach the
/// first without the second writes verdicts nothing tells the UI about. That is
/// not hypothetical here — a sync records itself on a step every reader of a
/// *run* excludes by design, so the pane rendering this row has no other way to
/// hear that a background resolution finished.
#[derive(Clone, Copy)]
struct ResolutionRecorder<'a> {
    merge_executor: &'a Arc<dyn MergeExecutor>,
    notif: &'a Arc<dyn NotificationPort>,
}

impl ResolutionRecorder<'_> {
    async fn record(&self, feature_id: &FeatureId, resolution: &SyncResolution) {
        let _ = self
            .merge_executor
            .record_sync_resolution(feature_id, resolution)
            .await;
        let _ = self.notif.emit(&DomainEvent::SyncStatusChanged {
            feature_id: feature_id.clone(),
            status: resolution.status(),
        });
    }
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
    let recorder = ResolutionRecorder {
        merge_executor: ctx.merge_executor,
        notif: ctx.notif,
    };
    let feature_id = ctx.feature_id;
    let exec = ctx.exec;
    let machine_str = ctx.machine_str;
    let repo_dir = ctx.repo_dir;
    let resolved_cwd = ctx.resolved_cwd;

    let declared = open_resolution(
        &**exec,
        recorder,
        feature_id,
        machine_str,
        resolved_cwd,
        ctx.declared_conflicts,
    )
    .await?;

    let outcome = run_resolver_turn(ctx, &declared).await;

    record_verdict(
        &**exec,
        recorder,
        feature_id,
        machine_str,
        repo_dir,
        resolved_cwd,
        &outcome,
    )
    .await;
    outcome
}

/// The half of a resolution that is the same whether an agent or a person does
/// the work: prove there is a merge here, settle what has to end up
/// marker-free, and say on the row that something is now working on it.
///
/// Shared rather than copied because the two callers of it drifted last time
/// they were not — only one recorded that a turn had started, which left the
/// other's resolution readable as a conflict nobody was touching, and offerable
/// to a second presser.
async fn open_resolution(
    exec: &dyn ExecutionPort,
    recorder: ResolutionRecorder<'_>,
    feature_id: &FeatureId,
    machine_str: &str,
    resolved_cwd: &str,
    ctx_declared: &[crate::domain::models::ConflictFile],
) -> Result<Vec<crate::domain::models::ConflictFile>, ResolveSyncError> {
    let pre_unmerged = match preflight(exec, machine_str, resolved_cwd).await {
        Ok(files) => files,
        // A verdict closes the session; an unreadable tree leaves it exactly as
        // it was, still naming the worktree, for whoever can reach the machine.
        Err(PreflightRefusal::Unreadable(why)) => return Err(ResolveSyncError::Failed(why)),
        Err(PreflightRefusal::NothingToResolve(why)) => {
            recorder
                .record(
                    feature_id,
                    &SyncResolution::Failed {
                        reason: why.clone(),
                    },
                )
                .await;
            return Err(ResolveSyncError::Failed(why));
        }
    };

    // Falls back to the tree's own answer for a row from before the column
    // carried a list, and for the empty seed a session opens with.
    let declared = if ctx_declared.is_empty() {
        pre_unmerged
    } else {
        ctx_declared.to_vec()
    };

    recorder.record(feature_id, &SyncResolution::Started).await;
    Ok(declared)
}

/// What the row is left saying, and what becomes of the worktree.
async fn record_verdict(
    exec: &dyn ExecutionPort,
    recorder: ResolutionRecorder<'_>,
    feature_id: &FeatureId,
    machine_str: &str,
    repo_dir: &str,
    resolved_cwd: &str,
    outcome: &Result<ResolvedSync, ResolveSyncError>,
) {
    let verdict = match outcome {
        // A published resolution has nothing left in the tree it was made in.
        // An unpublished one is the opposite: the branch it is committed on is
        // checked out there, so that tree is where the review's `Discard` puts
        // the branch back, and tearing it down here would leave the only way
        // out of the state a `reset` in the user's own clone against whatever
        // it happens to have checked out.
        Ok(resolved) if resolved.published => {
            discard_sync_worktree(exec, machine_str, repo_dir, resolved_cwd).await;
            SyncResolution::Succeeded {
                merge_commit_sha: resolved.merge_commit_sha.clone(),
                published: true,
                worktree_discarded: resolved_cwd == repo_dir
                    || crate::application::sync_session::worktree_confirmed_gone(
                        exec,
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
    recorder.record(feature_id, &verdict).await;
}

/// Everything the manual finish needs. No agent, and so none of what an agent
/// costs: no registry, no pricing, no spend to advance.
pub(crate) struct ContinueSyncContext<'a> {
    pub exec: &'a Arc<dyn ExecutionPort>,
    pub app_settings: &'a Arc<dyn AppSettingsRepository>,
    pub git_ops: &'a GitOpsHelper,
    pub merge_executor: &'a Arc<dyn MergeExecutor>,
    pub notif: &'a Arc<dyn NotificationPort>,
    pub feature_id: &'a FeatureId,
    pub repo_dir: &'a str,
    pub resolved_cwd: &'a str,
    pub machine_str: &'a str,
    pub feature_branch: &'a str,
    pub base_branch: &'a str,
    pub declared_conflicts: &'a [crate::domain::models::ConflictFile],
    pub gate: crate::ports::worktree_ops::MergeGate<'a>,
    pub review_before_push: Option<bool>,
    pub feature_status: &'a str,
    pub cancel: Option<watch::Receiver<bool>>,
}

/// Land a conflict a person resolved themselves.
///
/// The pane has told users to "finish it by hand in the worktree" since there
/// was a pane, and until this there was nothing that would then accept the
/// result: the only ways out of a conflict were an agent or abandoning the
/// sync. A resolution six hunks from done had no press that would take it.
///
/// It is `resolve_sync_conflicts` with the turn removed, and deliberately not a
/// second opinion about anything else. The same preflight decides there is a
/// merge, the same scan decides whether it is finished, the same harness gate
/// decides whether the tree may be committed, and the same [`land`] commits and
/// publishes it — so a hand-resolved sync and an agent-resolved one reach
/// origin identically, and the row cannot tell you which it was.
pub(crate) async fn continue_sync_resolution(
    ctx: ContinueSyncContext<'_>,
) -> Result<ResolvedSync, ResolveSyncError> {
    let ContinueSyncContext {
        exec,
        app_settings,
        git_ops,
        merge_executor,
        notif,
        feature_id,
        repo_dir,
        resolved_cwd,
        machine_str,
        feature_branch,
        base_branch,
        declared_conflicts,
        gate,
        review_before_push,
        feature_status,
        cancel,
    } = ctx;

    let recorder = ResolutionRecorder {
        merge_executor,
        notif,
    };
    let declared = open_resolution(
        &**exec,
        recorder,
        feature_id,
        machine_str,
        resolved_cwd,
        declared_conflicts,
    )
    .await?;

    let outcome = verify_and_land_by_hand(
        exec,
        app_settings,
        git_ops,
        machine_str,
        resolved_cwd,
        feature_branch,
        base_branch,
        &declared,
        gate,
        publish_policy(review_before_push, resolution_is_reviewable(feature_status)),
        cancel,
    )
    .await;

    record_verdict(
        &**exec,
        recorder,
        feature_id,
        machine_str,
        repo_dir,
        resolved_cwd,
        &outcome,
    )
    .await;
    outcome
}

#[allow(clippy::too_many_arguments)]
async fn verify_and_land_by_hand(
    exec: &Arc<dyn ExecutionPort>,
    app_settings: &Arc<dyn AppSettingsRepository>,
    git_ops: &GitOpsHelper,
    machine_str: &str,
    resolved_cwd: &str,
    feature_branch: &str,
    base_branch: &str,
    declared: &[crate::domain::models::ConflictFile],
    gate: crate::ports::worktree_ops::MergeGate<'_>,
    publish: ResolutionPublish,
    cancel: Option<watch::Receiver<bool>>,
) -> Result<ResolvedSync, ResolveSyncError> {
    if cancelled(&cancel) {
        return Err(ResolveSyncError::Cancelled(CANCELLED_REASON.to_string()));
    }

    let left = unresolved_files(&**exec, machine_str, resolved_cwd, declared)
        .await
        .map_err(ResolveSyncError::Failed)?;
    if !left.is_empty() {
        return Err(ResolveSyncError::Failed(format!(
            "{} Finish them in the sync worktree and press Continue again, or hand the rest to an agent.",
            remaining_conflicts_refusal(declared.len(), &left)
        )));
    }

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

    if let Verification::Gated { command } = verification {
        match run_gate_harness(
            &**exec,
            machine_str,
            Some(command),
            gate_opts,
            cancel.clone(),
        )
        .await
        {
            GateVerdict::NotGated | GateVerdict::Passed | GateVerdict::Unprepared { .. } => {}
            GateVerdict::Stopped => {
                return Err(ResolveSyncError::Cancelled(CANCELLED_REASON.to_string()))
            }
            // No repair round, unlike the agent path: the only thing that could
            // act on the harness's output is the person who just said they were
            // finished, and they are better told what it said than handed a
            // turn they did not ask for.
            GateVerdict::Failed { error } => {
                match resolution_verification_refusal(command, &error) {
                    Some(refusal) => return Err(ResolveSyncError::Failed(refusal)),
                    None => tracing::warn!(
                        machine = %machine_str,
                        worktree = %resolved_cwd,
                        command = %command,
                        error = %error,
                        "sync gate reached no verdict: the resolution lands unverified",
                    ),
                }
            }
        }
    }

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
        None,
    )
    .await?;

    Ok(ResolvedSync {
        merge_commit_sha,
        published,
        cache: CacheTokens::default(),
    })
}

/// Has a stop arrived? Read before the spawn and again after the turn, because
/// a cancel that lands while git is mid-flight otherwise surfaces as an
/// ordinary failure and the row records the wrong thing about who ended it.
fn cancelled(cancel: &Option<watch::Receiver<bool>>) -> bool {
    cancel.as_ref().is_some_and(|rx| *rx.borrow())
}

async fn run_resolver_turn(
    sync_ctx: ResolveSyncContext<'_>,
    // The persisted conflict list — what this resolution must prove
    // marker-free. See the call site for why it is not the index's answer.
    declared: &[crate::domain::models::ConflictFile],
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
    let mut resolution_rounds = 0u32;
    // What is still conflicted *right now*, which is neither what the caller
    // named nor what the index says. A round is judged against this, and the
    // next round's prompt is built from it — see `declared` at the call site
    // for why the two lists must not be the same list.
    let mut remaining = unresolved_files(&**exec, machine_str, resolved_cwd, declared)
        .await
        .map_err(ResolveSyncError::Failed)?;
    let resolution_start_cost = *accumulated_cost;
    // What a red gate is worth is `domain::sync_session::gate_follow_up`. Held
    // as the built prompt rather than the words it was built from, so the
    // harness's output cannot outlive the loop that read it.
    let mut repair_prompt: Option<String> = None;
    let turn_stop = loop {
        if cancelled(&cancel) {
            return Err(ResolveSyncError::Cancelled(CANCELLED_REASON.to_string()));
        }

        // Nothing carries a marker and no harness asked for a repair, so there
        // is no work an agent could do. Whoever cleared them — an earlier round,
        // or a person in the sync worktree — already finished; a turn spawned to
        // confirm that costs money and can only make the tree worse. Fall
        // through to the gate and the landing on the same terms a turn would
        // have reached them on.
        if remaining.is_empty() && repair_prompt.is_none() {
            break None;
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
                // What still has markers, not what the merge originally named.
                // A round handed the whole original list re-reads every file an
                // earlier round already finished — sixteen tool calls before the
                // first edit, on the conflict this loop was written for — and
                // then trips its turn cap somewhere it has been before.
                let work: Vec<String> = remaining.iter().map(|(path, _)| path.clone()).collect();
                build_resolver_prompt(feature_branch, incoming, &work, verification, &base_moves)
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

        // Reaped before the tree is read, not after: the turn is over, and
        // holding a live process — over SSH, its channel too — across a read of
        // every conflicted file and the multi-minute build after it is waste.
        let _ = registry.kill(&resolver_thread_id).await;

        // The agent's worktree fence deliberately excludes the linked-worktree
        // index. Demeteo owns staging and committing after the agent resolves
        // the conflicted content.
        //
        // Read over `declared`, and never over the index's live answer: clearing
        // a marker in the working tree leaves the path `UU` until something
        // stages it, so an index-derived list can neither see that a file was
        // finished nor — once `git add -A` has run — that one was not.
        let left = match unresolved_files(&**exec, machine_str, resolved_cwd, declared).await {
            Ok(left) => left,
            Err(why) => return Err(ResolveSyncError::Failed(why)),
        };
        if !left.is_empty() {
            let before: usize = remaining.iter().map(|(_, hunks)| hunks).sum();
            let now: usize = left.iter().map(|(_, hunks)| hunks).sum();
            let standing = remaining_conflicts_refusal(declared.len(), &left);
            match resolution_follow_up(now, before, resolution_rounds) {
                // The round moved the tree, so the next one starts from less
                // work than this one did. That is the whole of the loop: a turn
                // cap tripped mid-resolution is a stop, not a verdict, and the
                // press that hit one used to have to be repeated by hand — each
                // repetition re-reading the files the last one had finished.
                ResolutionFollowUp::Continue => {
                    resolution_rounds += 1;
                    remaining = left;
                    continue;
                }
                ResolutionFollowUp::Refuse(why) => {
                    return Err(ResolveSyncError::Failed(resolution_refusal(
                        turn_stop.as_deref(),
                        &format!("{} {}", standing, why),
                    )));
                }
            }
        }
        remaining = left;

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

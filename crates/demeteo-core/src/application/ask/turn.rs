//! One turn of an Ask conversation.
//!
//! Mirrors [`crate::application::discovery::turn`] end to end minus
//! attachments, decompose, and reseed-visible-UI fields — Ask has none of
//! those. Runs through [`stream_agent_turn`] for the same reason Discovery
//! does: it is what reports cost and tokens, which a turn has to fold onto
//! its own thread (§8.5 of `docs/PRD_DISCOVERY.md`'s Ask-adjacent counterpart
//! is `docs/ask-canvas/`).
//!
//! A canvas' path-bearing nodes are stat'd against the mounted worktree
//! between [`parse_ask_turn`] and [`persist_assistant`] — synchronously, in
//! the same call stack that later becomes eligible for idle reclaim, since a
//! hallucinated path is otherwise indistinguishable from a real one until
//! someone clicks it. [`verify_canvas_paths`] is that check.

use std::sync::Arc;

use super::events::{
    status_payload, AskTurnCompleted, Sink, EVENT_ASK_TURN_COMPLETED, EVENT_ASK_TURN_STATUS,
    STATUS_ERROR, STATUS_IDLE, STATUS_RUNNING, STATUS_SETTING_UP,
};
use super::running::{RunningTurn, ALREADY_RUNNING};
use crate::adapters::agent::event_stream::turn::{stream_agent_turn, TurnOutcome, TurnResult};
use crate::application::turn_retry::{should_reseed_and_retry, TurnEnding};
use crate::domain::ask_canvas::{parse_ask_turn, AskCanvas, AskTurn};
use crate::domain::ids::AskThreadId;
use crate::domain::models::{
    AskMessage, AskStatus, AskThread, CanvasPathVerdict, MessageRole, TurnActivity,
};
use crate::domain::permission::{Access, PermissionProfile};
use crate::ports::agent_runtime::AgentContext;
use crate::ports::ask::AskThreadPatch;
use crate::state::AppContext;

/// The outcome of a turn that never ran, or of the one ending that carries no
/// usage by construction.
pub(super) fn nothing_spent() -> TurnOutcome {
    TurnOutcome {
        text: String::new(),
        produced_artifacts: Vec::new(),
        cost_usd: 0.0,
        tokens: 0,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    }
}

pub(super) fn split(result: TurnResult) -> (TurnEnding, Option<String>, TurnOutcome) {
    match result {
        TurnResult::Success(spent) => (TurnEnding::Success, None, spent),
        TurnResult::Failed { reason, spent } => (TurnEnding::Failed, Some(reason), spent),
        TurnResult::Environmental { reason, spent } => {
            (TurnEnding::Environmental, Some(reason), spent)
        }
        TurnResult::Interrupted => (TurnEnding::Interrupted, None, nothing_spent()),
    }
}

/// Ask's posture: read the repository, run commands, reach the network when
/// the thread's own `network` field allows it; write nothing.
///
/// A `PermissionProfile` literal, never `StepCapability::base_profile()` or
/// any existing capability variant — matching
/// [`discovery::turn::interviewer_permissions`](crate::application::discovery::turn)
/// exactly, per AGENTS.md §3's rule against a new capability variant for a
/// case a literal already covers. The write stop is the artifact fence
/// [`super::worktree::ensure`] applies, and §4.6 of `docs/PRD_DISCOVERY.md`
/// is explicit that what it buys is intent rather than a platform guarantee.
fn ask_permissions(network: bool) -> PermissionProfile {
    PermissionProfile {
        read_fs: Access::Allow,
        write_fs: Access::Deny,
        execute: Access::Allow,
        network: if network { Access::Allow } else { Access::Deny },
    }
}

/// The registry key one Ask thread's session is held under. A distinct
/// prefix from [`discovery::turn::thread_id`](crate::application::discovery::turn)'s
/// `discovery-` so the two never collide inside the one `AgentRegistry`.
pub(crate) fn thread_id(id: &AskThreadId) -> String {
    format!("ask-{}", id.as_str())
}

/// Record the user's turn, then set it up and run it in the background.
///
/// Returns as soon as the user's message is persisted — setting the turn up
/// is not awaited here, on the same reasoning as
/// [`discovery::turn::send`](crate::application::discovery::turn): worktree
/// provisioning (`super::worktree::ensure`) can take minutes on a thread
/// whose tree was reclaimed by the idle sweep, and the bubble the user just
/// typed would not render until it finished.
pub async fn send<F>(
    ctx: &AppContext,
    thread_id: &AskThreadId,
    text: &str,
    emit_fn: F,
) -> Result<AskMessage, String>
where
    F: Fn(&str, serde_json::Value) + Send + Sync + 'static,
{
    let thread = ctx
        .ask
        .get(thread_id)?
        .ok_or_else(|| format!("Ask thread not found: {}", thread_id.as_str()))?;
    if thread.status != AskStatus::Open {
        return Err("This Ask thread is closed.".into());
    }
    if text.trim().is_empty() {
        return Err("A turn needs something to say.".into());
    }
    // Before the message is stored, not after: a turn this thread is refused
    // must leave no bubble behind to answer.
    let claim = ctx
        .ask_turns
        .clone()
        .try_claim(thread.id.as_str())
        .ok_or_else(|| ALREADY_RUNNING.to_string())?;

    let now = crate::paths::now_ms();
    let user_message = AskMessage {
        id: crate::shared::ids::new_id(),
        thread_id: thread.id.clone(),
        role: MessageRole::User,
        text: text.to_string(),
        cost_usd: None,
        tokens: None,
        turn_activity: None,
        canvas_paths: None,
        checked_commit_sha: None,
        created_at: now,
    };
    ctx.ask.append_message(&user_message)?;
    ctx.ask
        .update(&thread.id, &AskThreadPatch::default(), now)?;

    tokio::spawn(set_up_and_run(
        ctx.clone(),
        thread,
        user_message.clone(),
        claim,
        Arc::new(emit_fn),
    ));
    Ok(user_message)
}

/// The half of a turn that no longer has a caller to report to.
async fn set_up_and_run<F>(
    ctx: AppContext,
    thread: AskThread,
    asked: AskMessage,
    claim: RunningTurn,
    emit: Arc<F>,
) where
    F: Fn(&str, serde_json::Value) + Send + Sync + 'static,
{
    let Ok((prepared, running)) = announced(
        emit.as_ref(),
        &thread,
        claim,
        prepare(&ctx, &thread, &asked),
    )
    .await
    else {
        return;
    };
    emit(
        EVENT_ASK_TURN_STATUS,
        status_payload(&thread, STATUS_RUNNING, None),
    );
    run(prepared, running, emit).await;
}

/// Announce a turn, then let `preparing` run — in that order, mirroring
/// [`discovery::turn::announced`](crate::application::discovery::turn), whose
/// doc comment carries the reasoning for the ordering and for releasing the
/// claim before the failure event and never after.
async fn announced<F, T>(
    emit: &F,
    thread: &AskThread,
    claim: RunningTurn,
    preparing: impl std::future::Future<Output = Result<T, String>>,
) -> Result<(T, RunningTurn), String>
where
    F: Fn(&str, serde_json::Value),
{
    emit(
        EVENT_ASK_TURN_STATUS,
        status_payload(thread, STATUS_SETTING_UP, None),
    );
    match preparing.await {
        Ok(prepared) => Ok((prepared, claim)),
        Err(reason) => {
            drop(claim);
            emit(
                EVENT_ASK_TURN_STATUS,
                status_payload(thread, STATUS_ERROR, Some(reason.clone())),
            );
            Err(reason)
        }
    }
}

/// Everything the background turn needs, resolved while a caller is still
/// there to be told it failed.
struct Prepared {
    /// Held only for [`super::worktree::commit_sha`], which takes the whole
    /// context on the same terms [`super::worktree::ensure`] does — every
    /// other field on `Prepared` is pulled out of it individually because
    /// this ticket is the first caller that needs the sha helper's own
    /// signature rather than one port at a time.
    ctx: AppContext,
    ctx_ask: Arc<dyn crate::ports::ask::AskPort>,
    registry: Arc<crate::adapters::agent::registry::AgentRegistry>,
    exec: Arc<dyn crate::ports::execution::ExecutionPort>,
    pricing: Arc<dyn crate::ports::pricing::PricingTable>,
    timeouts: crate::domain::models::AgentTimeouts,
    thread: AskThread,
    agent_ctx: AgentContext,
    machine_str: String,
    thread_id: String,
    /// Live before this turn asked for it — the same "the harness still
    /// knows this session" observation
    /// [`discovery::turn::Prepared::session_was_live`](crate::application::discovery::turn)
    /// documents.
    session_was_live: bool,
    transcript: Vec<AskMessage>,
    context_text: String,
    user_text: String,
    pricing_model: Option<String>,
}

impl Prepared {
    fn render_prompt(&self, reseed: bool) -> String {
        super::question::render_turn_prompt(super::question::TurnPrompt {
            reseed,
            context: &self.context_text,
            transcript: &self.transcript,
            user_text: &self.user_text,
        })
    }
}

/// Resolve everything a turn against this Ask thread needs.
///
/// `asked` is the user message this turn is answering. It is excluded from
/// `transcript` because a re-seeded prompt renders the transcript and then
/// the new text, so leaving it in would ask the same question twice.
async fn prepare(
    ctx: &AppContext,
    thread: &AskThread,
    asked: &AskMessage,
) -> Result<Prepared, String> {
    let repo = super::worktree::resolve(ctx, thread).await?;
    let worktree_path = super::worktree::ensure(ctx, thread, &repo).await?;

    let features = ctx.features.get_active(&thread.project_id)?;
    let context_text = crate::application::turn_retry::render_project_context(&features);

    let transcript: Vec<AskMessage> = ctx
        .ask
        .list_messages(&thread.id)?
        .into_iter()
        .filter(|m| m.id != asked.id)
        .collect();

    let registry_thread_id = thread_id(&thread.id);
    let session_was_live = ctx
        .registry
        .session_handle_any(&registry_thread_id)
        .await
        .is_some();
    let binary = ctx
        .registry
        .runtime_for(&thread.agent_kind)
        .map(|r| r.binary().to_string())
        .unwrap_or_else(|| thread.agent_kind.clone());
    let env =
        crate::ports::agent_runtime::agent_base_env(ctx.exec.as_ref(), &repo.machine_str).await;
    let platform =
        crate::ports::agent_runtime::resolve_agent_platform(ctx.exec.as_ref(), &repo.machine_str)
            .await;

    Ok(Prepared {
        ctx: ctx.clone(),
        ctx_ask: ctx.ask.clone(),
        registry: ctx.registry.clone(),
        exec: ctx.exec.clone(),
        pricing: ctx.pricing.clone(),
        timeouts: crate::application::timeouts::resolve_effective(ctx.app_settings.as_ref()),
        agent_ctx: AgentContext {
            thread_id: registry_thread_id.clone(),
            machine_id: repo.machine_str.clone(),
            binary,
            args: vec![],
            env,
            cwd: worktree_path,
            model: thread.model.clone(),
            effort: thread.effort,
            title: Some(thread.title.clone()),
            platform,
            agent_exec: ctx.agent_exec.clone(),
            exec: ctx.exec.clone(),
            permissions: ask_permissions(thread.network),
            bare_mode: true,
            keep_harness_personalization: crate::domain::turn_role::TurnRole::Orchestrator
                .keeps_harness_personalization(),
            tool_allowlist: None,
            max_turns: None,
            max_budget_usd: None,
        },
        machine_str: repo.machine_str,
        thread_id: registry_thread_id,
        session_was_live,
        transcript,
        context_text,
        user_text: asked.text.clone(),
        pricing_model: thread
            .model
            .clone()
            .or_else(|| ctx.registry.default_model_for(&thread.agent_kind)),
        thread: thread.clone(),
    })
}

async fn run<F>(p: Prepared, running: RunningTurn, emit: Arc<F>)
where
    F: Fn(&str, serde_json::Value) + Send + Sync + 'static,
{
    let started = std::time::Instant::now();
    let mut resumed = p.session_was_live;
    let mut attempts = 0;

    let (ending, reason, spent, activity) = loop {
        attempts += 1;
        let prompt = p.render_prompt(!resumed);

        let session = match p
            .registry
            .get_or_spawn(&p.thread_id, &p.thread.agent_kind, p.agent_ctx.clone())
            .await
        {
            Ok(s) => s,
            Err(e) => {
                break (
                    TurnEnding::Environmental,
                    Some(format!("Could not start {}: {e}", p.thread.agent_kind)),
                    nothing_spent(),
                    TurnActivity::default(),
                );
            }
        };

        let sink = Sink::new(emit.clone(), p.thread.id.as_str().to_string());
        let mut activity = TurnActivity::default();
        let result = stream_agent_turn(
            session.as_ref(),
            &prompt,
            p.timeouts,
            None,
            &p.machine_str,
            p.exec.as_ref(),
            p.pricing_model.clone(),
            p.pricing.clone(),
            |event| {
                activity.observe(event);
                sink.push(event);
            },
        )
        .await;
        sink.flush();

        let (ending, reason, spent) = split(result);
        bill(&p, &spent);
        latch_session_id(&p, session.as_ref());

        if attempts == 1 && should_reseed_and_retry(resumed, !spent.text.trim().is_empty(), ending)
        {
            p.registry.kill(&p.thread_id).await;
            resumed = false;
            continue;
        }
        break (ending, reason, spent, activity);
    };

    let parsed = parse_ask_turn(&spent.text);
    let (canvas_paths, checked_commit_sha) = match &parsed.canvas {
        Some(canvas) => verify_canvas_paths(&p, canvas).await,
        None => (None, None),
    };
    let message_id = persist_assistant(
        &p,
        &spent,
        &parsed,
        activity,
        canvas_paths,
        checked_commit_sha,
    );

    // Before the events and after the message is stored, in that order — the
    // same ordering `discovery::turn::run` documents: a claim outliving the
    // event it answers would leave a refreshing surface waiting for a turn
    // that has already said everything it had to say.
    drop(running);
    emit(
        EVENT_ASK_TURN_COMPLETED,
        serde_json::to_value(AskTurnCompleted {
            thread_id: p.thread.id.as_str().to_string(),
            title: p.thread.title.clone(),
            message_id,
            ending: ending.as_str(),
            reason: reason.clone(),
            cost_usd: spent.cost_usd,
            tokens: spent.tokens,
            duration_ms: started.elapsed().as_millis() as u64,
        })
        .unwrap_or(serde_json::Value::Null),
    );
    emit(
        EVENT_ASK_TURN_STATUS,
        status_payload(
            &p.thread,
            if ending == TurnEnding::Success {
                STATUS_IDLE
            } else {
                STATUS_ERROR
            },
            reason,
        ),
    );
}

/// Fold what the turn spent onto the Ask thread, whatever it spent it on.
///
/// Mirrors [`discovery::turn::bill`](crate::application::discovery::turn)'s
/// zero-skip convention exactly: a turn that produced neither cost nor
/// tokens does not call `update` for billing. `add_turns` rides along with
/// `add_cost_usd`/`add_tokens` under the same skip, rather than as a
/// separate always-called increment — a turn worth counting is a turn that
/// spent something.
fn bill(p: &Prepared, spent: &TurnOutcome) {
    if spent.cost_usd == 0.0 && spent.tokens == 0 {
        return;
    }
    if let Err(e) = p.ctx_ask.update(
        &p.thread.id,
        &AskThreadPatch {
            add_cost_usd: spent.cost_usd,
            add_tokens: spent.tokens,
            add_turns: 1,
            ..Default::default()
        },
        crate::paths::now_ms(),
    ) {
        tracing::warn!(ask_thread = %p.thread.id.as_str(), error = %e, "ask: could not fold turn spend");
    }
}

/// Store the harness's own name for this session, once it has one. Mirrors
/// [`discovery::turn::latch_resume_id`](crate::application::discovery::turn)
/// exactly, including its caveat that the stored id is informational — it is
/// never handed back to a fresh process, since that would mean changing
/// agent spawn logic (AGENTS.md §2).
fn latch_session_id(p: &Prepared, session: &dyn crate::ports::agent_runtime::AgentSession) {
    let Some(latched) = session.harness_session_id() else {
        return;
    };
    if p.thread.session_id.as_deref() == Some(latched.as_str()) {
        return;
    }
    let _ = p.ctx_ask.update(
        &p.thread.id,
        &AskThreadPatch {
            session_id: Some(Some(latched)),
            ..Default::default()
        },
        crate::paths::now_ms(),
    );
}

/// Stat every path-bearing node of a parsed, validated canvas against the
/// mounted worktree, then record the commit it was checked at.
///
/// Each node's `path` is relative to the repository root — the same
/// convention [`crate::domain::ask_canvas::CanvasNode::path`]'s doc comment
/// names (`git_ops::scope`-style module names aside, a real path a node
/// cites is one the agent read inside its checkout) — so it is joined onto
/// `p.agent_ctx.cwd` (the mounted worktree) before the stat, exactly as the
/// research recorded: "joined against the mounted worktree path". A `path`
/// that is absolute, or whose `..` segments lexically walk the join back out
/// of `p.agent_ctx.cwd`, names something the canvas has no business
/// describing — see [`super::path_containment::resolve_within_root`] — and is
/// recorded unresolved without ever reaching
/// [`crate::ports::execution::ExecutionPort::get_metadata`].
///
/// Runs synchronously in `run`'s own call stack, before that turn's worktree
/// becomes eligible for reclaim — never deferred to a background task, and
/// never re-run lazily when a node is later read or clicked, per the spec's
/// explicit rejection of lazy resolution (a hallucinated path must be
/// indistinguishable from a real one only until the message is persisted).
///
/// A node with `path: None` contributes no entry — there is nothing to stat.
/// When no node in the canvas carries a path, both return values are `None`,
/// matching [`AskMessage::canvas_paths`]'s "nothing worth keeping" convention
/// and skipping the `commit_sha` call entirely.
async fn verify_canvas_paths(
    p: &Prepared,
    canvas: &AskCanvas,
) -> (Option<Vec<CanvasPathVerdict>>, Option<String>) {
    let mut verdicts = Vec::new();
    for node in &canvas.nodes {
        let Some(path) = node.path.as_deref() else {
            continue;
        };
        let resolved = match super::path_containment::resolve_within_root(&p.agent_ctx.cwd, path) {
            Some(full_path) => {
                let full_path = full_path.to_string_lossy().to_string();
                p.exec
                    .get_metadata(&p.machine_str, &full_path)
                    .await
                    .is_ok()
            }
            None => false,
        };
        verdicts.push(CanvasPathVerdict {
            node_id: node.id.clone(),
            path: path.to_string(),
            resolved,
        });
    }
    if verdicts.is_empty() {
        return (None, None);
    }
    let sha = super::worktree::commit_sha(&p.ctx, &p.machine_str, &p.agent_ctx.cwd)
        .await
        .ok();
    (Some(verdicts), sha)
}

/// Persist what Ask said, verbatim — the whole turn, canvas block included,
/// not the prose alone, on the same reasoning
/// [`discovery::turn::persist_assistant`](crate::application::discovery::turn)
/// documents: the stored text is what a re-seeded turn replays.
///
/// `canvas_paths`/`checked_commit_sha` are whatever [`verify_canvas_paths`]
/// produced for this turn's parsed canvas — `None` for a canvas-free turn, a
/// turn whose canvas failed `validate_canvas`, or a canvas whose nodes all
/// have `path: None`.
fn persist_assistant(
    p: &Prepared,
    spent: &TurnOutcome,
    parsed: &AskTurn,
    activity: TurnActivity,
    canvas_paths: Option<Vec<CanvasPathVerdict>>,
    checked_commit_sha: Option<String>,
) -> Option<String> {
    if parsed.prose.trim().is_empty() && spent.text.trim().is_empty() {
        return None;
    }
    let message = AskMessage {
        id: crate::shared::ids::new_id(),
        thread_id: p.thread.id.clone(),
        role: MessageRole::Assistant,
        text: spent.text.clone(),
        cost_usd: Some(spent.cost_usd),
        tokens: Some(spent.tokens),
        turn_activity: (!activity.is_empty()).then_some(activity),
        canvas_paths,
        checked_commit_sha,
        created_at: crate::paths::now_ms(),
    };
    match p.ctx_ask.append_message(&message) {
        Ok(()) => Some(message.id),
        Err(e) => {
            tracing::error!(ask_thread = %p.thread.id.as_str(), error = %e, "ask: could not store the turn");
            None
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/application/ask/turn.rs"]
mod tests;

//! One turn of the interview (§4.2, §4.4 of `docs/PRD_DISCOVERY.md`).
//!
//! A turn is a fresh one-shot CLI invocation, which is what keeps AGENTS.md
//! §2's one-shot-only invariant intact while the surface reads as a
//! conversation. It runs through [`stream_agent_turn`], not a hand-rolled
//! stream loop: the loop in `crates/demeteo-core/src/application/agents.rs`
//! predates it and reports neither cost nor tokens, which a Discovery has to
//! fold onto its own row (§8.5).

use std::sync::Arc;

use super::events::{
    status_payload, DiscoveryTurnCompleted, Sink, TurnEnding, EVENT_DISCOVERY_TURN_COMPLETED,
    EVENT_DISCOVERY_TURN_STATUS,
};
use crate::adapters::agent::event_stream::turn::{stream_agent_turn, TurnOutcome, TurnResult};
use crate::domain::attachment::AttachedFile;
use crate::domain::discovery_question::parse_interview_turn;
use crate::domain::ids::DiscoveryId;
use crate::domain::models::{
    Discovery, DiscoveryMessage, DiscoveryStatus, MessageRole, TurnActivity,
};
use crate::domain::permission::{Access, PermissionProfile};
use crate::ports::agent_runtime::AgentContext;
use crate::ports::discovery::DiscoveryPatch;
use crate::state::AppContext;

/// Whether a turn should be run again with the transcript carried in the
/// prompt, because the evidence says the harness no longer knows the session.
///
/// **No harness reports a lost session distinguishably.** A `claude --resume`
/// against an id its store has pruned exits with an error like any other
/// error; codex, opencode and hermes do the same. Demeteo sees a `Failed` or
/// `Environmental` ending with a message it has no grammar for, and matching
/// on that message would be matching on another product's copy — it changes
/// on their release schedule and nothing here would fail when it did.
///
/// So the discriminator is evidential rather than textual, and it is the one
/// piece of evidence that means something: **a turn that produced no assistant
/// text never reached the model.** A resumed turn that answered and then fell
/// over plainly resolved its session; the failure is the agent's and re-asking
/// would only repeat it. A resumed turn that emitted nothing is either a lost
/// session or a failure so early that re-seeding repeats it once — which is
/// the conservative side to be wrong on, because the alternative leaves a
/// Discovery permanently unable to take another turn, in exactly the
/// came-back-a-week-later case §4.4 exists for.
///
/// What it costs when it is wrong is one extra turn, billed. What it costs to
/// omit is the Discovery. `resumed` is what bounds it: a turn that already
/// carried the transcript has nothing left to fall back to, so it is never
/// retried and the loop cannot run more than twice.
pub(crate) fn should_reseed_and_retry(
    resumed: bool,
    produced_text: bool,
    ending: TurnEnding,
) -> bool {
    resumed && !produced_text && matches!(ending, TurnEnding::Failed | TurnEnding::Environmental)
}

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

/// The interviewer's posture: read the repository, run commands, reach the
/// network; write nothing.
///
/// A `PermissionProfile` literal rather than a new `StepCapability` variant,
/// which is what keeps this off AGENTS.md §6: no existing spawn path changes
/// and `opencode_permission_json` is untouched. The write stop is the artifact
/// fence applied by `crate::application::discovery::worktree`, and §4.6 is
/// explicit that what it buys is intent rather than a platform guarantee.
fn interviewer_permissions() -> PermissionProfile {
    PermissionProfile {
        read_fs: Access::Allow,
        write_fs: Access::Deny,
        execute: Access::Allow,
        network: Access::Allow,
    }
}

/// The registry key one Discovery's session is held under.
pub(crate) fn thread_id(id: &DiscoveryId) -> String {
    format!("discovery-{}", id.as_str())
}

/// Record the user's turn, then run the interviewer's in the background.
///
/// Returns as soon as the user's message is persisted, so the surface can
/// render it while the answer streams. Everything after that reaches the
/// frontend through the three events above.
pub async fn send<F>(
    ctx: &AppContext,
    discovery_id: &DiscoveryId,
    text: String,
    emit_fn: F,
) -> Result<DiscoveryMessage, String>
where
    F: Fn(&str, serde_json::Value) + Send + Sync + 'static,
{
    let discovery = ctx
        .discoveries
        .get(discovery_id)?
        .ok_or_else(|| format!("Discovery not found: {}", discovery_id.as_str()))?;
    if discovery.status != DiscoveryStatus::Open {
        return Err("This discovery is closed. Reopen it to keep interviewing.".into());
    }
    if text.trim().is_empty() {
        return Err("A turn needs something to say.".into());
    }

    let now = crate::paths::now_ms();
    let user_message = DiscoveryMessage {
        id: crate::shared::ids::new_id(),
        discovery_id: discovery.id.clone(),
        role: MessageRole::User,
        content: text.clone(),
        cost_usd: None,
        tokens: None,
        activity: None,
        created_at: now,
    };
    ctx.discoveries.append_message(&user_message)?;
    ctx.discoveries
        .update(&discovery.id, &DiscoveryPatch::default(), now)?;

    let prepared = prepare(ctx, &discovery, Some(&user_message)).await?;
    let emit = Arc::new(emit_fn);
    emit(
        EVENT_DISCOVERY_TURN_STATUS,
        status_payload(&discovery, "running", None),
    );

    tokio::spawn(run(prepared, emit));
    Ok(user_message)
}

/// Everything the background turn needs, resolved while a caller is still
/// there to be told it failed.
///
/// Shared with [`super::decompose`], which is one more turn against the same
/// session with a different prompt: it swaps [`Prepared::user_text`] and
/// otherwise runs the machinery below unchanged.
pub(super) struct Prepared {
    pub(super) ctx_discoveries: Arc<dyn crate::ports::discovery::DiscoveryPort>,
    /// Where the spawned turn says it is running. It is claimed inside
    /// [`run`] rather than here because preparing is not running, and a
    /// caller that failed to prepare never took a turn at all.
    pub(super) running: Arc<super::running::RunningTurns>,
    pub(super) registry: Arc<crate::adapters::agent::registry::AgentRegistry>,
    pub(super) exec: Arc<dyn crate::ports::execution::ExecutionPort>,
    pub(super) pricing: Arc<dyn crate::ports::pricing::PricingTable>,
    pub(super) timeouts: crate::domain::models::AgentTimeouts,
    pub(super) discovery: Discovery,
    pub(super) agent_ctx: AgentContext,
    pub(super) machine_str: String,
    pub(super) thread_id: String,
    /// Live before this turn asked for it, which is the whole of what "the
    /// harness still knows this session" can be observed to mean from here.
    pub(super) session_was_live: bool,
    /// Everything said before this turn — see [`prepare`] for what is left
    /// out of it.
    pub(super) transcript: Vec<DiscoveryMessage>,
    pub(super) context_text: String,
    /// The manifest as it stood when the turn was prepared, and the store the
    /// bytes are read back out of.
    pub(super) attachments: Vec<AttachedFile>,
    pub(super) attachment_store: Arc<dyn crate::ports::attachment_store::AttachmentStore>,
    /// The `_context` directory inside the worktree the files were copied to,
    /// or `None` when the copy did not land — in which case the prompt names
    /// the host-local store instead, which is a path the agent may not be able
    /// to open but is at least a path that exists.
    pub(super) attachment_context_dir: Option<String>,
    pub(super) reads_images: bool,
    pub(super) user_text: String,
    /// What the usage accumulator prices against when the harness reports no
    /// dollar figure of its own. The runtime's default stands in for a
    /// Discovery that named no model, which is what the run actually used.
    pub(super) pricing_model: Option<String>,
}

impl Prepared {
    /// The whole text one turn is sent.
    ///
    /// The two passes are one call because the second only means anything
    /// after the first: [`super::question::render_turn_prompt`] writes the
    /// `[attachment -- <name>]` placeholders, and the resolver — the same one
    /// a step's prompt goes through — turns each into the path manifest that
    /// makes the file openable. Rendering without resolving hands the agent a
    /// filename and no file.
    pub(super) fn render_prompt(&self, reseed: bool) -> String {
        let prompt = super::question::render_turn_prompt(super::question::TurnPrompt {
            reseed,
            context: &self.context_text,
            transcript: &self.transcript,
            attachments: &self.attachments,
            reads_images: self.reads_images,
            user_text: &self.user_text,
        });
        crate::adapters::step_executor::artifacts::resolve_attached_user_attachments(
            &prompt,
            self.discovery.id.as_str(),
            &self.attachments,
            self.attachment_store.as_ref(),
            self.attachment_context_dir.as_deref(),
        )
    }
}

/// Resolve everything a turn against this Discovery needs.
///
/// `asked` is the user message this turn is answering, and `None` for a turn
/// nobody typed — a decompose pass. It is excluded from `transcript` because a
/// re-seeded prompt renders the transcript and then the new text, so leaving
/// it in would ask the same question twice.
pub(super) async fn prepare(
    ctx: &AppContext,
    discovery: &Discovery,
    asked: Option<&DiscoveryMessage>,
) -> Result<Prepared, String> {
    let repo = super::worktree::resolve(ctx, discovery).await?;
    let worktree_path = super::worktree::ensure(ctx, discovery, &repo).await?;
    let attachment_context_dir =
        super::attachments::materialize(ctx, discovery, &worktree_path, &repo.machine_str).await;
    let context_text = super::context::render(ctx, discovery).await?;
    let transcript: Vec<DiscoveryMessage> = ctx
        .discoveries
        .list_messages(&discovery.id)?
        .into_iter()
        .filter(|m| Some(m.id.as_str()) != asked.map(|a| a.id.as_str()))
        .collect();

    let thread_id = thread_id(&discovery.id);
    let session_was_live = ctx.registry.session_handle_any(&thread_id).await.is_some();
    let binary = ctx
        .registry
        .runtime_for(&discovery.agent_kind)
        .map(|r| r.binary().to_string())
        .unwrap_or_else(|| discovery.agent_kind.clone());
    let env =
        crate::ports::agent_runtime::agent_base_env(ctx.exec.as_ref(), &repo.machine_str).await;
    let platform =
        crate::ports::agent_runtime::resolve_agent_platform(ctx.exec.as_ref(), &repo.machine_str)
            .await;

    Ok(Prepared {
        ctx_discoveries: ctx.discoveries.clone(),
        running: ctx.discovery_turns.clone(),
        registry: ctx.registry.clone(),
        exec: ctx.exec.clone(),
        pricing: ctx.pricing.clone(),
        timeouts: crate::application::timeouts::resolve_effective(ctx.app_settings.as_ref()),
        agent_ctx: AgentContext {
            thread_id: thread_id.clone(),
            machine_id: repo.machine_str.clone(),
            binary,
            args: vec![],
            env,
            cwd: worktree_path,
            model: discovery.model.clone(),
            effort: discovery.effort,
            title: Some(discovery.title.clone()),
            platform,
            agent_exec: ctx.agent_exec.clone(),
            exec: ctx.exec.clone(),
            permissions: interviewer_permissions(),
            bare_mode: true,
            keep_harness_personalization: crate::domain::turn_role::TurnRole::Orchestrator
                .keeps_harness_personalization(),
            tool_allowlist: None,
            max_turns: None,
            max_budget_usd: None,
        },
        machine_str: repo.machine_str,
        thread_id,
        session_was_live,
        transcript,
        context_text,
        attachments: discovery.attachments.clone(),
        attachment_store: ctx.attachments.clone(),
        attachment_context_dir,
        reads_images: match discovery.model.as_deref() {
            Some(model) => crate::application::agent_probe::model_supports_images_by_name(
                &discovery.agent_kind,
                model,
            ),
            None => true,
        },
        user_text: asked.map(|a| a.content.clone()).unwrap_or_default(),
        pricing_model: discovery
            .model
            .clone()
            .or_else(|| ctx.registry.default_model_for(&discovery.agent_kind)),
        discovery: discovery.clone(),
    })
}

async fn run<F>(p: Prepared, emit: Arc<F>)
where
    F: Fn(&str, serde_json::Value) + Send + Sync + 'static,
{
    let started = std::time::Instant::now();
    let running = p.running.claim(p.discovery.id.as_str());
    let mut resumed = p.session_was_live;
    let mut attempts = 0;

    let (ending, reason, spent, activity) = loop {
        attempts += 1;
        let prompt = p.render_prompt(!resumed);

        let session = match p
            .registry
            .get_or_spawn(&p.thread_id, &p.discovery.agent_kind, p.agent_ctx.clone())
            .await
        {
            Ok(s) => s,
            Err(e) => {
                break (
                    TurnEnding::Environmental,
                    Some(format!("Could not start {}: {e}", p.discovery.agent_kind)),
                    nothing_spent(),
                    TurnActivity::default(),
                );
            }
        };

        let sink = Sink::new(emit.clone(), p.discovery.id.as_str().to_string());
        // Per attempt, not per turn: a discarded re-seed attempt's reads are
        // not what the surviving turn did.
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
        latch_resume_id(&p, session.as_ref());

        if attempts == 1 && should_reseed_and_retry(resumed, !spent.text.trim().is_empty(), ending)
        {
            p.registry.kill(&p.thread_id).await;
            resumed = false;
            continue;
        }
        break (ending, reason, spent, activity);
    };

    let parsed = parse_interview_turn(&spent.text);
    let message_id = persist_assistant(&p, &spent, &parsed.prose, activity);

    // Before the events and after the message is stored, in that order: a
    // surface that refreshes on completion reads `turn_running`, and a claim
    // outliving the event it answers would leave it waiting for a turn that
    // has already said everything it had to say.
    drop(running);
    emit(
        EVENT_DISCOVERY_TURN_COMPLETED,
        serde_json::to_value(DiscoveryTurnCompleted {
            discovery_id: p.discovery.id.as_str().to_string(),
            title: p.discovery.title.clone(),
            message_id,
            ending: ending.as_str(),
            reason: reason.clone(),
            cost_usd: spent.cost_usd,
            tokens: spent.tokens,
            duration_ms: started.elapsed().as_millis() as u64,
            reseeded: !resumed,
            nothing_left_to_settle: parsed.nothing_left_to_settle,
        })
        .unwrap_or(serde_json::Value::Null),
    );
    emit(
        EVENT_DISCOVERY_TURN_STATUS,
        status_payload(
            &p.discovery,
            if ending == TurnEnding::Success {
                "idle"
            } else {
                "error"
            },
            reason,
        ),
    );
}

/// Fold what the turn spent onto the Discovery, whatever it spent it on.
///
/// Every ending except a stop carries usage, and a discarded first attempt
/// carries it too: the tokens were bought. It writes no message, though — the
/// only attempt this discards is one that produced no text to write.
pub(super) fn bill(p: &Prepared, spent: &TurnOutcome) {
    if spent.cost_usd == 0.0 && spent.tokens == 0 {
        return;
    }
    if let Err(e) = p.ctx_discoveries.update(
        &p.discovery.id,
        &DiscoveryPatch {
            add_cost: spent.cost_usd,
            add_tokens: spent.tokens,
            ..Default::default()
        },
        crate::paths::now_ms(),
    ) {
        tracing::warn!(discovery = %p.discovery.id.as_str(), error = %e, "discovery: could not fold turn spend");
    }
}

/// Store the harness's own name for this session, once it has one.
///
/// Cached, never authoritative: §4.4 makes the transcript the authority and
/// this a fast path. Written only forward — a turn that ends before the
/// harness names a session leaves the stored id alone, because "this turn saw
/// no id" is not evidence that the id it replaced was wrong.
///
/// What reads it is the resume decision, and what it reads is presence rather
/// than the string: a stored id whose registry session is no longer live is
/// the state §4.4 calls the sid no longer resolving. The string itself cannot
/// be handed back to a fresh process — seeding one would mean changing agent
/// spawn logic, which is a Gate item (AGENTS.md §6) — so within a process the
/// resume rides on the live session that already holds it, and across
/// processes the transcript carries the conversation instead.
pub(super) fn latch_resume_id(
    p: &Prepared,
    session: &dyn crate::ports::agent_runtime::AgentSession,
) {
    let Some(latched) = session.harness_session_id() else {
        return;
    };
    if p.discovery.resume_session_id.as_deref() == Some(latched.as_str()) {
        return;
    }
    let _ = p.ctx_discoveries.update(
        &p.discovery.id,
        &DiscoveryPatch {
            resume_session_id: Some(Some(latched)),
            ..Default::default()
        },
        crate::paths::now_ms(),
    );
}

/// Persist what the interviewer said, verbatim.
///
/// The stored text is the whole turn, block included, and not the prose alone:
/// the transcript is what a re-seeded turn replays, so trimming it would hand
/// the harness a record of a question it never asked. `prose` decides only
/// whether there was anything to keep.
///
/// `activity` is not replayed to the harness and never enters a prompt — it
/// exists so the settled bubble reads the same as the live one did.
fn persist_assistant(
    p: &Prepared,
    spent: &TurnOutcome,
    prose: &str,
    activity: TurnActivity,
) -> Option<String> {
    if prose.trim().is_empty() && spent.text.trim().is_empty() {
        return None;
    }
    let message = DiscoveryMessage {
        id: crate::shared::ids::new_id(),
        discovery_id: p.discovery.id.clone(),
        role: MessageRole::Assistant,
        content: spent.text.clone(),
        cost_usd: Some(spent.cost_usd),
        tokens: Some(spent.tokens),
        // A turn that used no tool stores nothing rather than a row of
        // zeroes: the meta line has to be able to say nothing, and a zeroed
        // summary is indistinguishable from one collected before V49.
        activity: (!activity.is_empty()).then_some(activity),
        created_at: crate::paths::now_ms(),
    };
    match p.ctx_discoveries.append_message(&message) {
        Ok(()) => Some(message.id),
        Err(e) => {
            tracing::error!(discovery = %p.discovery.id.as_str(), error = %e, "discovery: could not store the turn");
            None
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/application/discovery/turn.rs"]
mod tests;

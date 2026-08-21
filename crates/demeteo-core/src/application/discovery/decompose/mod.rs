//! Decomposition: the pass that turns an interview into schedulable work
//! (§5 of `docs/PRD_DISCOVERY.md`).
//!
//! Two acts, deliberately apart. [`run`] asks the interviewer for a plan and
//! refuses anything the rest of the system could not execute, **while the
//! agent is still in context and can be asked to fix its own graph** (§5.2);
//! [`apply`] writes the part of that plan the user checked. Nothing invalid
//! reaches a Ticket row, and nothing at all reaches one without the review in
//! between (§5.3).
//!
//! A pass is one more turn against the Discovery's own session, so it reuses
//! [`super::turn`]'s machinery whole rather than growing a second copy of it:
//! the same prompt assembly, the same re-seed decision, the same spend fold.
//! What it does not reuse is the message log — a proposal is not something the
//! conversation said, and persisting it would put a question the user never
//! answered into the transcript every later turn re-seeds from. What the next
//! pass needs of it is the *tickets*, which `super::context` already renders.
//!
//! Deciding to decompose is the **user's** (§5.1). The interviewer's
//! `nothing_left_to_settle` is advisory and nothing here reads it: a model that
//! keeps finding one more question would otherwise hold the interview open.

pub mod prompt;
pub mod proposal;

use std::collections::HashMap;
use std::sync::Arc;

use crate::adapters::agent::event_stream::turn::stream_agent_turn;
use crate::domain::ids::{DiscoveryId, TicketId, WorkflowId};
use crate::domain::models::{DiscoveryStatus, Ticket, TicketState};
use crate::domain::ticket_graph::{
    derive_board, validate_ticket_graph, ImmutableViolation, TicketLane,
};
use crate::domain::ticket_plan::{
    extract_ticket_plan, plan_application, ticket_plan_json_shape_example, validate_ticket_plan,
    Application, TicketBody, TicketPlan,
};
use crate::ports::discovery::TicketPatch;
use crate::state::AppContext;

use super::events::{status_payload, Sink, TurnEnding, EVENT_DISCOVERY_TURN_STATUS};
use super::question::{render_turn_prompt, TurnPrompt};
use super::turn::{self, Prepared};
use proposal::{Choices, DecomposeProposal, Pass, Rejected};

pub use proposal::DecomposeApply;

/// How many plans one press of Decompose will buy.
///
/// Three, because of how the validators answer. Both
/// [`validate_ticket_graph`] and
/// [`validate_ticket_plan`] report the **first** fault they find and stop, so
/// a plan with two independent mistakes — a cycle and a ticket with no
/// acceptance criteria — cannot be described in one message and cannot be
/// fixed in one re-ask however capable the agent is. Two re-asks is what it
/// takes to separate "the agent misread the contract once" from "the agent
/// keeps re-authoring the same graph", and each attempt is a full billed pass
/// over the whole plan. A third fault is a plan the user should be shown
/// rather than one the loop should keep paying for: the refusal comes back on
/// the proposal, the interview is still open, and asking again is one click.
pub const MAX_ASKS: usize = 3;

/// Ask for a plan, refuse what cannot be executed, and return the proposal
/// with what it would change.
///
/// The turn streams through the interview's own events so the surface can show
/// the agent working; the proposal itself comes back on the call, because
/// unlike a turn there is nothing to render until it is whole.
pub async fn run<F>(
    ctx: &AppContext,
    discovery_id: &DiscoveryId,
    emit_fn: F,
) -> Result<DecomposeProposal, String>
where
    F: Fn(&str, serde_json::Value) + Send + Sync + 'static,
{
    let discovery = ctx
        .discoveries
        .get(discovery_id)?
        .ok_or_else(|| format!("Discovery not found: {}", discovery_id.as_str()))?;
    if discovery.status != DiscoveryStatus::Open {
        return Err("This discovery is closed. Reopen it to decompose it again.".into());
    }
    let rows = ctx.tickets.list_for_discovery(discovery_id)?;
    let choices = Choices::read(ctx)?;

    let mut prepared = turn::prepare(ctx, &discovery, None).await?;
    prepared.user_text = prompt::decompose_request(&rows, &choices);

    let emit = Arc::new(emit_fn);
    emit(
        EVENT_DISCOVERY_TURN_STATUS,
        status_payload(&discovery, "running", None),
    );
    let asked = ask(&mut prepared, &emit, &rows, &choices).await;
    emit(
        EVENT_DISCOVERY_TURN_STATUS,
        status_payload(
            &discovery,
            if asked.is_ok() { "idle" } else { "error" },
            asked.as_ref().err().cloned(),
        ),
    );
    let asked = asked?;

    Ok(DecomposeProposal {
        discovery_id: discovery_id.as_str().to_string(),
        first_pass: rows.is_empty(),
        tickets: asked
            .pass
            .as_ref()
            .map(|pass| pass.plan.tickets.clone())
            .unwrap_or_default(),
        changes: asked
            .pass
            .as_ref()
            .map(|pass| pass.changes(&choices))
            .unwrap_or_default(),
        locked: proposal::locked(&rows, &lanes(ctx, &rows)?),
        refused: asked.refused,
        refusal: asked.refusal,
        violations: asked.violations,
        cost_usd: asked.cost_usd,
        tokens: asked.tokens,
    })
}

/// What the ask loop accumulated, whether or not it ended with a usable plan.
#[derive(Default)]
struct Asked {
    pass: Option<Pass>,
    refused: Vec<String>,
    refusal: Option<String>,
    violations: Vec<ImmutableViolation>,
    cost_usd: f64,
    tokens: i64,
}

/// `Err` only when the turn itself did not run. A plan that was refused is
/// still a completed pass: it cost money, the user has to see why, and the
/// refusal is the interviewer's to answer rather than the caller's.
async fn ask<F>(
    p: &mut Prepared,
    emit: &Arc<F>,
    rows: &[Ticket],
    choices: &Choices,
) -> Result<Asked, String>
where
    F: Fn(&str, serde_json::Value) + Send + Sync + 'static,
{
    let mut out = Asked::default();
    let mut resumed = p.session_was_live;
    let mut reseeded = false;
    let mut asks = 0usize;

    loop {
        let was_resumed = resumed;
        let text = render_turn_prompt(TurnPrompt {
            reseed: !resumed,
            context: &p.context_text,
            transcript: &p.transcript,
            user_text: &p.user_text,
        });
        let session = p
            .registry
            .get_or_spawn(&p.thread_id, &p.discovery.agent_kind, p.agent_ctx.clone())
            .await
            .map_err(|e| format!("Could not start {}: {e}", p.discovery.agent_kind))?;

        let sink = Sink::new(emit.clone(), p.discovery.id.as_str().to_string());
        let result = stream_agent_turn(
            session.as_ref(),
            &text,
            p.timeouts,
            None,
            &p.machine_str,
            p.exec.as_ref(),
            p.pricing_model.clone(),
            p.pricing.clone(),
            |event| sink.push(event),
        )
        .await;
        sink.flush();

        let (ending, reason, spent) = turn::split(result);
        out.cost_usd += spent.cost_usd;
        out.tokens += spent.tokens;
        turn::bill(p, &spent);
        turn::latch_resume_id(p, session.as_ref());

        if ending != TurnEnding::Success {
            if !reseeded
                && turn::should_reseed_and_retry(was_resumed, !spent.text.trim().is_empty(), ending)
            {
                p.registry.kill(&p.thread_id).await;
                reseeded = true;
                resumed = false;
                continue;
            }
            return Err(reason.unwrap_or_else(|| "The decomposition did not finish.".into()));
        }
        resumed = true;
        asks += 1;

        match read_plan(&spent.text, rows, choices) {
            Ok(pass) => {
                out.pass = Some(pass);
                return Ok(out);
            }
            Err(rejected) => {
                let message = rejected.message();
                out.refused.push(message.clone());
                if asks >= MAX_ASKS {
                    out.violations = rejected.violations();
                    out.refusal = Some(message);
                    return Ok(out);
                }
                p.user_text = prompt::re_ask(&message);
            }
        }
    }
}

fn read_plan(text: &str, rows: &[Ticket], choices: &Choices) -> Result<Pass, Rejected> {
    let plan = extract_ticket_plan(text).ok_or_else(|| {
        Rejected::Reason(format!(
            "I could not read a ticket list out of that answer. Send one JSON object and nothing \
             else, shaped exactly like this: {}",
            ticket_plan_json_shape_example()
        ))
    })?;
    if let Some(reason) = validate_ticket_plan(&plan) {
        return Err(Rejected::Reason(reason));
    }
    Pass::resolve(plan, rows.to_vec(), choices)
}

fn lanes(ctx: &AppContext, rows: &[Ticket]) -> Result<HashMap<String, TicketLane>, String> {
    let (nodes, _) = crate::application::tickets::nodes_for(rows, &*ctx.features)?;
    Ok(derive_board(&nodes)
        .standings
        .into_iter()
        .map(|standing| (standing.id, standing.lane))
        .collect())
}

/// Land the changes the user checked, and nothing else (§5.3).
///
/// The proposal is re-resolved and re-diffed against the rows as they stand
/// now, not against the ones the pass saw: a ticket can be started while the
/// modal is open, and §5.3's immutability is a fact about the row at the
/// moment of writing. The subset is then validated as a *graph* before
/// anything is written, because a subset of a valid proposal is not itself
/// valid — [`plan_application`] carries which combinations break and why that
/// check cannot live at the call site.
pub fn apply(
    ctx: &AppContext,
    input: DecomposeApply,
) -> Result<crate::application::tickets::DiscoveryBoard, String> {
    let discovery_id = DiscoveryId::from(input.discovery_id.clone());
    ctx.discoveries
        .get(&discovery_id)?
        .ok_or_else(|| format!("Discovery not found: {}", discovery_id.as_str()))?;
    let rows = ctx.tickets.list_for_discovery(&discovery_id)?;
    let choices = Choices::read(ctx)?;

    let pass = Pass::resolve(
        TicketPlan {
            tickets: input.tickets,
        },
        rows,
        &choices,
    )
    .map_err(|rejected| rejected.message())?;

    let application = plan_application(&pass.current, &pass.proposed, &pass.diff, &input.accept);
    if let Some(reason) = validate_ticket_graph(&application.resulting) {
        return Err(format!(
            "these changes cannot be applied together: {reason} Either accept the change that \
             would have carried it, or leave the one that names it unchecked."
        ));
    }
    write(ctx, &discovery_id, &application)?;
    crate::application::tickets::board(ctx, &discovery_id)
}

/// Write the accepted changes, in the one order that never leaves a stored
/// edge naming a row that is gone: additions and revisions first — a revision
/// is how an edge to a removed ticket disappears — and the deletes last.
///
/// An addition's stored id is minted here rather than taken from the plan.
/// The agent authors ids that are unique within *its* plan, and the ticket
/// table is keyed across every Discovery, so two conversations both proposing
/// `auth` would collide. The edges are remapped through the same minting, so
/// proposal ids never reach a row.
fn write(
    ctx: &AppContext,
    discovery_id: &DiscoveryId,
    application: &Application<TicketBody>,
) -> Result<(), String> {
    let now = crate::paths::now_ms();
    let minted: HashMap<&str, TicketId> = application
        .added
        .iter()
        .map(|ticket| {
            (
                ticket.id.as_str(),
                TicketId::from(crate::shared::ids::new_id()),
            )
        })
        .collect();
    let stored = |id: &str| {
        minted
            .get(id)
            .cloned()
            .unwrap_or_else(|| TicketId::from(id.to_string()))
    };

    let next_seq = ctx.tickets.next_seq(discovery_id)?;
    let mut rows = Vec::with_capacity(application.added.len());
    for (seq, ticket) in (next_seq..).zip(application.added.iter()) {
        rows.push(Ticket {
            id: stored(&ticket.id),
            discovery_id: discovery_id.clone(),
            seq,
            title: ticket.body.title.clone(),
            description: ticket.body.description.clone(),
            acceptance: ticket.body.acceptance.clone(),
            files: ticket.body.files.clone(),
            blocked_by: ticket.blocked_by.iter().map(|id| stored(id)).collect(),
            test_command: ticket.body.test_command.clone(),
            workflow_id: ticket.body.workflow_id.clone().map(WorkflowId::from),
            agent_kind: ticket.body.agent_kind.clone(),
            model: ticket.body.model.clone(),
            effort: ticket.body.effort,
            attachments: Vec::new(),
            state: TicketState::Unstarted,
            drop_reason: None,
            force_start_reason: None,
            force_started_at: None,
            feature_id: None,
            created_at: now,
            updated_at: now,
        });
    }
    if !rows.is_empty() {
        ctx.tickets.upsert_batch(&rows)?;
    }

    for ticket in &application.revised {
        ctx.tickets.update(
            &TicketId::from(ticket.id.clone()),
            &TicketPatch {
                title: Some(ticket.body.title.clone()),
                description: Some(ticket.body.description.clone()),
                acceptance: Some(ticket.body.acceptance.clone()),
                files: Some(ticket.body.files.clone()),
                blocked_by: Some(ticket.blocked_by.iter().map(|id| stored(id)).collect()),
                test_command: Some(ticket.body.test_command.clone()),
                workflow_id: Some(ticket.body.workflow_id.clone().map(WorkflowId::from)),
                agent_kind: Some(ticket.body.agent_kind.clone()),
                model: Some(ticket.body.model.clone()),
                effort: Some(ticket.body.effort),
                ..Default::default()
            },
            now,
        )?;
    }

    for id in &application.removed {
        ctx.tickets.delete(&TicketId::from(id.clone()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../../tests/application/discovery/decompose.rs"]
mod tests;

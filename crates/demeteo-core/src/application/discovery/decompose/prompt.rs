//! What the decomposer is asked for, and what it is told when its answer is
//! refused.
//!
//! Demeteo owns this text on the same terms as the interview prompt in
//! `crates/demeteo-core/src/application/discovery/question.rs`, which carries
//! why. Everything here is synchronous string assembly against values the
//! caller already read; no function reads a port.

use crate::domain::models::{EffortLevel, Ticket};
use crate::domain::ticket_plan::ticket_plan_json_shape_example;

use super::proposal::Choices;

/// The pass itself: what a ticket is for, what each field has to carry, and
/// the plan as it stands.
///
/// It is one prompt rather than a preamble plus a request because a decompose
/// pass happens once per press of the button — there is no second turn to
/// amortise a preamble over, and the re-ask below deliberately carries none of
/// it.
pub fn decompose_request(rows: &[Ticket], choices: &Choices) -> String {
    format!(
        r#"Stop interviewing and decompose this conversation into tickets.

Each ticket becomes one run: one branch, one pull request, one coding agent
working alone in its own checkout. That agent will read the ticket's
description and acceptance criteria and the repository — and nothing else of
this conversation. Everything you know that it needs has to be in the ticket.

Answer with one JSON object and nothing else. No prose before it, no prose
after it, no commentary between the tickets:

{shape}

WHAT EACH FIELD IS FOR

- `id` — kebab-case and unique in this plan. Every dependency edge and every
  later re-decomposition is matched on it. For a ticket that already exists,
  reuse the id it is listed under below, character for character; a different
  id is a different ticket.
- `title` — what someone calls this piece of work in a sentence. Imperative,
  one line, no ticket number.
- `description` — what to build, the decision behind it, and what was ruled
  out and why. The rejected alternative is the part that is worth writing:
  without it the agent re-opens a question this conversation already closed,
  and answers it differently.
- `acceptance` — one criterion per entry, each one something a reviewer who
  was not here could check against the diff and answer yes or no to. Prefer
  the observable: a command that exits zero, an input that now returns an
  error, a field that appears on a screen that had none. "Works correctly" is
  not a criterion. These are what the run is held to and what tells it that it
  is finished, so a criterion you leave out is one nothing checks, and a
  criterion nobody can check buys nothing.
- `files` — the paths you expect it to touch, as far as you actually know
  them. A hint for the agent, not a fence: a wrong path costs more than a
  missing one, so leave the list short or empty rather than guessing.
- `test_command` — what proves this particular ticket, when the project's own
  test command is not the right one for it. Null to use the project's.
- `blocked_by` — the ids of the tickets that must **land** before this one can
  start. An edge here stops a ticket from starting until the other one's pull
  request merges, so an edge added for tidiness serialises work that could
  have run at once. Two tickets touching the same file are not a dependency.
  A ticket that would build on code another has not written yet is one. Edges
  may only name tickets in this plan, and they must not form a loop.
- `workflow` — which workflow runs it, by name, spelled exactly as listed
  below.
- `agent`, `model`, `effort` — the harness to run it on and how hard to think.
  Null means inherit the project's setting, which is the right answer unless
  this ticket wants something different from the ones around it.
- `why` — one or two sentences addressed to the person reviewing this pass:
  why this ticket exists, or what changed about it since the last pass. It is
  shown beside the ticket in the review and is not stored on the ticket.

HOW TO SIZE ONE

A ticket is one agent's work in one sitting, ending in a pull request a person
can review in one. Splitting past that buys edges and merge conflicts; a
ticket that needs three unrelated decisions inside it buys a run that makes two
of them badly. When two pieces want different acceptance criteria, they are
two tickets.

{plan}

{catalog}"#,
        shape = ticket_plan_json_shape_example(),
        plan = current_plan_block(rows),
        catalog = catalog_block(choices),
    )
}

/// The plan as it stands, with the ids a re-decomposition has to reuse.
///
/// The interview's own context block names tickets by `#seq` (`context.rs`),
/// which is the number a user says out loud and not a key. A pass that was
/// shown only those numbers would author fresh ids for tickets that already
/// exist, and every one of them would read as a removal and an addition — so
/// this block, and only this block, shows the stored id.
fn current_plan_block(rows: &[Ticket]) -> String {
    if rows.is_empty() {
        return "THE PLAN SO FAR\n\nThere are no tickets yet. Everything you emit is new."
            .to_string();
    }
    let mut out = String::from(
        "THE PLAN SO FAR\n\nThis is every ticket this conversation has already produced. Emit \
         the whole plan you believe in, not just what changed: a ticket you leave out is a \
         ticket you are proposing to remove.\n\n",
    );
    for row in rows {
        out.push_str(&format!(
            "- id `{}` (#{}) [{}] {}\n",
            row.id.0,
            row.seq,
            row.state.as_str(),
            row.title.trim()
        ));
        if !row.blocked_by.is_empty() {
            out.push_str(&format!(
                "  blocked_by: {}\n",
                row.blocked_by
                    .iter()
                    .map(|id| format!("`{}`", id.0))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let locked: Vec<String> = rows
        .iter()
        .filter(|r| r.state == crate::domain::models::TicketState::Started)
        .map(|r| format!("`{}`", r.id.0))
        .collect();
    if !locked.is_empty() {
        out.push_str(&format!(
            "\nAlready started, and therefore fixed: {}. Re-emit each of them exactly as it \
             stands — same title, same description, same criteria, same edges. Work you have \
             changed your mind about there belongs in a new ticket that depends on it, not in a \
             rewrite of it.\n",
            locked.join(", ")
        ));
    }
    out
}

fn catalog_block(choices: &Choices) -> String {
    let workflows = if choices.workflows.is_empty() {
        "  (none configured — leave `workflow` null and the user will choose one)".to_string()
    } else {
        choices
            .workflows
            .iter()
            .map(|(_, name)| format!("  - {name}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "WHAT YOU MAY NAME\n\nWorkflows, by exact name:\n{workflows}\n\nAgents: {agents}\n\
         Effort levels: {efforts}",
        agents = choices.agents.join(", "),
        efforts = EffortLevel::ALL
            .iter()
            .map(|e| e.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// Hand one refusal back to the agent that caused it.
///
/// Deliberately short and carrying no repeat of the field guide: the session
/// still holds the request and the plan it just wrote, and restating the whole
/// contract invites a rewrite of tickets that were never at fault. What it
/// does repeat is the shape, because that is the one thing a malformed answer
/// proves the agent did not have.
pub fn re_ask(reason: &str) -> String {
    format!(
        r#"That plan was refused before anything was written, so nothing has changed yet:

{reason}

Fix it and send the whole plan again — every ticket, not just the one at
fault — as one JSON object and nothing else:

{shape}"#,
        shape = ticket_plan_json_shape_example(),
    )
}

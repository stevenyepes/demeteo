//! One decompose pass, resolved: what the agent's names mean, what the plan
//! would change, and what the review modal renders.
//!
//! Nothing here is persisted. §5.3 asks for a proposed-changes view before
//! anything is applied, not for a second table, so the proposal travels out to
//! the surface and back again — which is why [`Pass::plan`] keeps the agent's
//! own unresolved text and every id in the payload is proposal-space.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::domain::ids::WorkflowId;
use crate::domain::models::{AgentKind, EffortLevel, Ticket, TicketState};
use crate::domain::ticket_graph::{
    diff_proposal, CurrentTicket, ImmutableViolation, ProposedTicket, TicketDiff, TicketLane,
};
use crate::domain::ticket_plan::{
    field_changes, FieldChange, PlannedTicket, TicketBody, TicketPlan,
};
use crate::state::AppContext;

/// The workflows and agents a plan is allowed to name.
///
/// Read once per pass and used twice: the prompt lists them, and the resolver
/// refuses anything outside them. One value for both is what stops the
/// interviewer being offered a workflow the resolver would then reject.
pub struct Choices {
    pub workflows: Vec<(WorkflowId, String)>,
    pub agents: Vec<String>,
}

impl Choices {
    pub fn read(ctx: &AppContext) -> Result<Self, String> {
        Ok(Self {
            workflows: ctx
                .workflows
                .list()?
                .into_iter()
                .map(|w| (w.id, w.name))
                .collect(),
            agents: ctx
                .registry
                .runtimes()
                .iter()
                .map(|r| r.kind().to_string())
                .filter(|kind| AgentKind::is_supported(kind))
                .collect(),
        })
    }

    fn workflow_id(&self, name: &str) -> Option<&WorkflowId> {
        let name = name.trim();
        self.workflows
            .iter()
            .find(|(_, known)| known.eq_ignore_ascii_case(name))
            .map(|(id, _)| id)
    }

    fn workflow_name(&self, id: &str) -> Option<String> {
        self.workflows
            .iter()
            .find(|(known, _)| known.0 == id)
            .map(|(_, name)| name.clone())
    }

    fn names(&self) -> String {
        self.workflows
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Why a pass cannot be applied, in the two shapes the caller treats
/// differently.
///
/// Both are handed back to the agent — a workflow name it invented and a
/// started ticket it rewrote are equally its own to fix, and it is still in
/// context. They stay apart because only the second survives into the
/// proposal, where §4.9 of `docs/DISCOVERY_UI_SPEC.md` renders it per ticket
/// rather than as one sentence.
pub enum Rejected {
    Reason(String),
    Immutable(Vec<ImmutableViolation>),
}

impl Rejected {
    /// The one message to re-ask with.
    pub fn message(&self) -> String {
        match self {
            Self::Reason(reason) => reason.clone(),
            Self::Immutable(violations) => violations
                .iter()
                .map(|v| v.reason.clone())
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    pub fn violations(&self) -> Vec<ImmutableViolation> {
        match self {
            Self::Reason(_) => Vec::new(),
            Self::Immutable(violations) => violations.clone(),
        }
    }
}

/// A plan the agent wrote, resolved against the tickets it was diffed with.
pub struct Pass {
    /// Verbatim and unresolved, so applying it means handing this back.
    pub plan: TicketPlan,
    pub rows: Vec<Ticket>,
    pub current: Vec<CurrentTicket<TicketBody>>,
    pub proposed: Vec<ProposedTicket<TicketBody>>,
    pub diff: TicketDiff,
}

impl Pass {
    /// Turn the agent's names into ids and classify the result, or say what
    /// the agent has to fix.
    ///
    /// Run [`validate_ticket_plan`](crate::domain::ticket_plan::validate_ticket_plan)
    /// first: this assumes the ids and the edges are already known good and
    /// only reports what that validator cannot see.
    pub fn resolve(
        plan: TicketPlan,
        rows: Vec<Ticket>,
        choices: &Choices,
    ) -> Result<Self, Rejected> {
        let current: Vec<CurrentTicket<TicketBody>> = rows.iter().map(current_of).collect();
        let stored: HashMap<&str, &CurrentTicket<TicketBody>> =
            current.iter().map(|t| (t.id.as_str(), t)).collect();

        let mut proposed = Vec::with_capacity(plan.tickets.len());
        for planned in &plan.tickets {
            let id = planned.id.trim().to_string();
            let body = resolve_body(
                planned,
                choices,
                stored.get(id.as_str()).map(|stored| &stored.body),
            )
            .map_err(Rejected::Reason)?;
            proposed.push(ProposedTicket {
                id,
                blocked_by: edges(&planned.blocked_by),
                body,
            });
        }

        let diff = diff_proposal(&current, &proposed).map_err(Rejected::Immutable)?;
        Ok(Self {
            plan,
            rows,
            current,
            proposed,
            diff,
        })
    }

    /// Every reviewable change, in the order §4.3 of
    /// `docs/DISCOVERY_UI_SPEC.md` groups them. Unchanged tickets are absent:
    /// they are nothing to decide about, and counting them would make "Apply
    /// n of m" count the plan rather than the diff.
    pub fn changes(&self, choices: &Choices) -> Vec<ProposedChange> {
        let seq: HashMap<&str, i64> = self.rows.iter().map(|r| (r.id.0.as_str(), r.seq)).collect();
        let stored: HashMap<&str, &CurrentTicket<TicketBody>> =
            self.current.iter().map(|t| (t.id.as_str(), t)).collect();
        let why: HashMap<&str, Option<String>> = self
            .plan
            .tickets
            .iter()
            .map(|t| (t.id.trim(), t.why.clone().filter(|w| !w.trim().is_empty())))
            .collect();

        let mut out = Vec::new();
        for kind in [ChangeKind::Added, ChangeKind::Revised] {
            let ids = match kind {
                ChangeKind::Added => &self.diff.added,
                _ => &self.diff.revised,
            };
            for ticket in self.proposed.iter().filter(|t| ids.contains(&t.id)) {
                let id = ticket.id.as_str();
                out.push(ProposedChange {
                    id: ticket.id.clone(),
                    kind,
                    seq: seq.get(id).copied(),
                    title: ticket.body.title.clone(),
                    why: why.get(id).cloned().flatten(),
                    workflow_name: ticket
                        .body
                        .workflow_id
                        .as_deref()
                        .and_then(|id| choices.workflow_name(id)),
                    agent_kind: ticket.body.agent_kind.clone(),
                    blocked_by: ticket.blocked_by.clone(),
                    fields: match stored.get(id) {
                        Some(was) => field_changes(
                            &was.body,
                            &ticket.body,
                            &was.blocked_by,
                            &ticket.blocked_by,
                        ),
                        None => Vec::new(),
                    },
                });
            }
        }
        for row in self
            .rows
            .iter()
            .filter(|r| self.diff.removed.contains(&r.id.0))
        {
            out.push(ProposedChange {
                id: row.id.0.clone(),
                kind: ChangeKind::Removed,
                seq: Some(row.seq),
                title: row.title.clone(),
                why: why.get(row.id.0.as_str()).cloned().flatten(),
                workflow_name: row
                    .workflow_id
                    .as_ref()
                    .and_then(|id| choices.workflow_name(&id.0)),
                agent_kind: row.agent_kind.clone(),
                blocked_by: edges(
                    &row.blocked_by
                        .iter()
                        .map(|id| id.0.clone())
                        .collect::<Vec<_>>(),
                ),
                fields: Vec::new(),
            });
        }
        out
    }
}

/// The started tickets, listed so the user can see what the pass worked
/// around (§4.8 of `docs/DISCOVERY_UI_SPEC.md`).
pub fn locked(rows: &[Ticket], lanes: &HashMap<String, TicketLane>) -> Vec<LockedTicket> {
    rows.iter()
        .filter(|r| r.state == TicketState::Started)
        .map(|row| LockedTicket {
            id: row.id.0.clone(),
            seq: row.seq,
            title: row.title.clone(),
            lane: lanes.get(&row.id.0).copied(),
        })
        .collect()
}

/// Resolve one planned ticket's fields.
///
/// The two halves inherit differently, and the split is §5.4's: the *planned*
/// fields are the decomposition's to write, so an omitted `test_command`
/// means the project's default and clears whatever was there. The *execution
/// choices* are the user's — the ticket editor sets them by hand — so an
/// omitted one keeps what the ticket already had. Inheriting the first half
/// would make a field impossible to clear; not inheriting the second would let
/// every re-decomposition quietly undo the routing a user chose.
fn resolve_body(
    planned: &PlannedTicket,
    choices: &Choices,
    inherit: Option<&TicketBody>,
) -> Result<TicketBody, String> {
    let workflow_id = match named(&planned.workflow) {
        Some(name) => Some(
            choices
                .workflow_id(&name)
                .ok_or_else(|| {
                    format!(
                        "ticket '{}' names the workflow '{name}', which does not exist. The \
                         workflows are: {}.",
                        planned.id.trim(),
                        choices.names()
                    )
                })?
                .0
                .clone(),
        ),
        None => inherit.and_then(|b| b.workflow_id.clone()),
    };
    let agent_kind = match named(&planned.agent) {
        Some(kind) => {
            if !choices.agents.contains(&kind) {
                return Err(format!(
                    "ticket '{}' names the agent '{kind}', which is not one Demeteo can run. The \
                     agents are: {}.",
                    planned.id.trim(),
                    choices.agents.join(", ")
                ));
            }
            Some(kind)
        }
        None => inherit.and_then(|b| b.agent_kind.clone()),
    };
    let effort = match named(&planned.effort) {
        Some(word) => Some(
            EffortLevel::parse(&word.to_ascii_lowercase()).ok_or_else(|| {
                format!(
                "ticket '{}' names the effort '{word}', which is not a level. The levels are: {}.",
                planned.id.trim(),
                EffortLevel::ALL
                    .iter()
                    .map(|e| e.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            })?,
        ),
        None => inherit.and_then(|b| b.effort),
    };
    Ok(TicketBody {
        title: planned.title.trim().to_string(),
        description: planned.description.trim().to_string(),
        acceptance: trimmed(&planned.acceptance),
        files: trimmed(&planned.files),
        test_command: named(&planned.test_command),
        workflow_id,
        agent_kind,
        model: named(&planned.model).or_else(|| inherit.and_then(|b| b.model.clone())),
        effort,
    })
}

/// The stored half of the comparison, on the same terms
/// [`resolve_body`] builds the proposed half.
pub fn current_of(row: &Ticket) -> CurrentTicket<TicketBody> {
    CurrentTicket {
        id: row.id.0.clone(),
        state: crate::application::tickets::node_of(row, None).state,
        blocked_by: edges(
            &row.blocked_by
                .iter()
                .map(|id| id.0.clone())
                .collect::<Vec<_>>(),
        ),
        body: TicketBody {
            title: row.title.trim().to_string(),
            description: row.description.trim().to_string(),
            acceptance: trimmed(&row.acceptance),
            files: trimmed(&row.files),
            test_command: named(&row.test_command),
            workflow_id: row.workflow_id.as_ref().map(|id| id.0.clone()),
            agent_kind: named(&row.agent_kind),
            model: named(&row.model),
            effort: row.effort,
        },
    }
}

fn named(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn trimmed(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

fn edges(values: &[String]) -> Vec<String> {
    trimmed(values)
}

/// Which of the modal's three groups a change belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Revised,
    Removed,
}

/// One row of the review modal, and one checkbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedChange {
    /// Proposal-space, which is what `accept` names and what every other id in
    /// this payload is. For an addition it is the id the agent authored, not
    /// the one the row will be stored under.
    pub id: String,
    pub kind: ChangeKind,
    /// `None` for an addition: §5.3 assigns `seq` at apply and never reissues
    /// one, so a proposal has no number to show yet.
    pub seq: Option<i64>,
    pub title: String,
    pub why: Option<String>,
    pub workflow_name: Option<String>,
    pub agent_kind: Option<String>,
    pub blocked_by: Vec<String>,
    /// Empty except on a revision.
    pub fields: Vec<FieldChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedTicket {
    pub id: String,
    pub seq: i64,
    pub title: String,
    /// `None` only if the ticket vanished from its own discovery between the
    /// two reads.
    pub lane: Option<TicketLane>,
}

/// What one decompose pass produced, and everything the modal draws.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecomposeProposal {
    pub discovery_id: String,
    /// The discovery held no tickets before this pass — the `First pass`
    /// eyebrow. Derived rather than counted: nothing stores how many times
    /// decomposition has run, and a counter would be one more thing to keep
    /// true.
    pub first_pass: bool,
    /// The plan the agent wrote, verbatim. `discovery_apply_decomposition`
    /// takes it back unchanged.
    pub tickets: Vec<PlannedTicket>,
    pub changes: Vec<ProposedChange>,
    pub locked: Vec<LockedTicket>,
    /// Every refusal the pass was re-asked over, oldest first — including the
    /// ones it then fixed, which is what the validation bar reports.
    pub refused: Vec<String>,
    /// Set when the last attempt was refused too, so nothing here can be
    /// applied.
    pub refusal: Option<String>,
    pub violations: Vec<ImmutableViolation>,
    pub cost_usd: f64,
    pub tokens: i64,
}

/// The subset of a proposal the user chose to land.
#[derive(Debug, Clone, Deserialize)]
pub struct DecomposeApply {
    pub discovery_id: String,
    /// The proposal, exactly as [`DecomposeProposal::tickets`] carried it.
    pub tickets: Vec<PlannedTicket>,
    /// The [`ProposedChange::id`]s that were left checked. A change absent
    /// from this list leaves its stored row alone.
    pub accept: Vec<String>,
}

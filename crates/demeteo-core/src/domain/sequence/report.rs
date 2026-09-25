//! The implementation report: what each ticket's agent said it did, beside
//! what Demeteo saw it do.
//!
//! The validate step judges the finished feature against the spec's criteria,
//! and some criteria are about *how* the work was done ("watch each new test
//! fail"). The implement step had no channel for that: commit messages are
//! generated from ticket titles, it declares no artifacts, and the report
//! directory is excluded from commits. So validate could only ever answer
//! "no evidence" for them, and a verdict that no rework can change is a loop.
//!
//! The report is Demeteo-built, not agent-written, and it keeps its two kinds
//! of line apart on purpose:
//!
//! - **self-reported** — the agent's own closing summary. A claim.
//! - **observed** — shell commands the harness reported the agent running,
//!   with the final status the harness reported. A command whose end the
//!   harness never reported is *not observed*, never "passed"; harnesses that
//!   report no tool calls at all produce an empty observed list, which the
//!   report says in words rather than leaving the reader to infer.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::domain::action::ActionKind;
use crate::domain::agent_event::{AgentEvent, ToolCallStatus};
use crate::domain::text::tail_chars;

/// Tail kept of an agent's closing summary. Its verdict lines come last.
pub const MAX_SELF_REPORT_CHARS: usize = 6_000;
/// Commands kept per ticket; a longer session keeps its latest.
pub const MAX_OBSERVED_COMMANDS: usize = 60;
const MAX_COMMAND_CHARS: usize = 300;
const MAX_FAILURE_CHARS: usize = 300;

/// The artifact name every per-ticket fragment shares.
pub const REPORT_NAME: &str = "implementation-report";
const FRAGMENT_PREFIX: &str = "ticket-";
const FRAGMENT_SUFFIX: &str = "-report";

/// How the harness said a command ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ObservedStatus {
    Completed,
    Failed {
        reason: String,
    },
    /// Started, and the harness never reported how it ended.
    NotObserved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedCommand {
    pub command: String,
    #[serde(flatten)]
    pub status: ObservedStatus,
}

/// Folds a turn's event stream into the shell commands it ran.
#[derive(Debug, Default)]
pub struct CommandObserver {
    commands: Vec<ObservedCommand>,
    by_call: HashMap<String, usize>,
}

impl CommandObserver {
    pub fn observe(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::ToolCall {
                tool_call_id,
                action: ActionKind::RunBash,
                target,
                ..
            } => {
                self.by_call
                    .insert(tool_call_id.clone(), self.commands.len());
                self.commands.push(ObservedCommand {
                    command: tail_chars(target.trim(), MAX_COMMAND_CHARS),
                    status: ObservedStatus::NotObserved,
                });
            }
            AgentEvent::ToolCallUpdate {
                tool_call_id,
                status,
                ..
            } => {
                let Some(cmd) = self
                    .by_call
                    .get(tool_call_id)
                    .and_then(|&i| self.commands.get_mut(i))
                else {
                    return;
                };
                match status {
                    ToolCallStatus::Completed => cmd.status = ObservedStatus::Completed,
                    ToolCallStatus::Failed { reason } => {
                        cmd.status = ObservedStatus::Failed {
                            reason: tail_chars(reason.trim(), MAX_FAILURE_CHARS),
                        }
                    }
                    ToolCallStatus::Pending | ToolCallStatus::InProgress { .. } => {}
                }
            }
            _ => {}
        }
    }

    pub fn into_commands(self) -> Vec<ObservedCommand> {
        self.commands
    }
}

/// One ticket's contribution to the report, as persisted between the ticket's
/// turn and the step's completion — which may be separated by a restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketReport {
    pub ticket_id: String,
    pub title: String,
    /// Orders fragments across attempts and rework cycles.
    pub recorded_at: i64,
    pub self_reported: String,
    #[serde(default)]
    pub self_report_truncated: bool,
    pub observed: Vec<ObservedCommand>,
    /// Commands dropped from the front of `observed` by the cap.
    #[serde(default)]
    pub observed_omitted: usize,
}

impl TicketReport {
    pub fn new(
        ticket_id: &str,
        title: &str,
        agent_text: &str,
        observed: Vec<ObservedCommand>,
        recorded_at: i64,
    ) -> Self {
        let text = agent_text.trim();
        let observed_omitted = observed.len().saturating_sub(MAX_OBSERVED_COMMANDS);
        Self {
            ticket_id: ticket_id.to_string(),
            title: title.to_string(),
            recorded_at,
            self_reported: tail_chars(text, MAX_SELF_REPORT_CHARS),
            self_report_truncated: text.chars().count() > MAX_SELF_REPORT_CHARS,
            observed: observed.into_iter().skip(observed_omitted).collect(),
            observed_omitted,
        }
    }
}

/// The store name of one ticket's fragment. Unique per ticket id, so a ticket
/// re-run under a later attempt replaces its own fragment and nobody else's.
pub fn fragment_name(ticket_id: &str) -> String {
    format!("{FRAGMENT_PREFIX}{ticket_id}{FRAGMENT_SUFFIX}")
}

/// Whether an artifact-store reference names a ticket fragment, so a sweep of
/// the step's store can tell fragments from what the step hands downstream.
pub fn is_fragment_ref(reference: &str) -> bool {
    let file = reference.rsplit(['/', '\\']).next().unwrap_or(reference);
    file.strip_suffix(".json")
        .is_some_and(|stem| stem.starts_with(FRAGMENT_PREFIX) && stem.ends_with(FRAGMENT_SUFFIX))
}

/// Render every fragment into the one report validate reads, oldest first.
///
/// `None` when there is nothing to report — a step whose tasks all landed
/// under a build that predates fragments has no evidence to render, and an
/// empty report would read as "no ticket ran anything".
pub fn render_implementation_report(fragments: &[TicketReport]) -> Option<String> {
    if fragments.is_empty() {
        return None;
    }
    let mut sorted: Vec<&TicketReport> = fragments.iter().collect();
    sorted.sort_by_key(|f| f.recorded_at);

    let mut out = String::from(
        "# Implementation report\n\n\
         Built by Demeteo from each ticket's turn, not written by an agent. Two kinds of line, \
         and they are not interchangeable:\n\n\
         - **self-reported** — the agent's own closing summary. A claim, not evidence.\n\
         - **observed** — a shell command the agent's harness reported running, with the \
         status the harness reported for it. `completed` means the tool call finished, not \
         that its tests passed; read the claim beside it. `not observed` means the harness \
         never reported how the command ended.\n\n",
    );
    for f in sorted {
        out.push_str(&format!("## Ticket `{}` — {}\n\n", f.ticket_id, f.title));
        out.push_str("### Observed\n\n");
        if f.observed.is_empty() {
            out.push_str(
                "No shell command was observed. Either the agent ran none, or its harness \
                 does not report tool calls — this is absence of evidence either way.\n\n",
            );
        } else {
            if f.observed_omitted > 0 {
                out.push_str(&format!(
                    "_{} earlier command(s) omitted; the latest {} follow._\n\n",
                    f.observed_omitted,
                    f.observed.len()
                ));
            }
            for c in &f.observed {
                let status = match &c.status {
                    ObservedStatus::Completed => "completed".to_string(),
                    ObservedStatus::Failed { reason } => {
                        format!("failed — {}", reason.replace('\n', " "))
                    }
                    ObservedStatus::NotObserved => "not observed".to_string(),
                };
                out.push_str(&format!(
                    "- observed: `{}` — {}\n",
                    c.command.replace('`', "'").replace('\n', " "),
                    status
                ));
            }
            out.push('\n');
        }
        out.push_str("### Self-reported\n\n");
        if f.self_reported.is_empty() {
            out.push_str("The agent ended its turn without a summary.\n\n");
        } else {
            if f.self_report_truncated {
                out.push_str("_Truncated; the end of the summary follows._\n\n");
            }
            for line in f.self_reported.lines() {
                out.push_str("> ");
                out.push_str(line);
                out.push('\n');
            }
            out.push('\n');
        }
    }
    Some(out)
}

#[cfg(test)]
#[path = "../../../tests/domain/sequence/report.rs"]
mod tests;

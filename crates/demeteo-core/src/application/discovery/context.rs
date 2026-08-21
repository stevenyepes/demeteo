//! What the interview is told about the world outside its checkout (§4.6 of
//! `docs/PRD_DISCOVERY.md`).
//!
//! Two halves, bounded on different principles.
//!
//! The **project half** is capped at [`RECENT_FEATURES`] rows because it is
//! the half that grows without limit. §4.6 rejects full project history for
//! exactly that: every turn pays for it, and past some small number the facts
//! the current question needs are buried in ones it does not. Ten is the
//! window a person actually holds in their head about a project they are
//! working on, and `get_active` already orders newest-first and drops the
//! archived, so ten is ten *live or lately-live* runs rather than ten rows.
//!
//! The **ticket half** is not capped, and that is deliberate rather than an
//! oversight. §6.2 closes the graph over one Discovery, so this set is one
//! aggregate's rows and is bounded by the plan itself; and §5.3's additive
//! decomposition re-proposes work it cannot see, so a truncated ticket list
//! produces duplicate tickets — the one failure this context exists to
//! prevent. Each row is summarised to a line instead, which is what keeps the
//! cost proportional to the plan rather than to its prose.

use crate::domain::models::{Discovery, Feature, Ticket};
use crate::state::AppContext;

/// How many of the project's own runs the interview is shown.
pub const RECENT_FEATURES: usize = 10;

/// Longest a borrowed title or description runs before it is cut. Long enough
/// to identify the thing, short enough that a hundred of them still fit beside
/// the transcript.
const SUMMARY_CHARS: usize = 120;

/// Read both halves and render them.
pub async fn render(ctx: &AppContext, discovery: &Discovery) -> Result<String, String> {
    let features = ctx.features.get_active(&discovery.project_id)?;
    let tickets = ctx.tickets.list_for_discovery(&discovery.id)?;
    Ok(render_from(&features, &tickets))
}

/// The synchronous half, so the bound and the wording are reachable from a
/// test without a database.
pub(crate) fn render_from(features: &[Feature], tickets: &[Ticket]) -> String {
    let mut out = String::new();
    out.push_str("WHAT ELSE IS GOING ON IN THIS PROJECT\n\n");
    if features.is_empty() {
        out.push_str("No runs in flight or recently finished.\n");
    } else {
        out.push_str(
            "The project's most recent runs, newest first. Work already in flight is work you \
             should not propose again.\n",
        );
        for f in features.iter().take(RECENT_FEATURES) {
            out.push_str("- [");
            out.push_str(&f.status);
            if let Some(mr) = f.mr_state.as_deref().filter(|s| !s.trim().is_empty()) {
                out.push_str(", pr ");
                out.push_str(mr);
            }
            out.push_str("] ");
            out.push_str(&clip(&f.title));
            out.push('\n');
        }
        if features.len() > RECENT_FEATURES {
            out.push_str(&format!(
                "({} older runs not listed.)\n",
                features.len() - RECENT_FEATURES
            ));
        }
    }

    out.push_str("\nTICKETS THIS CONVERSATION HAS ALREADY PRODUCED\n\n");
    if tickets.is_empty() {
        out.push_str("None yet.\n");
    } else {
        out.push_str(
            "The whole plan so far. Anything here already exists — refine it or build on it, \
             but do not propose it a second time under a new name.\n",
        );
        for t in tickets {
            out.push_str(&format!("- #{} [{}] ", t.seq, t.state.as_str()));
            out.push_str(&clip(&t.title));
            if let Some(reason) = t.drop_reason.as_deref().filter(|r| !r.trim().is_empty()) {
                out.push_str(" — dropped: ");
                out.push_str(&clip(reason));
            }
            if !t.blocked_by.is_empty() {
                let names: Vec<String> = t
                    .blocked_by
                    .iter()
                    .map(|id| {
                        tickets
                            .iter()
                            .find(|other| other.id == *id)
                            .map(|other| format!("#{}", other.seq))
                            .unwrap_or_else(|| id.as_str().to_string())
                    })
                    .collect();
                out.push_str(" — blocked by ");
                out.push_str(&names.join(", "));
            }
            out.push('\n');
        }
    }
    out
}

fn clip(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= SUMMARY_CHARS {
        return trimmed.replace('\n', " ");
    }
    let cut: String = trimmed.chars().take(SUMMARY_CHARS).collect();
    format!("{}…", cut.trim_end().replace('\n', " "))
}

#[cfg(test)]
#[path = "../../../tests/application/discovery/context.rs"]
mod tests;

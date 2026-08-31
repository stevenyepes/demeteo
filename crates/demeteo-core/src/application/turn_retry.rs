//! Whether a turn should be run again with the transcript carried in the
//! prompt, because the evidence says the harness no longer knows the session.
//!
//! **No harness reports a lost session distinguishably.** A `claude --resume`
//! against an id its store has pruned exits with an error like any other
//! error; codex, opencode and hermes do the same. Demeteo sees a `Failed` or
//! `Environmental` ending with a message it has no grammar for, and matching
//! on that message would be matching on another product's copy — it changes
//! on their release schedule and nothing here would fail when it did.
//!
//! So the discriminator is evidential rather than textual, and it is the one
//! piece of evidence that means something: **a turn that produced no assistant
//! text never reached the model.** A resumed turn that answered and then fell
//! over plainly resolved its session; the failure is the agent's and re-asking
//! would only repeat it. A resumed turn that emitted nothing is either a lost
//! session or a failure so early that re-seeding repeats it once — which is
//! the conservative side to be wrong on, because the alternative leaves a
//! Discovery permanently unable to take another turn, in exactly the
//! came-back-a-week-later case §4.4 exists for.
//!
//! What it costs when it is wrong is one extra turn, billed. What it costs to
//! omit is the Discovery. `resumed` is what bounds it: a turn that already
//! carried the transcript has nothing left to fall back to, so it is never
//! retried and the loop cannot run more than twice.
pub(crate) use crate::application::discovery::events::TurnEnding;
use crate::domain::models::Feature;

pub(crate) fn should_reseed_and_retry(
    resumed: bool,
    produced_text: bool,
    ending: TurnEnding,
) -> bool {
    resumed && !produced_text && matches!(ending, TurnEnding::Failed | TurnEnding::Environmental)
}

/// How many of the project's own runs the interview is shown. Re-exported by
/// [`crate::application::discovery::context`], whose module doc and test
/// module resolve it through that re-export rather than a copy.
pub(crate) const RECENT_FEATURES: usize = 10;

/// Longest a borrowed title or description runs before it is cut. Long enough
/// to identify the thing, short enough that a hundred of them still fit
/// beside the transcript.
const SUMMARY_CHARS: usize = 120;

/// The "what else is going on in this project" half of a turn's context —
/// shared by Discovery's [`render_from`](crate::application::discovery::context::render_from)
/// (which appends its own ticket half) and Ask, which has no ticket concept
/// and renders nothing else.
pub(crate) fn render_project_context(features: &[Feature]) -> String {
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
#[path = "../../tests/application/turn_retry.rs"]
mod tests;

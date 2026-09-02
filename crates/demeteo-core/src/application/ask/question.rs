//! The Ask prompt, and how one turn's prompt is assembled from it.
//!
//! Mirrors [`crate::application::discovery::question`]'s split exactly: pure
//! string assembly, no I/O, so a whole turn's prompt is renderable from a
//! test without a port. Ask's preamble differs from the interviewer's in
//! posture (answer, don't interrogate) and in what it may append (an
//! optional canvas block, not a decision question).

use crate::domain::ask_canvas::{canvas_block_shape_example, canvas_block_vocabulary};
use crate::domain::models::{AskMessage, MessageRole};

/// What Ask is, what it may do, and what a turn of it looks like.
pub fn ask_preamble() -> String {
    format!(
        r#"You are answering questions inside Demeteo, a coding-agent orchestrator. The
person you are talking to wants to understand this project — its code, its
architecture, how something works or why it was built a certain way. This is
a conversation, not a task: you make nothing here, you only explain.

WHAT YOU CAN DO HERE

You have a checkout of the project's repository and a shell. Read files, grep,
run `git log`, run the build or the tests — ground every claim in what the
repository actually says rather than what seems plausible. You may also use
the web, for anything the repository itself cannot answer (a library's own
docs, a spec, a changelog).

You cannot write. There are no edit tools, and the checkout is fenced
read-only. Do not try to write a file, create a branch, or commit; nothing
you produce here lands in the repository.

HOW A TURN LOOKS

Answer in prose, in your own voice. Say the thing that answers the question,
then the reasoning or the evidence behind it — lead with the answer, not the
investigation.

When a diagram would make the answer clearer than prose alone — an
architecture, a journey through the system, a dataflow — append one JSON
block, and only one, at the very end of the turn:

{shape}

Nothing after the block. Prose above it, block below it, at most one per
turn. Most turns need no block at all; do not force one.

THE BLOCK'S FIXED VOCABULARY

{vocabulary}

WHAT MAKES A CANVAS WORTH DRAWING

- Only draw one when stages, lanes, and the arrows between nodes actually
  clarify something prose would leave the reader to reconstruct themselves.
- Every node's `title` names a real thing in the codebase or the
  conversation — a type, a module, a step, a person — never a placeholder.
- Set `path` on a node only when it names a file or module you have actually
  read or found in this repository. A node with no real path leaves it unset
  rather than guessing one.
- Keep it small enough to read at a glance. A canvas nobody can parse in a
  few seconds is prose that should have stayed prose."#,
        shape = canvas_block_shape_example(),
        vocabulary = canvas_block_vocabulary(),
    )
}

/// Everything one prompt carries besides the preamble.
pub struct TurnPrompt<'a> {
    /// `true` when the harness has no memory of this conversation and the
    /// transcript has to carry it — see [`super::turn`] for who decides.
    pub reseed: bool,
    /// Rendered by [`crate::application::turn_retry::render_project_context`].
    pub context: &'a str,
    pub transcript: &'a [AskMessage],
    pub user_text: &'a str,
}

/// Assemble the text one turn is prompted with.
///
/// The context block is rebuilt every turn, resumed or not, on the same
/// reasoning as the interviewer's: it describes work that moves while the
/// thread is open, so the harness's own history of it is stale by
/// construction.
pub fn render_turn_prompt(p: TurnPrompt<'_>) -> String {
    let mut out = String::new();
    if p.reseed {
        out.push_str(&ask_preamble());
        out.push_str("\n\n");
        if !p.transcript.is_empty() {
            out.push_str(
                "THE CONVERSATION SO FAR\n\nThis is the record of this thread. It is the \
                 authority on what was said; anything you remember that disagrees with it is \
                 wrong.\n\n",
            );
            for m in p.transcript {
                let who = match m.role {
                    MessageRole::User => "USER",
                    MessageRole::Assistant => "YOU",
                };
                out.push_str(who);
                out.push_str(": ");
                out.push_str(m.text.trim());
                out.push_str("\n\n");
            }
        }
    }
    if !p.context.trim().is_empty() {
        out.push_str(p.context.trim());
        out.push_str("\n\n");
    }
    out.push_str("USER: ");
    out.push_str(p.user_text.trim());
    out
}

#[cfg(test)]
#[path = "../../../tests/application/ask/question.rs"]
mod tests;

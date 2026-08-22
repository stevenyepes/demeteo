//! The interview prompt, and how one turn's prompt is assembled from it.
//!
//! Demeteo owns this text (§12 #4 of `docs/PRD_DISCOVERY.md`): the alternative
//! was depending on whatever grilling command the user happens to have
//! installed under `~/.claude`, which AGENTS.md §2 forbids Demeteo from
//! reading or writing in the first place.
//!
//! Everything here is synchronous string assembly. It is in `application/`
//! rather than `domain/` only because a prompt is not a decision — nothing
//! branches on it — but it obeys the same rule about I/O: no function here
//! reads a port, so a test can render a whole turn's prompt without one.

use crate::domain::attachment::AttachedFile;
use crate::domain::discovery_question::interview_block_shape_example;
use crate::domain::models::{DiscoveryMessage, MessageRole};

/// What the interviewer is, what it may do, and what a turn of it looks like.
///
/// The behaviour this has to produce is demonstrated, not described, by the
/// seeded questions in `docs/DISCOVERY_UI_SPEC.md` §3.4.4 — read those before
/// editing this, because they are the acceptance criteria for it.
pub fn interview_preamble() -> String {
    format!(
        r#"You are conducting a planning interview inside Demeteo. The person you are
talking to wants to turn an idea into work that can actually be scheduled. The
conversation is the deliverable; the code comes later, from other agents.

WHAT YOU CAN DO HERE

You have a checkout of the project's repository and a shell. Read files, grep,
run `git log`, run the build or the tests — do that whenever a fact about the
repository decides the question you are about to ask. An interview that cannot
check a fact degrades into guessing, and a question built on a guess wastes the
user's turn.

You cannot write. There are no edit tools, and the checkout is fenced read-only.
Do not try to write a file, create a branch, or commit; nothing you produce here
lands in the repository. Say what should change — do not change it.

HOW A TURN LOOKS

Answer in prose, in your own voice, and keep it short: a few sentences, not a
report. Say the thing that changes what the user does next. If a fact you just
read contradicts what they said, lead with it.

Then, when there is a real decision to settle, append one JSON block — and only
one, at the very end of the turn:

{shape}

Nothing after the block. Prose above it, block below it, one question per turn.

WHAT MAKES A QUESTION WORTH ASKING

- Ask about the thing that is genuinely undecided and that changes what gets
  built. Never ask for a preference you could have read out of the repository,
  and never ask two questions in one turn.
- `header` names the constraint the question turns on in two or three words —
  `Identity`, `Refusal`, `First move`. Not a restatement of the question.
- `text` says why the question exists, then asks it. One sentence of stake, one
  question.
- Every option must be an answer someone could reasonably choose. If one of
  them is there to be rejected, delete it and offer fewer — two real bets beat
  three where only one is real. Two to four options.
- Every `description` says what choosing it commits to *and what it costs*. An
  option with only upside in its description is one you have not thought about.
- `recommended` is optional and often absent. Name an option only when there is
  a reason you can state, and state that reason inside that option's own
  description. Recommending whichever option you listed first is how a
  recommendation stops meaning anything.
- The user may ignore your options entirely and answer in their own words. When
  they do, take what they wrote as written rather than fitting it to the
  nearest option you offered, and say so.

WHEN YOU THINK IT IS DONE

Set `"nothing_left_to_settle": true` on a turn when you believe there is no
open question worth asking. It is advisory. The user decides when to stop
interviewing and decompose the conversation into tickets; you never do, and you
never refuse to keep going.

Omit `"question"` entirely on a turn that has nothing to ask. A turn that is
only prose is a normal turn."#,
        shape = interview_block_shape_example(),
    )
}

/// Everything one prompt carries besides the preamble.
pub struct TurnPrompt<'a> {
    /// `true` when the harness has no memory of this conversation and the
    /// transcript has to carry it — see [`super::turn`] for who decides.
    pub reseed: bool,
    /// Rendered by [`super::context::render`].
    pub context: &'a str,
    pub transcript: &'a [DiscoveryMessage],
    /// What the user handed the interviewer (§4.6), named the one way an
    /// agent already understands. The bytes are put where a `Read` can reach
    /// them while [`super::turn`] sets the turn up; this only says they exist.
    pub attachments: &'a [AttachedFile],
    /// Whether the interviewer's model can see an image, which decides only
    /// whether the block warns that it cannot.
    pub reads_images: bool,
    pub user_text: &'a str,
}

/// Assemble the text one turn is prompted with.
///
/// The context block is rebuilt every turn, resumed or not: it describes work
/// that moves while the interview is open, so the copy the harness already has
/// in its own history is stale by construction. The attachment block rides on
/// the same terms — a file removed between turns must stop being offered. The
/// transcript is the part that is only sent when the harness cannot supply it
/// itself.
pub fn render_turn_prompt(p: TurnPrompt<'_>) -> String {
    let mut out = String::new();
    if p.reseed {
        out.push_str(&interview_preamble());
        out.push_str("\n\n");
        if !p.transcript.is_empty() {
            out.push_str(
                "THE CONVERSATION SO FAR\n\nThis is the record of this interview. It is the \
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
                out.push_str(m.content.trim());
                out.push_str("\n\n");
            }
        }
    }
    if !p.context.trim().is_empty() {
        out.push_str(p.context.trim());
        out.push_str("\n\n");
    }
    if let Some(block) = crate::domain::attachment::attachment_block(p.attachments, p.reads_images)
    {
        out.push_str(&block);
        out.push_str("\n\n");
    }
    out.push_str("USER: ");
    out.push_str(p.user_text.trim());
    out
}

#[cfg(test)]
#[path = "../../../tests/application/discovery/question.rs"]
mod tests;

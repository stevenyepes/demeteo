//! The structured question an interview turn may carry, and the advisory
//! signal that ends one (`docs/DISCOVERY_UI_SPEC.md` §3.4.4, §3.4.5).
//!
//! Synchronous and total, per the rule on [`crate::domain`].
//!
//! A turn's output is prose **plus an optional JSON block**. The block is not
//! stored anywhere the prose is not: it rides inside the assistant message's
//! own text and is re-derived on read by [`parse_interview_turn`]. A column of
//! its own would let the two disagree about what a turn said, and answeredness
//! — the property the UI actually renders — is derived either way, from
//! whether a user message follows.
//!
//! [`interview_block_shape_example`] is the single source for the shape: the
//! prompt that asks for it and the message that rejects a malformed one both
//! call it, so the two cannot drift. `task_list_json_shape_example` in
//! `crates/demeteo-core/src/domain/sequence/tasks.rs` is the precedent and
//! carries why.

use serde::{Deserialize, Serialize};

/// The largest option count a question may carry.
///
/// Four, because the surface that renders one gives every option a single
/// keycap (`1`..`4`, then `↵` for the free-text answer) and because a
/// question offering more than four is no longer asking the user to choose
/// between bets — it is listing. The lower bound is two for the same reason
/// from the other side: one option is a statement.
pub const MAX_OPTIONS: usize = 4;
const MIN_OPTIONS: usize = 2;

/// One of the bets a question offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    /// Stable within its question, and what an answer names. Not a position:
    /// the same option keeps its id if the interviewer re-asks with one more.
    pub id: String,
    pub label: String,
    /// What choosing it commits to, including what it costs. An option with
    /// no cost stated is the straw man the prompt forbids.
    #[serde(default)]
    pub description: String,
}

/// One open question, as the turn that asked it spelled it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryQuestion {
    /// The two-or-three-word chip beside the asker's name — the constraint
    /// the question turns on, not a restatement of it.
    pub header: String,
    pub text: String,
    pub options: Vec<QuestionOption>,
    /// The [`QuestionOption::id`] the interviewer would pick, or `None`.
    ///
    /// `None` is a real answer and the prompt says so: recommending whichever
    /// option came first is how a recommendation stops meaning anything.
    #[serde(default)]
    pub recommended: Option<String>,
}

/// The JSON object an interview turn may append to its prose.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterviewBlock {
    #[serde(default)]
    pub question: Option<DiscoveryQuestion>,
    /// The interviewer's belief that nothing is left to settle.
    ///
    /// Advisory, always: §5.1 of `docs/PRD_DISCOVERY.md` gives the user the
    /// decision to decompose, because a model that keeps finding one more
    /// question can otherwise hold the interview open indefinitely. Nothing
    /// downstream may read this as permission to act.
    #[serde(default)]
    pub nothing_left_to_settle: bool,
}

/// One assistant turn, split into what a human reads and what the UI renders
/// as a card.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterviewTurn {
    /// The turn text with the block cut out of it — whether or not that block
    /// turned out to be usable.
    pub prose: String,
    pub question: Option<DiscoveryQuestion>,
    pub nothing_left_to_settle: bool,
    /// Why the turn asked nothing the surface could offer.
    ///
    /// Set both by a block that parsed and was refused and by one that never
    /// parsed at all — the two are one event to a reader, who in either case
    /// was asked something they cannot answer. The block itself is cut out of
    /// [`prose`](Self::prose) either way: it is addressed to Demeteo, and a
    /// turn that renders its own JSON at the user has failed twice.
    #[serde(default)]
    pub question_error: Option<String>,
}

/// The shape the interviewer is asked to emit, and the shape an error message
/// quotes back at it.
pub fn interview_block_shape_example() -> String {
    let option = |id: &str| format!(r#"{{"id": "{id}", "label": "...", "description": "..."}}"#);
    format!(
        r#"{{"question": {{"header": "...", "text": "...", "options": [{}, {}], "recommended": "opt-a"}}, "nothing_left_to_settle": false}}"#,
        option("opt-a"),
        option("opt-b"),
    )
}

/// Reject a question the surface could not render or the user could not
/// answer. Returns a human-readable reason, or `None` when it is askable.
///
/// Every rule is a rendering or answering failure rather than a taste
/// judgement — whether the options are *genuinely different bets* is the
/// prompt's job and cannot be checked here.
pub fn validate_question(q: &DiscoveryQuestion) -> Option<String> {
    if q.header.trim().is_empty() {
        return Some("the question has no `header`; it labels the card and cannot be blank".into());
    }
    if q.text.trim().is_empty() {
        return Some("the question has no `text`".into());
    }
    if q.options.len() < MIN_OPTIONS {
        return Some(format!(
            "a question needs at least {MIN_OPTIONS} options; one option is a statement, not a choice"
        ));
    }
    if q.options.len() > MAX_OPTIONS {
        return Some(format!(
            "a question may offer at most {MAX_OPTIONS} options; it offered {}",
            q.options.len()
        ));
    }
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (i, opt) in q.options.iter().enumerate() {
        let id = opt.id.trim();
        if id.is_empty() {
            return Some(format!("option {} has no `id`", i + 1));
        }
        if !seen.insert(id) {
            return Some(format!(
                "option id '{id}' appears more than once; an answer naming it would be ambiguous"
            ));
        }
        if opt.label.trim().is_empty() {
            return Some(format!("option '{id}' has no `label`"));
        }
        if opt.description.trim().is_empty() {
            return Some(format!(
                "option '{id}' has no `description`; an option that does not say what it costs \
                 cannot be weighed against the others"
            ));
        }
    }
    match q.recommended.as_deref().map(str::trim) {
        Some("") => Some(
            "`recommended` is present but empty; omit it entirely when no option is worth \
             recommending"
                .into(),
        ),
        Some(id) if !seen.contains(id) => Some(format!(
            "`recommended` names '{id}', which is not one of the options"
        )),
        _ => None,
    }
}

/// Split an assistant turn into prose and the block it carried.
///
/// Tolerant about where the block sits, through
/// [`crate::domain::json_block`]. A turn with no block is prose — the
/// interviewer is not obliged to ask a question every turn.
///
/// A turn that *tried* to carry one and failed is not prose, though, which is
/// the difference [`refused_tail`] makes: the raw object would otherwise be
/// rendered at the reader as though the interviewer had said it.
pub fn parse_interview_turn(text: &str) -> InterviewTurn {
    if let Some((span, block)) = find_block(text) {
        let error = block.question.as_ref().and_then(validate_question);
        return InterviewTurn {
            prose: without(text, span),
            question: error.is_none().then_some(block.question).flatten(),
            nothing_left_to_settle: block.nothing_left_to_settle,
            question_error: error,
        };
    }
    match refused_tail(text) {
        Some((span, question_error)) => InterviewTurn {
            prose: without(text, span),
            question_error,
            ..Default::default()
        },
        None => InterviewTurn {
            prose: text.trim().to_string(),
            ..Default::default()
        },
    }
}

fn without(text: &str, span: (usize, usize)) -> String {
    let mut kept = String::with_capacity(text.len());
    kept.push_str(&text[..span.0]);
    kept.push_str(&text[span.1..]);
    kept.trim().to_string()
}

/// The block and the byte span it occupied, so the prose can be cut free of
/// it.
///
/// The accept rule is what stops an unrelated object answering as the block:
/// every field of an [`InterviewBlock`] is optional, so `{}` — or any JSON at
/// all — deserializes into a default one. A block that neither asks nor
/// signals is not a block.
fn find_block(text: &str) -> Option<((usize, usize), InterviewBlock)> {
    crate::domain::json_block::find_json_block(text, |block: &InterviewBlock| {
        block.question.is_some() || block.nothing_left_to_settle
    })
}

/// The keys that make a trailing object this turn's own block rather than
/// something the interviewer quoted. Both, because a turn may signal without
/// asking.
const DECLARED_KEYS: [&str; 2] = ["\"question\"", "\"nothing_left_to_settle\""];

/// A block the turn ended on that [`find_block`] would not take: the span to
/// cut, and what to tell the user about it.
///
/// The two ways to get here are a block that does not deserialize — a missing
/// `options`, a turn truncated mid-object — and one that deserializes into
/// nothing declared, which is `{"question": null}` and little else. Only the
/// first is worth a sentence; the second asked nothing and there is nothing to
/// report about it, but it is still JSON and still gets cut.
///
/// Naming one of [`DECLARED_KEYS`] is what makes this safe to run on every
/// turn with no block. Prose ends in a brace-wrapped identifier often enough
/// that position alone would accuse the interviewer of a malformed question
/// every time it signed off with `{feature_id}`.
fn refused_tail(text: &str) -> Option<((usize, usize), Option<String>)> {
    let (start, end) = crate::domain::json_block::trailing_object(text)?;
    let tail = &text[start..end];
    if !DECLARED_KEYS.iter().any(|key| tail.contains(key)) {
        return None;
    }
    let reason = serde_json::from_str::<InterviewBlock>(tail)
        .err()
        .map(|e| format!("the block it ended on could not be read as a question ({e})"));
    Some(((start, end), reason))
}

#[cfg(test)]
#[path = "../../tests/domain/discovery_question.rs"]
mod tests;

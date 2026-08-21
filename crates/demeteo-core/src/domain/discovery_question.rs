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
    /// The turn text with the block cut out of it, when there was a usable
    /// one to cut.
    pub prose: String,
    pub question: Option<DiscoveryQuestion>,
    pub nothing_left_to_settle: bool,
    /// Why a block that parsed was refused.
    ///
    /// The block stays in [`prose`](Self::prose) when this is set, so the turn
    /// renders as what the agent actually said rather than as nothing.
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
/// Tolerant in the same three ways, and the same order, as
/// `extract_task_plan` in `crates/demeteo-core/src/domain/sequence/tasks.rs`:
/// a ```json fence, any fence, then the first balanced top-level object. A
/// turn with no block, or with one that does not deserialize, is prose — the
/// interviewer is not obliged to ask a question every turn.
pub fn parse_interview_turn(text: &str) -> InterviewTurn {
    let Some((span, block)) = find_block(text) else {
        return InterviewTurn {
            prose: text.trim().to_string(),
            ..Default::default()
        };
    };
    let error = block.question.as_ref().and_then(validate_question);
    let prose = if error.is_some() {
        text.trim().to_string()
    } else {
        let mut kept = String::with_capacity(text.len());
        kept.push_str(&text[..span.0]);
        kept.push_str(&text[span.1..]);
        kept.trim().to_string()
    };
    InterviewTurn {
        prose,
        question: if error.is_some() {
            None
        } else {
            block.question
        },
        nothing_left_to_settle: block.nothing_left_to_settle,
        question_error: error,
    }
}

/// The block and the byte span it occupied, so the prose can be cut free of
/// it. A fenced block reports the span of the whole fence, which is what the
/// reader would otherwise be left staring at.
fn find_block(text: &str) -> Option<((usize, usize), InterviewBlock)> {
    if let Ok(block) = serde_json::from_str::<InterviewBlock>(text.trim()) {
        if block.question.is_some() || block.nothing_left_to_settle {
            return Some(((0, text.len()), block));
        }
    }
    for tag in ["```json", "```"] {
        let mut from = 0;
        while let Some(rel) = text[from..].find(tag) {
            let open = from + rel;
            let after = open + tag.len();
            let body_start = if tag == "```" {
                match text[after..].find('\n') {
                    Some(nl) => after + nl + 1,
                    None => break,
                }
            } else {
                after
            };
            if let Some(rel_end) = text[body_start..].find("```") {
                let body = text[body_start..body_start + rel_end].trim();
                if let Ok(block) = serde_json::from_str::<InterviewBlock>(body) {
                    if block.question.is_some() || block.nothing_left_to_settle {
                        return Some(((open, body_start + rel_end + 3), block));
                    }
                }
            }
            from = after;
        }
    }
    let (start, end) = crate::domain::sequence::tasks::find_top_level_object(text)?;
    let block = serde_json::from_str::<InterviewBlock>(&text[start..end]).ok()?;
    (block.question.is_some() || block.nothing_left_to_settle).then_some(((start, end), block))
}

#[cfg(test)]
#[path = "../../tests/domain/discovery_question.rs"]
mod tests;

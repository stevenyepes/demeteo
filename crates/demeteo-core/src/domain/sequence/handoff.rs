//! What one ticket's agent leaves for the next.
//!
//! A ticket prompt ends its reply with a `## Handoff` section. The agent that
//! runs the next ticket is a fresh session, and the branch history it is given
//! names ids, titles and files — nothing an agent *learned*. This section is
//! the one channel for that, so it is extracted from the reply and rendered
//! into the next ticket's "already done" block. Nothing else in the reply
//! travels, and the prompt says so.

use crate::domain::text::tail_chars;

/// The most of one handoff that reaches the next prompt. The head is kept:
/// an agent leads with what matters and trails into detail, and a note that
/// needs more than this is a spec change, not a handoff.
pub const MAX_HANDOFF_CHARS: usize = 600;

/// Pull the handoff section out of a ticket agent's reply.
///
/// The last `Handoff` heading wins — a reply that quotes its own prompt
/// carries an earlier one. The older prompt wording, a bullet reading
/// "anything a later ticket needs to know", is recognised too, so a stored
/// workflow authored against it starts carrying its notes forward without
/// an edit. `None` when there is no section, or when it says nothing.
pub fn extract_handoff(reply: &str) -> Option<String> {
    let lines: Vec<&str> = reply.lines().collect();
    let (idx, inline) = lines
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, line)| marker_body(line).map(|body| (i, body)))?;

    let mut body = String::new();
    if !inline.is_empty() {
        body.push_str(inline);
    }
    for line in &lines[idx + 1..] {
        if line.trim_start().starts_with('#') {
            break;
        }
        body.push('\n');
        body.push_str(line);
    }
    let body = body.trim();
    if body.is_empty() || says_nothing(body) {
        return None;
    }
    Some(cap(body))
}

/// `Some(rest)` when `line` opens a handoff section, carrying whatever text
/// follows the label on the same line (empty for a bare heading).
fn marker_body(line: &str) -> Option<&str> {
    let stripped = line
        .trim()
        .trim_start_matches(['#', '-', '*', '_', ' '])
        .trim_end_matches(['*', '_', ' ']);
    let lower = stripped.to_lowercase();
    let rest_after = |label_len: usize| -> &str {
        stripped[label_len..]
            .trim_start_matches(['*', '_', ' '])
            .trim_start_matches([':', '—', '-', ' '])
            .trim()
    };
    if lower.starts_with("handoff") || lower.starts_with("hand-off") {
        let len = if lower.starts_with("handoff") { 7 } else { 8 };
        return Some(rest_after(len));
    }
    if (lower.contains("later ticket") || lower.contains("later task")) && lower.contains("need") {
        let end = lower.find("know").map(|i| i + 4).unwrap_or(lower.len());
        return Some(rest_after(end));
    }
    None
}

fn says_nothing(body: &str) -> bool {
    let b = body
        .trim()
        .trim_end_matches('.')
        .trim_matches(['*', '_', '`', '-', ' '])
        .to_lowercase();
    matches!(b.as_str(), "" | "none" | "nothing" | "n/a" | "na" | "no")
}

fn cap(body: &str) -> String {
    let total = body.chars().count();
    if total <= MAX_HANDOFF_CHARS {
        return body.to_string();
    }
    let dropped = tail_chars(body, total - MAX_HANDOFF_CHARS);
    let kept: String = body.chars().take(MAX_HANDOFF_CHARS).collect();
    format!(
        "{}… [{} more characters not carried forward]",
        kept.trim_end(),
        dropped.chars().count()
    )
}

#[cfg(test)]
#[path = "../../../tests/domain/sequence/handoff.rs"]
mod tests;

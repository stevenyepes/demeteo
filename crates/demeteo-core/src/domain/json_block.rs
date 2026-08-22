//! Find the JSON object an agent turn carries, wherever in the turn it put it.
//!
//! Synchronous and total, per the rule on [`crate::domain`].
//!
//! A harness asked for JSON answers in three shapes and the caller does not
//! get to choose which: the whole turn is the object, the object sits in a
//! fence, or the object is somewhere inside prose. Every caller that reads a
//! declared block wants the same three, in the same order, so there is one of
//! them. `extract_task_plan` in
//! `crates/demeteo-core/src/domain/sequence/tasks.rs` is the precedent this
//! generalises and predates it; it kept a single-candidate scan of its own
//! until the shape below turned up on the sequence step too — a planner turn
//! naming a brace-wrapped identifier before emitting its task list — and it
//! now reads through [`find_json_block`] like every other caller.
//!
//! The span comes back with the value because a turn that is prose *plus* a
//! block has to be renderable as the prose alone, and only the search knows
//! where the block was.

use serde::de::DeserializeOwned;

/// The **last** block that both deserializes into `T` and satisfies `accept`,
/// with the byte span it occupied.
///
/// `accept` exists because deserializing is not recognising. A type whose
/// every field is optional matches any object at all, so the first fence
/// holding unrelated JSON would answer as the block and the real one further
/// down would never be reached. Returning `false` resumes the search past that
/// candidate rather than ending it — and so does failing to deserialize, which
/// is the same event one layer down: prose that mentions `{feature_id}` offers
/// a balanced object as the first candidate in the turn, and a search that
/// ended there never reached the block the turn was actually carrying.
///
/// Last rather than first, because every prompt that asks for a block quotes a
/// filled-in example of the shape before asking for the real thing.
/// `interview_block_shape_example` and `ticket_plan_json_shape_example` are
/// quoted into instructions that put the block at the very end and say nothing
/// follows it; `task_list_json_shape_example` is quoted into the planner's as a
/// complete one-task plan. A turn holding two acceptable candidates is
/// therefore one that discussed the shape before declaring it, and the
/// declaration is the one at the bottom. Preferring the first takes the
/// illustration — which for the task list is not a parse failure but a run: the
/// example deserializes into a plan of one task whose description is `...`.
///
/// A fenced candidate reports the span of the whole fence, not of its body:
/// the fence markers are what a reader would otherwise be left staring at.
pub(crate) fn find_json_block<T, A>(text: &str, accept: A) -> Option<((usize, usize), T)>
where
    T: DeserializeOwned,
    A: Fn(&T) -> bool,
{
    if let Ok(value) = serde_json::from_str::<T>(text.trim()) {
        if accept(&value) {
            return Some(((0, text.len()), value));
        }
    }
    for tag in ["```json", "```"] {
        let mut from = 0;
        let mut found = None;
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
                if let Ok(value) = serde_json::from_str::<T>(body) {
                    if accept(&value) {
                        found = Some(((open, body_start + rel_end + 3), value));
                    }
                }
            }
            from = after;
        }
        if found.is_some() {
            return found;
        }
    }
    let mut found = None;
    for (start, end) in top_level_objects(text) {
        if let Ok(value) = serde_json::from_str::<T>(&text[start..end]) {
            if accept(&value) {
                found = Some(((start, end), value));
            }
        }
    }
    found
}

/// The object a turn *ends* on, parsed or not — the span from the last
/// top-level `{` that reaches the end of the text.
///
/// What it is for is the failure the search above cannot report: a block the
/// agent meant as its declaration but got wrong is not a candidate at all, so
/// [`find_json_block`] answers `None` and the caller renders the turn whole,
/// JSON included, at a reader who was never meant to see it. Recognising the
/// tail is what lets that be said out loud instead.
///
/// It is deliberately positional and says nothing about content: whether a
/// tail is the block the caller wanted is the caller's to decide, and it is
/// the only one that knows what the block's own keys are called.
///
/// Unterminated counts. A turn cut off mid-object — the budget ran out, the
/// process was killed — is precisely the case worth naming, and it is the one
/// no balanced-brace scan can see.
pub(crate) fn trailing_object(text: &str) -> Option<(usize, usize)> {
    let ends_at = text.trim_end().len();
    let mut after_last = 0;
    let mut last = None;
    for (start, end) in top_level_objects(text) {
        last = Some((start, end));
        after_last = end;
    }
    match last {
        Some((start, end)) if end == ends_at => Some((start, end)),
        _ => unterminated_object(text, after_last).map(|start| (start, ends_at)),
    }
}

/// The (start, end) indices of the first top-level `{...}` object in `s`, `end`
/// exclusive (one past the matching `}`).
fn find_top_level_object(s: &str) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    let mut in_str = false;
    let mut escape = false;
    let mut depth: i32 = 0;
    let mut start: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if in_str {
            if b == b'\\' {
                escape = true;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                if depth > 0 {
                    depth -= 1;
                }
                if depth == 0 {
                    if let Some(st) = start {
                        if st < i {
                            return Some((st, i + 1));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Every balanced top-level object in `text`, left to right, none of them
/// inside another.
///
/// Resuming past a candidate rather than one byte into it is what bounds this:
/// an object nested in a rejected one is not reachable, which costs nothing
/// (both prompts ask for the block at the top level) and is what stops a turn
/// full of braces from being rescanned once per brace.
fn top_level_objects(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut from = 0;
    std::iter::from_fn(move || {
        let (start, end) = find_top_level_object(&text[from..])?;
        let found = (from + start, from + end);
        from = found.1;
        Some(found)
    })
}

/// Where an object that never closes begins, searching `text` from `from`.
///
/// String-aware for the same reason the balanced scan is: a brace inside
/// quotes is content, not structure. Prose is not JSON, so a quotation mark in
/// it opens a "string" that is nothing of the sort — which only ever hides a
/// brace from this, and hiding one is the conservative way to be wrong.
fn unterminated_object(text: &str, from: usize) -> Option<usize> {
    let mut in_str = false;
    let mut escape = false;
    for (i, &b) in text.as_bytes().iter().enumerate().skip(from) {
        if escape {
            escape = false;
        } else if in_str {
            match b {
                b'\\' => escape = true,
                b'"' => in_str = false,
                _ => {}
            }
        } else if b == b'"' {
            in_str = true;
        } else if b == b'{' {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
#[path = "../../tests/domain/json_block.rs"]
mod tests;

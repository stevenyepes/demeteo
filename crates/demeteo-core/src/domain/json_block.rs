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
//! generalises and predates it; it is left where it is because the sequence
//! step reads its result on every run, and folding it in would mean
//! re-proving that path for no behaviour anyone asked to change.
//!
//! The span comes back with the value because a turn that is prose *plus* a
//! block has to be renderable as the prose alone, and only the search knows
//! where the block was.

use serde::de::DeserializeOwned;

/// The first block that both deserializes into `T` and satisfies `accept`,
/// with the byte span it occupied.
///
/// `accept` exists because deserializing is not recognising. A type whose
/// every field is optional matches any object at all, so the first fence
/// holding unrelated JSON would answer as the block and the real one further
/// down would never be reached. Returning `false` resumes the search past that
/// candidate rather than ending it.
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
                        return Some(((open, body_start + rel_end + 3), value));
                    }
                }
            }
            from = after;
        }
    }
    let (start, end) = crate::domain::sequence::tasks::find_top_level_object(text)?;
    let value = serde_json::from_str::<T>(&text[start..end]).ok()?;
    accept(&value).then_some(((start, end), value))
}

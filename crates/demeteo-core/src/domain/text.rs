/// Remove `<think>…</think>` blocks and orphaned `</think>` closing tags from
/// agent text output. Extended-thinking models emit these as raw text deltas;
/// they are internal reasoning, not user-facing content.
///
/// Stripping is greedy: the first `<think>` matched with the next `</think>`
/// is removed, handling the common case of a single thinking block. Nested
/// `<think>` tags are not supported by any current model; if they appear,
/// only the outermost pair is stripped per pass (a second call would strip
/// the inner one, but that scenario doesn't arise in practice).
///
/// After balanced-pair stripping, any remaining orphaned `</think>` tags
/// (e.g. from a block that started before the text window) are also removed.
pub fn strip_think_tags(s: &str) -> String {
    const OPEN: &str = "<think>";
    const CLOSE: &str = "</think>";

    // Fast path: most agent turns (Claude Code, non-thinking models) never
    // emit think tags. Avoid the heap allocation for the majority case.
    if !s.contains(OPEN) && !s.contains(CLOSE) {
        return s.to_string();
    }

    let mut result = s.to_string();
    // Strip all balanced <think>...</think> spans.
    while let Some(start) = result.find(OPEN) {
        let search_from = start + OPEN.len();
        let Some(rel_end) = result[search_from..].find(CLOSE) else {
            // Unclosed open tag — remove from <think> to end of string so
            // partial thinking blocks don't leak either.
            result.truncate(start);
            break;
        };
        let end = search_from + rel_end + CLOSE.len();
        result.drain(start..end);
    }
    // Remove any orphaned </think> closing tags (thinking started before
    // the captured window, so no matching open tag is present).
    result.replace(CLOSE, "")
}

#[cfg(test)]
#[path = "../../tests/domain/text.rs"]
mod tests;

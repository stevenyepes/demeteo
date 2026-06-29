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
mod tests {
    use super::*;

    #[test]
    fn identity_when_no_tags() {
        assert_eq!(strip_think_tags("hello world"), "hello world");
    }

    #[test]
    fn empty_string() {
        assert_eq!(strip_think_tags(""), "");
    }

    #[test]
    fn strips_single_balanced_pair() {
        assert_eq!(
            strip_think_tags("<think>internal reasoning</think>answer"),
            "answer"
        );
    }

    #[test]
    fn strips_multiple_balanced_pairs() {
        assert_eq!(
            strip_think_tags("<think>a</think>mid<think>b</think>end"),
            "midend"
        );
    }

    #[test]
    fn strips_orphaned_closing_tag() {
        assert_eq!(
            strip_think_tags("</think>answer"),
            "answer"
        );
    }

    #[test]
    fn mixed_content_around_tag() {
        assert_eq!(
            strip_think_tags("prefix<think>thinking</think>suffix"),
            "prefixsuffix"
        );
    }

    #[test]
    fn unclosed_open_tag_truncates_from_open() {
        // An unclosed <think> means the rest is internal reasoning.
        assert_eq!(
            strip_think_tags("visible<think>never shown"),
            "visible"
        );
    }

    #[test]
    fn multiple_orphaned_closing_tags() {
        assert_eq!(
            strip_think_tags("</think></think>actual output"),
            "actual output"
        );
    }

    #[test]
    fn real_world_hermes_pattern() {
        let input = "</think></think></think></think></think>Research report written to `artifacts/research-report.md`";
        assert_eq!(
            strip_think_tags(input),
            "Research report written to `artifacts/research-report.md`"
        );
    }
}

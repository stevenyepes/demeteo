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

/// Scan a full agent turn's text for a JSON object carrying `key`, and
/// return it.
///
/// Agents are asked to answer with "ONLY a JSON object", and reliably
/// don't: they wrap it in prose, fence it as ```json, or precede it with a
/// `<think>` block. This is the one place that tolerance lives — the
/// verifier's pass/fail verdict, the harness triage classifier, and the
/// finalize step's commit/PR authoring all read their structured answer
/// through it.
///
/// Nested objects are searched too, so a model that wraps its answer
/// (`{"result": {"verdict": "pass"}}`) is still understood: a balanced
/// span that parses but lacks `key` is stepped *into* rather than over.
pub fn find_json_object_with_key(raw_text: &str, key: &str) -> Option<serde_json::Value> {
    let text = strip_think_tags(raw_text);
    let bytes = text.as_bytes();
    let mut found: Option<serde_json::Value> = None;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(close) = find_matching_close_brace(bytes, i) {
                match serde_json::from_str::<serde_json::Value>(&text[i..=close]) {
                    Ok(val) if val.is_object() && val.get(key).is_some() => {
                        found = Some(val);
                        i = close + 1;
                        continue;
                    }
                    // Valid JSON but no `key` at the top level: step forward
                    // by one so any nested object is evaluated on its own.
                    Ok(_) => {}
                    // Balanced braces but not valid JSON: skip the whole span
                    // rather than re-parsing every prefix of it (O(n²)).
                    Err(_) => {
                        i = close + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    found
}

/// Find the index of the `}` that closes the `{` at `start` in `bytes`,
/// correctly skipping over string literals (including escaped characters).
/// Returns `None` if the braces are unbalanced.
pub(crate) fn find_matching_close_brace(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut in_str = false;
    let mut escaped = false;
    for (offset, &b) in bytes[start..].iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_str {
            match b {
                b'\\' => escaped = true,
                b'"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
#[path = "../../tests/domain/text.rs"]
mod tests;

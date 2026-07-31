// Tests extracted from `crates/demeteo-core/src/domain/text.rs` (mirrored-tests convention). `super` = that module.

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
    assert_eq!(strip_think_tags("</think>answer"), "answer");
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
    assert_eq!(strip_think_tags("visible<think>never shown"), "visible");
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

// ── find_json_object_with_key ────────────────────────────────────────────
// The tolerance every structured agent answer depends on: the verifier's
// verdict, the harness triage classifier, and the finalize step's commit /
// PR authoring all read their JSON through this one scan.

#[test]
fn finds_a_bare_json_object() {
    let val = find_json_object_with_key(r#"{"pr_title": "feat: x"}"#, "pr_title").unwrap();
    assert_eq!(val["pr_title"], "feat: x");
}

#[test]
fn finds_json_wrapped_in_prose_and_a_code_fence() {
    let raw = "Sure! Here's the summary:\n\n```json\n{\"pr_title\": \"feat: x\", \
               \"pr_body\": \"why\"}\n```\n\nLet me know if you'd like changes.";
    let val = find_json_object_with_key(raw, "pr_title").unwrap();
    assert_eq!(val["pr_body"], "why");
}

#[test]
fn finds_json_after_a_thinking_block() {
    let raw = "<think>I should use a feat: prefix here.</think>{\"pr_title\": \"feat: x\"}";
    let val = find_json_object_with_key(raw, "pr_title").unwrap();
    assert_eq!(val["pr_title"], "feat: x");
}

#[test]
fn steps_into_a_nested_object_when_the_model_wraps_its_answer() {
    let raw = r#"{"result": {"pr_title": "feat: x"}}"#;
    let val = find_json_object_with_key(raw, "pr_title").unwrap();
    assert_eq!(val["pr_title"], "feat: x");
}

/// Braces inside string values must not end the span early — a commit body
/// quoting code (`fn main() { … }`) is the common case.
#[test]
fn tolerates_braces_and_escaped_quotes_inside_string_values() {
    let raw = r#"{"pr_title": "fix: handle {} in body", "pr_body": "he said \"hi\" { nested }"}"#;
    let val = find_json_object_with_key(raw, "pr_title").unwrap();
    assert_eq!(val["pr_title"], "fix: handle {} in body");
    assert_eq!(val["pr_body"], r#"he said "hi" { nested }"#);
}

/// A malformed object earlier in the text must not shadow the real one.
#[test]
fn skips_a_malformed_object_and_finds_the_valid_one() {
    let raw = "{not json at all} then {\"pr_title\": \"feat: x\"}";
    let val = find_json_object_with_key(raw, "pr_title").unwrap();
    assert_eq!(val["pr_title"], "feat: x");
}

#[test]
fn returns_none_when_the_key_is_absent() {
    assert!(find_json_object_with_key(r#"{"other": 1}"#, "pr_title").is_none());
    assert!(find_json_object_with_key("no json here", "pr_title").is_none());
}

// ── tail_chars ──────────────────────────────────────────────────────────

#[test]
fn returns_input_unchanged_when_under_limit() {
    assert_eq!(tail_chars("short output", 2000), "short output");
}

#[test]
fn returns_input_unchanged_when_exactly_at_limit() {
    let s = "x".repeat(2000);
    assert_eq!(tail_chars(&s, 2000), s);
}

#[test]
fn keeps_the_tail_not_the_head() {
    // The failing assertion lives at the end of a long build log —
    // the truncated output must contain it, not the install banner.
    let head = "npm install banner...\n".repeat(200);
    let tail =
        "\nFAIL src/foo.test.ts\n  ✕ should do the thing\nAssertionError: expected 1 to be 2";
    let full = format!("{head}{tail}");
    let max = tail.chars().count();
    let truncated = tail_chars(&full, max);
    assert_eq!(
        truncated, tail,
        "expected exactly the tail (no banner leakage) when max == tail length"
    );
}

#[test]
fn truncated_length_matches_max() {
    let s = "a".repeat(5000);
    let truncated = tail_chars(&s, 2000);
    assert_eq!(truncated.chars().count(), 2000);
}

#[test]
fn respects_char_boundaries_with_multibyte_content() {
    // Every char is 3 bytes (multi-byte UTF-8); a byte-oriented slice
    // (e.g. naive `s[s.len() - max..]`) would panic mid-character.
    let s = "€".repeat(3000);
    let truncated = tail_chars(&s, 2000);
    assert_eq!(truncated.chars().count(), 2000);
    assert!(truncated.chars().all(|c| c == '€'));
}

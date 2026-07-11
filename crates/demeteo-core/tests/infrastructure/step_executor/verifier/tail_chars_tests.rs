// Tests extracted from `src/adapters/step_executor/driver/verifier.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::tail_chars;

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

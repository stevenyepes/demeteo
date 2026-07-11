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

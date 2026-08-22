// Tests extracted from `crates/demeteo-core/src/domain/json_block.rs` (mirrored-tests convention). `super` = that module.

use super::*;

use serde::Deserialize;

/// Stands in for the declared blocks the real callers read: every field
/// optional, so anything at all deserializes into it and `accept` is the only
/// thing telling a block from an object that merely parsed.
#[derive(Debug, Default, Deserialize)]
struct Block {
    #[serde(default)]
    question: Option<String>,
}

fn find(text: &str) -> Option<((usize, usize), Block)> {
    find_json_block(text, |block: &Block| block.question.is_some())
}

/// The turn that found this: its prose named `{feature_id}` and a glob over
/// `{feature_branch}{SUBTASK_INFIX}*` before it declared anything, so the
/// first balanced object in the text was an identifier in braces. A search
/// that stopped at the first candidate rendered the real block as prose.
#[test]
fn an_identifier_in_braces_does_not_end_the_search() {
    let text = "A subtask id is `{feature_id}-step-{step_id}`, and the sweep globs \
                `{feature_branch}{SUBTASK_INFIX}*`.\n{\"question\": \"Rework collision\"}";

    let ((start, end), block) = find(text).expect("the trailing block is the block");
    assert_eq!(block.question.as_deref(), Some("Rework collision"));
    assert_eq!(&text[start..end], "{\"question\": \"Rework collision\"}");
}

/// A candidate that parses but is refused is the same event one layer up, and
/// the rustdoc promises both resume the search.
#[test]
fn a_refused_candidate_does_not_end_the_search_either() {
    let text = "{\"question\": null} then {\"question\": \"the real one\"}";
    let (_, block) = find(text).expect("the accepted candidate is further down");
    assert_eq!(block.question.as_deref(), Some("the real one"));
}

/// Both prompts put the block last and say nothing follows it, so a turn that
/// carries two is one that illustrated the shape before declaring it.
#[test]
fn the_last_acceptable_candidate_wins() {
    let fenced = "It looks like ```json\n{\"question\": \"an example\"}\n``` — here is mine.\n\
                  ```json\n{\"question\": \"the declaration\"}\n```";
    let bare = "Like {\"question\": \"an example\"}. Mine: {\"question\": \"the declaration\"}";
    for text in [fenced, bare] {
        let (_, block) = find(text).expect("a block is there");
        assert_eq!(block.question.as_deref(), Some("the declaration"), "{text}");
    }
}

/// The whole turn being the object is the first shape a harness answers in,
/// and it stays first: nothing is cut from a turn that is nothing but block.
#[test]
fn a_turn_that_is_only_the_block_reports_the_whole_span() {
    let text = "  {\"question\": \"only this\"}  ";
    let ((start, end), _) = find(text).expect("the whole turn is the block");
    assert_eq!((start, end), (0, text.len()));
}

#[test]
fn prose_with_no_object_in_it_is_no_block() {
    assert!(find("Taking that as written rather than fitting it to an option.").is_none());
}

#[test]
fn the_object_a_turn_ends_on_is_the_trailing_one() {
    let text = "First {\"a\": 1} then {\"question\": \"last\"}";
    let (start, end) = trailing_object(text).expect("the turn ends on an object");
    assert_eq!(&text[start..end], "{\"question\": \"last\"}");
}

/// The case no balanced-brace scan can see: a turn cut off mid-object.
#[test]
fn a_tail_that_never_closes_is_still_the_object_the_turn_ends_on() {
    let text = "Here it is.\n{\"question\": {\"header\": \"Identity\"\n";
    let (start, end) = trailing_object(text).expect("an unterminated tail counts");
    assert_eq!(
        &text[start..end],
        "{\"question\": {\"header\": \"Identity\""
    );
}

/// Positional and nothing more: a turn that closed its object and then kept
/// talking did not end on one.
#[test]
fn prose_after_the_object_means_the_turn_did_not_end_on_it() {
    assert!(trailing_object("{\"question\": \"x\"} — that is the shape.").is_none());
    assert!(trailing_object("No object here at all.").is_none());
}

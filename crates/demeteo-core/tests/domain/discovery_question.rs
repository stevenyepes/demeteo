// Tests extracted from `crates/demeteo-core/src/domain/discovery_question.rs` (mirrored-tests convention). `super` = that module.

use super::*;

fn option(id: &str) -> QuestionOption {
    QuestionOption {
        id: id.to_string(),
        label: format!("label {id}"),
        description: format!("what {id} costs"),
    }
}

fn question() -> DiscoveryQuestion {
    DiscoveryQuestion {
        header: "Identity".to_string(),
        text: "How should a client prove who it is?".to_string(),
        options: vec![option("keypair"), option("shared-token")],
        recommended: Some("keypair".to_string()),
    }
}

#[test]
fn the_shape_example_is_what_the_parser_accepts() {
    let turn = parse_interview_turn(&interview_block_shape_example());
    assert_eq!(turn.question.map(|q| q.options.len()), Some(2));
    assert_eq!(turn.question_error, None);
}

#[test]
fn a_fenced_block_is_lifted_out_of_the_prose() {
    let text = format!(
        "The sketch settles the transport and nothing else.\n\n```json\n{}\n```\n",
        serde_json::to_string(&InterviewBlock {
            question: Some(question()),
            nothing_left_to_settle: false,
        })
        .unwrap()
    );
    let turn = parse_interview_turn(&text);
    assert_eq!(
        turn.prose,
        "The sketch settles the transport and nothing else."
    );
    assert_eq!(
        turn.question.as_ref().map(|q| q.header.as_str()),
        Some("Identity")
    );
    assert!(!turn.prose.contains("keypair"));
}

#[test]
fn a_bare_trailing_object_is_lifted_out_of_the_prose() {
    let text = format!(
        "Two things it leaves open.\n{}",
        serde_json::to_string(&InterviewBlock {
            question: Some(question()),
            nothing_left_to_settle: false,
        })
        .unwrap()
    );
    let turn = parse_interview_turn(&text);
    assert_eq!(turn.prose, "Two things it leaves open.");
    assert!(turn.question.is_some());
}

#[test]
fn a_turn_with_no_block_is_prose() {
    let turn = parse_interview_turn(
        "Taking that as written rather than fitting it to the nearest option.",
    );
    assert_eq!(
        turn.prose,
        "Taking that as written rather than fitting it to the nearest option."
    );
    assert!(turn.question.is_none());
    assert!(!turn.nothing_left_to_settle);
    assert!(turn.question_error.is_none());
}

#[test]
fn the_advisory_signal_rides_without_a_question() {
    let text = "That is the whole shape.\n```json\n{\"nothing_left_to_settle\": true}\n```";
    let turn = parse_interview_turn(text);
    assert!(turn.nothing_left_to_settle);
    assert!(turn.question.is_none());
    assert_eq!(turn.prose, "That is the whole shape.");
}

/// A refused block is still cut: the reason reaches the reader as a sentence,
/// and the JSON that caused it was addressed to Demeteo.
#[test]
fn a_refused_question_is_reported_rather_than_rendered() {
    let mut q = question();
    q.recommended = Some("nonexistent".to_string());
    let text = format!(
        "Prose.\n```json\n{}\n```",
        serde_json::to_string(&InterviewBlock {
            question: Some(q),
            nothing_left_to_settle: false,
        })
        .unwrap()
    );
    let turn = parse_interview_turn(&text);
    assert!(turn.question.is_none());
    assert!(turn.question_error.unwrap().contains("nonexistent"));
    assert_eq!(turn.prose, "Prose.");
}

/// The turn this was found on: its prose named `{feature_id}` and globbed
/// `{feature_branch}{SUBTASK_INFIX}*`, so the first balanced object in it was
/// an identifier in braces. The search stopped there and the real block was
/// rendered at the user as raw JSON.
#[test]
fn braces_in_the_prose_do_not_hide_the_block_below_them() {
    let text = format!(
        "A subtask id is `{{feature_id}}-step-{{step_id}}`, and the sweep globs \
         `{{feature_branch}}{{SUBTASK_INFIX}}*`.\n\n{}",
        serde_json::to_string(&InterviewBlock {
            question: Some(question()),
            nothing_left_to_settle: false,
        })
        .unwrap()
    );
    let turn = parse_interview_turn(&text);
    assert_eq!(
        turn.question.as_ref().map(|q| q.header.as_str()),
        Some("Identity")
    );
    assert!(turn.prose.contains("{feature_id}"));
    assert!(!turn.prose.contains("keypair"));
}

/// A block that never deserializes is the case the accept rule cannot see —
/// it is not a candidate at all — and the fallback used to be rendering it.
#[test]
fn a_block_that_will_not_parse_is_reported_rather_than_rendered() {
    let text = "Two things it leaves open.\n{\"question\": {\"header\": \"Rework collision\"}}";
    let turn = parse_interview_turn(text);
    assert_eq!(turn.prose, "Two things it leaves open.");
    assert!(turn.question.is_none());
    assert!(turn.question_error.is_some());
}

/// A turn cut off mid-block: the tail never closes, so nothing balanced-brace
/// can find it, and it is the shape most likely to be truncated JSON.
#[test]
fn a_truncated_block_is_reported_rather_than_rendered() {
    let text = "Two things it leaves open.\n{\"question\": {\"header\": \"Rework";
    let turn = parse_interview_turn(text);
    assert_eq!(turn.prose, "Two things it leaves open.");
    assert!(turn.question_error.is_some());
}

/// The other half of that rule: a turn signing off with an identifier in
/// braces is prose, and accusing it of a malformed question would be the
/// false positive this is measured by.
#[test]
fn prose_ending_in_braces_is_not_read_as_a_failed_block() {
    let turn = parse_interview_turn("The branch is `{feature_branch}`");
    assert_eq!(turn.prose, "The branch is `{feature_branch}`");
    assert!(turn.question_error.is_none());
}

#[test]
fn a_recommendation_must_name_an_option() {
    let mut q = question();
    q.recommended = Some("neither".to_string());
    assert!(validate_question(&q).unwrap().contains("neither"));
}

#[test]
fn an_empty_recommendation_is_refused_rather_than_read_as_none() {
    let mut q = question();
    q.recommended = Some("   ".to_string());
    assert!(validate_question(&q).unwrap().contains("omit it"));
}

#[test]
fn no_recommendation_is_a_legal_question() {
    let mut q = question();
    q.recommended = None;
    assert_eq!(validate_question(&q), None);
}

#[test]
fn one_option_is_not_a_question() {
    let mut q = question();
    q.options.truncate(1);
    assert!(validate_question(&q).is_some());
}

#[test]
fn more_options_than_the_surface_can_key_is_refused() {
    let mut q = question();
    q.options = (0..MAX_OPTIONS + 1)
        .map(|i| option(&format!("o{i}")))
        .collect();
    q.recommended = Some("o0".to_string());
    assert!(validate_question(&q).unwrap().contains("at most"));
}

#[test]
fn an_option_with_no_description_is_refused() {
    let mut q = question();
    q.options[1].description = "  ".to_string();
    assert!(validate_question(&q).unwrap().contains("shared-token"));
}

#[test]
fn duplicate_option_ids_are_refused() {
    let mut q = question();
    q.options[1].id = "keypair".to_string();
    assert!(validate_question(&q).unwrap().contains("more than once"));
}

#[test]
fn a_blank_header_is_refused() {
    let mut q = question();
    q.header = " ".to_string();
    assert!(validate_question(&q).is_some());
}

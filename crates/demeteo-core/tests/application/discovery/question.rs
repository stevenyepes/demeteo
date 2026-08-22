// Tests extracted from `crates/demeteo-core/src/application/discovery/question.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::ids::DiscoveryId;

fn said(role: MessageRole, content: &str) -> DiscoveryMessage {
    DiscoveryMessage {
        id: content.to_string(),
        discovery_id: DiscoveryId::from("d-1".to_string()),
        role,
        content: content.to_string(),
        cost_usd: None,
        tokens: None,
        activity: None,
        created_at: 0,
    }
}

#[test]
fn the_preamble_quotes_the_one_shape_the_parser_accepts() {
    assert!(interview_preamble().contains(&interview_block_shape_example()));
}

#[test]
fn a_resumed_turn_carries_neither_the_preamble_nor_the_transcript() {
    let transcript = [said(MessageRole::User, "the old sketch")];
    let rendered = render_turn_prompt(TurnPrompt {
        reseed: false,
        context: "WHAT ELSE IS GOING ON",
        transcript: &transcript,
        attachments: &[],
        reads_images: true,
        user_text: "and now?",
    });
    assert!(!rendered.contains("You are conducting a planning interview"));
    assert!(!rendered.contains("the old sketch"));
    assert!(rendered.contains("WHAT ELSE IS GOING ON"));
    assert!(rendered.ends_with("USER: and now?"));
}

#[test]
fn a_reseeded_turn_carries_the_whole_transcript_and_says_it_is_the_authority() {
    let transcript = [
        said(MessageRole::User, "the old sketch"),
        said(MessageRole::Assistant, "two things it leaves open"),
    ];
    let rendered = render_turn_prompt(TurnPrompt {
        reseed: true,
        context: "",
        transcript: &transcript,
        attachments: &[],
        reads_images: true,
        user_text: "and now?",
    });
    assert!(rendered.contains("You are conducting a planning interview"));
    assert!(rendered.contains("USER: the old sketch"));
    assert!(rendered.contains("YOU: two things it leaves open"));
    assert!(rendered.contains("It is the authority on what was said"));
}

#[test]
fn a_reseeded_first_turn_skips_the_transcript_heading_it_has_nothing_for() {
    let rendered = render_turn_prompt(TurnPrompt {
        reseed: true,
        context: "",
        transcript: &[],
        attachments: &[],
        reads_images: true,
        user_text: "I want the runner to serve more than one client.",
    });
    assert!(!rendered.contains("THE CONVERSATION SO FAR"));
    assert!(rendered.ends_with("USER: I want the runner to serve more than one client."));
}

#[test]
fn the_context_block_is_rebuilt_on_a_resumed_turn_too() {
    let rendered = render_turn_prompt(TurnPrompt {
        reseed: false,
        context: "TICKETS THIS CONVERSATION HAS ALREADY PRODUCED",
        transcript: &[],
        attachments: &[],
        reads_images: true,
        user_text: "?",
    });
    assert!(rendered.contains("TICKETS THIS CONVERSATION HAS ALREADY PRODUCED"));
}

fn dropped(name: &str, mime: &str) -> crate::domain::attachment::AttachedFile {
    crate::domain::attachment::AttachedFile {
        id: format!("at-{name}"),
        name: name.to_string(),
        mime: mime.to_string(),
        sha256: "b".repeat(64),
        size: 4,
        source_filename: name.to_string(),
    }
}

/// §4.6's files reach a resumed turn too. The harness may hold the whole
/// conversation and still be told about a file added since the last one — and
/// a file removed since must stop being offered, which only a per-turn block
/// can do.
#[test]
fn the_attachment_block_rides_on_every_turn_and_names_the_open_question_of_vision() {
    let rendered = render_turn_prompt(TurnPrompt {
        reseed: false,
        context: "",
        transcript: &[],
        attachments: &[dropped("wire.png", "image/png")],
        reads_images: false,
        user_text: "does this fit?",
    });
    assert!(rendered.contains("Attached: [attachment -- wire.png]"));
    assert!(rendered.contains("does not read images"));
    assert!(rendered.ends_with("USER: does this fit?"));
}

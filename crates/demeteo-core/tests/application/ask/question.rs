// Tests extracted from `crates/demeteo-core/src/application/ask/question.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::ids::AskThreadId;

fn message(role: MessageRole, text: &str) -> AskMessage {
    AskMessage {
        id: "m-1".to_string(),
        thread_id: AskThreadId::from("t-1".to_string()),
        role,
        text: text.to_string(),
        cost_usd: None,
        tokens: None,
        turn_activity: None,
        canvas_paths: None,
        checked_commit_sha: None,
        created_at: 0,
    }
}

#[test]
fn a_reseed_carries_the_preamble_and_the_transcript() {
    let transcript = [
        message(MessageRole::User, "what does the scope fence do?"),
        message(MessageRole::Assistant, "it denies writes outside the tree"),
    ];
    let rendered = render_turn_prompt(TurnPrompt {
        reseed: true,
        context: "",
        transcript: &transcript,
        user_text: "and on remote hosts?",
    });

    assert!(rendered.contains("answering questions inside Demeteo"));
    assert!(rendered.contains(&canvas_block_shape_example()));
    assert!(rendered.contains(&canvas_block_vocabulary()));
    assert!(rendered.contains("USER: what does the scope fence do?"));
    assert!(rendered.contains("YOU: it denies writes outside the tree"));
    assert!(rendered.ends_with("USER: and on remote hosts?"));
}

#[test]
fn a_resumed_turn_carries_neither_preamble_nor_transcript() {
    let transcript = [message(MessageRole::User, "earlier question")];
    let rendered = render_turn_prompt(TurnPrompt {
        reseed: false,
        context: "",
        transcript: &transcript,
        user_text: "a follow-up",
    });

    assert!(!rendered.contains("answering questions inside Demeteo"));
    assert!(!rendered.contains("earlier question"));
    assert!(rendered.ends_with("USER: a follow-up"));
}

#[test]
fn the_context_block_renders_whether_or_not_the_turn_reseeds() {
    let rendered = render_turn_prompt(TurnPrompt {
        reseed: false,
        context: "WHAT ELSE IS GOING ON\n\nsomething",
        transcript: &[],
        user_text: "hi",
    });
    assert!(rendered.contains("WHAT ELSE IS GOING ON"));
    assert!(rendered.contains("something"));
}

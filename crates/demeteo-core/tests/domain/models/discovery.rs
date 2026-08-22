// Tests extracted from `crates/demeteo-core/src/domain/models/discovery.rs` (mirrored-tests convention). `super` = that module.

use super::*;

use crate::domain::agent_event::{StopReason, ToolCallStatus};

fn call(action: ActionKind, target: &str) -> AgentEvent {
    AgentEvent::ToolCall {
        tool_call_id: format!("t-{target}"),
        intercept_id: "i".to_string(),
        action,
        target: target.to_string(),
        preview: None,
    }
}

#[test]
fn a_turn_that_did_nothing_says_so() {
    let activity = TurnActivity::default();
    assert!(activity.is_empty());
}

#[test]
fn each_action_lands_on_its_own_counter() {
    let mut activity = TurnActivity::default();
    activity.observe(&call(ActionKind::Read, "src/a.rs"));
    activity.observe(&call(ActionKind::Read, "src/b.rs"));
    activity.observe(&call(ActionKind::Edit, "src/a.rs"));
    activity.observe(&call(ActionKind::Write, "src/c.rs"));
    activity.observe(&call(ActionKind::RunBash, "cargo metadata"));

    assert_eq!(activity.reads, 2);
    assert_eq!(activity.edits, 1);
    assert_eq!(activity.writes, 1);
    assert_eq!(activity.ran, 1);
    assert_eq!(activity.commands, vec!["cargo metadata".to_string()]);
    assert!(!activity.is_empty());
}

#[test]
fn nothing_but_a_tool_call_is_counted() {
    let mut activity = TurnActivity::default();
    activity.observe(&AgentEvent::Text {
        delta: "reading".to_string(),
    });
    activity.observe(&AgentEvent::ToolCallUpdate {
        tool_call_id: "t".to_string(),
        status: ToolCallStatus::Completed,
        preview: None,
    });
    activity.observe(&AgentEvent::TurnComplete {
        stop_reason: StopReason::EndOfTurn,
        usage: None,
    });
    assert!(activity.is_empty());
}

#[test]
fn the_same_command_twice_is_named_once_and_counted_twice() {
    let mut activity = TurnActivity::default();
    activity.observe(&call(ActionKind::RunBash, "rg discovery"));
    activity.observe(&call(ActionKind::RunBash, "rg discovery"));
    assert_eq!(activity.ran, 2);
    assert_eq!(activity.commands.len(), 1);
}

#[test]
fn the_command_sample_is_bounded_but_the_count_is_not() {
    let mut activity = TurnActivity::default();
    for i in 0..(TurnActivity::MAX_COMMANDS + 4) {
        activity.observe(&call(ActionKind::RunBash, &format!("cmd-{i}")));
    }
    assert_eq!(activity.commands.len(), TurnActivity::MAX_COMMANDS);
    assert_eq!(activity.ran as usize, TurnActivity::MAX_COMMANDS + 4);
}

#[test]
fn a_script_shaped_command_is_kept_as_its_first_line_and_capped() {
    let mut activity = TurnActivity::default();
    let script = format!("{}\nrm -rf /\n", "e".repeat(400));
    activity.observe(&call(ActionKind::RunBash, &script));

    let kept = &activity.commands[0];
    assert_eq!(kept.chars().count(), TurnActivity::MAX_COMMAND_CHARS);
    assert!(!kept.contains('\n'));
    assert!(!kept.contains("rm -rf"));
}

#[test]
fn a_blank_command_is_not_remembered() {
    let mut activity = TurnActivity::default();
    activity.observe(&call(ActionKind::RunBash, "   "));
    assert_eq!(activity.ran, 1);
    assert!(activity.commands.is_empty());
}

/// The surface reads these keys by name; renaming one silently empties the
/// meta line rather than failing anything.
#[test]
fn the_wire_shape_is_the_one_the_surface_reads() {
    let mut activity = TurnActivity::default();
    activity.observe(&call(ActionKind::Read, "src/a.rs"));
    activity.observe(&call(ActionKind::RunBash, "git log --oneline"));

    let json = serde_json::to_value(&activity).expect("serializable");
    assert_eq!(json["reads"], 1);
    assert_eq!(json["edits"], 0);
    assert_eq!(json["writes"], 0);
    assert_eq!(json["ran"], 1);
    assert_eq!(json["commands"][0], "git log --oneline");
}

/// A row written before V49 carries no activity, and reads back as absent
/// rather than as a turn that did nothing.
#[test]
fn a_message_stored_without_activity_reads_as_none() {
    let stored = serde_json::json!({
        "id": "m-1",
        "discovery_id": "d-1",
        "role": "assistant",
        "content": "hello",
        "created_at": 1,
    });
    let message: DiscoveryMessage = serde_json::from_value(stored).expect("deserializable");
    assert!(message.activity.is_none());
}

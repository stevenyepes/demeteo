// Tests extracted from `crates/demeteo-core/src/domain/models/ask.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn status_as_str_parse_round_trip() {
    for status in [AskStatus::Open, AskStatus::Closed] {
        assert_eq!(AskStatus::parse(status.as_str()), Some(status));
    }
}

#[test]
fn status_parse_rejects_unknown_values() {
    for garbage in ["", "OPEN", "pending", "closed ", "archived"] {
        assert_eq!(AskStatus::parse(garbage), None, "accepted {garbage:?}");
    }
}

#[test]
fn status_serde_uses_the_canonical_lowercase_spelling() {
    assert_eq!(serde_json::to_string(&AskStatus::Open).unwrap(), "\"open\"");
    assert_eq!(
        serde_json::from_str::<AskStatus>("\"closed\"").unwrap(),
        AskStatus::Closed
    );
}

/// A row whose `turn_activity_json` column is absent reads back as an
/// absent activity, not as a turn that did nothing — mirrors
/// `DiscoveryMessage`'s `a_message_stored_without_activity_reads_as_none`.
#[test]
fn a_message_stored_without_turn_activity_reads_as_none() {
    let stored = serde_json::json!({
        "id": "m-1",
        "thread_id": "t-1",
        "role": "assistant",
        "text": "hello",
        "created_at": 1,
    });
    let message: AskMessage = serde_json::from_value(stored).expect("deserializable");
    assert!(message.turn_activity.is_none());
    assert!(message.cost_usd.is_none());
    assert!(message.tokens.is_none());
}

/// A thread row written before the nullable/roll-up columns had values
/// present reads back with zeroed telemetry and absent optionals, rather
/// than failing to deserialize.
#[test]
fn a_thread_stored_with_only_required_fields_defaults_the_rest() {
    let stored = serde_json::json!({
        "id": "t-1",
        "project_id": "p-1",
        "title": "Quick question",
        "status": "open",
        "agent_kind": "claude-code",
        "machine_id": "local",
        "created_at": 1,
        "updated_at": 1,
    });
    let thread: AskThread = serde_json::from_value(stored).expect("deserializable");
    assert!(thread.model.is_none());
    assert!(thread.effort.is_none());
    assert!(thread.worktree_path.is_none());
    assert!(thread.session_id.is_none());
    assert_eq!(thread.turn_count, 0);
    assert_eq!(thread.cost_usd, 0.0);
    assert_eq!(thread.tokens, 0);
}

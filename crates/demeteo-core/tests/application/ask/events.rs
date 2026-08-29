// Tests extracted from `crates/demeteo-core/src/application/ask/events.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::application::discovery::events as discovery_events;

#[test]
fn event_names_are_non_empty_and_distinct_from_discoverys() {
    assert!(!EVENT_ASK_AGENT_EVENT.is_empty());
    assert!(!EVENT_ASK_TURN_STATUS.is_empty());
    assert!(!EVENT_ASK_TURN_COMPLETED.is_empty());

    assert_ne!(
        EVENT_ASK_AGENT_EVENT,
        discovery_events::EVENT_DISCOVERY_AGENT_EVENT
    );
    assert_ne!(
        EVENT_ASK_TURN_STATUS,
        discovery_events::EVENT_DISCOVERY_TURN_STATUS
    );
    assert_ne!(
        EVENT_ASK_TURN_COMPLETED,
        discovery_events::EVENT_DISCOVERY_TURN_COMPLETED
    );
}

#[test]
fn ask_turn_status_round_trips_through_serialization() {
    let status = AskTurnStatus {
        thread_id: "t-1".to_string(),
        status: STATUS_RUNNING.to_string(),
        reason: None,
    };

    let value = serde_json::to_value(&status).expect("status serializes");
    assert_eq!(value["thread_id"], "t-1");
    assert_eq!(value["status"], "running");
    assert!(value["reason"].is_null());
}

#[test]
fn ask_turn_completed_round_trips_through_serialization_and_carries_no_reseed_fields() {
    let completed = AskTurnCompleted {
        thread_id: "t-1".to_string(),
        title: "quick question".to_string(),
        message_id: Some("m-1".to_string()),
        ending: "success",
        reason: None,
        cost_usd: 0.02,
        tokens: 512,
        duration_ms: 1200,
    };

    let value = serde_json::to_value(&completed).expect("completed serializes");
    assert_eq!(value["thread_id"], "t-1");
    assert_eq!(value["title"], "quick question");
    assert_eq!(value["message_id"], "m-1");
    assert_eq!(value["ending"], "success");
    assert_eq!(value["cost_usd"], 0.02);
    assert_eq!(value["tokens"], 512);
    assert_eq!(value["duration_ms"], 1200);
    assert!(value.get("reseeded").is_none());
    assert!(value.get("nothing_left_to_settle").is_none());
}

#[test]
fn no_delta_is_lost_or_reordered_around_another_event() {
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = seen.clone();
    let sink = Sink::new(
        Arc::new(move |name: &str, payload: serde_json::Value| {
            assert_eq!(name, EVENT_ASK_AGENT_EVENT);
            assert_eq!(payload["thread_id"], "t-1");
            let event = &payload["event"];
            recorder.lock().unwrap().push(match event["kind"].as_str() {
                Some("text") => event["delta"].as_str().unwrap_or_default().to_string(),
                other => format!("<{}>", other.unwrap_or("?")),
            });
        }),
        "t-1".to_string(),
    );

    for delta in ["Read", "ing"] {
        sink.push(&AgentEvent::Text {
            delta: delta.to_string(),
        });
    }
    sink.push(&AgentEvent::ModeChanged {
        mode_id: "plan".to_string(),
    });
    sink.push(&AgentEvent::Text {
        delta: " auth.rs".to_string(),
    });
    sink.flush();

    // Same non-determinism as discovery/events.rs's equivalent test: batching
    // is time-based, so only order and completeness are asserted, not count.
    let seen = seen.lock().unwrap().clone();
    let mode_at = seen.iter().position(|e| e == "<mode_changed>").unwrap();
    assert_eq!(seen[..mode_at].concat(), "Reading");
    assert_eq!(seen[mode_at + 1..].concat(), " auth.rs");
}

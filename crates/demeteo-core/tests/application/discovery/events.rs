// Tests extracted from `crates/demeteo-core/src/application/discovery/events.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn the_ending_names_itself_the_same_way_on_the_wire_every_time() {
    assert_eq!(TurnEnding::Success.as_str(), "success");
    assert_eq!(TurnEnding::Failed.as_str(), "failed");
    assert_eq!(TurnEnding::Environmental.as_str(), "environmental");
    assert_eq!(TurnEnding::Interrupted.as_str(), "interrupted");
}

#[test]
fn no_delta_is_lost_or_reordered_around_another_event() {
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = seen.clone();
    let sink = Sink::new(
        Arc::new(move |name: &str, payload: serde_json::Value| {
            assert_eq!(name, EVENT_DISCOVERY_AGENT_EVENT);
            assert_eq!(payload["discovery_id"], "d-1");
            let event = &payload["event"];
            recorder.lock().unwrap().push(match event["kind"].as_str() {
                Some("text") => event["delta"].as_str().unwrap_or_default().to_string(),
                other => format!("<{}>", other.unwrap_or("?")),
            });
        }),
        "d-1".to_string(),
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

    // Batching is time-based, so how many text events arrive is not fixed —
    // what is fixed is that every delta arrives exactly once and none of them
    // crosses the event that was emitted between them.
    let seen = seen.lock().unwrap().clone();
    let mode_at = seen.iter().position(|e| e == "<mode_changed>").unwrap();
    assert_eq!(seen[..mode_at].concat(), "Reading");
    assert_eq!(seen[mode_at + 1..].concat(), " auth.rs");
}

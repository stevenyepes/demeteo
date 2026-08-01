use super::*;

#[test]
fn text_event_serializes_with_snake_case_kind() {
    let e = AgentEvent::Text { delta: "hi".into() };
    let s = serde_json::to_string(&e).unwrap();
    assert!(s.contains("\"kind\":\"text\""));
}

#[test]
fn artifact_produced_event_round_trips() {
    use crate::domain::artifact::Artifact;
    let a = Artifact::agent_text("spec", "# Spec\n");
    let e = AgentEvent::ArtifactProduced {
        artifact: a.clone(),
    };
    let s = serde_json::to_string(&e).unwrap();
    assert!(s.contains("\"kind\":\"artifact_produced\""));
    let back: AgentEvent = serde_json::from_str(&s).unwrap();
    match back {
        AgentEvent::ArtifactProduced { artifact } => assert_eq!(artifact.name, "spec"),
        other => panic!("expected ArtifactProduced, got {:?}", other),
    }
}

#[test]
fn turn_complete_serializes_with_snake_case_stop_reason() {
    let e = AgentEvent::TurnComplete {
        stop_reason: StopReason::EndOfTurn,
        usage: None,
    };
    let s = serde_json::to_string(&e).unwrap();
    assert!(s.contains("\"stop_reason\":\"end_of_turn\""));
}

#[test]
fn tool_call_status_failed_carries_reason() {
    let e = ToolCallStatus::Failed {
        reason: "no".into(),
    };
    let s = serde_json::to_string(&e).unwrap();
    assert!(s.contains("\"status\":\"failed\""));
    assert!(s.contains("\"reason\":\"no\""));
}

#[test]
fn mode_changed_serializes_with_snake_case_kind() {
    let e = AgentEvent::ModeChanged {
        mode_id: "code".into(),
    };
    let s = serde_json::to_string(&e).unwrap();
    assert!(s.contains("\"kind\":\"mode_changed\""));
    assert!(s.contains("\"mode_id\":\"code\""));
}

#[test]
fn a_recoverable_error_does_not_end_the_turn() {
    let e = AgentEvent::Error {
        code: "item_error".into(),
        message: "in-process app-server event stream lagged; dropped 13 events".into(),
        recoverable: true,
        usage: None,
    };
    assert!(!e.ends_turn());
}

#[test]
fn a_non_recoverable_error_ends_the_turn() {
    let e = AgentEvent::Error {
        code: "cli_error".into(),
        message: "context window exceeded".into(),
        recoverable: false,
        usage: None,
    };
    assert!(e.ends_turn());
}

#[test]
fn only_errors_and_turn_complete_end_the_turn() {
    assert!(AgentEvent::TurnComplete {
        stop_reason: StopReason::EndOfTurn,
        usage: None,
    }
    .ends_turn());
    assert!(!AgentEvent::Text { delta: "hi".into() }.ends_turn());
    assert!(!AgentEvent::Usage(Usage::default()).ends_turn());
    assert!(!AgentEvent::ModeChanged {
        mode_id: "code".into(),
    }
    .ends_turn());
}

#[test]
fn config_changed_serializes_correctly() {
    let e = AgentEvent::ConfigChanged {
        config_id: "model".into(),
        value: "claude-4".into(),
    };
    let s = serde_json::to_string(&e).unwrap();
    assert!(s.contains("\"kind\":\"config_changed\""));
    assert!(s.contains("\"config_id\":\"model\""));
    assert!(s.contains("\"value\":\"claude-4\""));
}

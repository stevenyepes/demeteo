use std::collections::BTreeSet;

use demeteo_hub_protocol::{
    CursorAck, Hello, RunEvent, RunEventKind, RunEventPayload, Scope, PROTOCOL_VERSION,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

const HELLO: &str = include_str!("fixtures/hello.json");
const RUN_EVENT: &str = include_str!("fixtures/run_event.json");
const CURSOR_ACK: &str = include_str!("fixtures/cursor_ack.json");

fn round_trip<T: Serialize + DeserializeOwned>(fixture: &str) -> T {
    let a: Value = serde_json::from_str(fixture).unwrap();
    let parsed: T = serde_json::from_str(fixture).unwrap();
    let b = serde_json::to_value(&parsed).unwrap();
    assert_eq!(a, b);
    parsed
}

fn keys(v: &Value) -> BTreeSet<&str> {
    v.as_object().unwrap().keys().map(String::as_str).collect()
}

fn run_event_value() -> Value {
    serde_json::from_str(RUN_EVENT).unwrap()
}

#[test]
fn hello_round_trips_and_matches_hand_built_value() {
    let parsed: Hello = round_trip(HELLO);
    assert_eq!(
        parsed,
        Hello {
            v: PROTOCOL_VERSION,
            install_id: "inst-7f3a9c".into(),
            app_version: "1.4.0".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            scopes: vec![Scope::Read, Scope::Gates],
        }
    );
}

#[test]
fn run_event_round_trips_and_matches_hand_built_value() {
    let parsed: RunEvent = round_trip(RUN_EVENT);
    assert_eq!(
        parsed,
        RunEvent {
            v: PROTOCOL_VERSION,
            feature_id: "f-1791043410984".into(),
            offset: 4812,
            kind: RunEventKind::StepFailed,
            occurred_at: 1_790_213_898,
            payload: RunEventPayload {
                step_id: Some("s-implement".into()),
                step_execution_id: Some("se-1791043500123".into()),
                ticket_id: Some("t-3".into()),
            },
        }
    );
}

#[test]
fn cursor_ack_round_trips_and_matches_hand_built_value() {
    let parsed: CursorAck = round_trip(CURSOR_ACK);
    assert_eq!(
        parsed,
        CursorAck {
            v: PROTOCOL_VERSION,
            offset: 4812
        }
    );
}

#[test]
fn run_event_serialises_exactly_the_closed_key_set() {
    let event: RunEvent = serde_json::from_str(RUN_EVENT).unwrap();
    let v = serde_json::to_value(&event).unwrap();
    assert_eq!(
        keys(&v),
        BTreeSet::from([
            "v",
            "feature_id",
            "offset",
            "kind",
            "occurred_at",
            "payload"
        ])
    );
    let allowed = BTreeSet::from(["step_id", "step_execution_id", "ticket_id"]);
    assert!(keys(&v["payload"]).is_subset(&allowed));
}

#[test]
fn empty_payload_serialises_with_only_allowed_keys() {
    let mut value = run_event_value();
    value["payload"] = json!({});
    let event: RunEvent = serde_json::from_value(value).unwrap();
    assert_eq!(event.payload, RunEventPayload::default());
    let out = serde_json::to_value(&event).unwrap();
    let allowed = BTreeSet::from(["step_id", "step_execution_id", "ticket_id"]);
    assert!(keys(&out["payload"]).is_subset(&allowed));
}

#[test]
fn run_event_rejects_an_extra_key() {
    let mut value = run_event_value();
    value["message"] = json!("agent said hello");
    assert!(serde_json::from_value::<RunEvent>(value).is_err());
}

#[test]
fn run_event_payload_rejects_an_extra_key() {
    let mut value = run_event_value();
    value["payload"]["summary"] = json!("tests failed in src/main.rs");
    assert!(serde_json::from_value::<RunEvent>(value).is_err());
}

#[test]
fn hello_and_cursor_ack_reject_an_extra_key() {
    let mut hello: Value = serde_json::from_str(HELLO).unwrap();
    hello["local_path"] = json!("/home/u");
    assert!(serde_json::from_value::<Hello>(hello).is_err());
    let mut ack: Value = serde_json::from_str(CURSOR_ACK).unwrap();
    ack["extra"] = json!(1);
    assert!(serde_json::from_value::<CursorAck>(ack).is_err());
}

#[test]
fn hello_rejects_the_logs_scope() {
    let mut value: Value = serde_json::from_str(HELLO).unwrap();
    value["scopes"] = json!(["logs"]);
    assert!(serde_json::from_value::<Hello>(value).is_err());
}

#[test]
fn run_event_kind_rejects_an_unknown_string() {
    let mut value = run_event_value();
    value["kind"] = json!("step_exploded");
    assert!(serde_json::from_value::<RunEvent>(value).is_err());
}

#[test]
fn run_event_kind_accepts_exactly_its_twelve_spellings() {
    let names = [
        "feature_started",
        "feature_completed",
        "feature_failed",
        "feature_cancelled",
        "step_started",
        "step_completed",
        "step_failed",
        "ticket_started",
        "ticket_completed",
        "ticket_failed",
        "gate_pending",
        "gate_decided",
    ];
    for name in names {
        let kind: RunEventKind = serde_json::from_value(json!(name)).unwrap();
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(name));
    }
}

#[test]
fn a_different_protocol_version_still_deserialises() {
    let mut hello: Value = serde_json::from_str(HELLO).unwrap();
    hello["v"] = json!(2);
    assert_eq!(serde_json::from_value::<Hello>(hello).unwrap().v, 2);
    let mut event = run_event_value();
    event["v"] = json!(2);
    assert_eq!(serde_json::from_value::<RunEvent>(event).unwrap().v, 2);
    let mut ack: Value = serde_json::from_str(CURSOR_ACK).unwrap();
    ack["v"] = json!(2);
    assert_eq!(serde_json::from_value::<CursorAck>(ack).unwrap().v, 2);
}

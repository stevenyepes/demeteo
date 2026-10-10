use demeteo_hub_protocol::{
    Decision, RefusalReason, RequestHeader, RequestOutcome, RequestPayload, RequestResult,
    SignedRequest, PROTOCOL_VERSION,
};
use serde_json::{json, Value};

const ACCEPTED: &str = include_str!("fixtures/request_result_accepted.json");
const REFUSED: &str = include_str!("fixtures/request_result_refused.json");

const REASONS: [&str; 20] = [
    "scope_off",
    "expired",
    "replayed",
    "wrong_instance",
    "unknown_kind",
    "malformed_body",
    "over_run_ceiling",
    "over_rolling_ceiling",
    "unknown_project",
    "unknown_workflow",
    "unknown_feature",
    "unknown_step",
    "gate_not_pending",
    "credential_not_pinned",
    "assertion_invalid",
    "step_not_reassignable",
    "harness_not_registered",
    "attachment_mismatch",
    "last_credential",
    "self_revocation",
];

fn round_trip(fixture: &str) -> RequestResult {
    let a: Value = serde_json::from_str(fixture).unwrap();
    let parsed: RequestResult = serde_json::from_str(fixture).unwrap();
    assert_eq!(a, serde_json::to_value(&parsed).unwrap());
    parsed
}

#[test]
fn accepted_result_round_trips_and_matches_hand_built_value() {
    assert_eq!(
        round_trip(ACCEPTED),
        RequestResult {
            v: PROTOCOL_VERSION,
            request_id: "req-9b2e41".into(),
            outcome: RequestOutcome::Accepted,
        }
    );
}

#[test]
fn refused_result_round_trips_and_matches_hand_built_value() {
    assert_eq!(
        round_trip(REFUSED),
        RequestResult {
            v: PROTOCOL_VERSION,
            request_id: "req-9b2e42".into(),
            outcome: RequestOutcome::Refused {
                reason: RefusalReason::GateNotPending
            },
        }
    );
}

#[test]
fn outcome_serialises_as_a_tagged_object() {
    assert_eq!(
        serde_json::to_value(RequestOutcome::Accepted).unwrap(),
        json!({"result": "accepted"})
    );
    assert_eq!(
        serde_json::to_value(RequestOutcome::Refused {
            reason: RefusalReason::Expired
        })
        .unwrap(),
        json!({"result": "refused", "reason": "expired"})
    );
}

#[test]
fn refusal_reason_accepts_exactly_its_twenty_spellings() {
    assert_eq!(REASONS.len(), 20);
    for name in REASONS {
        let reason: RefusalReason = serde_json::from_value(json!(name)).unwrap();
        assert_eq!(serde_json::to_value(reason).unwrap(), json!(name));
    }
}

#[test]
fn refusal_reason_rejects_an_unknown_string() {
    assert!(serde_json::from_value::<RefusalReason>(json!("because")).is_err());
    let mut value: Value = serde_json::from_str(REFUSED).unwrap();
    value["outcome"]["reason"] = json!("because");
    assert!(serde_json::from_value::<RequestResult>(value).is_err());
}

#[test]
fn expired_is_not_a_third_outcome() {
    let mut value: Value = serde_json::from_str(ACCEPTED).unwrap();
    value["outcome"] = json!({"result": "expired"});
    assert!(serde_json::from_value::<RequestResult>(value).is_err());
}

#[test]
fn refused_without_a_reason_is_rejected() {
    let mut value: Value = serde_json::from_str(REFUSED).unwrap();
    value["outcome"] = json!({"result": "refused"});
    assert!(serde_json::from_value::<RequestResult>(value).is_err());
}

#[test]
fn request_result_rejects_an_extra_key() {
    let mut value: Value = serde_json::from_str(ACCEPTED).unwrap();
    value["detail"] = json!("see /home/u/log");
    assert!(serde_json::from_value::<RequestResult>(value).is_err());
}

#[test]
fn request_header_reads_the_id_of_a_request_with_an_unknown_kind() {
    let text = r#"{
        "v": 1,
        "request_id": "req-77",
        "install_id": "inst-7f3a9c",
        "payload": { "kind": "launch_missiles", "target": "x" },
        "signature": "AAAA"
    }"#;
    let header: RequestHeader = serde_json::from_str(text).unwrap();
    assert_eq!(
        header,
        RequestHeader {
            request_id: "req-77".into()
        }
    );
}

#[test]
fn request_header_requires_a_request_id() {
    assert!(serde_json::from_value::<RequestHeader>(json!({"v": 1})).is_err());
}

const FIXTURES: [(&str, &str); 6] = [
    (
        "signed_request_start_feature",
        include_str!("fixtures/signed_request_start_feature.json"),
    ),
    (
        "signed_request_cancel_feature",
        include_str!("fixtures/signed_request_cancel_feature.json"),
    ),
    (
        "signed_request_set_step_assignment",
        include_str!("fixtures/signed_request_set_step_assignment.json"),
    ),
    (
        "signed_request_gate_decision",
        include_str!("fixtures/signed_request_gate_decision.json"),
    ),
    (
        "signed_request_passkey_endorsement",
        include_str!("fixtures/signed_request_passkey_endorsement.json"),
    ),
    (
        "signed_request_passkey_revocation",
        include_str!("fixtures/signed_request_passkey_revocation.json"),
    ),
];

const KINDS: [&str; 6] = [
    "start_feature",
    "cancel_feature",
    "set_step_assignment",
    "gate_decision",
    "passkey_endorsement",
    "passkey_revocation",
];

/// The wildcard-free `match` stops compiling when a variant is added, so a new
/// request kind cannot arrive without a fixture being written for it.
fn fixture_name(payload: &RequestPayload) -> &'static str {
    match payload {
        RequestPayload::StartFeature(_) => "signed_request_start_feature",
        RequestPayload::CancelFeature(_) => "signed_request_cancel_feature",
        RequestPayload::SetStepAssignment(_) => "signed_request_set_step_assignment",
        RequestPayload::GateDecision(_) => "signed_request_gate_decision",
        RequestPayload::PasskeyEndorsement(_) => "signed_request_passkey_endorsement",
        RequestPayload::PasskeyRevocation(_) => "signed_request_passkey_revocation",
    }
}

#[test]
fn every_request_fixture_round_trips_and_names_its_own_variant() {
    for (name, text) in FIXTURES {
        let value: Value = serde_json::from_str(text).unwrap();
        let parsed: SignedRequest = serde_json::from_str(text).unwrap();
        assert_eq!(value, serde_json::to_value(&parsed).unwrap(), "{name}");
        assert_eq!(parsed.v, PROTOCOL_VERSION, "{name}");
        assert_eq!(fixture_name(&parsed.payload), name);
    }
}

#[test]
fn the_six_payload_kinds_are_spelled_as_specified() {
    let (written, serialised): (Vec<String>, Vec<String>) = FIXTURES
        .iter()
        .map(|(_, text)| {
            let value: Value = serde_json::from_str(text).unwrap();
            let written = value["payload"]["kind"].as_str().unwrap().to_owned();
            let request: SignedRequest = serde_json::from_value(value).unwrap();
            let out = serde_json::to_value(&request.payload).unwrap();
            (written, out["kind"].as_str().unwrap().to_owned())
        })
        .unzip();
    assert_eq!(written, KINDS);
    assert_eq!(serialised, KINDS);
}

#[test]
fn start_feature_fixture_exercises_every_optional_and_both_lists() {
    let request: SignedRequest = serde_json::from_str(FIXTURES[0].1).unwrap();
    let RequestPayload::StartFeature(body) = request.payload else {
        panic!("not a start_feature");
    };
    assert!(body.agent_kind.is_some() && body.model.is_some() && body.effort.is_some());
    assert!(body.max_budget_cents.is_some() && body.max_wall_clock_secs.is_some());
    assert!(!body.step_overrides.is_empty() && !body.attachments.is_empty());
}

#[test]
fn start_feature_optionals_and_lists_default_when_absent() {
    let mut value: Value = serde_json::from_str(FIXTURES[0].1).unwrap();
    value["payload"]["body"] = json!({
        "project_id": "p", "workflow_id": "w", "title": "t", "description": "d"
    });
    let request: SignedRequest = serde_json::from_value(value).unwrap();
    let RequestPayload::StartFeature(body) = request.payload else {
        panic!("not a start_feature");
    };
    assert_eq!(body.agent_kind, None);
    assert_eq!(body.max_budget_cents, None);
    assert!(body.step_overrides.is_empty() && body.attachments.is_empty());
}

#[test]
fn decision_rejects_redirect() {
    assert!(serde_json::from_value::<Decision>(json!("redirect")).is_err());
    let mut value: Value = serde_json::from_str(FIXTURES[3].1).unwrap();
    value["payload"]["body"]["decision"] = json!("redirect");
    assert!(serde_json::from_value::<SignedRequest>(value).is_err());
}

#[test]
fn an_unknown_payload_kind_fails_signed_request_but_not_the_header() {
    let text = r#"{
        "v": 1,
        "request_id": "req-77",
        "instance_id": "inst-7f3a9c",
        "issued_at": 1,
        "expires_at": 2,
        "payload": { "kind": "enable_scope", "body": { "scope": "spend" } },
        "signature": "AAAA"
    }"#;
    assert!(serde_json::from_str::<SignedRequest>(text).is_err());
    let header: RequestHeader = serde_json::from_str(text).unwrap();
    assert_eq!(header.request_id, "req-77");
}

#[test]
fn a_different_protocol_version_still_deserialises() {
    let mut value: Value = serde_json::from_str(FIXTURES[1].1).unwrap();
    value["v"] = json!(2);
    assert_eq!(serde_json::from_value::<SignedRequest>(value).unwrap().v, 2);
}

#[test]
fn request_bodies_reject_an_extra_key() {
    for (name, text) in FIXTURES {
        let mut value: Value = serde_json::from_str(text).unwrap();
        value["payload"]["body"]["workdir"] = json!("/home/u/repo");
        assert!(
            serde_json::from_value::<SignedRequest>(value).is_err(),
            "{name}"
        );
    }
}

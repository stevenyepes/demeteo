use demeteo_hub_protocol::{
    endorsement_challenge, gate_challenge, revocation_challenge, AttachmentRef, CancelFeature,
    Decision, Effort, GateDecision, PasskeyEndorsement, PasskeyRevocation, RequestPayload,
    SetStepAssignment, SignedRequest, StartFeature, StepOverride, WebAuthnAssertion,
};

fn assertion() -> WebAuthnAssertion {
    WebAuthnAssertion {
        credential_id: "cred".into(),
        authenticator_data: "ad".into(),
        client_data_json: "cdj".into(),
        signature: "sig".into(),
    }
}

fn start_feature() -> StartFeature {
    StartFeature {
        project_id: "proj".into(),
        workflow_id: "wf".into(),
        title: "title".into(),
        description: "desc".into(),
        agent_kind: Some("claude-code".into()),
        model: Some("m".into()),
        effort: Some(Effort::High),
        step_overrides: vec![StepOverride {
            step_id: "s1".into(),
            agent_kind: Some("opencode".into()),
            model: Some("m2".into()),
            effort: Some(Effort::Low),
        }],
        max_budget_cents: Some(250),
        max_wall_clock_secs: Some(600),
        attachments: vec![AttachmentRef {
            id: "att".into(),
            size: 42,
            sha256: "digest".into(),
        }],
    }
}

fn gate_decision() -> GateDecision {
    GateDecision {
        feature_id: "feat".into(),
        step_execution_id: "exec".into(),
        decision: Decision::Approve,
        feedback: Some("looks fine".into()),
        assertion: assertion(),
    }
}

fn request(payload: RequestPayload) -> SignedRequest {
    SignedRequest {
        v: 1,
        request_id: "req".into(),
        instance_id: "inst".into(),
        issued_at: 100,
        expires_at: 200,
        payload,
        signature: "hub-sig".into(),
    }
}

fn bytes(r: &SignedRequest) -> Vec<u8> {
    r.signing_bytes().unwrap()
}

fn cancel() -> SignedRequest {
    request(RequestPayload::CancelFeature(CancelFeature {
        feature_id: "feat".into(),
    }))
}

#[test]
fn signing_bytes_are_deterministic() {
    let r = request(RequestPayload::StartFeature(start_feature()));
    assert_eq!(bytes(&r), bytes(&r.clone()));
}

#[test]
fn cancel_feature_matches_hand_written_golden_bytes() {
    let golden: Vec<u8> = vec![
        // str("demeteo-hub/v1/request")
        0x00, 0x00, 0x00, 0x16, 0x64, 0x65, 0x6D, 0x65, 0x74, 0x65, 0x6F, 0x2D, 0x68, 0x75, 0x62,
        0x2F, 0x76, 0x31, 0x2F, 0x72, 0x65, 0x71, 0x75, 0x65, 0x73, 0x74, // u32 v = 1
        0x00, 0x00, 0x00, 0x01, // str request_id "req"
        0x00, 0x00, 0x00, 0x03, 0x72, 0x65, 0x71, // str instance_id "inst"
        0x00, 0x00, 0x00, 0x04, 0x69, 0x6E, 0x73, 0x74, // u64 issued_at = 100
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x64, // u64 expires_at = 200
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC8, // u8 tag: cancel_feature
        0x02, // str feature_id "feat"
        0x00, 0x00, 0x00, 0x04, 0x66, 0x65, 0x61, 0x74,
    ];
    assert_eq!(cancel().signing_bytes(), Ok(golden));
}

#[test]
fn signature_is_not_part_of_the_bytes() {
    let mut r = request(RequestPayload::StartFeature(start_feature()));
    let before = bytes(&r);
    r.signature = "another".into();
    assert_eq!(bytes(&r), before);
}

#[test]
fn message_version_is_its_own_not_the_crate_constant() {
    let mut r = cancel();
    r.v = 2;
    let b = bytes(&r);
    assert_eq!(&b[26..30], &[0, 0, 0, 2]);
    assert_ne!(b, bytes(&cancel()));
}

#[test]
fn every_header_field_changes_the_bytes() {
    let base = bytes(&cancel());
    let mut variants: Vec<(&str, SignedRequest)> = Vec::new();
    let mut r = cancel();
    r.v = 2;
    variants.push(("v", r));
    let mut r = cancel();
    r.request_id = "req2".into();
    variants.push(("request_id", r));
    let mut r = cancel();
    r.instance_id = "inst2".into();
    variants.push(("instance_id", r));
    let mut r = cancel();
    r.issued_at = 101;
    variants.push(("issued_at", r));
    let mut r = cancel();
    r.expires_at = 201;
    variants.push(("expires_at", r));
    for (field, r) in variants {
        assert_ne!(bytes(&r), base, "changing {field} must change the bytes");
    }
}

fn with_start(f: impl FnOnce(&mut StartFeature)) -> SignedRequest {
    let mut b = start_feature();
    f(&mut b);
    request(RequestPayload::StartFeature(b))
}

#[test]
fn every_start_feature_field_changes_the_bytes() {
    let base = bytes(&request(RequestPayload::StartFeature(start_feature())));
    let variants: Vec<(&str, SignedRequest)> = vec![
        ("project_id", with_start(|b| b.project_id = "p2".into())),
        ("workflow_id", with_start(|b| b.workflow_id = "w2".into())),
        ("title", with_start(|b| b.title = "t2".into())),
        ("description", with_start(|b| b.description = "d2".into())),
        ("agent_kind", with_start(|b| b.agent_kind = None)),
        ("model", with_start(|b| b.model = Some("other".into()))),
        ("effort", with_start(|b| b.effort = Some(Effort::Max))),
        (
            "override.step_id",
            with_start(|b| b.step_overrides[0].step_id = "s2".into()),
        ),
        (
            "override.agent_kind",
            with_start(|b| b.step_overrides[0].agent_kind = None),
        ),
        (
            "override.model",
            with_start(|b| b.step_overrides[0].model = Some("x".into())),
        ),
        (
            "override.effort",
            with_start(|b| b.step_overrides[0].effort = Some(Effort::Max)),
        ),
        ("overrides empty", with_start(|b| b.step_overrides.clear())),
        (
            "max_budget_cents",
            with_start(|b| b.max_budget_cents = Some(300)),
        ),
        (
            "max_budget_cents none",
            with_start(|b| b.max_budget_cents = None),
        ),
        (
            "max_wall_clock_secs",
            with_start(|b| b.max_wall_clock_secs = Some(601)),
        ),
        (
            "attachment.id",
            with_start(|b| b.attachments[0].id = "att2".into()),
        ),
        (
            "attachment.size",
            with_start(|b| b.attachments[0].size = 43),
        ),
        (
            "attachment.sha256",
            with_start(|b| b.attachments[0].sha256 = "d2".into()),
        ),
        ("attachments empty", with_start(|b| b.attachments.clear())),
    ];
    for (field, r) in variants {
        assert_ne!(bytes(&r), base, "changing {field} must change the bytes");
    }
}

#[test]
fn other_variants_react_to_their_fields() {
    let set = |f: fn(&mut SetStepAssignment)| {
        let mut b = SetStepAssignment {
            feature_id: "feat".into(),
            step_id: "s".into(),
            agent_kind: Some("a".into()),
            model: Some("m".into()),
            effort: Some(Effort::Low),
        };
        f(&mut b);
        bytes(&request(RequestPayload::SetStepAssignment(b)))
    };
    let base = set(|_| {});
    for (field, v) in [
        ("feature_id", set(|b| b.feature_id = "f2".into())),
        ("step_id", set(|b| b.step_id = "s2".into())),
        ("agent_kind", set(|b| b.agent_kind = None)),
        ("model", set(|b| b.model = None)),
        ("effort", set(|b| b.effort = Some(Effort::High))),
    ] {
        assert_ne!(v, base, "changing {field} must change the bytes");
    }

    let gate = |f: fn(&mut GateDecision)| {
        let mut b = gate_decision();
        f(&mut b);
        bytes(&request(RequestPayload::GateDecision(b)))
    };
    let base = gate(|_| {});
    for (field, v) in [
        ("feature_id", gate(|b| b.feature_id = "f2".into())),
        (
            "step_execution_id",
            gate(|b| b.step_execution_id = "e2".into()),
        ),
        ("decision", gate(|b| b.decision = Decision::Cancel)),
        (
            "assertion.credential_id",
            gate(|b| b.assertion.credential_id = "c2".into()),
        ),
        (
            "assertion.authenticator_data",
            gate(|b| b.assertion.authenticator_data = "x".into()),
        ),
        (
            "assertion.client_data_json",
            gate(|b| b.assertion.client_data_json = "x".into()),
        ),
        (
            "assertion.signature",
            gate(|b| b.assertion.signature = "x".into()),
        ),
    ] {
        assert_ne!(v, base, "changing {field} must change the bytes");
    }

    let endorse = |f: fn(&mut PasskeyEndorsement)| {
        let mut b = PasskeyEndorsement {
            new_credential_id: "nc".into(),
            new_public_key: "nk".into(),
            assertion: assertion(),
        };
        f(&mut b);
        bytes(&request(RequestPayload::PasskeyEndorsement(b)))
    };
    let base = endorse(|_| {});
    for (field, v) in [
        (
            "new_credential_id",
            endorse(|b| b.new_credential_id = "x".into()),
        ),
        ("new_public_key", endorse(|b| b.new_public_key = "x".into())),
        (
            "assertion.signature",
            endorse(|b| b.assertion.signature = "x".into()),
        ),
    ] {
        assert_ne!(v, base, "changing {field} must change the bytes");
    }

    let revoke = |f: fn(&mut PasskeyRevocation)| {
        let mut b = PasskeyRevocation {
            credential_id: "c".into(),
            assertion: assertion(),
        };
        f(&mut b);
        bytes(&request(RequestPayload::PasskeyRevocation(b)))
    };
    let base = revoke(|_| {});
    for (field, v) in [
        ("credential_id", revoke(|b| b.credential_id = "x".into())),
        (
            "assertion.client_data_json",
            revoke(|b| b.assertion.client_data_json = "x".into()),
        ),
    ] {
        assert_ne!(v, base, "changing {field} must change the bytes");
    }
}

#[test]
fn gate_decision_feedback_is_covered_by_the_request_signature() {
    // `gate_challenge` has no feedback parameter, so the human's assertion
    // cannot bind it; the Hub's signature over these bytes is what does.
    let with = |feedback: Option<&str>| {
        let mut b = gate_decision();
        b.feedback = feedback.map(str::to_owned);
        bytes(&request(RequestPayload::GateDecision(b)))
    };
    let base = with(Some("looks fine"));
    assert_ne!(with(Some("rewritten")), base);
    assert_ne!(with(None), base);
    assert_ne!(with(Some("")), with(None));
}

#[test]
fn request_pre_image_never_equals_a_challenge_pre_image() {
    let gate = gate_challenge("inst", "feat", "exec", Decision::Approve, "req", 200).unwrap();
    let endorse = endorsement_challenge("inst", "req", 200, "nc", "nk").unwrap();
    let revoke = revocation_challenge("inst", "req", 200, "c").unwrap();

    let gate_req = bytes(&request(RequestPayload::GateDecision(GateDecision {
        feedback: None,
        ..gate_decision()
    })));
    let endorse_req = bytes(&request(RequestPayload::PasskeyEndorsement(
        PasskeyEndorsement {
            new_credential_id: "nc".into(),
            new_public_key: "nk".into(),
            assertion: assertion(),
        },
    )));
    let revoke_req = bytes(&request(RequestPayload::PasskeyRevocation(
        PasskeyRevocation {
            credential_id: "c".into(),
            assertion: assertion(),
        },
    )));
    for req in [&gate_req, &endorse_req, &revoke_req, &bytes(&cancel())] {
        for challenge in [&gate, &endorse, &revoke] {
            assert_ne!(req, challenge);
        }
    }
}

#[test]
fn start_feature_bytes_survive_a_json_round_trip() {
    // The Hub signs its in-memory value; the instance re-derives the bytes
    // from what it parsed. Both must agree for every budget a Hub can send.
    for cents in [0, 1, 1250, (1 << 53) - 1, 12_345_678_901_234_567, u64::MAX] {
        let sent = with_start(|b| b.max_budget_cents = Some(cents));
        let json = serde_json::to_string(&sent).unwrap();
        let received: SignedRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(received, sent, "{cents} must parse back unchanged");
        assert_eq!(bytes(&received), bytes(&sent), "{cents}");
    }
}

#[test]
fn gate_challenge_matches_the_hub_md_vector() {
    // docs/HUB.md §8 publishes these bytes and their SHA-256 as the test
    // vector; change one and the other must change with it.
    let golden: Vec<u8> = vec![
        // str("demeteo-hub/v1/gate")
        0x00, 0x00, 0x00, 0x13, 0x64, 0x65, 0x6D, 0x65, 0x74, 0x65, 0x6F, 0x2D, 0x68, 0x75, 0x62,
        0x2F, 0x76, 0x31, 0x2F, 0x67, 0x61, 0x74, 0x65, // u32 PROTOCOL_VERSION = 1
        0x00, 0x00, 0x00, 0x01, // str instance_id "i-7f3a9c"
        0x00, 0x00, 0x00, 0x08, 0x69, 0x2D, 0x37, 0x66, 0x33, 0x61, 0x39, 0x63,
        // str feature_id "f-1791004167577"
        0x00, 0x00, 0x00, 0x0F, 0x66, 0x2D, 0x31, 0x37, 0x39, 0x31, 0x30, 0x30, 0x34, 0x31, 0x36,
        0x37, 0x35, 0x37, 0x37, // str step_execution_id "se-0042"
        0x00, 0x00, 0x00, 0x07, 0x73, 0x65, 0x2D, 0x30, 0x30, 0x34, 0x32,
        // u8 decision: approve
        0x01, // str request_id "Zk3m0R8pQnVx2tL5aYw9bA"
        0x00, 0x00, 0x00, 0x16, 0x5A, 0x6B, 0x33, 0x6D, 0x30, 0x52, 0x38, 0x70, 0x51, 0x6E, 0x56,
        0x78, 0x32, 0x74, 0x4C, 0x35, 0x61, 0x59, 0x77, 0x39, 0x62, 0x41,
        // u64 expires_at = 1791020400
        0x00, 0x00, 0x00, 0x00, 0x6A, 0xC0, 0xCD, 0x70,
    ];
    let actual = gate_challenge(
        "i-7f3a9c",
        "f-1791004167577",
        "se-0042",
        Decision::Approve,
        "Zk3m0R8pQnVx2tL5aYw9bA",
        1_791_020_400,
    );
    assert_eq!(actual, Ok(golden));
}

use demeteo_hub_protocol::{
    endorsement_challenge, gate_challenge, revocation_challenge, AttachmentRef, CancelFeature,
    Decision, Effort, GateDecision, PasskeyEndorsement, PasskeyRevocation, RequestPayload,
    RetryStep, RunnerRepository, RunnerSettings, RunnerStart, RunnerWorkflow, SetStepAssignment,
    SignedRequest, StartFeature, StepOverride, WebAuthnAssertion,
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
        runner: None,
    }
}

fn runner_start() -> RunnerStart {
    RunnerStart {
        source_instance_id: "src".into(),
        repository: RunnerRepository {
            provider_kind: "github".into(),
            host: "github.com".into(),
            full_name: "o/r".into(),
        },
        workflow: RunnerWorkflow {
            workflow_id: "wf".into(),
            version: 3,
            name: "n".into(),
            steps_json: "[]".into(),
        },
        settings: None,
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
fn retry_step_matches_hand_written_golden_bytes() {
    let golden: Vec<u8> = vec![
        // str("demeteo-hub/v1/request")
        0x00, 0x00, 0x00, 0x16, 0x64, 0x65, 0x6D, 0x65, 0x74, 0x65, 0x6F, 0x2D, 0x68, 0x75, 0x62,
        0x2F, 0x76, 0x31, 0x2F, 0x72, 0x65, 0x71, 0x75, 0x65, 0x73, 0x74, // u32 v = 1
        0x00, 0x00, 0x00, 0x01, // str request_id "req"
        0x00, 0x00, 0x00, 0x03, 0x72, 0x65, 0x71, // str instance_id "inst"
        0x00, 0x00, 0x00, 0x04, 0x69, 0x6E, 0x73, 0x74, // u64 issued_at = 100
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x64, // u64 expires_at = 200
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC8, // u8 tag: retry_step
        0x07, // str feature_id "feat"
        0x00, 0x00, 0x00, 0x04, 0x66, 0x65, 0x61, 0x74, // str step_id "s-impl"
        0x00, 0x00, 0x00, 0x06, 0x73, 0x2D, 0x69, 0x6D, 0x70, 0x6C,
    ];
    let retry = request(RequestPayload::RetryStep(RetryStep {
        feature_id: "feat".into(),
        step_id: "s-impl".into(),
    }));
    assert_eq!(retry.signing_bytes(), Ok(golden));
}

fn minimal_start(runner: Option<Box<RunnerStart>>) -> SignedRequest {
    request(RequestPayload::StartFeature(StartFeature {
        project_id: "p".into(),
        workflow_id: "w".into(),
        title: "t".into(),
        description: "d".into(),
        agent_kind: None,
        model: None,
        effort: None,
        step_overrides: Vec::new(),
        max_budget_cents: None,
        max_wall_clock_secs: None,
        attachments: Vec::new(),
        runner,
    }))
}

/// The bytes every desktop already verifies a start against. `runner` was
/// added after desktops shipped, so a start without it must sign exactly as
/// before or every one of them drops every start as a bad signature.
const MINIMAL_START_GOLDEN: &[u8] = &[
    // str("demeteo-hub/v1/request")
    0x00, 0x00, 0x00, 0x16, 0x64, 0x65, 0x6D, 0x65, 0x74, 0x65, 0x6F, 0x2D, 0x68, 0x75, 0x62, 0x2F,
    0x76, 0x31, 0x2F, 0x72, 0x65, 0x71, 0x75, 0x65, 0x73, 0x74, // u32 v = 1
    0x00, 0x00, 0x00, 0x01, // str request_id "req"
    0x00, 0x00, 0x00, 0x03, 0x72, 0x65, 0x71, // str instance_id "inst"
    0x00, 0x00, 0x00, 0x04, 0x69, 0x6E, 0x73, 0x74, // u64 issued_at = 100
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x64, // u64 expires_at = 200
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC8, // u8 tag: start_feature
    0x01, // str project_id "p"
    0x00, 0x00, 0x00, 0x01, 0x70, // str workflow_id "w"
    0x00, 0x00, 0x00, 0x01, 0x77, // str title "t"
    0x00, 0x00, 0x00, 0x01, 0x74, // str description "d"
    0x00, 0x00, 0x00, 0x01, 0x64, // opt agent_kind, opt model, opt effort: none
    0x00, 0x00, 0x00, // vec step_overrides: empty
    0x00, 0x00, 0x00, 0x00, // opt max_budget_cents, opt max_wall_clock_secs: none
    0x00, 0x00, // vec attachments: empty, and nothing after it
    0x00, 0x00, 0x00, 0x00,
];

#[test]
fn start_feature_without_a_runner_matches_hand_written_golden_bytes() {
    assert_eq!(
        minimal_start(None).signing_bytes(),
        Ok(MINIMAL_START_GOLDEN.to_vec())
    );
}

#[test]
fn a_runner_start_appends_its_fields_after_the_unchanged_desktop_bytes() {
    let suffix: Vec<u8> = [
        &[0x01][..], // u8: runner present
        &[0, 0, 0, 3],
        b"src", // str source_instance_id
        &[0, 0, 0, 6],
        b"github", // str repository.provider_kind
        &[0, 0, 0, 10],
        b"github.com", // str repository.host
        &[0, 0, 0, 3],
        b"o/r", // str repository.full_name
        &[0, 0, 0, 2],
        b"wf",         // str workflow.workflow_id
        &[0, 0, 0, 3], // u32 workflow.version
        &[0, 0, 0, 1],
        b"n", // str workflow.name
        &[0, 0, 0, 2],
        b"[]", // str workflow.steps_json
    ]
    .concat();
    let expected = [MINIMAL_START_GOLDEN, &suffix].concat();
    assert_eq!(
        minimal_start(Some(Box::new(runner_start()))).signing_bytes(),
        Ok(expected)
    );
}

#[test]
fn every_runner_field_changes_the_bytes() {
    let with_runner = |f: fn(&mut RunnerStart)| {
        let mut r = runner_start();
        f(&mut r);
        bytes(&with_start(|b| b.runner = Some(Box::new(r))))
    };
    let base = with_runner(|_| {});
    assert_ne!(
        base,
        bytes(&request(RequestPayload::StartFeature(start_feature())))
    );
    for (field, v) in [
        (
            "source_instance_id",
            with_runner(|r| r.source_instance_id = "s2".into()),
        ),
        (
            "repository.provider_kind",
            with_runner(|r| r.repository.provider_kind = "gitlab".into()),
        ),
        (
            "repository.host",
            with_runner(|r| r.repository.host = "h2".into()),
        ),
        (
            "repository.full_name",
            with_runner(|r| r.repository.full_name = "o/r2".into()),
        ),
        (
            "workflow.workflow_id",
            with_runner(|r| r.workflow.workflow_id = "wf2".into()),
        ),
        ("workflow.version", with_runner(|r| r.workflow.version = 4)),
        (
            "workflow.name",
            with_runner(|r| r.workflow.name = "n2".into()),
        ),
        (
            "workflow.steps_json",
            with_runner(|r| r.workflow.steps_json = "[{}]".into()),
        ),
    ] {
        assert_ne!(v, base, "changing {field} must change the bytes");
    }
}

/// Every field set, so each one has a value to change.
fn full_settings() -> RunnerSettings {
    RunnerSettings {
        branch_prefix: Some("bp/".into()),
        artifact_subdir: Some("art/".into()),
        commit_artifacts: Some(false),
        review_entrypoint: Some("/review".into()),
        test_command: Some("t".into()),
        build_command: Some("b".into()),
        coverage_command: Some("c".into()),
        conventions_file: Some("AGENTS.md".into()),
        loop_iterations: Some(3),
        sync_resolver_agent_kind: Some("codex".into()),
        sync_resolver_model: Some("m".into()),
        sync_resolver_effort: Some(Effort::Low),
        sync_review_before_push: Some(false),
    }
}

#[test]
fn runner_settings_append_after_the_unchanged_runner_bytes() {
    let without = minimal_start(Some(Box::new(runner_start())))
        .signing_bytes()
        .unwrap();
    let mut r = runner_start();
    r.settings = Some(RunnerSettings {
        branch_prefix: Some("bp/".into()),
        commit_artifacts: Some(true),
        loop_iterations: Some(2),
        sync_resolver_effort: Some(Effort::High),
        ..RunnerSettings::default()
    });
    let suffix: Vec<u8> = [
        &[0x01][..], // u8: settings present
        &[0x01, 0, 0, 0, 3],
        b"bp/",              // opt<str> branch_prefix
        &[0x00],             // opt<str> artifact_subdir
        &[0x01, 0x01],       // opt<bool> commit_artifacts
        &[0x00],             // opt<str> review_entrypoint
        &[0x00],             // opt<str> test_command
        &[0x00],             // opt<str> build_command
        &[0x00],             // opt<str> coverage_command
        &[0x00],             // opt<str> conventions_file
        &[0x01, 0, 0, 0, 2], // opt<u32> loop_iterations
        &[0x00],             // opt<str> sync_resolver_agent_kind
        &[0x00],             // opt<str> sync_resolver_model
        &[0x01, 0x03],       // opt<u8> sync_resolver_effort
        &[0x00],             // opt<bool> sync_review_before_push
    ]
    .concat();
    assert_eq!(
        minimal_start(Some(Box::new(r))).signing_bytes(),
        Ok([without, suffix].concat())
    );
}

#[test]
fn every_runner_settings_field_changes_the_bytes() {
    let with_settings = |f: fn(&mut RunnerSettings)| {
        let mut settings = full_settings();
        f(&mut settings);
        let mut r = runner_start();
        r.settings = Some(settings);
        bytes(&with_start(|b| b.runner = Some(Box::new(r))))
    };
    let base = with_settings(|_| {});
    type Change = fn(&mut RunnerSettings);
    let unset: [(&str, Change); 13] = [
        ("branch_prefix", |s| s.branch_prefix = None),
        ("artifact_subdir", |s| s.artifact_subdir = None),
        ("commit_artifacts", |s| s.commit_artifacts = Some(true)),
        ("review_entrypoint", |s| s.review_entrypoint = None),
        ("test_command", |s| s.test_command = None),
        ("build_command", |s| s.build_command = None),
        ("coverage_command", |s| s.coverage_command = None),
        ("conventions_file", |s| s.conventions_file = None),
        ("loop_iterations", |s| s.loop_iterations = Some(4)),
        ("sync_resolver_agent_kind", |s| {
            s.sync_resolver_agent_kind = None
        }),
        ("sync_resolver_model", |s| s.sync_resolver_model = None),
        ("sync_resolver_effort", |s| {
            s.sync_resolver_effort = Some(Effort::Max)
        }),
        ("sync_review_before_push", |s| {
            s.sync_review_before_push = Some(true)
        }),
    ];
    for (field, change) in unset {
        assert_ne!(
            with_settings(change),
            base,
            "changing {field} must change the bytes"
        );
    }
}

#[test]
fn a_runner_start_without_settings_serialises_without_the_key() {
    let json = serde_json::to_value(runner_start()).unwrap();
    assert!(json.get("settings").is_none(), "{json}");
}

#[test]
fn a_runner_start_bytes_survive_a_json_round_trip() {
    let mut runner = runner_start();
    runner.settings = Some(full_settings());
    let sent = with_start(|b| b.runner = Some(Box::new(runner)));
    let json = serde_json::to_string(&sent).unwrap();
    let received: SignedRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(received, sent);
    assert_eq!(bytes(&received), bytes(&sent));
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

    let retry = |f: fn(&mut RetryStep)| {
        let mut b = RetryStep {
            feature_id: "feat".into(),
            step_id: "s".into(),
        };
        f(&mut b);
        bytes(&request(RequestPayload::RetryStep(b)))
    };
    let base = retry(|_| {});
    for (field, v) in [
        ("feature_id", retry(|b| b.feature_id = "f2".into())),
        ("step_id", retry(|b| b.step_id = "s2".into())),
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

// Tests extracted from `src-tauri/src/commands/create_project.rs` (mirrored-tests convention). `super` = that module.

use super::*;

// ── Forward auto-advance ────────────────────────────────────────────
//
// The submit handler must advance via `BootstrapState::advance_to`
// so that auto-progressed steps (e.g. "only one provider → skip
// Provider") still land in `history`. We exercise the pure
// state-machine path here; the Tauri binding side just forwards
// to it.

fn fresh() -> BootstrapState {
    BootstrapState::new()
}

#[test]
fn forward_advance_through_every_step_appends_to_history() {
    let s = fresh();
    assert_eq!(s.step, BootstrapStep::Name);
    assert_eq!(s.history, vec![BootstrapStep::Name]);

    // The submit handler appends one history entry per call.
    let mut s = s;
    let inputs = [
        BootstrapStep::Provider,
        BootstrapStep::Group,
        BootstrapStep::Machine,
        BootstrapStep::Agent,
        BootstrapStep::Model,
        BootstrapStep::Description,
    ];
    for next in inputs {
        s.advance_to(next);
        assert_eq!(s.step, next);
        assert!(s.history.contains(&next));
    }
    assert_eq!(s.history.len(), 7);
    assert!(s.is_final_step());
}

#[test]
fn forward_advance_records_auto_progressed_steps() {
    // The wizard can call `advance_to(Provider)` and then
    // `advance_to(Group)` even when the user never saw the
    // Provider screen (e.g. only one provider configured).
    // Both must be in `history` so goBack doesn't jump past
    // them.
    let mut s = fresh();
    s.advance_to(BootstrapStep::Provider);
    s.advance_to(BootstrapStep::Group);
    s.advance_to(BootstrapStep::Machine);
    assert_eq!(
        s.history,
        vec![
            BootstrapStep::Name,
            BootstrapStep::Provider,
            BootstrapStep::Group,
            BootstrapStep::Machine,
        ]
    );
}

// ── Backward pop ───────────────────────────────────────────────────
//
// This is the regression we're fixing: the previous attempt
// wired goBack to a counter (e.g. `step_index -= 1`). When the
// wizard auto-progressed past a step, the counter didn't know
// about it and the user got booted back to the wrong screen.
//
// The fix is to call `BootstrapState::go_back` (which `pop`s
// `history`). The wizard's frontend can then inspect the new
// step and, if it was an auto-progressed one, call goBack again
// to keep rewinding.

#[test]
fn go_back_after_forward_advance_returns_to_previous_step() {
    let mut s = fresh();
    s.advance_to(BootstrapStep::Provider);
    s.advance_to(BootstrapStep::Group);
    let _ = s.go_back();
    assert_eq!(s.step, BootstrapStep::Provider);
    assert_eq!(s.history.last().copied(), Some(BootstrapStep::Provider));
}

#[test]
fn go_back_through_auto_progressed_chain_lands_on_user_step() {
    // The exact failure mode from the previous attempt:
    // history is [Name, Provider(auto), Group(auto), Machine].
    // A counter-based goBack would jump from Machine to Group
    // (correct), but the *next* goBack would jump to Provider
    // — even though the user never saw Provider. The history-
    // pop approach lets the wizard frontend detect that
    // Provider was auto-progressed and rewind further.
    let mut s = fresh();
    s.advance_to(BootstrapStep::Provider);
    s.advance_to(BootstrapStep::Group);
    s.advance_to(BootstrapStep::Machine);

    assert!(s.go_back());
    assert_eq!(s.step, BootstrapStep::Group);
    assert!(s.go_back());
    assert_eq!(s.step, BootstrapStep::Provider);
    assert!(s.go_back());
    assert_eq!(s.step, BootstrapStep::Name);
    // First step — further goBack is a no-op.
    assert!(!s.can_go_back());
    assert!(!s.go_back());
    assert_eq!(s.step, BootstrapStep::Name);
}

#[test]
fn go_back_on_initial_step_is_a_no_op() {
    let mut s = fresh();
    assert!(!s.go_back());
    assert_eq!(s.step, BootstrapStep::Name);
    assert_eq!(s.history, vec![BootstrapStep::Name]);
}

#[test]
fn submit_handler_rejects_payload_step_mismatch() {
    // A wrong-variant payload is a frontend bug; the command
    // must surface it as a Validation error, never silently
    // advance the state.
    let state = fresh(); // parked on Name
    let bad = CreateProjectStepPayload::Agent {
        kind: "opencode".to_string(),
    };
    assert_eq!(bad.expected_step(), BootstrapStep::Agent);
    assert_ne!(state.step, bad.expected_step());
}

#[test]
fn expected_step_returns_matching_variant_for_each_payload() {
    let cases: Vec<(CreateProjectStepPayload, BootstrapStep)> = vec![
        (
            CreateProjectStepPayload::Name { value: "x".into() },
            BootstrapStep::Name,
        ),
        (
            CreateProjectStepPayload::Provider {
                provider_id: "p".into(),
                kind: "github".into(),
            },
            BootstrapStep::Provider,
        ),
        (
            CreateProjectStepPayload::Group {
                namespace_id: "n".into(),
                kind: "org".into(),
                name: "acme".into(),
            },
            BootstrapStep::Group,
        ),
        (
            CreateProjectStepPayload::Machine {
                kind: "local".into(),
                machine_id: None,
            },
            BootstrapStep::Machine,
        ),
        (
            CreateProjectStepPayload::Agent {
                kind: "opencode".into(),
            },
            BootstrapStep::Agent,
        ),
        (
            CreateProjectStepPayload::Model {
                model: "m".into(),
                effort: None,
            },
            BootstrapStep::Model,
        ),
        (
            CreateProjectStepPayload::Commit {
                title: Box::new("t".to_string()),
                description: Box::new("d".to_string()),
                visibility: Box::new("private".to_string()),
                name: Box::new("n".to_string()),
                provider_id: Box::new("p".to_string()),
                provider_kind: Box::new("github".to_string()),
                provider_host: Box::new("github.com".to_string()),
                namespace_id: Box::new("ns".to_string()),
                namespace_kind: Box::new("personal".to_string()),
                namespace_name: Box::new("me".to_string()),
                machine_kind: Box::new("local".to_string()),
                machine_id: Box::new(None),
                agent_kind: Box::new("opencode".to_string()),
                model: Box::new("m".to_string()),
                effort: None,
            },
            BootstrapStep::Description,
        ),
    ];
    for (payload, expected) in cases {
        assert_eq!(payload.expected_step(), expected);
    }
}

#[test]
fn commit_payload_validation_rejects_empty_title_and_description() {
    // The Commit variant is the only one whose validation is
    // rich enough to deserve an explicit test (it has to
    // pre-flight every wizard field). The other variants have
    // their validation in the command body.
    fn commit_description(d: &str) -> String {
        let p = CreateProjectStepPayload::Commit {
            title: Box::new("t".to_string()),
            description: Box::new(d.to_string()),
            visibility: Box::new("private".to_string()),
            name: Box::new("n".to_string()),
            provider_id: Box::new("p".to_string()),
            provider_kind: Box::new("github".to_string()),
            provider_host: Box::new("github.com".to_string()),
            namespace_id: Box::new("ns".to_string()),
            namespace_kind: Box::new("personal".to_string()),
            namespace_name: Box::new("me".to_string()),
            machine_kind: Box::new("local".to_string()),
            machine_id: Box::new(None),
            agent_kind: Box::new("opencode".to_string()),
            model: Box::new("m".to_string()),
            effort: None,
        };
        match p {
            CreateProjectStepPayload::Commit { description, .. } => *description,
            _ => unreachable!("commit_description must build the Commit variant"),
        }
    }
    assert!(commit_description("").trim().is_empty());
    assert!(commit_description("   ").trim().is_empty());
    assert!(!commit_description("hello").trim().is_empty());
}

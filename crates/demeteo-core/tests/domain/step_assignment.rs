//! The upsert and the status guard, reached directly: no repository, no
//! executor, no runtime.
//!
//! The load-bearing assertions are the two that encode a *choice* rather than
//! an implementation: that a partial change is stored verbatim instead of
//! merged over the existing row, and that an all-`None` change removes the row
//! instead of storing a dead one. Both are what make "reset to inherited" a
//! plain submit with nothing selected, and either one flipping silently would
//! leave a user unable to un-pin a dimension.

use super::*;

fn pin(step_id: &str, agent: Option<&str>, model: Option<&str>) -> StepOverride {
    StepOverride {
        step_id: step_id.to_string(),
        agent_kind: agent.map(str::to_string),
        model: model.map(str::to_string),
        effort: None,
    }
}

fn change(agent: Option<&str>, model: Option<&str>, effort: Option<EffortLevel>) -> StepAssignment {
    StepAssignment {
        agent_kind: agent.map(str::to_string),
        model: model.map(str::to_string),
        effort,
    }
}

#[test]
fn a_step_with_no_entry_gains_one() {
    let out = apply_step_assignment(
        &[],
        "s-implement",
        &change(Some("claude-code"), Some("opus"), Some(EffortLevel::High)),
    );

    assert_eq!(
        out,
        vec![StepOverride {
            step_id: "s-implement".to_string(),
            agent_kind: Some("claude-code".to_string()),
            model: Some("opus".to_string()),
            effort: Some(EffortLevel::High),
        }]
    );
}

#[test]
fn an_unrelated_entry_keeps_its_place_ahead_of_the_new_one() {
    let existing = vec![pin("s-research", Some("opencode"), None)];

    let out = apply_step_assignment(
        &existing,
        "s-implement",
        &change(Some("hermes"), None, None),
    );

    assert_eq!(out.len(), 2);
    assert_eq!(out[0], existing[0]);
    assert_eq!(out[1].step_id, "s-implement");
}

#[test]
fn replacing_an_entry_neither_duplicates_nor_reorders_it() {
    let existing = vec![
        pin("s-research", Some("opencode"), None),
        pin("s-implement", Some("claude-code"), Some("opus")),
        pin("s-review", Some("hermes"), None),
    ];

    let out = apply_step_assignment(
        &existing,
        "s-implement",
        &change(Some("opencode"), Some("sonnet"), None),
    );

    assert_eq!(
        out.iter().map(|o| o.step_id.as_str()).collect::<Vec<_>>(),
        ["s-research", "s-implement", "s-review"]
    );
    assert_eq!(out[1].agent_kind.as_deref(), Some("opencode"));
    assert_eq!(out[1].model.as_deref(), Some("sonnet"));
}

/// The decided semantics (spec Open Question 3): the submitted assignment *is*
/// the stored row. A change that names only an effort therefore drops the
/// previously pinned agent and model rather than keeping them — which is the
/// only way clearing one dimension is expressible at all, since a merge has no
/// spelling for "stop pinning the model".
#[test]
fn a_partial_change_is_stored_verbatim_and_not_merged_over_the_existing_row() {
    let existing = vec![pin("s-implement", Some("claude-code"), Some("opus"))];

    let out = apply_step_assignment(
        &existing,
        "s-implement",
        &change(None, None, Some(EffortLevel::Max)),
    );

    assert_eq!(
        out,
        vec![StepOverride {
            step_id: "s-implement".to_string(),
            agent_kind: None,
            model: None,
            effort: Some(EffortLevel::Max),
        }]
    );
}

#[test]
fn an_all_none_change_removes_the_entry_rather_than_storing_an_empty_one() {
    let existing = vec![
        pin("s-research", Some("opencode"), None),
        pin("s-implement", Some("claude-code"), Some("opus")),
    ];

    let out = apply_step_assignment(&existing, "s-implement", &StepAssignment::default());

    assert_eq!(out, vec![existing[0].clone()]);
}

#[test]
fn an_all_none_change_for_a_step_with_no_entry_returns_the_input_unchanged() {
    let existing = vec![pin("s-research", Some("opencode"), None)];

    let out = apply_step_assignment(&existing, "s-implement", &StepAssignment::default());

    assert_eq!(out, existing);
}

#[test]
fn only_a_change_with_something_in_it_is_not_an_inherit_request() {
    assert!(StepAssignment::default().is_inherit());
    assert!(!change(Some("hermes"), None, None).is_inherit());
    assert!(!change(None, Some("opus"), None).is_inherit());
    assert!(!change(None, None, Some(EffortLevel::Low)).is_inherit());
}

/// Exactly the two statuses whose spawn already happened are refused; every
/// other one is assignable, including a status this build does not know, so
/// the pin applies whenever that node next runs.
#[test]
fn only_the_two_statuses_with_a_live_spawn_refuse_an_assignment() {
    for status in ["running", "verifying"] {
        let refusal = assignment_refusal(status).unwrap_or_else(|| {
            panic!("expected '{status}' to refuse an assignment change");
        });
        assert!(refusal.contains(status), "refusal must name the status");
    }

    for status in [
        "pending",
        "awaiting_gate",
        "failed",
        "interrupted",
        "completed",
        "something_a_later_migration_adds",
    ] {
        assert_eq!(assignment_refusal(status), None, "status '{status}'");
    }
}

/// Every kind a handler resolves an agent for accepts a pin, and the two that
/// spawn nothing refuse one — as does a kind this build has never heard of,
/// which is the case an allowlist exists for.
#[test]
fn only_kinds_that_spawn_an_agent_take_an_assignment() {
    for kind in ["agent", "sequence", "parallel", "sync", "finalize"] {
        assert_eq!(kind_refusal(kind), None, "kind '{kind}'");
    }
    for kind in ["gate", "command", "something_a_later_release_adds"] {
        let refusal = kind_refusal(kind).unwrap_or_else(|| {
            panic!("expected a '{kind}' step to refuse an assignment");
        });
        assert!(
            refusal.contains(kind),
            "refusal must name the kind: {refusal}"
        );
    }
}

#[test]
fn only_a_registered_harness_can_be_pinned() {
    let registered = ["opencode", "claude-code"];
    assert_eq!(harness_refusal("claude-code", &registered), None);

    let refusal =
        harness_refusal("claude_code", &registered).expect("a misspelled harness was accepted");
    assert!(refusal.contains("claude_code"), "{refusal}");
    assert!(refusal.contains("opencode, claude-code"), "{refusal}");
}

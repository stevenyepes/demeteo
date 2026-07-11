// Tests extracted from `src/domain/bootstrap.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::*;

#[test]
fn step_order_has_exactly_seven_entries() {
    // Locked: no strategy / workflow / launching screen sits in
    // the wizard. If you find yourself tempted to extend this,
    // add the screen *after* the wizard completes instead.
    assert_eq!(STEP_ORDER.len(), 7);
}

#[test]
fn step_order_is_documented_sequence() {
    assert_eq!(
        STEP_ORDER,
        [
            BootstrapStep::Name,
            BootstrapStep::Provider,
            BootstrapStep::Group,
            BootstrapStep::Machine,
            BootstrapStep::Agent,
            BootstrapStep::Model,
            BootstrapStep::Description,
        ]
    );
}

#[test]
fn step_order_serialises_as_expected_kebab_strings() {
    let rendered: Vec<&str> = STEP_ORDER.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        rendered,
        [
            "name",
            "provider",
            "group",
            "machine",
            "agent",
            "model",
            "description"
        ]
    );
}

#[test]
fn step_indices_are_zero_through_six() {
    for (i, step) in STEP_ORDER.iter().enumerate() {
        assert_eq!(step.index(), i, "step {step:?} should have index {i}");
        assert_eq!(BootstrapStep::index_of(*step), i);
    }
}

#[test]
fn first_step_has_no_previous() {
    assert_eq!(BootstrapStep::Name.previous(), None);
    assert_eq!(BootstrapStep::Name.next(), Some(BootstrapStep::Provider));
}

#[test]
fn last_step_has_no_next() {
    assert_eq!(BootstrapStep::Description.next(), None);
    assert_eq!(
        BootstrapStep::Description.previous(),
        Some(BootstrapStep::Model)
    );
}

#[test]
fn mid_step_previous_and_next_round_trip() {
    for (i, step) in STEP_ORDER.iter().enumerate() {
        if i > 0 {
            assert_eq!(step.previous(), Some(STEP_ORDER[i - 1]));
        }
        if i + 1 < STEP_ORDER.len() {
            assert_eq!(step.next(), Some(STEP_ORDER[i + 1]));
        }
    }
}

#[test]
fn new_state_is_parked_on_name_with_single_entry_history() {
    let s = BootstrapState::new();
    assert_eq!(s.step, BootstrapStep::Name);
    assert_eq!(s.history, vec![BootstrapStep::Name]);
    assert_eq!(s.step_index(), 0);
    assert!(!s.can_go_back());
    assert_eq!(s.last_user_visible_step(), None);
    assert!(!s.is_final_step());
}

#[test]
fn advance_to_pushes_step_and_updates_current() {
    let mut s = BootstrapState::new();
    s.advance_to(BootstrapStep::Provider);
    assert_eq!(s.step, BootstrapStep::Provider);
    assert_eq!(
        s.history,
        vec![BootstrapStep::Name, BootstrapStep::Provider]
    );
    assert_eq!(s.step_index(), 1);
    assert!(s.can_go_back());
}

#[test]
fn advance_through_full_sequence_records_every_step() {
    let mut s = BootstrapState::new();
    for next in STEP_ORDER.iter().copied().skip(1) {
        s.advance_to(next);
    }
    assert_eq!(s.step, BootstrapStep::Description);
    assert_eq!(s.step_index(), 6);
    assert!(s.is_final_step());
    assert_eq!(s.history, STEP_ORDER.to_vec());
}

#[test]
fn can_go_back_is_false_on_initial_step_and_true_after_one_advance() {
    let mut s = BootstrapState::new();
    assert!(!s.can_go_back());
    s.advance_to(BootstrapStep::Provider);
    assert!(s.can_go_back());
}

#[test]
fn last_user_visible_step_returns_previous_entry() {
    let mut s = BootstrapState::new();
    assert_eq!(s.last_user_visible_step(), None);
    s.advance_to(BootstrapStep::Provider);
    assert_eq!(s.last_user_visible_step(), Some(BootstrapStep::Name));
    s.advance_to(BootstrapStep::Group);
    assert_eq!(s.last_user_visible_step(), Some(BootstrapStep::Provider));
}

#[test]
fn history_records_auto_progressed_steps() {
    // When the wizard auto-progresses past a screen (e.g. only
    // one provider configured), the auto-progressed step must
    // still appear in `history` so `goBack` cannot accidentally
    // jump past it.
    let mut s = BootstrapState::new();
    s.advance_to(BootstrapStep::Provider);
    s.advance_to(BootstrapStep::Group); // assume this is auto-progressed
    s.advance_to(BootstrapStep::Machine);

    assert_eq!(s.step, BootstrapStep::Machine);
    assert_eq!(
        s.history,
        vec![
            BootstrapStep::Name,
            BootstrapStep::Provider,
            BootstrapStep::Group,
            BootstrapStep::Machine,
        ]
    );
    // Group is still in history; if the wizard frontend wants to
    // skip past it on goBack, it can call go_back() and check
    // whether the new step is one it considers auto-progressed.
    assert_eq!(
        s.last_user_visible_step(),
        Some(BootstrapStep::Group),
        "auto-progressed steps must be returned, not filtered out"
    );
}

#[test]
fn go_back_rewinds_to_previous_history_entry() {
    let mut s = BootstrapState::new();
    s.advance_to(BootstrapStep::Provider);
    s.advance_to(BootstrapStep::Group);
    assert!(s.go_back());
    assert_eq!(s.step, BootstrapStep::Provider);
    assert_eq!(
        s.history,
        vec![BootstrapStep::Name, BootstrapStep::Provider]
    );
    assert!(s.can_go_back());
}

#[test]
fn go_back_returns_false_on_initial_step() {
    let mut s = BootstrapState::new();
    assert!(!s.go_back());
    assert_eq!(s.step, BootstrapStep::Name);
    assert_eq!(s.history, vec![BootstrapStep::Name]);
}

#[test]
fn go_back_through_auto_progressed_chain_lands_on_user_step() {
    // history: [Name, Provider(auto), Group(auto), Machine]
    // goBack → Group(auto) → wizard frontend sees auto-progressed
    // and calls goBack again → Provider(auto) → again → Name.
    let mut s = BootstrapState::new();
    s.advance_to(BootstrapStep::Provider);
    s.advance_to(BootstrapStep::Group);
    s.advance_to(BootstrapStep::Machine);

    assert!(s.go_back());
    assert_eq!(s.step, BootstrapStep::Group);
    assert!(s.go_back());
    assert_eq!(s.step, BootstrapStep::Provider);
    assert!(s.go_back());
    assert_eq!(s.step, BootstrapStep::Name);
    // Now at the initial step — further goBack is a no-op.
    assert!(!s.can_go_back());
    assert!(!s.go_back());
    assert_eq!(s.step, BootstrapStep::Name);
}

#[test]
fn starting_at_sets_history_to_single_entry() {
    let s = BootstrapState::starting_at(BootstrapStep::Machine);
    assert_eq!(s.step, BootstrapStep::Machine);
    assert_eq!(s.history, vec![BootstrapStep::Machine]);
    assert_eq!(s.step_index(), 3);
    assert!(!s.can_go_back());
}

#[test]
fn starting_at_final_step_marks_final() {
    let s = BootstrapState::starting_at(BootstrapStep::Description);
    assert!(s.is_final_step());
}

#[test]
fn default_impl_matches_new() {
    let a = BootstrapState::default();
    let b = BootstrapState::new();
    assert_eq!(a, b);
}

#[test]
fn serde_round_trips_state_with_history() {
    let mut s = BootstrapState::new();
    s.advance_to(BootstrapStep::Provider);
    s.advance_to(BootstrapStep::Machine);
    let json = serde_json::to_string(&s).unwrap();
    let back: BootstrapState = serde_json::from_str(&json).unwrap();
    assert_eq!(back, s);
    assert_eq!(back.history, s.history);
    assert_eq!(back.step, s.step);
}

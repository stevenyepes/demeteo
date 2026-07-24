// Tests for `crates/demeteo-core/src/adapters/step_executor/driver/run_loop/`.
// Mirrored-tests convention: `super` is the `run_loop` module.
//
// The testable surface from this decomposition is `RunAction` (the enum
// `apply_outcome` returns to the orchestrator). Full coverage of
// `apply_completed` / `apply_failed` / `apply_environmental` /
// `apply_non_retryable` / `apply_cancelled` / `apply_redirect` would need
// a fully wired `ExecutionDriver` (every port, every repository) — that's
// integration-test territory and lives elsewhere. What we *can* unit-test
// here is the enum's identity semantics: Continue / RedirectTo(idx) /
// Terminate stay distinct, Debug renders, and PartialEq / Eq are sound.

use super::outcome::RunAction;

#[test]
fn run_action_continue_is_distinct_from_terminate() {
    assert_ne!(RunAction::Continue, RunAction::Terminate);
}

#[test]
fn run_action_terminate_is_distinct_from_redirect() {
    assert_ne!(RunAction::Terminate, RunAction::RedirectTo(0));
    assert_ne!(RunAction::Continue, RunAction::RedirectTo(0));
}

#[test]
fn run_action_redirect_carries_target_index() {
    let a = RunAction::RedirectTo(3);
    let b = RunAction::RedirectTo(7);
    assert_ne!(a, b, "different target indices must not compare equal");
    assert_eq!(
        a,
        RunAction::RedirectTo(3),
        "same target index must compare equal"
    );
}

#[test]
fn run_action_continue_is_its_own_singleton() {
    assert_eq!(RunAction::Continue, RunAction::Continue);
}

#[test]
fn run_action_debug_renders_variant_names() {
    // Smoke test: Debug is wired and the variant names show up. Useful
    // for log forensics when a `tracing::debug!` lands one of these.
    assert!(format!("{:?}", RunAction::Continue).contains("Continue"));
    assert!(format!("{:?}", RunAction::Terminate).contains("Terminate"));
    assert!(format!("{:?}", RunAction::RedirectTo(42)).contains("42"));
}

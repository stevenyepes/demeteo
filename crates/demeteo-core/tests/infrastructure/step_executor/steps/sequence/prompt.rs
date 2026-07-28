// Tests extracted from `crates/demeteo-core/src/adapters/step_executor/steps/sequence/prompt.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn no_criteria_declared_renders_the_explicit_fallback() {
    assert_eq!(
        format_acceptance_criteria(&[]),
        "None declared — the task description and the test command define done."
    );
}

/// A stray blank entry (a partially-filled planner template, or an agent
/// emitting `["", ""]`) must not slip past the emptiness check and render
/// as a bare, content-less bullet.
#[test]
fn blank_only_criteria_render_the_explicit_fallback() {
    let acceptance = vec!["".to_string(), "   ".to_string()];
    assert_eq!(
        format_acceptance_criteria(&acceptance),
        "None declared — the task description and the test command define done."
    );
}

#[test]
fn real_criteria_render_as_a_bullet_list() {
    let acceptance = vec![
        "the button is disabled while loading".to_string(),
        "a 4xx response shows the error toast".to_string(),
    ];
    assert_eq!(
        format_acceptance_criteria(&acceptance),
        "- the button is disabled while loading\n- a 4xx response shows the error toast"
    );
}

/// A mix of real and blank entries keeps only the real ones — the blank
/// entry does not become an empty bullet in the middle of the list.
#[test]
fn blank_entries_are_dropped_from_an_otherwise_real_list() {
    let acceptance = vec![
        "the button is disabled while loading".to_string(),
        "".to_string(),
        "a 4xx response shows the error toast".to_string(),
    ];
    assert_eq!(
        format_acceptance_criteria(&acceptance),
        "- the button is disabled while loading\n- a 4xx response shows the error toast"
    );
}

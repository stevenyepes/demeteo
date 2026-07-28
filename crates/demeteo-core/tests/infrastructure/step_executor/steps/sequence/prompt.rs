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

// --- format_completed_tasks --------------------------------------------------
//
// What the agent is told is already on the branch. The three cases are not
// interchangeable wordings of one fact: each wrong one produces a specific,
// expensive failure — reimplementing a finished feature, or hunting for an
// earlier version of a ticket that never had one.

fn done(id: &str, files: &[&str]) -> CompletedTask {
    CompletedTask {
        id: id.to_string(),
        title: format!("{id} title"),
        files: files.iter().map(|f| (*f).to_string()).collect(),
    }
}

#[test]
fn a_pristine_first_task_is_told_so() {
    let out = format_completed_tasks(&[], PlanKind::Greenfield, false);
    assert_eq!(out, "None — this is the first task.");
}

#[test]
fn a_retry_with_nothing_re_run_yet_is_not_told_the_tree_is_empty() {
    // The regression this guards: "this is the first task" over a worktree
    // holding the previous attempt sends the agent to reimplement code it
    // is looking at.
    let out = format_completed_tasks(&[], PlanKind::Greenfield, true);
    assert!(out.contains("previous attempt is already"), "{out}");
    assert!(out.contains("revise it in place"), "{out}");
    assert!(!out.contains("first task"), "{out}");
}

#[test]
fn a_rework_cycle_names_the_landed_feature_and_forbids_redoing_it() {
    // The 25-tickets-re-run bug, at the prompt layer: on a rework cycle the
    // tasks above are the finished feature, and the agent must be told to
    // leave them alone.
    let landed = [done("ticket-01", &["src/lib.rs"]), done("ticket-02", &[])];
    let out = format_completed_tasks(&landed, PlanKind::Rework, true);
    assert!(out.contains("- [ticket-01] ticket-01 title (already committed; touched src/lib.rs)"));
    assert!(out.contains("- [ticket-02] ticket-02 title (already committed)"));
    assert!(out.contains("already implemented and committed"), "{out}");
    assert!(
        out.contains("do not redo, revert, or re-implement"),
        "{out}"
    );
}

#[test]
fn a_rework_cycle_is_never_told_to_revise_the_task_in_place() {
    // A rework ticket is new work. The retry wording ("an earlier version
    // of the task below") describes something that does not exist, and an
    // agent that believes it goes looking for code nobody wrote.
    let landed = [done("ticket-01", &[])];
    for completed in [&landed[..], &[][..]] {
        let out = format_completed_tasks(completed, PlanKind::Rework, true);
        assert!(
            !out.contains("earlier version of the task below"),
            "rework must not claim an earlier version of this ticket exists: {out}"
        );
    }
}

#[test]
fn a_rework_cycle_that_named_nothing_still_says_the_branch_is_not_empty() {
    let out = format_completed_tasks(&[], PlanKind::Rework, true);
    assert!(out.contains("complete previous implementation"), "{out}");
    assert!(out.contains("not a reimplementation"), "{out}");
    assert!(!out.contains("first task"), "{out}");
}

#[test]
fn a_greenfield_list_mid_run_reports_the_prefix_plainly() {
    // No retry, no rework: the ordinary "task 3 of 25" case, which must
    // gain neither of the two warnings.
    let landed = [done("ticket-01", &["a.rs"])];
    let out = format_completed_tasks(&landed, PlanKind::Greenfield, false);
    assert_eq!(
        out,
        "- [ticket-01] ticket-01 title (already committed; touched a.rs)"
    );
}

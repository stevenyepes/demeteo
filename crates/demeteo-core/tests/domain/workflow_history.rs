use super::*;
use crate::domain::ids::{WorkflowId, WorkflowVersionId};
use crate::domain::models::WorkflowVersion;

fn row(workflow: &str, version: u32) -> WorkflowVersion {
    WorkflowVersion {
        id: version_id(&WorkflowId::from(workflow.to_string()), version),
        workflow_id: WorkflowId::from(workflow.to_string()),
        version,
        steps_json: "[]".to_string(),
        definition_json: None,
        note: None,
        created_at: 0,
    }
}

#[test]
fn next_version_number_is_one_past_the_highest() {
    assert_eq!(next_version_number(&[]), 1);

    // A gap must not hand out a number that was already used.
    let rows: Vec<WorkflowVersion> = [1u32, 2, 7].iter().map(|n| row("wf", *n)).collect();
    assert_eq!(next_version_number(&rows), 8);
}

/// Numbering off the *highest*, not the count, is what makes the derived id
/// unique: a deleted middle row must not let the next save reuse a live id.
#[test]
fn next_version_number_is_not_the_row_count() {
    let rows = vec![row("wf", 1), row("wf", 4)];
    assert_eq!(next_version_number(&rows), 5);
}

#[test]
fn version_id_pairs_the_workflow_with_the_number() {
    assert_eq!(
        version_id(&WorkflowId::from("wf-hist".to_string()), 3),
        WorkflowVersionId::from("wf-hist-v3".to_string())
    );
}

#[test]
fn a_version_of_the_named_workflow_is_accepted() {
    let mine = WorkflowId::from("wf-mine".to_string());
    assert!(ensure_owned_by(&row("wf-mine", 1), &mine).is_ok());
}

/// Version ids are guessable, so the pairing is what stops one workflow's
/// history being restored onto another.
#[test]
fn a_version_of_another_workflow_is_refused_and_names_both() {
    let mine = WorkflowId::from("wf-mine".to_string());
    let err = ensure_owned_by(&row("wf-theirs", 1), &mine).expect_err("cross-workflow is refused");
    assert!(err.contains("wf-theirs-v1"), "names the version: {err}");
    assert!(err.contains("wf-theirs"), "names the owner: {err}");
    assert!(err.contains("wf-mine"), "names the caller: {err}");
}

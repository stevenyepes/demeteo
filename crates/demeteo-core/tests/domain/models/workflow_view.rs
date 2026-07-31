use super::*;
use crate::domain::ids::{WorkflowId, WorkflowVersionId};

fn workflow() -> Workflow {
    Workflow {
        id: WorkflowId::from("wf-a".to_string()),
        name: "Alpha".to_string(),
        description: "does alpha".to_string(),
        is_starter: true,
        created_at: 10,
        updated_at: 20,
        schedule: None,
    }
}

fn version(steps_json: &str) -> WorkflowVersion {
    WorkflowVersion {
        id: WorkflowVersionId::from("wf-a-v3".to_string()),
        workflow_id: WorkflowId::from("wf-a".to_string()),
        version: 3,
        steps_json: steps_json.to_string(),
        definition_json: None,
        note: None,
        created_at: 30,
    }
}

#[test]
fn the_version_supplies_the_steps_and_its_own_identity() {
    let joined = WorkflowWithSteps::joined(
        workflow(),
        Some(version(
            r#"[{ "id": "plan", "kind": "agent", "title": "Plan" }]"#,
        )),
    );

    assert_eq!(joined.id, "wf-a");
    assert_eq!(joined.name, "Alpha");
    assert_eq!(joined.description, "does alpha");
    assert!(joined.is_starter);
    assert_eq!((joined.created_at, joined.updated_at), (10, 20));
    assert_eq!(joined.version, 3);
    assert_eq!(joined.version_id, "wf-a-v3");
    assert_eq!(joined.steps.len(), 1);
}

/// A row with no versions is rendered, not refused — `version: 0` with an
/// empty id is the reading the library shows for an entry it cannot open.
#[test]
fn no_version_reads_as_zero_with_an_empty_id() {
    let joined = WorkflowWithSteps::joined(workflow(), None);

    assert_eq!(joined.version, 0);
    assert_eq!(joined.version_id, "");
    assert!(joined.steps.is_empty());
    assert_eq!(joined.name, "Alpha");
}

/// A version whose `steps_json` no longer parses degrades to an empty list
/// rather than failing the read: the row's own identity is still the answer
/// the caller asked for.
#[test]
fn an_unreadable_step_list_degrades_to_empty() {
    let joined = WorkflowWithSteps::joined(workflow(), Some(version("{not json")));

    assert!(joined.steps.is_empty());
    assert_eq!(joined.version, 3);
    assert_eq!(joined.version_id, "wf-a-v3");
}

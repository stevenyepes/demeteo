//! The seed decision, reached without a repository.
//!
//! Before it moved here it was spelled inside the first-launch write loop, so
//! "does this install republish the starter" was observable only by launching
//! the app twice against a database in a known state.

use super::*;
use crate::domain::ids::{WorkflowId, WorkflowVersionId};
use crate::domain::models::{Workflow, WorkflowVersion};

fn file(id: &str, name: &str, description: &str, step_ids: &[&str]) -> String {
    let steps: Vec<serde_json::Value> = step_ids
        .iter()
        .map(|s| serde_json::json!({ "id": s, "kind": "agent", "title": s }))
        .collect();
    serde_json::json!({
        "id": id,
        "name": name,
        "description": description,
        "is_starter": true,
        "steps": steps,
    })
    .to_string()
}

fn stored(id: &str, name: &str, description: &str) -> Workflow {
    Workflow {
        id: WorkflowId::from(id.to_string()),
        name: name.to_string(),
        description: description.to_string(),
        is_starter: true,
        created_at: 0,
        updated_at: 0,
        schedule: None,
    }
}

fn version(id: &str, steps_json: String) -> WorkflowVersion {
    WorkflowVersion {
        id: WorkflowVersionId::from(format!("{id}-v1")),
        workflow_id: WorkflowId::from(id.to_string()),
        version: 1,
        steps_json,
        definition_json: None,
        note: None,
        created_at: 0,
    }
}

fn parsed(json: &str) -> StarterDefinition {
    StarterDefinition::parse(json).expect("fixture is JSON")
}

#[test]
fn a_starter_file_reads_into_its_fields() {
    let starter = parsed(&file("wf-a", "Alpha", "does alpha", &["plan", "build"]));
    assert_eq!(starter.id, WorkflowId::from("wf-a".to_string()));
    assert_eq!(starter.name, "Alpha");
    assert_eq!(starter.description, "does alpha");
    assert!(starter.is_starter);
    assert_eq!(starter.steps.len(), 2);
}

#[test]
fn the_workflow_row_takes_its_timestamps_from_the_seed() {
    let starter = parsed(&file("wf-a", "Alpha", "does alpha", &["plan"]));
    let row = starter.workflow_row(1_700);

    assert_eq!(row.id, starter.id);
    assert_eq!(row.name, "Alpha");
    assert_eq!(row.description, "does alpha");
    assert!(row.is_starter);
    assert_eq!((row.created_at, row.updated_at), (1_700, 1_700));
    assert!(row.schedule.is_none(), "a starter ships unscheduled");
}

/// A starter's version row carries the v1 step list and *no* v2 document:
/// readers migrate the bundled file, which keeps it the single source.
#[test]
fn the_version_row_derives_its_id_and_stores_no_v2_document() {
    let starter = parsed(&file("wf-a", "Alpha", "", &["plan"]));
    let row = starter.version_row(4, "System auto-update", 1_700);

    assert_eq!(row.id, WorkflowVersionId::from("wf-a-v4".to_string()));
    assert_eq!(row.workflow_id, starter.id);
    assert_eq!(row.version, 4);
    assert_eq!(row.steps_json, starter.steps_json().expect("serialize"));
    assert_eq!(row.definition_json, None);
    assert_eq!(row.note.as_deref(), Some("System auto-update"));
    assert_eq!(row.created_at, 1_700);
}

/// Only a file that is not JSON at all leaves nothing to seed; a starter that
/// has lost a field is still seeded, so the library is never short an entry.
#[test]
fn only_unparseable_json_yields_nothing() {
    assert!(StarterDefinition::parse("{ not json").is_none());

    let sparse = parsed(r#"{ "steps": [] }"#);
    assert_eq!(sparse.id, WorkflowId::default());
    assert_eq!(sparse.name, "");
    assert!(!sparse.is_starter);
    assert!(sparse.steps.is_empty());
}

#[test]
fn find_matches_on_the_workflow_id() {
    let a = file("wf-a", "Alpha", "", &["plan"]);
    let b = file("wf-b", "Beta", "", &["plan"]);
    let files = [a.as_str(), b.as_str()];

    let hit = find(&files, &WorkflowId::from("wf-b".to_string())).expect("wf-b is bundled");
    assert_eq!(hit.name, "Beta");
    assert!(find(&files, &WorkflowId::from("wf-c".to_string())).is_none());
}

#[test]
fn an_unknown_starter_is_created() {
    let starter = parsed(&file("wf-a", "Alpha", "", &["plan"]));
    assert_eq!(plan_seed(&starter, None, None), SeedAction::Create);
}

#[test]
fn a_starter_whose_steps_still_match_is_left_alone() {
    let json = file("wf-a", "Alpha", "does alpha", &["plan"]);
    let starter = parsed(&json);
    let latest = version("wf-a", starter.steps_json().expect("serialize"));

    let action = plan_seed(
        &starter,
        Some(&stored("wf-a", "Alpha", "does alpha")),
        Some(&latest),
    );
    assert_eq!(action, SeedAction::Skip);
}

/// The comparison is on the parsed step list, so re-serializing the bundled
/// file through a different field order must not mint a version.
#[test]
fn a_reserialized_but_equal_step_list_is_left_alone() {
    let starter = parsed(&file("wf-a", "Alpha", "", &["plan", "build"]));
    let reordered = serde_json::json!([
        { "kind": "agent", "title": "plan", "id": "plan" },
        { "title": "build", "id": "build", "kind": "agent" },
    ])
    .to_string();

    let action = plan_seed(
        &starter,
        Some(&stored("wf-a", "Alpha", "")),
        Some(&version("wf-a", reordered)),
    );
    assert_eq!(action, SeedAction::Skip);
}

#[test]
fn a_starter_whose_steps_changed_is_republished() {
    let starter = parsed(&file("wf-a", "Alpha", "does alpha", &["plan", "build"]));
    let latest = version(
        "wf-a",
        parsed(&file("wf-a", "Alpha", "does alpha", &["plan"]))
            .steps_json()
            .expect("serialize"),
    );

    let action = plan_seed(
        &starter,
        Some(&stored("wf-a", "Alpha", "does alpha")),
        Some(&latest),
    );
    assert_eq!(action, SeedAction::Republish { rename: false });
}

#[test]
fn a_renamed_starter_republishes_its_metadata_too() {
    let starter = parsed(&file(
        "wf-a",
        "Alpha II",
        "does alpha better",
        &["plan", "build"],
    ));
    let latest = version(
        "wf-a",
        parsed(&file("wf-a", "Alpha", "does alpha", &["plan"]))
            .steps_json()
            .expect("serialize"),
    );

    let action = plan_seed(
        &starter,
        Some(&stored("wf-a", "Alpha", "does alpha")),
        Some(&latest),
    );
    assert_eq!(action, SeedAction::Republish { rename: true });
}

/// The description is metadata the bundle owns too, so a starter whose
/// summary alone was rewritten still republishes it.
#[test]
fn a_restated_description_alone_counts_as_a_rename() {
    let starter = parsed(&file(
        "wf-a",
        "Alpha",
        "does alpha better",
        &["plan", "build"],
    ));
    let latest = version(
        "wf-a",
        parsed(&file("wf-a", "Alpha", "does alpha", &["plan"]))
            .steps_json()
            .expect("serialize"),
    );

    let action = plan_seed(
        &starter,
        Some(&stored("wf-a", "Alpha", "does alpha")),
        Some(&latest),
    );
    assert_eq!(action, SeedAction::Republish { rename: true });
}

/// A workflow row with no version to compare against is left alone rather
/// than republished: nothing has been observed to differ.
#[test]
fn a_stored_workflow_with_no_versions_is_left_alone() {
    let starter = parsed(&file("wf-a", "Alpha", "", &["plan"]));
    let action = plan_seed(
        &starter,
        Some(&stored("wf-a", "Renamed", "elsewhere")),
        None,
    );
    assert_eq!(action, SeedAction::Skip);
}

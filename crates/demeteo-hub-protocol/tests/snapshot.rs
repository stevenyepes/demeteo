use std::collections::BTreeSet;

use demeteo_hub_protocol::{ProjectSnapshot, Snapshot, WorkflowVersionSnapshot};
use serde_json::{json, Value};

const SNAPSHOT: &str = include_str!("fixtures/snapshot.json");

const FORBIDDEN_KEYS: [&str; 3] = ["path", "local_path", "worktree"];

fn collect_keys<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                out.push(k);
                collect_keys(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| collect_keys(i, out)),
        _ => {}
    }
}

fn parsed() -> Snapshot {
    serde_json::from_str(SNAPSHOT).unwrap()
}

#[test]
fn snapshot_round_trips_against_the_fixture() {
    let a: Value = serde_json::from_str(SNAPSHOT).unwrap();
    let b = serde_json::to_value(parsed()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn fixture_covers_the_shapes_the_ticket_names() {
    let s = parsed();
    assert!(s.projects.len() >= 2);
    assert!(s.projects.iter().any(|p| p.remote_url.is_none()));
    assert!(!s.workflows.is_empty());
    let r = &s.run_shape[0];
    assert!(r.default_agent_kind.is_some() && r.default_model.is_some());
    assert!(r.default_effort.is_some() && r.default_workflow_id.is_some());
    assert!(r.default_loop_iterations.is_some() && r.default_max_budget_cents.is_some());
}

#[test]
fn serialised_snapshot_has_no_path_like_key_anywhere() {
    let value = serde_json::to_value(parsed()).unwrap();
    let mut keys = Vec::new();
    collect_keys(&value, &mut keys);
    assert!(!keys.is_empty());
    for forbidden in FORBIDDEN_KEYS {
        assert!(!keys.contains(&forbidden), "found key {forbidden:?}");
    }
}

#[test]
fn each_project_has_exactly_id_name_remote_url() {
    let value = serde_json::to_value(parsed()).unwrap();
    for p in value["projects"].as_array().unwrap() {
        let keys: BTreeSet<&str> = p.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, BTreeSet::from(["id", "name", "remote_url"]));
    }
}

#[test]
fn a_project_with_a_path_key_does_not_parse() {
    let with_path = json!({"id": "p", "name": "n", "remote_url": null, "path": "/home/u/repo"});
    assert!(serde_json::from_value::<ProjectSnapshot>(with_path).is_err());
}

/// The crate cannot see inside a Workflow's documents, so a local path in one
/// crosses untouched: stripping it is the snapshot builder's obligation, on
/// [`WorkflowVersionSnapshot`]'s rustdoc.
#[test]
fn workflow_documents_carry_a_local_path_unchanged_because_only_the_builder_can_strip_it() {
    let steps = r#"[{"id":"s-test","kind":"command","command":"make test","cwd":"/Users/x"}]"#;
    let definition = r#"{"nodes":[{"config":{"cwd":"/Users/x"}}],"edges":[]}"#;
    let wire = json!({
        "id": "wv-1", "workflow_id": "wf", "version": 1,
        "steps_json": steps, "definition_json": definition,
        "note": null, "created_at": 0,
    });
    let parsed: WorkflowVersionSnapshot = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(parsed.steps_json, steps);
    assert_eq!(parsed.definition_json.as_deref(), Some(definition));
    assert_eq!(serde_json::to_value(parsed).unwrap(), wire);
}

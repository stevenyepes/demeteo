//! Published-schema + boundary-validation coverage (task P1.3).

use super::*;
use crate::domain::models::workflow_migrate::migrate_definition;
use std::path::Path;

fn published_schema_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs-site/workflow-schema-v2.json")
}

/// The committed schema file must match what the structs generate.
/// Regenerate with `UPDATE_SCHEMAS=1 cargo test -p demeteo-core published_schema`.
#[test]
fn published_schema_is_current() {
    let generated = serde_json::to_string_pretty(&workflow_v2_schema()).unwrap() + "\n";
    let path = published_schema_path();

    if std::env::var("UPDATE_SCHEMAS").is_ok() {
        std::fs::write(&path, &generated).expect("write schema");
        return;
    }

    let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e}\n(run UPDATE_SCHEMAS=1 cargo test -p demeteo-core published_schema to create it)",
            path.display()
        )
    });
    assert_eq!(
        committed, generated,
        "docs-site/workflow-schema-v2.json is stale — regenerate with \
         UPDATE_SCHEMAS=1 cargo test -p demeteo-core published_schema"
    );
}

#[test]
fn valid_v2_documents_pass_validation() {
    // Every migrated starter is schema-valid — the schema and the
    // migration cannot drift apart without this failing.
    for name in [
        "bugfix-pipeline",
        "ci-fix",
        "code-review",
        "docs-update",
        "experiment",
        "refactor",
        "simple-task",
        "standard-feature-pipeline",
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src-tauri/workflows")
            .join(format!("{name}.json"));
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let migrated = migrate_definition(&doc).unwrap();
        let v2 = serde_json::to_value(&migrated).unwrap();
        validate_workflow_v2(&v2).unwrap_or_else(|e| panic!("{name} failed validation:\n{e}"));
    }
}

#[test]
fn invalid_v2_documents_fail_with_located_readable_errors() {
    // Missing required node fields.
    let err = validate_workflow_v2(&serde_json::json!({
        "schema_version": 2,
        "id": "wf-x",
        "name": "X",
        "nodes": [ { "id": "a" } ],
        "edges": []
    }))
    .unwrap_err();
    assert!(err.contains("/nodes/0"), "points at the offender: {err}");

    // Wrong type for a whole section.
    let err = validate_workflow_v2(&serde_json::json!({
        "schema_version": 2,
        "id": "wf-x",
        "name": "X",
        "nodes": "not-a-list",
        "edges": []
    }))
    .unwrap_err();
    assert!(err.contains("/nodes"), "{err}");

    // Enum out of range.
    let err = validate_workflow_v2(&serde_json::json!({
        "schema_version": 2,
        "id": "wf-x",
        "name": "X",
        "nodes": [],
        "edges": [],
        "defaults": { "join": "most_success" }
    }))
    .unwrap_err();
    assert!(err.contains("/defaults/join"), "{err}");

    // Bad retry strategy nested deep.
    let err = validate_workflow_v2(&serde_json::json!({
        "schema_version": 2,
        "id": "wf-x",
        "name": "X",
        "nodes": [ { "id": "a", "type": "agent", "title": "A",
            "retry": { "verdict": { "strategy": "pray" } } } ],
        "edges": []
    }))
    .unwrap_err();
    // Option<T> schemas are anyOf [T, null], so the reported path stops at
    // the optional field rather than descending into the variant.
    assert!(err.contains("/nodes/0/retry"), "{err}");
}

#[test]
fn unknown_fields_stay_schema_valid() {
    // Mirrors the serde posture: forward-compatible documents validate.
    validate_workflow_v2(&serde_json::json!({
        "schema_version": 2,
        "id": "wf-x",
        "name": "X",
        "future_field": { "anything": true },
        "nodes": [ { "id": "a", "type": "agent", "title": "A", "future": 1 } ],
        "edges": []
    }))
    .expect("additional properties are allowed");
}

//! What a run's declared start point survives on the wire, and what happens
//! to one this build cannot read. `super` is `crate::domain::run_spec`.

use super::*;

/// A spec with every required field and nothing optional, so each test states
/// only the part it is about.
fn spec_carrying(fields: serde_json::Value) -> RunSpec {
    let mut doc = serde_json::json!({
        "title": "review PR 12",
        "description": "d",
        "provider": { "kind": "github", "host": "github.com" },
        "repo_path": "acme/widgets",
        "workflow_json": { "steps": [] },
        "agent_kind": "claude-code",
        "model": "opus",
    });
    let object = doc.as_object_mut().expect("the base spec is an object");
    for (key, value) in fields.as_object().expect("fields are an object") {
        object.insert(key.clone(), value.clone());
    }
    serde_json::from_value(doc).expect("the spec parses")
}

fn round_trip(spec: &RunSpec) -> RunSpec {
    let json = serde_json::to_string(spec).expect("a spec serializes");
    serde_json::from_str(&json).expect("a spec round-trips")
}

#[test]
fn a_spec_from_a_client_that_predates_origins_starts_from_the_default_branch() {
    let spec = spec_carrying(serde_json::json!({}));
    assert_eq!(spec.origin, None);
    assert_eq!(
        spec.origin_to_honour(),
        Ok(FeatureOrigin::DefaultBranch),
        "every detached run before this field cut its branch from the default branch"
    );
}

#[test]
fn a_declared_origin_survives_the_wire() {
    for declared in [
        FeatureOrigin::Branch {
            base: "release/2.0".to_string(),
        },
        FeatureOrigin::Ref {
            fetch_spec: "refs/pull/12/head".to_string(),
            label: "PR #12".to_string(),
        },
    ] {
        let spec = spec_carrying(serde_json::json!({
            "origin": serde_json::to_value(&declared).expect("an origin serializes"),
        }));
        assert_eq!(
            round_trip(&spec).origin_to_honour(),
            Ok(declared.clone()),
            "a runner reads this back out of `runner_runs.spec_json` on every resume"
        );
    }
}

#[test]
fn the_review_base_survives_the_wire() {
    let spec = spec_carrying(serde_json::json!({ "diff_base_branch": "develop" }));
    assert_eq!(
        round_trip(&spec).diff_base_branch.as_deref(),
        Some("develop")
    );
}

#[test]
fn an_origin_this_build_cannot_read_is_refused_and_not_defaulted() {
    let spec = spec_carrying(serde_json::json!({
        "origin": { "kind": "stacked_on", "parent": "f-1" },
    }));
    let refusal = spec
        .origin_to_honour()
        .expect_err("an origin from a newer client is not this runner's to reinterpret");
    assert!(
        refusal.contains("stacked_on"),
        "the refusal has to name what it could not honour: {refusal}"
    );
}

#[test]
fn an_unreadable_origin_does_not_cost_the_rest_of_the_spec() {
    let spec = spec_carrying(serde_json::json!({
        "origin": { "kind": "stacked_on", "parent": "f-1" },
        "diff_base_branch": "develop",
    }));
    assert_eq!(spec.title, "review PR 12");
    assert_eq!(spec.diff_base_branch.as_deref(), Some("develop"));
}

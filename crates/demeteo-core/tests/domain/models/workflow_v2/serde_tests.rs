//! Serde round-trip coverage for the schema-v2 definition model (task P1.1).
//!
//! The fixture below is the PRD §5.1 example (docs/PRD_DAG_WORKFLOWS.md),
//! with the JSONC comments stripped (serde_json parses strict JSON) and the
//! `retry` placeholder comment replaced by the §5.4 policy block so the
//! round-trip also exercises the retry structs.

use super::*;

const PRD_EXAMPLE: &str = r#"{
  "schema_version": 2,
  "id": "wf-starter-standard",
  "name": "Standard Feature Pipeline",
  "nodes": [
    {
      "id": "research",
      "type": "agent",
      "type_version": 1,
      "title": "Research Codebase",
      "config": {
        "prompt_template": "...",
        "agent_kind": null,
        "model": null,
        "effort": null,
        "capability": "artifacts",
        "outputs": [ { "name": "report", "type": "file", "path": "artifacts/research-report.md" } ]
      },
      "retry": {
        "environment":   { "strategy": "in_place", "max_attempts": 2, "backoff_secs": 30 },
        "verdict":       { "strategy": "redirect", "to": "implement", "max_attempts": 3, "feedback": true },
        "agent_failure": { "strategy": "in_place", "max_attempts": 2, "feedback": true },
        "non_retryable": { "strategy": "fail" }
      },
      "position": { "x": 0, "y": 0 }
    }
  ],
  "edges": [
    { "from": "research", "to": "tickets" },
    { "from": "critic", "to": "gate-ship", "when": "${{ nodes.critic.outputs.verdict != 'FAIL' }}" }
  ],
  "defaults": { "join": "all_success" }
}"#;

#[test]
fn prd_example_round_trips() {
    let def: WorkflowDefinitionV2 = serde_json::from_str(PRD_EXAMPLE).unwrap();

    assert_eq!(def.schema_version, WORKFLOW_SCHEMA_V2);
    assert_eq!(def.id.as_str(), "wf-starter-standard");
    assert_eq!(def.name, "Standard Feature Pipeline");
    assert_eq!(def.nodes.len(), 1);
    assert_eq!(def.edges.len(), 2);
    assert_eq!(def.defaults.join, Some(JoinSemantics::AllSuccess));

    let node = &def.nodes[0];
    assert_eq!(node.id.as_str(), "research");
    assert_eq!(node.node_type, "agent");
    assert_eq!(node.type_version, 1);
    assert_eq!(node.title, "Research Codebase");
    // The per-type payload stays opaque; spot-check it survived intact.
    assert_eq!(node.config["capability"], "artifacts");
    assert_eq!(
        node.config["outputs"][0]["path"],
        "artifacts/research-report.md"
    );
    assert_eq!(node.position, Some(Position { x: 0.0, y: 0.0 }));

    let retry = node.retry.as_ref().unwrap();
    let verdict = retry.verdict.as_ref().unwrap();
    assert_eq!(verdict.strategy, RetryStrategy::Redirect);
    assert_eq!(verdict.redirect_to.as_ref().unwrap().as_str(), "implement");
    assert_eq!(verdict.max_attempts, Some(3));
    assert!(verdict.feedback);
    let env = retry.environment.as_ref().unwrap();
    assert_eq!(env.strategy, RetryStrategy::InPlace);
    assert_eq!(env.backoff_secs, Some(30));
    assert!(!env.feedback);
    assert_eq!(
        retry.non_retryable.as_ref().unwrap().strategy,
        RetryStrategy::Fail
    );

    let guarded = &def.edges[1];
    assert_eq!(guarded.from.as_str(), "critic");
    assert_eq!(guarded.to.as_str(), "gate-ship");
    assert_eq!(
        guarded.when.as_deref(),
        Some("${{ nodes.critic.outputs.verdict != 'FAIL' }}")
    );

    // Value-level round-trip: serialize → reparse → identical model.
    let json = serde_json::to_string_pretty(&def).unwrap();
    let back: WorkflowDefinitionV2 = serde_json::from_str(&json).unwrap();
    assert_eq!(back, def);
}

#[test]
fn redirect_short_form_to_normalizes_to_redirect_to() {
    // PRD §5.4 writes the target as `"to"`; the canonical field is
    // `redirect_to`. Input accepts both; output emits only the canonical.
    let rule: RetryRule = serde_json::from_str(
        r#"{ "strategy": "redirect", "to": "implement", "max_attempts": 3, "feedback": true }"#,
    )
    .unwrap();
    assert_eq!(rule.redirect_to.as_ref().unwrap().as_str(), "implement");

    let out = serde_json::to_value(&rule).unwrap();
    assert_eq!(out["redirect_to"], "implement");
    assert!(out.get("to").is_none());

    let canonical: RetryRule =
        serde_json::from_str(r#"{ "strategy": "redirect", "redirect_to": "implement" }"#).unwrap();
    assert_eq!(
        canonical.redirect_to.as_ref().unwrap().as_str(),
        "implement"
    );
}

#[test]
fn minimal_node_gets_defaults() {
    let node: NodeConfig =
        serde_json::from_str(r#"{ "id": "gate-ship", "type": "gate", "title": "Ship?" }"#).unwrap();
    assert_eq!(node.type_version, 1, "type_version defaults to 1");
    assert!(node.config.is_null());
    assert!(node.retry.is_none());
    assert!(node.join.is_none());
    assert!(node.position.is_none());
}

#[test]
fn unknown_fields_are_tolerated_not_rejected() {
    // Machine-checkable rejection is P1.3's JSON Schema at the write
    // boundaries; the serde layer stays permissive so old app versions
    // can read definitions written by newer ones.
    let def: WorkflowDefinitionV2 = serde_json::from_str(
        r#"{
          "schema_version": 2,
          "id": "wf-x",
          "name": "X",
          "future_top_level": true,
          "nodes": [
            { "id": "a", "type": "agent", "title": "A", "future_node_field": 7,
              "retry": { "verdict": { "strategy": "fail", "future_rule_field": [] } } }
          ],
          "edges": [ { "from": "a", "to": "a", "future_edge_field": "x" } ]
        }"#,
    )
    .unwrap();
    assert_eq!(def.nodes[0].id.as_str(), "a");
    assert_eq!(
        def.nodes[0]
            .retry
            .as_ref()
            .unwrap()
            .verdict
            .as_ref()
            .unwrap()
            .strategy,
        RetryStrategy::Fail
    );
}

#[test]
fn empty_defaults_are_omitted_from_output() {
    let def = WorkflowDefinitionV2 {
        schema_version: WORKFLOW_SCHEMA_V2,
        id: "wf-min".into(),
        name: "Min".into(),
        nodes: vec![],
        edges: vec![],
        defaults: WorkflowDefaults::default(),
    };
    let out = serde_json::to_value(&def).unwrap();
    assert!(out.get("defaults").is_none());
}

#[test]
fn enum_wire_forms_are_snake_case() {
    for (variant, wire) in [
        (JoinSemantics::AllSuccess, "\"all_success\""),
        (JoinSemantics::AnySuccess, "\"any_success\""),
        (JoinSemantics::AllDone, "\"all_done\""),
    ] {
        assert_eq!(serde_json::to_string(&variant).unwrap(), wire);
        let back: JoinSemantics = serde_json::from_str(wire).unwrap();
        assert_eq!(back, variant);
    }
    for (variant, wire) in [
        (PortType::Text, "\"text\""),
        (PortType::File, "\"file\""),
        (PortType::TaskList, "\"task_list\""),
        (PortType::Verdict, "\"verdict\""),
        (PortType::Approval, "\"approval\""),
        (PortType::Any, "\"any\""),
    ] {
        assert_eq!(serde_json::to_string(&variant).unwrap(), wire);
        let back: PortType = serde_json::from_str(wire).unwrap();
        assert_eq!(back, variant);
    }
    for (variant, wire) in [
        (RetryStrategy::InPlace, "\"in_place\""),
        (RetryStrategy::Redirect, "\"redirect\""),
        (RetryStrategy::Fail, "\"fail\""),
    ] {
        assert_eq!(serde_json::to_string(&variant).unwrap(), wire);
        let back: RetryStrategy = serde_json::from_str(wire).unwrap();
        assert_eq!(back, variant);
    }
}

#[test]
fn port_type_compatibility() {
    assert!(PortType::TaskList.compatible_with(PortType::TaskList));
    assert!(PortType::Any.compatible_with(PortType::Verdict));
    assert!(PortType::Verdict.compatible_with(PortType::Any));
    assert!(!PortType::Text.compatible_with(PortType::File));
    assert!(!PortType::Verdict.compatible_with(PortType::Approval));
}

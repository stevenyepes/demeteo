// Tests extracted from `crates/demeteo-core/src/adapters/step_executor/steps/command.rs`
// (mirrored-tests convention). `super` = that module.
//
// The end-to-end proof that a command node *runs* lives in
// `tests/conformance/command_node.rs` (task P3.5 "Done when"); this file
// covers the pure surfaces: config parsing/validation, the lint rules the
// builder renders, the resume policy that ties `idempotent` to P1.14, and
// the registry projection that puts the type in the palette for free.

use super::*;
use crate::domain::models::workflow_v2::WorkflowDefinitionV2;

fn step_from(config: serde_json::Value) -> StepConfig {
    let mut value = serde_json::json!({
        "id": "s-harness",
        "kind": "command",
        "title": "Baseline harness",
    });
    let obj = value.as_object_mut().unwrap();
    for (k, v) in config.as_object().unwrap() {
        obj.insert(k.clone(), v.clone());
    }
    serde_json::from_value(value).expect("step parses")
}

// ── Config parsing ───────────────────────────────────────────────────────────

#[test]
fn parses_a_full_command_config() {
    let spec = parse_spec(&step_from(serde_json::json!({
        "command": "  cargo test --all  ",
        "cwd": "crates/core",
        "env_allowlist": ["CI", "RUSTFLAGS"],
        "timeout_secs": 900,
        "idempotent": true,
    })))
    .expect("valid config");

    // The command is trimmed but never rewritten — it is the author's shell.
    assert_eq!(spec.command, "cargo test --all");
    assert_eq!(spec.cwd.as_deref(), Some("crates/core"));
    assert_eq!(spec.env_allowlist, vec!["CI", "RUSTFLAGS"]);
    assert_eq!(spec.timeout, Some(Duration::from_secs(900)));
    assert!(spec.idempotent);
}

#[test]
fn defaults_are_the_cautious_reading() {
    let spec = parse_spec(&step_from(serde_json::json!({ "command": "make" })))
        .expect("command alone is enough to run");
    assert_eq!(spec.cwd, None);
    assert!(
        spec.env_allowlist.is_empty(),
        "nothing crosses unnamed (D2)"
    );
    assert_eq!(spec.timeout, None);
    assert!(
        !spec.idempotent,
        "an undeclared command must not be silently re-runnable"
    );
}

#[test]
fn a_missing_or_blank_command_is_a_config_error() {
    for config in [
        serde_json::json!({}),
        serde_json::json!({ "command": "" }),
        serde_json::json!({ "command": "   " }),
    ] {
        let err = parse_spec(&step_from(config)).expect_err("must be refused");
        assert!(err.contains("no `command`"), "unexpected message: {err}");
    }
}

#[test]
fn cwd_may_not_escape_the_worktree() {
    for bad in ["/etc", "~/secrets", "../sibling", "src/../../out"] {
        let err = parse_spec(&step_from(
            serde_json::json!({ "command": "ls", "cwd": bad }),
        ))
        .expect_err("must be refused");
        assert!(
            err.contains("worktree"),
            "'{bad}' should be refused as escaping, got: {err}"
        );
    }
    // A path that merely *contains* dots is fine — only a `..` segment escapes.
    assert!(parse_spec(&step_from(
        serde_json::json!({ "command": "ls", "cwd": "src/..hidden" })
    ))
    .is_ok());
}

#[test]
fn a_zero_timeout_is_refused_rather_than_meaning_forever() {
    let err = parse_spec(&step_from(
        serde_json::json!({ "command": "ls", "timeout_secs": 0 }),
    ))
    .expect_err("must be refused");
    assert!(err.contains("greater than zero"), "got: {err}");
}

// ── Env allowlist ────────────────────────────────────────────────────────────

#[test]
fn env_forwards_only_named_and_set_variables() {
    std::env::set_var("DEMETEO_CMD_TEST_VAR", "present");
    std::env::remove_var("DEMETEO_CMD_TEST_ABSENT");
    let env = resolve_env(&[
        "DEMETEO_CMD_TEST_VAR".to_string(),
        "DEMETEO_CMD_TEST_ABSENT".to_string(),
    ]);
    assert_eq!(
        env.get("DEMETEO_CMD_TEST_VAR").map(String::as_str),
        Some("present")
    );
    assert!(
        !env.contains_key("DEMETEO_CMD_TEST_ABSENT"),
        "an unset name is skipped, not exported empty"
    );
    std::env::remove_var("DEMETEO_CMD_TEST_VAR");
}

// ── Failure feedback ─────────────────────────────────────────────────────────

#[test]
fn long_output_is_tailed_for_feedback_but_marked() {
    let short = "all good";
    assert_eq!(tail(short, 100), short);

    let long: String = "x".repeat(5_000);
    let cut = tail(&long, 100);
    assert!(cut.starts_with("…(truncated)…"));
    assert!(cut.ends_with("xxxx"));
    assert!(cut.len() < long.len());
}

#[test]
fn tail_respects_char_boundaries() {
    // A multi-byte tail must not panic on a mid-codepoint slice.
    let text: String = "é".repeat(200);
    let cut = tail(&text, 51);
    assert!(cut.contains('é'));
}

// ── Lint (what the builder shows before a run costs anything) ────────────────

fn lint_node(config: serde_json::Value) -> Vec<LintFinding> {
    let def: WorkflowDefinitionV2 = serde_json::from_value(serde_json::json!({
        "schema_version": 2,
        "id": "wf-command-lint",
        "name": "command lint",
        "nodes": [
            { "id": "n1", "type": "command", "title": "Run it", "config": config }
        ],
        "edges": []
    }))
    .expect("definition parses");
    let graph = WorkflowGraph::build(&def).expect("single node graph");
    CommandNodeHandler.lint(&def.nodes[0], &graph)
}

fn codes(findings: &[LintFinding]) -> Vec<&'static str> {
    findings.iter().map(|f| f.code).collect()
}

#[test]
fn lint_flags_a_command_node_with_nothing_to_run() {
    let findings = lint_node(serde_json::json!({}));
    assert!(codes(&findings).contains(&"command-missing"));
    assert!(findings.iter().any(|f| f.code == "command-missing"
        && f.severity == crate::domain::workflow_graph::LintSeverity::Error));
}

#[test]
fn lint_flags_an_escaping_cwd() {
    let findings = lint_node(serde_json::json!({ "command": "ls", "cwd": "../../etc" }));
    assert!(codes(&findings).contains(&"command-cwd-escapes"));
}

#[test]
fn lint_warns_but_does_not_block_when_idempotence_is_undeclared() {
    let findings = lint_node(serde_json::json!({ "command": "make build" }));
    let warning = findings
        .iter()
        .find(|f| f.code == "command-not-idempotent")
        .expect("undeclared idempotence is surfaced");
    // Warnings must never block a save (PRD §6.3).
    assert_eq!(
        warning.severity,
        crate::domain::workflow_graph::LintSeverity::Warning
    );
    assert!(
        !findings
            .iter()
            .any(|f| f.severity == crate::domain::workflow_graph::LintSeverity::Error),
        "a runnable command node must stay savable"
    );
}

#[test]
fn a_fully_declared_command_node_lints_clean() {
    let findings = lint_node(serde_json::json!({
        "command": "cargo test",
        "cwd": "crates/core",
        "idempotent": true,
    }));
    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

// ── Resume policy (the P1.14 tie-in) ─────────────────────────────────────────

#[test]
fn only_a_declared_idempotent_command_may_auto_resume() {
    let idempotent = step_from(serde_json::json!({ "command": "cargo test", "idempotent": true }));
    assert_eq!(
        CommandNodeHandler.resume_policy(&idempotent),
        ResumePolicy::WhenUnchanged
    );

    for config in [
        serde_json::json!({ "command": "./deploy.sh", "idempotent": false }),
        // Undeclared and unparseable both resolve to "ask a human".
        serde_json::json!({ "command": "./deploy.sh" }),
        serde_json::json!({}),
    ] {
        assert_eq!(
            CommandNodeHandler.resume_policy(&step_from(config)),
            ResumePolicy::AlwaysAsk
        );
    }
}

// ── Registry projection (the extensibility claim) ────────────────────────────

#[test]
fn the_command_type_reaches_the_palette_without_a_frontend_edit() {
    let entry = crate::adapters::step_executor::node_catalog::node_type_catalog()
        .into_iter()
        .find(|e| e.kind == KIND)
        .expect("registration alone puts it in the catalog");

    assert_eq!(entry.label, "Command");
    assert!(!entry.summary.is_empty());
    // The config panel renders from this; an empty schema would leave the
    // node unconfigurable in the builder.
    assert!(entry.config_schema["properties"]["command"].is_object());
    assert!(entry.max_instances.is_none(), "commands are unbounded");
    // Produces both port types, so it can feed an agent step (text) or a
    // sequence's task list (file).
    assert!(entry.outputs.contains(&PortType::Text));
    assert!(entry.outputs.contains(&PortType::File));
}

// ── The baseline node (HB2b / P4.2a) ─────────────────────────────────────────
//
// One `command` node whose commands are not in the workflow: it runs *this
// project's* prepare command and validation gates, which a workflow file
// cannot know. Everything below is about that one exception not leaking into
// the ordinary command node's contract.

#[test]
fn a_baseline_node_needs_no_command_of_its_own() {
    let spec = parse_spec(&step_from(serde_json::json!({ "measure_baseline": true })))
        .expect("the commands come from the project, not the workflow");
    assert!(spec.measure_baseline);
    assert!(spec.command.is_empty());
}

#[test]
fn measure_baseline_is_off_unless_asked_for() {
    // The exemption above must not reach any other command node: an ordinary
    // one that forgot its command is still a definition bug.
    let spec = parse_spec(&step_from(serde_json::json!({ "command": "make" }))).unwrap();
    assert!(!spec.measure_baseline);
    assert!(parse_spec(&step_from(serde_json::json!({ "measure_baseline": false }))).is_err());
}

#[test]
fn lint_does_not_demand_a_command_from_a_baseline_node() {
    let findings = lint_node(serde_json::json!({ "measure_baseline": true }));
    assert!(
        !codes(&findings).contains(&"command-missing"),
        "unexpected findings: {findings:?}"
    );
    assert!(
        findings.is_empty(),
        "a baseline node is fully declared by that one flag: {findings:?}"
    );
}

#[test]
fn a_baseline_node_is_not_nagged_about_idempotence() {
    // It measures a commit that cannot change. Asking the author to confirm it
    // is safe to repeat is a question they cannot act on.
    let findings = lint_node(serde_json::json!({ "measure_baseline": true }));
    assert!(!codes(&findings).contains(&"command-not-idempotent"));
}

#[test]
fn a_baseline_node_auto_resumes_after_an_interrupt() {
    // Re-running it cannot do anything twice — it only reads a commit and
    // writes a record — so it never parks at the synthetic gate.
    assert_eq!(
        CommandNodeHandler
            .resume_policy(&step_from(serde_json::json!({ "measure_baseline": true }))),
        ResumePolicy::WhenUnchanged
    );
}

#[test]
fn the_baseline_flag_is_configurable_from_the_builder() {
    let entry = crate::adapters::step_executor::node_catalog::node_type_catalog()
        .into_iter()
        .find(|e| e.kind == KIND)
        .expect("registered");
    assert!(
        entry.config_schema["properties"]["measure_baseline"].is_object(),
        "a field the config panel cannot render is a field nobody can set"
    );
}

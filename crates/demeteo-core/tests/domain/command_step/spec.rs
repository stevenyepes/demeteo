use super::*;

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

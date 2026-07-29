// Tests extracted from `src/domain/verifier.rs` (mirrored-tests convention).
// `super` resolves to that module.
//
// HB5 (`docs/HARNESS_BASELINE.md`): which harnesses gate a step, and what
// deadline each of them gets. Both are *policy decisions*, so they live in
// `domain/` as synchronous free functions and are decidable here without a
// single port double — the `async fn` that runs the commands only executes what
// these returned.

use std::collections::HashMap;

use super::{resolve_harnesses, ResolvedHarness, VerifierConfig, DEFAULT_HARNESS_NAME};
use crate::domain::models::WorktreeStrategy;

const CEILING_S: u64 = 600;

fn strategy(
    test_command: Option<&str>,
    harnesses: &[(&str, &str)],
    gates: Option<&[&str]>,
) -> WorktreeStrategy {
    WorktreeStrategy {
        default_branch: "main".to_string(),
        branch_prefix: "demeteo/features/".to_string(),
        test_command: test_command.map(str::to_string),
        build_command: None,
        coverage_command: None,
        conventions_file: None,
        pr_template: None,
        harnesses: (!harnesses.is_empty()).then(|| {
            harnesses
                .iter()
                .map(|(n, c)| (n.to_string(), c.to_string()))
                .collect::<HashMap<_, _>>()
        }),
        validation_gates: gates.map(|g| g.iter().map(|s| s.to_string()).collect()),
        prepare_command: None,
        extra_writable_paths: Vec::new(),
    }
}

fn declared(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

/// `(name, command)` pairs, for readable assertions.
fn pairs(resolved: &[ResolvedHarness]) -> Vec<(&str, &str)> {
    resolved
        .iter()
        .map(|r| (r.name.as_str(), r.command.as_str()))
        .collect()
}

// ── The resolution chain: most specific wins ─────────────────────────────────

#[test]
fn tier_one_the_step_declaration_beats_everything_below_it() {
    let s = strategy(
        Some("npm test"),
        &[("lint", "npm run lint"), ("unit", "npm run unit")],
        Some(&["lint"]),
    );

    let resolved = resolve_harnesses(&declared(&["unit"]), &s, CEILING_S);

    assert_eq!(
        pairs(&resolved),
        vec![("unit", "npm run unit")],
        "the workflow author was explicit — neither the project's selection nor \
         its test_command gets a say"
    );
}

#[test]
fn tier_two_gates_the_step_when_the_workflow_declares_nothing() {
    // The tier that makes the `harnesses` map reachable at all: **all seven
    // starters declare no harness**, so before this a user could add
    // `lint → npm run lint`, see it accepted, and nothing would ever run it.
    let s = strategy(
        Some("npm test"),
        &[("lint", "npm run lint"), ("unit", "npm run unit")],
        Some(&["lint", "unit"]),
    );

    let resolved = resolve_harnesses(&[], &s, CEILING_S);

    assert_eq!(
        pairs(&resolved),
        vec![("lint", "npm run lint"), ("unit", "npm run unit")],
        "a starter that declares nothing must fall through to the project's \
         selection, in the user's order"
    );
}

#[test]
fn tier_two_is_not_additive() {
    // Tempting as "always *also* run these" is for a safety property, it makes
    // narrowing impossible and produces the surprise where a workflow pinned to
    // `unit` still runs the 20-minute integration suite.
    let s = strategy(
        Some("npm test"),
        &[
            ("lint", "npm run lint"),
            ("unit", "npm run unit"),
            ("integration", "npm run integration"),
        ],
        Some(&["lint", "integration"]),
    );

    let resolved = resolve_harnesses(&declared(&["unit"]), &s, CEILING_S);

    assert_eq!(
        pairs(&resolved),
        vec![("unit", "npm run unit")],
        "the project's gates must not be appended to an explicit declaration"
    );
}

#[test]
fn an_empty_tier_two_reproduces_todays_test_command_fallback() {
    // The compatibility claim the whole task rests on: with no selection made,
    // every starter must behave exactly as it did before this existed.
    for gates in [None, Some(&[] as &[&str])] {
        let s = strategy(Some("npm test"), &[("lint", "npm run lint")], gates);
        let resolved = resolve_harnesses(&[], &s, CEILING_S);
        assert_eq!(
            pairs(&resolved),
            vec![(DEFAULT_HARNESS_NAME, "npm test")],
            "gates={gates:?} must resolve to the project's test_command alone"
        );
    }
}

#[test]
fn a_stale_gate_selection_falls_through_rather_than_gating_on_nothing() {
    // The harness it named was renamed or deleted. A tick that no longer points
    // at anything is not an authored declaration, so it must not silently leave
    // the step ungated *or* claim a harness ran.
    let s = strategy(
        Some("npm test"),
        &[("lint", "npm run lint")],
        Some(&["gone"]),
    );

    assert_eq!(
        pairs(&resolve_harnesses(&[], &s, CEILING_S)),
        vec![(DEFAULT_HARNESS_NAME, "npm test")]
    );
}

#[test]
fn nothing_configured_anywhere_resolves_to_nothing() {
    // An absence of evidence, which `HarnessOutcome::NotConfigured` renders as
    // such — never a pass.
    assert!(resolve_harnesses(&[], &strategy(None, &[], None), CEILING_S).is_empty());
    assert!(
        resolve_harnesses(&declared(&["lint"]), &strategy(None, &[], None), CEILING_S).is_empty()
    );
}

#[test]
fn a_declared_name_the_map_does_not_define_still_runs_the_test_command_once() {
    // What the singular field did: `harness_name: "lint"` with no map ran
    // `test_command` under the name `lint`. Preserved, because a typo silently
    // leaving a step ungated is worse than a mislabelled block — but capped at
    // one, so three unknown names cannot run the same suite three times.
    let s = strategy(Some("npm test"), &[], None);

    assert_eq!(
        pairs(&resolve_harnesses(&declared(&["lint"]), &s, CEILING_S)),
        vec![("lint", "npm test")]
    );
    assert_eq!(
        pairs(&resolve_harnesses(
            &declared(&["lint", "unit"]),
            &s,
            CEILING_S
        )),
        vec![("lint", "npm test")]
    );
}

#[test]
fn a_repeated_declaration_runs_the_gate_once() {
    let s = strategy(None, &[("lint", "npm run lint")], None);
    assert_eq!(
        pairs(&resolve_harnesses(
            &declared(&["lint", "lint"]),
            &s,
            CEILING_S
        )),
        vec![("lint", "npm run lint")]
    );
}

// ── The deadline is per harness, not per step ────────────────────────────────

#[test]
fn every_harness_gets_the_whole_ceiling_rather_than_a_share_of_it() {
    // `wall_cap_s` is the ceiling *one command* may consume (S10), so N gates
    // get N ceilings. Dividing would make a gate's deadline depend on how many
    // *other* gates a workflow happens to declare — a suite that passes alone
    // would start timing out the moment someone adds a lint gate beside it.
    let s = strategy(
        None,
        &[
            ("lint", "npm run lint"),
            ("unit", "npm run unit"),
            ("integration", "npm run integration"),
        ],
        None,
    );

    let resolved = resolve_harnesses(&declared(&["lint", "unit", "integration"]), &s, CEILING_S);

    assert_eq!(resolved.len(), 3);
    for h in &resolved {
        assert_eq!(
            h.deadline_s, CEILING_S,
            "harness '{}' was given {}s of the {CEILING_S}s ceiling — the cap is \
             per command, not per step",
            h.name, h.deadline_s
        );
    }
}

#[test]
fn one_harness_gets_the_same_deadline_as_each_of_three() {
    // The invariant stated as a comparison, so a change that divides by the
    // count cannot pass by scaling both sides.
    let s = strategy(Some("npm test"), &[("lint", "npm run lint")], None);
    let alone = resolve_harnesses(&declared(&["lint"]), &s, CEILING_S);
    let s3 = strategy(
        None,
        &[("a", "cmd a"), ("b", "cmd b"), ("c", "cmd c")],
        None,
    );
    let three = resolve_harnesses(&declared(&["a", "b", "c"]), &s3, CEILING_S);

    assert_eq!(alone[0].deadline_s, three[0].deadline_s);
    assert_eq!(alone[0].deadline_s, CEILING_S);
}

// ── Back-compat: the singular `harness_name` keeps parsing ───────────────────

fn parse(verifier_json: serde_json::Value) -> VerifierConfig {
    serde_json::from_value(verifier_json).expect("verifier config must parse")
}

#[test]
fn the_null_harness_name_every_starter_ships_still_parses_to_no_declaration() {
    // All seven `src-tauri/workflows/*.json` spell it exactly this way. If this
    // regresses, every shipped starter fails to load.
    let cfg = parse(serde_json::json!({
        "instructions": "Return the harness verdict.",
        "harness_name": null,
        "verdict_key": "verdict"
    }));

    assert!(cfg.harness_names.is_empty());
    assert_eq!(
        cfg.harness_label(),
        DEFAULT_HARNESS_NAME,
        "an undeclared harness must still title the verifier turn 'default'"
    );
}

#[test]
fn the_singular_harness_name_parses_as_a_one_element_list() {
    let cfg = parse(serde_json::json!({
        "instructions": "check",
        "harness_name": "integration"
    }));
    assert_eq!(cfg.harness_names, vec!["integration".to_string()]);
}

#[test]
fn a_blank_harness_name_is_no_declaration_not_a_harness_called_empty() {
    // The canvas writes `""` for a cleared text input before it writes `null`.
    let cfg = parse(serde_json::json!({ "instructions": "check", "harness_name": "" }));
    assert!(cfg.harness_names.is_empty());
}

#[test]
fn the_plural_field_accepts_an_ordered_list() {
    let cfg = parse(serde_json::json!({
        "instructions": "check",
        "harness_names": ["lint", "unit"]
    }));
    assert_eq!(
        cfg.harness_names,
        vec!["lint".to_string(), "unit".to_string()]
    );
    assert_eq!(cfg.harness_label(), "lint, unit");
}

#[test]
fn the_old_field_name_also_accepts_a_list() {
    // A user who hand-edits their stored workflow JSON is likelier to add a
    // bracket than to rename the key.
    let cfg = parse(serde_json::json!({
        "instructions": "check",
        "harness_name": ["lint", "unit"]
    }));
    assert_eq!(
        cfg.harness_names,
        vec!["lint".to_string(), "unit".to_string()]
    );
}

#[test]
fn an_undeclared_harness_serializes_to_nothing_at_all() {
    // `skip_serializing_if` keeps the round-tripped workflow free of a key the
    // author never wrote — an empty list is the same statement as no key.
    let cfg = parse(serde_json::json!({ "instructions": "check" }));
    let json = serde_json::to_value(&cfg).expect("serialize");
    assert!(json.get("harness_names").is_none());
    assert!(json.get("harness_name").is_none());
}

// ── HB2c: which gates will judge this run (the `{{harness_baseline}}` list) ──

fn step_with_verifier(id: &str, declared: &[&str]) -> crate::domain::models::StepConfig {
    crate::domain::models::StepConfig {
        id: crate::domain::ids::StepId(id.to_string()),
        kind: "agent".to_string(),
        verifier: Some(VerifierConfig {
            agent_kind: None,
            model: None,
            effort: None,
            instructions: String::new(),
            harness_names: declared.iter().map(|s| s.to_string()).collect(),
            verdict_key: "verdict".to_string(),
        }),
        ..Default::default()
    }
}

fn plain_step(id: &str) -> crate::domain::models::StepConfig {
    crate::domain::models::StepConfig {
        id: crate::domain::ids::StepId(id.to_string()),
        kind: "agent".to_string(),
        ..Default::default()
    }
}

/// A step that gates nothing contributes nothing. Listing its (nonexistent)
/// harness would tell a spec author about a command no one will run — the same
/// class of lie as telling them about none, and the one that cost two rework
/// cycles.
#[test]
fn only_steps_that_actually_gate_contribute_to_the_list() {
    let strategy = strategy(Some("cargo test"), &[], None);
    let steps = [plain_step("s-research"), plain_step("s-spec")];

    assert!(
        crate::domain::verifier::resolve_gating_harnesses(&steps, &strategy, CEILING_S).is_empty()
    );
}

/// The starter shape: no step declares a harness, so the whole list comes from
/// the project — through the same chain validate will resolve through, tier for
/// tier.
#[test]
fn a_starter_shaped_workflow_reports_the_projects_own_gates() {
    let strategy = strategy(
        Some("cargo test"),
        &[("lint", "npm run lint"), ("unit", "npm test")],
        Some(&["lint", "unit"]),
    );
    let steps = [plain_step("s-spec"), step_with_verifier("s-validate", &[])];

    let gates = crate::domain::verifier::resolve_gating_harnesses(&steps, &strategy, CEILING_S);

    assert_eq!(
        gates,
        vec![
            ResolvedHarness {
                name: "lint".to_string(),
                command: "npm run lint".to_string(),
                deadline_s: CEILING_S,
            },
            ResolvedHarness {
                name: "unit".to_string(),
                command: "npm test".to_string(),
                deadline_s: CEILING_S,
            },
        ],
        "the selected gates, not the test_command fallback"
    );
}

/// A step that pins its own gates is reported as pinning them — reporting the
/// project's selection instead would describe a run that is not going to happen.
#[test]
fn a_step_that_pins_its_gates_is_reported_as_pinning_them() {
    let strategy = strategy(
        Some("cargo test"),
        &[("lint", "npm run lint"), ("unit", "npm test")],
        Some(&["lint"]),
    );
    let steps = [step_with_verifier("s-validate", &["unit"])];

    let gates = crate::domain::verifier::resolve_gating_harnesses(&steps, &strategy, CEILING_S);

    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].name, "unit");
}

/// Two gating steps means the union, in first-declared order, with no gate
/// listed twice — a workflow that lints twice does not run two lints.
#[test]
fn two_gating_steps_union_without_repeating_a_gate() {
    let strategy = strategy(
        None,
        &[
            ("lint", "npm run lint"),
            ("unit", "npm test"),
            ("e2e", "npm run e2e"),
        ],
        None,
    );
    let steps = [
        step_with_verifier("s-early", &["lint", "unit"]),
        step_with_verifier("s-late", &["unit", "e2e"]),
    ];

    let gates = crate::domain::verifier::resolve_gating_harnesses(&steps, &strategy, CEILING_S);

    assert_eq!(
        gates.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
        vec!["lint", "unit", "e2e"]
    );
}

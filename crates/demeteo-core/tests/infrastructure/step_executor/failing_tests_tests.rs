// Tests extracted from `src/adapters/step_executor/failing_tests.rs`
// (mirrored-tests convention). `super` resolves to that module.
//
// Two properties are under test here and neither is visible from `domain/`:
//
// * **which gates cost an agent call.** Rung 3 is the only part of this
//   subsystem that spends tokens on the comparison side, so "a green gate is
//   never asked" and "a gate rungs 1-2 settled is never asked" are claims about
//   a call that must *not* happen — and a double that only returns values cannot
//   witness them.
// * **what a malfunctioning extractor does.** Every failure mode has to leave
//   the determination exactly as rungs 1-2 left it. An extractor is agent-shaped
//   and will therefore sometimes be wrong; the design's whole defence is that
//   being wrong cannot matter.
//
// [`compare_gates_with_extraction`] is a free function over one port precisely
// so both are reachable without an `ExecutionDriver` and its twenty-odd unread
// ports (AGENTS.md §3).

use super::*;
use crate::domain::harness_baseline::{BaselineProducer, HarnessBaselineRun};
use crate::domain::harness_delta::GateDetermination;
use std::collections::HashMap;
use std::sync::Mutex;

const BASE: &str = "abc1234def5678";
const WT: &str = "/repo_wt";

/// A [`FailingTestExtractor`] double that answers from a script and **records
/// every gate it was asked about**.
///
/// Refuses anything it was not told to answer — but its refusal has to be a
/// `Vec`, so it returns the empty one and records the miss for an assertion to
/// name. That is also the honest model of production: `extract_failing_tests`
/// cannot fail, it can only read nothing.
struct ScriptedExtractor {
    answers: HashMap<String, Vec<String>>,
    asked: Mutex<Vec<String>>,
}

impl ScriptedExtractor {
    /// Keyed by the gate's *command*, which is what the extractor is handed.
    fn new(answers: &[(&str, &[&str])]) -> Self {
        Self {
            answers: answers
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
                .collect(),
            asked: Mutex::new(Vec::new()),
        }
    }

    /// Reads nothing, ever — the shape every "no call should happen" assertion
    /// uses, and the one a malfunctioning extractor degrades to.
    fn none() -> Self {
        Self::new(&[])
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl FailingTestExtractor for ScriptedExtractor {
    async fn extract(&self, cmd: &str, _output: &str) -> Vec<String> {
        self.asked.lock().unwrap().push(cmd.to_string());
        self.answers.get(cmd).cloned().unwrap_or_default()
    }
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn red(name: &str, output: &str) -> HarnessRun {
    HarnessRun {
        name: name.to_string(),
        cmd: format!("npm run {name}"),
        output: output.to_string(),
    }
}

/// A baseline measurement of `name` built the way `measure_gates` builds one, so
/// the fingerprints on both sides of the comparison are strings of the same
/// shape rather than two things that merely look alike.
fn measured(name: &str, exit_ok: bool, output: &str, tests: Option<&[&str]>) -> HarnessBaselineRun {
    let cmd = format!("npm run {name}");
    HarnessBaselineRun {
        name: name.to_string(),
        command: cmd.clone(),
        exit_ok,
        fingerprint: if exit_ok {
            String::new()
        } else {
            normalize_failure_fingerprint(&harness_block(name, &cmd, output), WT)
        },
        output_ref: None,
        environment: None,
        failing_tests: tests.map(|t| t.iter().map(|s| s.to_string()).collect()),
        measured_at: 1_000,
        producer: BaselineProducer::Node,
    }
}

fn record(runs: Vec<HarnessBaselineRun>) -> HarnessBaseline {
    HarnessBaseline {
        base_sha: BASE.to_string(),
        harnesses: runs,
    }
}

async fn compare(
    extractor: &ScriptedExtractor,
    baseline: Option<&HarnessBaseline>,
    failed: &[HarnessRun],
) -> Vec<GateDetermination> {
    compare_gates_with_extraction(extractor, baseline, BASE, WT, failed)
        .await
        .into_iter()
        .map(|c| c.determination)
        .collect()
}

// ── The one gate rung 3 is paid for ──────────────────────────────────────────

#[tokio::test]
async fn a_differently_red_gate_is_scoped_to_the_failures_that_are_new() {
    let base = record(vec![measured(
        "unit",
        false,
        "FAIL auth::expired",
        Some(&["auth::expired"]),
    )]);
    let extractor = ScriptedExtractor::new(&[(
        "npm run unit",
        &["auth::expired", "cart::totals"] as &[&str],
    )]);

    let out = compare(
        &extractor,
        Some(&base),
        &[red("unit", "FAIL auth::expired\nFAIL cart::totals")],
    )
    .await;

    assert_eq!(
        out,
        vec![GateDetermination::NewFailures {
            new_failures: ids(&["cart::totals"]),
        }],
        "the retry is scoped to what this feature added, not to the whole gate"
    );
    assert_eq!(extractor.asked(), vec!["npm run unit"]);
}

// ── The gates it must never be paid for ──────────────────────────────────────

#[tokio::test]
async fn a_green_gate_is_never_handed_to_the_extractor() {
    // Green *at the base*, red now: rung 1 settled it, and every failure on a
    // green base is new by construction. There is nothing a reading could
    // narrow, so paying for one would make every regressing run fund it.
    let base = record(vec![measured("unit", true, "", None)]);
    let extractor = ScriptedExtractor::none();

    let out = compare(&extractor, Some(&base), &[red("unit", "FAIL cart::totals")]).await;

    assert_eq!(out, vec![GateDetermination::Regression]);
    assert!(
        extractor.asked().is_empty(),
        "rung 1 answered — no agent call may be made: {:?}",
        extractor.asked()
    );
}

#[tokio::test]
async fn a_gate_the_cheaper_rungs_settled_is_never_handed_to_the_extractor() {
    let output = "FAIL auth::expired";
    let extractor = ScriptedExtractor::none();

    // Identically red both sides: subtracted, so there is no retry to scope.
    let same = record(vec![measured(
        "unit",
        false,
        output,
        Some(&["auth::expired"]),
    )]);
    let out = compare(&extractor, Some(&same), &[red("unit", output)]).await;
    assert_eq!(out, vec![GateDetermination::PreExisting]);

    // No record at all: nothing to diff against.
    let out = compare(&extractor, None, &[red("unit", "FAIL cart::totals")]).await;
    assert_eq!(out, vec![GateDetermination::NoBaseline]);

    // Differently red, but the record named nothing — every live name would read
    // as new, which fabricates scope rather than narrowing it.
    let unnamed = record(vec![measured("unit", false, output, None)]);
    let out = compare(&extractor, Some(&unnamed), &[red("unit", "FAIL other")]).await;
    assert_eq!(
        out,
        vec![GateDetermination::NewFailures {
            new_failures: Vec::new()
        }]
    );

    assert!(
        extractor.asked().is_empty(),
        "none of those three can be narrowed, so none may cost a call: {:?}",
        extractor.asked()
    );
}

// ── A malfunctioning extractor changes nothing ───────────────────────────────

#[tokio::test]
async fn an_extractor_that_reads_nothing_leaves_the_determination_alone() {
    let base = record(vec![measured(
        "unit",
        false,
        "FAIL auth::expired",
        Some(&["auth::expired"]),
    )]);
    // Asked, and answers nothing — a spawn failure, a timeout, an unparseable
    // reply. All of them arrive here as the empty list.
    let extractor = ScriptedExtractor::none();

    let out = compare(&extractor, Some(&base), &[red("unit", "FAIL cart::totals")]).await;

    assert_eq!(
        out,
        vec![GateDetermination::NewFailures {
            new_failures: Vec::new()
        }],
        "a failed reading degrades to rung 2 — a verdict over the whole gate"
    );
    assert_eq!(
        extractor.asked(),
        vec!["npm run unit"],
        "it was asked; what it read is what came back empty"
    );
}

// ── Several gates ────────────────────────────────────────────────────────────

#[tokio::test]
async fn each_gate_is_compared_and_extracted_independently() {
    // One gate excluded, one scoped: the mixed shape a real validate hits, and
    // the one where a per-gate loop could quietly apply the wrong side's answer.
    let base = record(vec![
        measured("lint", false, "LINT-RED", Some(&["style/indent"])),
        measured(
            "unit",
            false,
            "FAIL auth::expired",
            Some(&["auth::expired"]),
        ),
    ]);
    let extractor = ScriptedExtractor::new(&[(
        "npm run unit",
        &["auth::expired", "cart::totals"] as &[&str],
    )]);

    let out = compare(
        &extractor,
        Some(&base),
        &[
            red("lint", "LINT-RED"),
            red("unit", "FAIL auth::expired\nFAIL cart::totals"),
        ],
    )
    .await;

    assert_eq!(
        out,
        vec![
            GateDetermination::PreExisting,
            GateDetermination::NewFailures {
                new_failures: ids(&["cart::totals"]),
            },
        ],
    );
    assert_eq!(
        extractor.asked(),
        vec!["npm run unit"],
        "the identically-red gate must not have cost a call"
    );
}

// ── Parsing what the agent said ──────────────────────────────────────────────

#[test]
fn a_well_formed_reading_is_taken_verbatim() {
    let ids = parse_test_ids_text(
        "Here is what I found.\n\
         {\"failing_tests\": [\"auth::expired\", \" src/app.test.ts > header \"]}",
    );
    assert_eq!(
        ids,
        vec![
            "auth::expired".to_string(),
            "src/app.test.ts > header".to_string()
        ],
        "identifiers survive surrounding prose; only their own padding is trimmed"
    );
}

#[test]
fn every_unusable_reply_reads_as_no_scope() {
    // Each of these is a way the extraction can go wrong in production, and each
    // must be indistinguishable from "nobody asked".
    for reply in [
        "",
        "I could not tell which tests failed.",
        "{\"category\": \"regression\"}",
        "{\"failing_tests\": \"auth::expired\"}",
        "{\"failing_tests\": [1, 2, 3]}",
        "{\"failing_tests\": [\"\", \"   \"]}",
    ] {
        assert!(
            parse_test_ids_text(reply).is_empty(),
            "unusable reply must yield no scope: {reply}"
        );
    }
}

#[test]
fn a_reading_longer_than_the_cap_is_not_a_scope() {
    // 400 failures is "the build is broken", not "these tests regressed", and
    // threading it into a rework prompt would bury the reason it is there. The
    // prompt asks for an empty array in that case; this is what makes a model
    // that ignores the instruction harmless.
    let many: Vec<String> = (0..MAX_TEST_IDS + 1).map(|i| format!("t{i}")).collect();
    let json = serde_json::json!({ "failing_tests": many }).to_string();
    assert!(parse_test_ids_text(&json).is_empty());

    let at_cap: Vec<String> = (0..MAX_TEST_IDS).map(|i| format!("t{i}")).collect();
    let json = serde_json::json!({ "failing_tests": at_cap }).to_string();
    assert_eq!(parse_test_ids_text(&json).len(), MAX_TEST_IDS);
}

#[test]
fn the_prompt_forbids_guessing_and_never_asks_for_a_verdict() {
    let prompt = build_test_ids_prompt("npm test", "FAIL auth::expired");

    assert!(
        prompt.contains("verbatim"),
        "an identifier the agent tidied cannot be diffed against the record"
    );
    assert!(
        prompt.contains("empty array is a correct"),
        "refusing must be cheaper than guessing, or a wrong reading mis-scopes a retry"
    );
    // The boundary decision 44 turns on: this agent reads evidence, it never
    // produces any. A prompt that offered it a pass/fail would be the thing the
    // decision forbids, whatever the code around it did with the answer.
    for banned in [
        "\"pass\"",
        "\"fail\"",
        "verdict",
        "regression",
        "environment",
    ] {
        assert!(
            !prompt.contains(banned),
            "the extractor must never be offered a verdict vocabulary: found {banned}"
        );
    }
}

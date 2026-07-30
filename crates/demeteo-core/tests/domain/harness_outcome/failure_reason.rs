// Tests for `src/domain/harness_outcome.rs` (mirrored-tests convention).
// `super` resolves to that module.
//
// The two wordings a *failure* reaches somebody through, and the budget each
// carries: the retry feedback an implementer reads (shared character tail), and
// the fingerprint input (no budget at all). Plus S11, the wrapper that decides
// whether a green run's output exists to be worded in the first place.

use super::{build_failure_reason, combined_failure_output, merge_stderr_into_stdout, HarnessRun};

fn run(name: &str, cmd: &str, output: &str) -> HarnessRun {
    HarnessRun {
        name: name.to_string(),
        cmd: cmd.to_string(),
        output: output.to_string(),
    }
}

// ── HB5: a failure says which gate went red ──────────────────────────────────

#[test]
fn a_failure_names_the_harness_that_failed() {
    let reason = build_failure_reason(&[run("lint", "npm run lint", "3 problems")]);

    assert!(
        reason.contains("'lint'"),
        "the retry feedback must name the gate, not just the command; got:\n{reason}"
    );
    assert!(reason.contains("npm run lint"));
    assert!(
        reason.contains("exited with failure"),
        "the wording every consumer of this string matches on must survive"
    );
}

#[test]
fn both_failing_harnesses_reach_the_retry_feedback() {
    // If only the first red gate reached the implementer it would fix that one
    // and rediscover the second on the next cycle — one wasted cycle turned
    // into two, which is exactly what running every declared harness exists to
    // prevent. Reporting only half of what ran would give the saving back.
    let reason = build_failure_reason(&[
        run("lint", "npm run lint", "3 problems"),
        run("unit", "npm test", "1 failing: adds two numbers"),
    ]);

    assert!(reason.contains("'lint'") && reason.contains("3 problems"));
    assert!(reason.contains("'unit'") && reason.contains("1 failing: adds two numbers"));
    assert!(
        reason.contains("2 of this step's harnesses failed"),
        "the count must lead, so the reader knows to look for more than one; got:\n{reason}"
    );
}

#[test]
fn a_single_failure_reads_exactly_as_it_did_before_the_list() {
    // Back-compat where it is observable: one red gate must not acquire a
    // "1 of this step's harnesses failed" preamble it never had.
    let reason = build_failure_reason(&[run("default", "cargo test", "boom")]);
    assert!(!reason.contains("harnesses failed"));
    assert_eq!(
        reason,
        "'default' — command 'cargo test' exited with failure:\nboom"
    );
}

#[test]
fn the_tail_budget_is_shared_not_multiplied() {
    // A step with five red gates must not grow the retry prompt fivefold. Each
    // gate still gets a floor worth of tail (enough for a stack), and the
    // failing *end* of each output is what survives — the assertion, not the
    // build banner.
    let long = "x".repeat(10_000);
    let five: Vec<_> = ["a", "b", "c", "d", "e"]
        .iter()
        .map(|n| run(n, "cmd", &format!("{long}TAIL-{n}")))
        .collect();
    let reason = build_failure_reason(&five);

    assert!(
        reason.len() < 5 * 2000,
        "budget must be shared; got {} chars",
        reason.len()
    );
    for n in ["a", "b", "c", "d", "e"] {
        assert!(
            reason.contains(&format!("TAIL-{n}")),
            "every gate keeps the tail of its own output; {n} lost"
        );
    }
}

/// The fingerprint compares two *whole* failures — truncating it first would let
/// a difference past the window read as a reproduction, which is what decides
/// whether a failure goes to triage as an environment problem. The prompt budget
/// must not leak into that path.
#[test]
fn the_failure_fingerprint_path_is_still_unwindowed() {
    let huge = format!("HEAD\n{}\nTAIL\n", "noise\n".repeat(40_000));
    let combined = combined_failure_output(&[run("default", "npm run checks", &huge)]);

    assert!(
        combined.contains(&huge),
        "the fingerprint input must carry the output whole"
    );
    assert!(!combined.contains("omitted from the middle"));
}

// ── S11: a green run's stderr must survive ───────────────────────────────────

#[test]
fn merge_wraps_in_a_subshell_redirecting_stderr() {
    assert_eq!(
        merge_stderr_into_stdout("cargo test"),
        "(\ncargo test\n) 2>&1"
    );
}

#[test]
fn merge_survives_a_command_ending_in_a_comment() {
    // The newlines are not cosmetic. `(cargo test # note) 2>&1` comments out
    // the closing paren and the redirect, turning valid shell into a syntax
    // error — and the harness command is user-authored, so this is reachable.
    let wrapped = merge_stderr_into_stdout("cargo test # run the suite");
    assert!(
        wrapped.ends_with("\n) 2>&1"),
        "closing paren must sit on its own line; got: {wrapped}"
    );
}

#[test]
fn merge_preserves_multi_command_harnesses() {
    // The shape `detect_worktree_strategy` emits for a marker that lives below
    // the repository root (HB3). The wrapping subshell must not disturb the
    // inner one, whose whole job is to keep the `cd` from leaking.
    let cmd = "(cd src-tauri && cargo test)";
    let wrapped = merge_stderr_into_stdout(cmd);
    assert!(wrapped.contains(cmd));
    assert!(wrapped.starts_with("(\n") && wrapped.ends_with("\n) 2>&1"));
}

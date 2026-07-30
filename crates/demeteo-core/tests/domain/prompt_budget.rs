// Tests extracted from `src/domain/prompt_budget.rs` (mirrored-tests convention).
// `super` resolves to that module.
//
// The bug these exist for: `s-validate` embedded a harness log verbatim in a
// prompt that is passed as one `execve` argument, so a 212 KB `npm run checks`
// log blew the OS's 128 KiB per-argument ceiling and the agent process never
// started — four times, on two different harnesses, after `s-implement` had
// already spent 3.8 M tokens.

use super::{
    argv_too_long_message, per_gate_budget, window_harness_log, ARGV_STRING_LIMIT_BYTES,
    HARNESS_GATE_FLOOR_BYTES, HARNESS_SECTION_BUDGET_BYTES, HARNESS_SECTION_CEILING_BYTES,
};

// ── the ceiling is actually cleared ──────────────────────────────────────────

/// The one assertion that makes the whole module worth having, and it has to hold
/// for *any* gate count — not just the realistic ones. A shared budget with a
/// per-gate floor and no section ceiling grows linearly past the floor, which is
/// the original bug rediscovered at sixteen gates.
#[test]
fn no_gate_count_can_push_the_section_past_the_argv_ceiling() {
    // 212 KB across 2,971 lines — the observed `npm run checks` output.
    let realistic = "a line of test output that says something\n".repeat(5_000);
    assert!(realistic.len() > 200_000, "fixture must be the real size");

    for gate_count in 1..=64 {
        let budget = per_gate_budget(gate_count);
        let total: usize = (0..gate_count)
            .map(|_| window_harness_log(&realistic, budget).len())
            .sum();

        assert!(
            total < ARGV_STRING_LIMIT_BYTES,
            "{gate_count} gates rendered {total} bytes, over the {ARGV_STRING_LIMIT_BYTES}-byte \
             argv ceiling — this is the E2BIG bug"
        );
        // And with real headroom: the template, the attached spec, the implement
        // summaries and the verdict contract all still have to fit alongside.
        assert!(
            total <= HARNESS_SECTION_CEILING_BYTES + gate_count * 512,
            "{gate_count} gates rendered {total} bytes, over the \
             {HARNESS_SECTION_CEILING_BYTES}-byte section ceiling (+ banners)"
        );
    }
}

/// Shared, not multiplied — the same policy `build_failure_reason` follows. A
/// per-gate budget converges straight back on `E2BIG` as gates are added.
#[test]
fn the_budget_is_shared_across_gates_not_paid_per_gate() {
    assert!(per_gate_budget(4) < per_gate_budget(1));
    assert_eq!(per_gate_budget(1), HARNESS_SECTION_BUDGET_BYTES);
    assert_eq!(
        per_gate_budget(0),
        HARNESS_SECTION_BUDGET_BYTES,
        "no divide by zero"
    );

    // Where sharing would starve a gate, the floor takes over — a window too
    // short to hold a stack trace is not evidence. Six gates is the most that
    // can each afford it, which covers every realistic step.
    assert_eq!(per_gate_budget(5), HARNESS_GATE_FLOOR_BYTES);
    assert_eq!(per_gate_budget(6), HARNESS_GATE_FLOOR_BYTES);

    // Past that the section ceiling outranks the floor. An unreadably short
    // window is a bad outcome; a spawn that never happens is a worse one.
    assert!(
        per_gate_budget(12) < HARNESS_GATE_FLOOR_BYTES,
        "the floor must yield to the ceiling, or 16 gates is E2BIG again"
    );
    for gate_count in 1..=64 {
        assert!(
            gate_count * per_gate_budget(gate_count) <= HARNESS_SECTION_CEILING_BYTES,
            "{gate_count} gates claim more than the section ceiling"
        );
    }
}

// ── under budget, nothing changes ────────────────────────────────────────────

/// Every existing prompt expectation was written against unwindowed output, and
/// almost every real run is under budget. Untouched means untouched.
#[test]
fn a_log_that_fits_is_returned_byte_for_byte() {
    let log = "test result: ok. 57 passed; 0 failed\n";
    assert_eq!(window_harness_log(log, HARNESS_SECTION_BUDGET_BYTES), log);
    assert_eq!(window_harness_log("", 0), "");
    // Exactly at the budget is still a fit, not a window.
    let exact = "x".repeat(100);
    assert_eq!(window_harness_log(&exact, 100), exact);
}

// ── over budget, both ends survive and the gap is named ──────────────────────

#[test]
fn both_the_head_and_the_tail_survive_the_window() {
    // The tail carries the verdict; the head carries which worktree the gate ran
    // in. A tail-only window loses the second, and an agent that cannot tell
    // where the evidence came from cannot tell green from green-for-the-wrong-
    // reason.
    let body = format!(
        "RUN v3.2.7 /worktrees/demeteo_wt_f-1\n{}\ntest result: FAILED. 1 failed\n",
        "noise\n".repeat(20_000)
    );
    let windowed = window_harness_log(&body, 4096);

    assert!(
        windowed.contains("RUN v3.2.7 /worktrees/demeteo_wt_f-1"),
        "the head identifies the run; got:\n{windowed}"
    );
    assert!(
        windowed.contains("test result: FAILED. 1 failed"),
        "the tail carries the verdict; got:\n{windowed}"
    );
    assert!(windowed.len() < body.len());
}

/// The surrounding prompt calls this output *authoritative* and forbids
/// re-running the suite. A silently-shortened log therefore reads as a complete
/// one, and an agent will report the counts it can see as totals.
#[test]
fn the_omission_is_named_with_its_size_and_the_counts_disclaimed() {
    let body = "line\n".repeat(20_000);
    let windowed = window_harness_log(&body, 4096);

    assert!(
        windowed.contains("omitted from the middle"),
        "the gap must be visible; got:\n{windowed}"
    );
    assert!(
        windowed.contains("NOT totals"),
        "an agent handed a window must be told not to read partial counts as \
         totals; got:\n{windowed}"
    );
    assert!(
        windowed.contains("unprovable"),
        "and given the exit the workflow already understands when the evidence \
         it needs fell in the gap; got:\n{windowed}"
    );
    // The size is quantified, so a reader can tell a trim from a gutting.
    assert!(
        windowed.contains("lines /") && windowed.contains("KiB"),
        "got:\n{windowed}"
    );
}

// ── degenerate shapes that must not panic or return nothing ──────────────────

/// Minified output, or a progress bar with no `\n` in it at all. The line-
/// boundary search finds nothing and must fall back to char boundaries rather
/// than collapsing the window to just the banner.
#[test]
fn a_log_with_no_line_breaks_still_yields_a_head_and_a_tail() {
    let body = format!("HEAD{}TAIL", "x".repeat(100_000));
    let windowed = window_harness_log(&body, 4096);

    assert!(windowed.starts_with("HEAD"), "got:\n{}", &windowed[..80]);
    assert!(windowed.ends_with("TAIL"));
}

/// Harness output is not guaranteed ASCII — a vitest run emits `✓`/`✗`, cargo
/// emits `─`. Slicing on a byte index inside a multi-byte char panics, and a
/// panic here takes down a step that had already cost the implement budget.
#[test]
fn multibyte_output_is_never_sliced_mid_character() {
    for budget in [1, 2, 3, 7, 64, 1000, 4095, 4096] {
        let body = "✓ src/lib/shortcuts.test.ts (67 tests) 12ms\n".repeat(5_000);
        let windowed = window_harness_log(&body, budget);
        assert!(
            windowed.is_char_boundary(0),
            "budget {budget} produced invalid UTF-8"
        );
        // A no-newline multibyte body exercises the char-boundary fallbacks on
        // both ends rather than the line-boundary path.
        let dense = "✓".repeat(50_000);
        let _ = window_harness_log(&dense, budget);
    }
}

/// A budget smaller than one line still has to produce something valid — the
/// floor keeps real callers away from here, but `window_harness_log` is public.
#[test]
fn a_budget_of_zero_does_not_panic() {
    let windowed = window_harness_log("some output\nmore output\n", 0);
    assert!(windowed.contains("omitted from the middle"));
}

// ── the diagnostic ───────────────────────────────────────────────────────────

/// Raw `Argument list too long (os error 7)` is surfaced as an environmental
/// failure, which sends the user auditing their machine for a defect that is
/// entirely ours. The message has to say whose fault it is, and carry the two
/// numbers that make the next occurrence diagnosable from the message alone.
#[test]
fn the_e2big_diagnostic_names_the_size_the_ceiling_and_the_culprit() {
    let msg = argv_too_long_message("claude", 230_400);

    assert!(msg.contains("claude"));
    assert!(msg.contains("230400"), "the actual size; got: {msg}");
    assert!(
        msg.contains(&ARGV_STRING_LIMIT_BYTES.to_string()),
        "the ceiling it cleared; got: {msg}"
    );
    assert!(
        msg.contains("Nothing about the machine is wrong"),
        "the whole point — this is not an environment problem; got: {msg}"
    );
}

// Tests for `src/domain/harness_outcome.rs` (mirrored-tests convention).
// `super` resolves to that module.
//
// What a harness block *claims*: that something ran (S12), that every gate is
// attributable to its own name (HB5), and that the claim fits inside the
// `execve` argument it is handed to the agent in.

use super::{HarnessOutcome, HarnessRun};

fn run(name: &str, cmd: &str, output: &str) -> HarnessRun {
    HarnessRun {
        name: name.to_string(),
        cmd: cmd.to_string(),
        output: output.to_string(),
    }
}

// ── S12: an absent harness must not read as a passing one ────────────────────

#[test]
fn absent_harness_never_claims_anything_was_executed() {
    let rendered = HarnessOutcome::NotConfigured.render_section();

    // The exact phrase that made this a bug: the fallback string used to be
    // printed under the caller's `## Harness Results (already executed by the
    // orchestrator)` heading. An agent told nothing ran, that the nothing is
    // authoritative, and that it may not check, certifies a feature nobody
    // tested.
    assert!(
        !rendered.contains("already executed"),
        "the no-harness block must not claim execution; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("authoritative"),
        "nothing ran, so there is nothing to call authoritative; got:\n{rendered}"
    );
    // And it must not carry the ban that only makes sense for a real result —
    // otherwise the agent is forbidden from establishing anything at all.
    assert!(
        !rendered.contains("Do NOT re-run"),
        "the ban belongs to the ran-harness block only; got:\n{rendered}"
    );

    assert!(rendered.contains("NOTHING RAN"));
    assert!(
        rendered.contains("environment"),
        "an unevidenceable criterion is a config problem — the block must point \
         at the verdict that says so rather than leaving `fail` as the only exit"
    );
}

#[test]
fn ran_harness_carries_its_output_and_the_ban() {
    let rendered = HarnessOutcome::from_runs(vec![run(
        "default",
        "cargo test",
        "test result: ok. 57 passed",
    )])
    .render_section();

    assert!(rendered.contains("already executed by the orchestrator"));
    assert!(rendered.contains("cargo test"));
    assert!(rendered.contains("test result: ok. 57 passed"));
    assert!(rendered.contains("Do NOT re-run"));
}

#[test]
fn the_two_outcomes_share_no_misleading_wording() {
    // The headings must be distinguishable at a glance in a long prompt: this
    // is the one signal the agent has for "is there evidence here or not".
    let ran = HarnessOutcome::from_runs(vec![run("default", "true", "")]).render_section();
    let absent = HarnessOutcome::NotConfigured.render_section();

    let heading = |s: &str| s.lines().next().unwrap_or_default().to_string();
    assert_ne!(heading(&ran), heading(&absent));
}

// ── HB5: several gates, each attributable ────────────────────────────────────

#[test]
fn every_harness_gets_its_own_named_block() {
    // The whole point of the list. `&&`-chaining two commands hands the agent
    // one undifferentiated blob in which "which gate produced this line" is
    // unanswerable — and that attribution is what a per-gate verdict needs.
    let rendered = HarnessOutcome::from_runs(vec![
        run("lint", "npm run lint", "0 problems"),
        run("unit", "npm test", "42 passing"),
    ])
    .render_section();

    for (name, cmd, output) in [
        ("lint", "npm run lint", "0 problems"),
        ("unit", "npm test", "42 passing"),
    ] {
        assert!(
            rendered.contains(&format!("### Harness `{name}`")),
            "every gate needs its own labelled heading; {name} missing from:\n{rendered}"
        );
        assert!(rendered.contains(cmd), "missing {cmd} in:\n{rendered}");
        assert!(
            rendered.contains(output),
            "missing {output} in:\n{rendered}"
        );
    }
    // Declared order is the user's order (cheap gates first), so the blocks
    // must not be reordered on the way into the prompt.
    assert!(
        rendered.find("### Harness `lint`") < rendered.find("### Harness `unit`"),
        "blocks must follow declared order; got:\n{rendered}"
    );
}

#[test]
fn no_runs_at_all_is_not_configured_rather_than_an_empty_pass() {
    // The constructor is the only thing standing between "we ran nothing" and
    // a `Ran([])` that would render the authoritative heading over no evidence
    // at all — the S12 bug with an extra step.
    assert!(matches!(
        HarnessOutcome::from_runs(Vec::new()),
        HarnessOutcome::NotConfigured
    ));
    let rendered = HarnessOutcome::from_runs(Vec::new()).render_section();
    assert!(rendered.contains("NOTHING RAN"));
    assert!(!rendered.contains("already executed"));
}

// ── the prompt section is bounded by the argv ceiling ────────────────────────

/// The section is handed to the agent as one `execve` argument, and the OS caps
/// that at 128 KiB. `s-validate` pastes the orchestrator's own harness output
/// into it verbatim, so a chatty suite — 212 KB from `npm run checks` — made the
/// spawn fail with `E2BIG` *after* the implement budget had been spent. The
/// budget lives in `domain::prompt_budget`; this asserts the renderer applies it.
#[test]
fn an_oversized_harness_log_is_windowed_before_it_reaches_the_prompt() {
    let huge = format!(
        "RUN v3.2.7 /worktrees/wt-1\n{}\ntest result: FAILED. 1 failed\n",
        "a line of vitest output\n".repeat(10_000)
    );
    assert!(huge.len() > 200_000, "fixture must exceed the argv ceiling");

    let rendered =
        HarnessOutcome::from_runs(vec![run("default", "npm run checks", &huge)]).render_section();

    assert!(
        rendered.len() < crate::domain::prompt_budget::ARGV_STRING_LIMIT_BYTES,
        "the section alone is {} bytes — the spawn would die on E2BIG",
        rendered.len()
    );
    // Both ends of the evidence survive, and the gap is declared.
    assert!(rendered.contains("RUN v3.2.7 /worktrees/wt-1"), "head lost");
    assert!(
        rendered.contains("test result: FAILED. 1 failed"),
        "tail lost"
    );
    assert!(
        rendered.contains("omitted from the middle"),
        "gap not named"
    );
}

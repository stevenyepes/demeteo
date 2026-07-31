//! What the harness pass established, and how it is worded.
//!
//! One question — *what did running the gates prove?* — and every wording that
//! answer reaches a reader through: the prompt section a validate turn is
//! handed, the exclusion note a rework loop reads, the labelled block both are
//! built out of. None of it needs a port: the caller has already run the
//! commands and holds the strings.
//!
//! It sits beside [`harness_baseline`](crate::domain::harness_baseline) and
//! [`harness_delta`](crate::domain::harness_delta) rather than under the
//! verifier, because it belongs to the *harness* — `baseline/`, the failing-
//! test extractor and the agent step all read it, and only one of them is a
//! verifier turn.
//!
//! # Two budgets, deliberately
//!
//! Two consumers window the same output differently and must go on doing so.
//! [`HarnessOutcome::render_section`] windows each gate through
//! [`prompt_budget`](crate::domain::prompt_budget), because the section is
//! handed to an agent as a single `execve` argument the OS caps at 128 KiB.
//! [`combined_failure_output`] is **unwindowed**, because it feeds the
//! fingerprint and a fingerprint taken over a truncated failure lets a
//! difference past the cut read as a reproduction.
//! [`build_failure_reason`] has a third budget again — a character tail shared
//! across the failing gates, because it is retry feedback rather than evidence.
//! Now that all three are siblings, the thing to not do is give them one
//! shared truncation rule.

use crate::domain::text::tail_chars;

/// What the harness-first pass actually established, before anyone words it.
///
/// This is an enum rather than the rendered string it used to be because the
/// two cases are opposites and the old shape let a caller treat them alike. The
/// "no test harness was configured" sentence was returned on the `Ok` path,
/// indistinguishable from a real result, and the caller then printed it under
/// `## Harness Results (already executed by the orchestrator)`, followed by
/// "the results above are authoritative", followed by a ban on re-running
/// anything. An agent told that nothing ran, that the nothing is authoritative,
/// and that it may not check for itself has one coherent move left, and it
/// certifies a feature nobody tested (S12).
pub enum HarnessOutcome {
    /// Every gating harness ran, in declared order, and none of them failed
    /// *this feature*. Non-empty by construction — build it through
    /// [`from_runs`](HarnessOutcome::from_runs) or
    /// [`from_runs_with_exclusions`](HarnessOutcome::from_runs_with_exclusions).
    Ran {
        /// The gates that exited zero. Each [`HarnessRun::output`] is merged
        /// stdout+stderr.
        passed: Vec<HarnessRun>,
        /// The gates that exited **non-zero** and were subtracted as
        /// pre-existing (HB2c): red at the base with the identical failure, so
        /// not this feature's defect.
        ///
        /// A separate list rather than a flag on `HarnessRun` for the same
        /// reason this enum exists at all — "it passed" and "it failed, but not
        /// because of you" are different claims, and a caller that cannot tell
        /// them apart will eventually word one as the other.
        excluded: Vec<ExcludedRun>,
    },
    /// No `test_command` (and no named harness) is configured — **nothing was
    /// executed**. Not a pass, not a fail: an absence of evidence.
    NotConfigured,
}

/// A red gate the baseline excused: what it ran and said, plus why it does not
/// count against this feature.
///
/// The `reason` is not decoration. Decision 44 subtracts a failure the user did
/// not ask to have subtracted, and a subtraction nobody can audit is one nobody
/// will trust the first time it is wrong — so the exclusion is named wherever
/// the failure would have been.
pub struct ExcludedRun {
    pub run: HarnessRun,
    /// One sentence naming the base commit and the producer that measured it.
    pub reason: String,
}

/// One harness's execution: which gate it was, what it ran, and what it said.
///
/// The *name* is the field that earns this struct. With several gates in one
/// step, an output blob that does not say which gate produced it cannot tell a
/// failing lint from a failing test suite, which is exactly the attribution
/// `&&`-chaining commands used to destroy. HB2a records this shape and HB7
/// renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessRun {
    /// The gate's name — `default` when it came from the project's
    /// `test_command`, `prepare` for the prepare command.
    pub name: String,
    /// The command as the user authored it (not the `2>&1` wrapper).
    pub cmd: String,
    /// Merged stdout+stderr.
    pub output: String,
}

impl HarnessOutcome {
    /// Build the outcome from the runs that completed. An empty list is
    /// [`NotConfigured`](HarnessOutcome::NotConfigured) — "nothing ran" and
    /// "everything passed" are opposites, and the enum exists so no caller can
    /// treat them alike (S12).
    pub fn from_runs(runs: Vec<HarnessRun>) -> Self {
        Self::from_runs_with_exclusions(runs, Vec::new())
    }

    /// As [`from_runs`](Self::from_runs), with the gates HB2c subtracted.
    ///
    /// "Nothing ran" is still `NotConfigured`, but an *excluded* gate ran — so
    /// a pass whose every gate was excused must not collapse into the
    /// no-harness block, which tells the agent nothing was executed. That is
    /// S12's bug arrived from the other direction.
    pub fn from_runs_with_exclusions(passed: Vec<HarnessRun>, excluded: Vec<ExcludedRun>) -> Self {
        if passed.is_empty() && excluded.is_empty() {
            HarnessOutcome::NotConfigured
        } else {
            HarnessOutcome::Ran { passed, excluded }
        }
    }

    /// Render the harness block for an agent prompt, **heading included**.
    ///
    /// The heading is deliberately part of this method rather than the caller's
    /// format string: a caller that cannot choose the heading cannot put
    /// "already executed by the orchestrator" above an empty result. That
    /// coupling is the whole fix — the prompt-side mitigation in `2257ffb`
    /// relied on the agent obeying prose that the surrounding template
    /// contradicted.
    ///
    /// Every gate gets **its own labelled block**, so "which gate says what" is
    /// answerable from the prompt alone — and an excluded gate gets a block
    /// that says *why* it does not count, so the report can name the
    /// subtraction (HB2c).
    ///
    /// Each gate's output is windowed to its share of
    /// [`HARNESS_SECTION_BUDGET_BYTES`](crate::domain::prompt_budget::HARNESS_SECTION_BUDGET_BYTES).
    /// The prompt is handed to the agent as a single `execve` argument, which the
    /// OS caps at 128 KiB — so an unbudgeted log does not make the prompt merely
    /// expensive, it makes the spawn fail outright with `E2BIG` after the whole
    /// implement budget has already been spent. See `domain::prompt_budget`.
    /// The *fingerprint* path (`combined_failure_output`) stays unwindowed on
    /// purpose: it compares two whole failures.
    pub fn render_section(&self) -> String {
        match self {
            HarnessOutcome::Ran { passed, excluded } => format!(
                "## Harness Results (already executed by the orchestrator)\n\
                 We ran {count} in this exact worktree, in this order:\n\n\
                 {blocks}\n{exclusions}\
                 This output is authoritative. Do NOT re-run the build or test \
                 suite.\n",
                count = plural_harnesses(passed.len() + excluded.len()),
                blocks = passed
                    .iter()
                    .map(|r| harness_block(
                        &r.name,
                        &r.cmd,
                        &crate::domain::prompt_budget::window_harness_log(
                            &r.output,
                            crate::domain::prompt_budget::per_gate_budget(
                                passed.len() + excluded.len()
                            ),
                        ),
                    ))
                    .collect::<Vec<_>>()
                    .join("\n"),
                exclusions = render_exclusions(excluded, passed.len() + excluded.len()),
            ),
            // Everything here is load-bearing. Naming the absence, refusing the
            // inference, and pointing at the verdict that fits it are what stop
            // the agent from filling the silence with a pass.
            HarnessOutcome::NotConfigured => "## Harness Results — NOTHING RAN\n\
                 This project has no test command configured, so the orchestrator \
                 executed nothing and there is no test evidence for this step.\n\n\
                 That is an absence of evidence, not a passing result. Do not \
                 report any criterion as MET on the strength of a harness that \
                 never ran, and do not describe tests as passing.\n\n\
                 Judge only what you can establish by reading the diff. If the \
                 acceptance criteria require a command this project is not \
                 configured to run, no amount of re-implementation can satisfy \
                 them — that is a project-configuration problem, so say so and \
                 use the `environment` verdict rather than `fail`.\n"
                .to_string(),
        }
    }
}

/// Why one gate was subtracted, in one sentence: the base commit it was
/// identically red at, and which producer measured that.
///
/// The producer is named because the two have very different stories — the node
/// measured at the head of this run, the fallback measured on this very failure
/// path — and a support question ("how do you know?") is answered by one word.
/// A missing record is a shape this function should never be handed (a gate is
/// only excluded on the strength of one), but it degrades to a sentence that
/// claims no evidence rather than to a panic.
pub fn build_exclusion_reason(
    base_sha: &str,
    measured: Option<&crate::domain::harness_baseline::HarnessBaselineRun>,
) -> String {
    let Some(measured) = measured else {
        return "This gate was excluded from the verdict as pre-existing.".to_string();
    };
    let producer = match measured.producer {
        crate::domain::harness_baseline::BaselineProducer::Node => {
            "measured at the head of this run, before any work started"
        }
        crate::domain::harness_baseline::BaselineProducer::Fallback => {
            "measured on this failure path against a fresh checkout of the base commit"
        }
    };
    format!(
        "EXCLUDED — this gate failed here, and it failed with the *identical* output at the \
         base commit {sha} ({producer}). The failure predates this feature, so it is not part \
         of this step's verdict.",
        sha = short_sha(base_sha),
    )
}

/// First 12 characters of a sha — enough to identify a commit, short enough to
/// read inline. Anything shorter is echoed whole rather than truncated to
/// nonsense.
fn short_sha(sha: &str) -> String {
    sha.chars().take(12).collect()
}

/// The excluded-gates section of the prompt's harness block.
///
/// Empty when nothing was subtracted, so a run with no exclusions renders
/// byte-for-byte as it did before HB2c — the overwhelmingly common case, and
/// the one every existing prompt expectation was written against.
///
/// `gate_count` is the *whole* section's gate count, not `excluded.len()`: the
/// output budget is shared across passed and excluded gates alike, because both
/// render their full block into the same single `execve` argument.
fn render_exclusions(excluded: &[ExcludedRun], gate_count: usize) -> String {
    if excluded.is_empty() {
        return String::new();
    }
    let budget = crate::domain::prompt_budget::per_gate_budget(gate_count);
    let blocks = excluded
        .iter()
        .map(|e| {
            format!(
                "{}\n{}",
                e.reason,
                harness_block(
                    &e.run.name,
                    &e.run.cmd,
                    &crate::domain::prompt_budget::window_harness_log(&e.run.output, budget),
                )
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "### Excluded — {count} that already failed before this feature\n\n\
         {blocks}\n\
         Record each excluded gate in your report, by name, as a pre-existing failure. Do NOT \
         report it as an implementation defect, do not let it decide your verdict, and do not \
         ask for it to be fixed — the work under review did not cause it. Saying nothing about \
         it is also wrong: a reader who can see the failure in the log and no mention of it in \
         the report cannot tell a deliberate subtraction from an oversight.\n\n",
        count = plural_harnesses(excluded.len()),
        blocks = blocks,
    )
}

/// The exclusion note appended to a verdict's retry feedback.
///
/// The rework loop reads the verdict reason and turns it into tickets. Without
/// this, an implementer handed "the `unit` gate failed" while the log also
/// shows a red `lint` gate has every reason to go and fix `lint` too — work
/// nobody asked for, on a defect the feature did not cause.
pub fn build_exclusion_note(excluded: &[ExcludedRun]) -> String {
    if excluded.is_empty() {
        return String::new();
    }
    let names = excluded
        .iter()
        .map(|e| format!("'{}'", e.run.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "\n\nAlso red, but NOT part of this verdict: {names}. {plural} failing identically \
         before this feature started, so {pronoun} excluded — do not try to fix {pronoun2}.",
        names = names,
        plural = if excluded.len() == 1 {
            "That gate was already"
        } else {
            "Those gates were already"
        },
        pronoun = if excluded.len() == 1 {
            "it is"
        } else {
            "they are"
        },
        pronoun2 = if excluded.len() == 1 { "it" } else { "them" },
    )
}

/// One gate's labelled block: which harness, the command it ran, and its
/// combined output. Shared by the prompt section and the verdict reason so the
/// two cannot describe the same run differently.
pub fn harness_block(name: &str, cmd: &str, body: &str) -> String {
    format!(
        "### Harness `{name}`\n\n\
         \x20   {cmd}\n\n\
         Its combined stdout and stderr:\n\
         ```\n{body}\n```\n",
    )
}

/// "the 'lint' harness" / "2 harnesses". Keeps the singular reading exactly as
/// it read before harnesses became a list — the overwhelmingly common case.
fn plural_harnesses(count: usize) -> String {
    if count == 1 {
        "1 harness".to_string()
    } else {
        format!("{} harnesses", count)
    }
}

/// The failing gates' outputs, labelled, as one string — the input to the
/// fingerprint. Not truncated: the fingerprint compares two whole failures, and
/// truncating first would let a difference past the tail read as a reproduction.
pub fn combined_failure_output(failures: &[HarnessRun]) -> String {
    failures
        .iter()
        .map(|f| harness_block(&f.name, &f.cmd, &f.output))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The actionable reason injected as retry feedback, naming **which gate** went
/// red — with several red, naming all of them, because an implementer told
/// about one failure fixes it and rediscovers the next on the following cycle,
/// which is the wasted-cycle problem this whole subsystem exists to prevent.
///
/// The 2000-character tail budget is shared out among the failures rather than
/// paid per failure, so a step with five red gates cannot silently grow the
/// retry prompt fivefold; a single failure therefore reads byte-for-byte as it
/// did before. The floor keeps each gate's tail long enough to carry a stack.
pub fn build_failure_reason(failures: &[HarnessRun]) -> String {
    const TAIL_BUDGET: usize = 2000;
    const TAIL_FLOOR: usize = 500;
    let per_failure = (TAIL_BUDGET / failures.len().max(1)).max(TAIL_FLOOR);

    let blocks = failures
        .iter()
        .map(|f| {
            format!(
                "'{}' — command '{}' exited with failure:\n{}",
                f.name,
                f.cmd,
                tail_chars(&f.output, per_failure)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    if failures.len() == 1 {
        blocks
    } else {
        format!(
            "{} of this step's harnesses failed — all of them must pass.\n\n{}",
            failures.len(),
            blocks
        )
    }
}

/// Wrap a user-authored command so its stderr is merged into stdout.
///
/// The `ExecutionPort` contract is "stdout on success, stdout+stderr on
/// failure" (D3) — right for a port, wrong for any caller that shows a *green*
/// run's output to somebody, because the suites this codebase runs report
/// heavily on stderr. Both such callers (the harness-first pass and the
/// `command` node) use this, so they cannot drift apart.
///
/// The exit status survives: it is the subshell's last command's. The newlines
/// are load-bearing — a command whose final line is a `#` comment would
/// otherwise swallow the closing paren and turn valid shell into a syntax
/// error.
pub fn merge_stderr_into_stdout(cmd: &str) -> String {
    format!("(\n{}\n) 2>&1", cmd)
}

#[cfg(test)]
#[path = "../../tests/domain/harness_outcome/render_section.rs"]
mod render_section_tests;

#[cfg(test)]
#[path = "../../tests/domain/harness_outcome/exclusions.rs"]
mod exclusions_tests;

#[cfg(test)]
#[path = "../../tests/domain/harness_outcome/failure_reason.rs"]
mod failure_reason_tests;

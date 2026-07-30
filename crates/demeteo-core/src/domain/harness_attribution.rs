//! Which red gates *this feature* answers for.
//!
//! [`harness_delta`](crate::domain::harness_delta) answers per gate — *is this
//! one attributable?* This module answers for the **step**: given every gate's
//! determination, which runs feed the rework loop, which are excluded and named
//! in the report, whether one of them is terminal, and whether C6's classifier
//! still has anything to add. Deliberately not `harness_subtraction.rs`:
//! `harness_delta`'s own header already calls itself "the subtraction", and two
//! modules claiming that word is worse than the nine copies this started from.
//!
//! The fold in [`split_by_determination`] enforces four positional rules at
//! once, each of them one character away from being silently wrong, and none of
//! them was reachable from a test while it sat inside an `async fn` on
//! `ExecutionDriver`. The `async` was never the fold's — the caller awaits the
//! comparison, and hands the answer here. It stops being safe the moment
//! anything in here needs to `.await`, which is exactly why `domain/` has none.

use crate::domain::harness_delta::{GateComparison, GateOutcome};
use crate::domain::harness_outcome::{build_exclusion_reason, ExcludedRun, HarnessRun};

/// A red gate whose baseline says the gate **could not run on this machine**,
/// so its failure is not a verdict about anything.
///
/// Distinct from [`ExcludedRun`] on purpose, and the distinction is the defect
/// HB2c shipped with: both are "not this feature's fault", and they have
/// opposite consequences. An excluded gate is subtracted and the step passes on
/// what the *other* gates proved; an unrunnable one proved nothing, so passing
/// on it is evidence-free and the run has to stop with remediation instead.
pub struct UnrunnableGate {
    pub run: HarnessRun,
    /// The classifier's sentence, as recorded at baseline-measurement time.
    pub reason: String,
    /// Its provisioning step; may be empty.
    pub remediation: String,
}

/// Everything the subtraction concluded about one harness pass.
///
/// A struct rather than the tuple this used to return because there are now
/// three destinations a red gate can reach, and a positional tuple of two
/// vectors plus two more fields is exactly where a caller starts binding the
/// wrong one.
pub struct SubtractedFailures {
    /// The gates this feature answers for — a verdict, feeding the rework loop.
    pub attributable: Vec<HarnessRun>,
    /// The gates the baseline excused as pre-existing.
    pub excluded: Vec<ExcludedRun>,
    /// Whether C6's classifier may still be consulted about `attributable`.
    pub triage_allowed: bool,
    /// The first gate the baseline says cannot run here. `Some` short-circuits
    /// everything else: it is terminal, so no verdict and no subtraction is
    /// reached.
    pub unrunnable: Option<UnrunnableGate>,
    /// Rung 3's scope across `attributable`: the individual tests failing now
    /// that were not failing at the base.
    ///
    /// **Advisory, and additive.** It travels into
    /// [`VerdictFailure::failing_tests`](crate::domain::verifier::VerdictFailure::failing_tests)
    /// so the rework producer can write one ticket per genuinely new failure
    /// instead of re-deriving a whole gate — the `{{failing_tests}}` a rework
    /// template already renders. Empty is the common case and means *unscoped*,
    /// never "nothing failed": the verdict reason carries every failing gate's
    /// output regardless, so nothing is hidden by an extraction that read
    /// nothing.
    pub new_failing_tests: Vec<String>,
}

/// The red gates of one harness pass, split by who answers for them.
///
/// One struct rather than three parameters because they are three views of a
/// single decision and are meaningless apart: excluded gates without the
/// attributable ones cannot be reported, and `triage_allowed` is a property of
/// the first attributable gate. Bundling also keeps
/// `classify_harness_failures` under the argument ceiling without reaching for
/// `too_many_arguments`, which AGENTS.md §3 calls a review trigger rather than
/// a fix.
pub struct HarnessFailureSet<'a> {
    /// The gates this feature answers for. Non-empty at every call site: a set
    /// with nothing attributable never reaches classification.
    pub attributable: &'a [HarnessRun],
    /// The gates the baseline excused, carried so the verdict names them —
    /// otherwise the rework loop is handed a failure list that silently omits
    /// half of what the user can see in the log.
    pub excluded: &'a [ExcludedRun],
    /// Whether C6's triage classifier may still be consulted — see
    /// [`GateDetermination::allows_triage`](crate::domain::harness_delta::GateDetermination::allows_triage).
    pub triage_allowed: bool,
    /// Rung 3's scope: the individual tests among `attributable` that were not
    /// failing at the base. Empty means unscoped, and the verdict then reads
    /// exactly as it did before rung 3 existed.
    pub failing_tests: &'a [String],
}

/// Join every gate's determination back onto the run that produced it, and
/// answer the four questions the step needs before it can word anything.
///
/// The caller performs the I/O — running the gates, resolving the base commit,
/// paying for rung 3's extraction — and arrives here holding two slices in the
/// same order. Everything below is positional, and each rule is one character
/// from being silently wrong:
///
/// * **`triage_allowed` comes from the first attributable gate** — the
///   `get_or_insert`. That is the single gate the classifier is asked about, so
///   gating on a determination reached for some *other* gate would ask the
///   agent a question the baseline already answered, or withhold one it did not.
/// * **The unrunnable gate is the first one only** — the `get_or_insert_with`.
///   The message carries a reproduce line, which means nothing for more than one
///   command; the same reason the 127 fast path and the classifier each name a
///   single gate.
/// * **`new_failing_tests` is deduped in first-seen order**, unioned across the
///   attributable gates.
/// * **An all-excluded set yields `triage_allowed: true`.**
///
/// `failed` and `comparisons` are zipped, so a caller that hands slices of
/// different lengths silently drops the tail. Every caller derives the second
/// from the first.
pub fn split_by_determination(
    failed: &[HarnessRun],
    comparisons: &[GateComparison],
    base_sha: &str,
) -> SubtractedFailures {
    let mut attributable = Vec::new();
    let mut excluded = Vec::new();
    let mut unrunnable: Option<UnrunnableGate> = None;
    let mut triage_allowed = None;
    let mut new_failing_tests: Vec<String> = Vec::new();
    for (run, cmp) in failed.iter().zip(comparisons) {
        match cmp.determination.outcome() {
            GateOutcome::Attributable => {
                triage_allowed.get_or_insert(cmp.determination.allows_triage());
                // Rung 3's scope, unioned across the gates this feature
                // answers for. Empty for every gate rung 3 could not narrow,
                // which is what keeps a partial reading from claiming to be
                // the whole failure set: the verdict reason still carries
                // every gate's output either way.
                for name in cmp.determination.new_failures() {
                    if !new_failing_tests.contains(name) {
                        new_failing_tests.push(name.clone());
                    }
                }
                attributable.push(run.clone());
            }
            GateOutcome::Excluded => excluded.push(ExcludedRun {
                run: run.clone(),
                reason: build_exclusion_reason(base_sha, cmp.baseline.as_ref()),
            }),
            // The **first** one only: the message carries a reproduce line,
            // which means nothing for more than one command — the same
            // reason the 127 fast path and the classifier each name a single
            // gate.
            GateOutcome::Unrunnable {
                reason,
                remediation,
            } => {
                unrunnable.get_or_insert_with(|| UnrunnableGate {
                    run: run.clone(),
                    reason: reason.to_string(),
                    remediation: remediation.to_string(),
                });
            }
        }
    }
    SubtractedFailures {
        attributable,
        excluded,
        // No attributable gate ⇒ nothing will be classified, so the flag is
        // moot; `true` keeps it from reading as a suppression that was never
        // decided.
        triage_allowed: triage_allowed.unwrap_or(true),
        unrunnable,
        new_failing_tests,
    }
}

#[cfg(test)]
#[path = "../../tests/domain/harness_attribution.rs"]
mod tests;

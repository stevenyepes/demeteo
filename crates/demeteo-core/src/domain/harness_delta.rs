//! The subtraction — deciding whether a red gate is *this feature's* fault.
//!
//! `run_harness_first` used to treat any non-zero exit as this step's verdict.
//! Nothing established what the suite did on the base commit, so a repository
//! whose tests were already failing sent the run into a rework loop for a
//! defect it did not introduce, at ~$14 and 11M tokens per cycle
//! (`docs/HARNESS_BASELINE.md` §1). Decision 44 replaces the absolute with a
//! delta, and this module is that delta: given what
//! [`HarnessBaseline`](crate::domain::harness_baseline::HarnessBaseline)
//! recorded at the base and what the gate just did, it answers one question per
//! gate — *is this attributable to the feature?*
//!
//! # Why it is here and not in the `async fn` that runs the commands
//!
//! It is a policy decision, and AGENTS.md §3 puts those in `domain/`:
//! synchronous, port-free, and reachable from a test with no doubles at all.
//! The caller performs I/O — running gates, resolving the merge-base, reading
//! the stored record — and then calls this to find out what any of it *means*.
//! Every input below is a value the caller already holds.
//!
//! # The ladder, and the rung that was deliberately not built
//!
//! Comparison escalates cheapest-first, and stops as soon as one rung answers:
//!
//! 1. **exit status** — green at the base and red now is a regression outright;
//!    no further comparison can change that, and no agent is needed to know it.
//! 2. **fingerprint** ([`normalize_failure_fingerprint`]) — red both sides with
//!    the *same* normalized output is the same failure; a different one is new
//!    failures on top of a pre-existing one.
//! 3. **what that same failure *was*** — read off the record, not computed here.
//!    A gate red at the base because the machine cannot run it is not a
//!    pre-existing defect to subtract; it is a gate that proved nothing, and it
//!    terminates with remediation ([`GateDetermination::Environment`]).
//!
//! Rung 3 is a *lookup*, and that is the point: the classification was made
//! once, by C6's triage agent, at baseline-measurement time — the head of the
//! graph, where **zero implement budget has been spent**. Reaching the same
//! question through `should_triage` instead costs a full rework cycle first,
//! because the classifier is only consulted once a failure has reproduced
//! unchanged. Same agent, same fail-safe, one cycle earlier and once per red
//! gate rather than once per validate attempt.
//!
//! `docs/HARNESS_BASELINE.md` describes a **finer** rung beyond these: an agent
//! reading both outputs and answering "which failures in B are absent from A",
//! which would scope the delta to individual test names rather than to the whole
//! gate. **It is deliberately not built here.** It costs an agent call on every
//! red validate — unlike rung 3, which is paid once per red gate at measurement
//! time and read back for free — and whether that is worth paying is a judgement
//! better made after rungs 1–2 have been watched in practice: the fingerprint's
//! own false-miss rate is unknown until then, and its failure direction is the
//! safe one (a perturbed fingerprint reads as *new*, i.e. today's behaviour,
//! never as pre-existing). Build it when a real run shows rung 2 conceding too
//! much, not before.
//!
//! # The direction every ambiguity resolves in
//!
//! **Absent is not green.** A record that never measured a gate, a record
//! measured against a different commit, and a record whose command no longer
//! matches the one that just ran are all *no evidence*
//! ([`GateDetermination::NoBaseline`]) — never "it passed at the base". The
//! consequence of no evidence is today's behaviour, which costs a rework cycle;
//! the consequence of inventing a green baseline is reporting every
//! pre-existing failure as a fresh regression, and the consequence of inventing
//! a red one is excusing a real regression. Only the first is survivable, so
//! every gap resolves to it.
//!
//! Rung 3 resolves the other way round, for the same reason. A *positive*
//! `environment` classification is the only thing that terminates a run, so an
//! absent one — never classified, classified as a regression, or written by a
//! build that predates the field — reads as a pre-existing defect and stays
//! excluded. A malfunctioning classifier therefore withholds an escalation; it
//! can never manufacture one.
//!
//! [`normalize_failure_fingerprint`]: crate::adapters::step_executor::driver::verifier::normalize_failure_fingerprint

use crate::domain::harness_baseline::{HarnessBaseline, HarnessBaselineRun};

/// What one red gate's comparison against the baseline determined.
///
/// Three of the five are the same *outcome* — a verdict that feeds the rework
/// loop — and they are still distinct variants because they answer two further
/// questions differently: whether the exclusion has to be named to the user,
/// and whether C6's triage agent has anything left to add.
///
/// The fifth, [`Environment`](Self::Environment), is the one that is *not* a
/// verdict and *not* a subtraction, and it is the reason this enum is read
/// through [`outcome`](Self::outcome) rather than through a boolean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDetermination {
    /// Red at the base with this same normalized failure, and red now with it
    /// again. **Not this feature's.** Subtracted from the verdict — and named
    /// in the report, because a subtraction the user cannot audit will not be
    /// trusted the first time it is wrong.
    PreExisting,
    /// Red at the base with this same normalized failure, and red now with it
    /// again — but the baseline measurement recorded that the gate was red
    /// *because it could not run on this machine*
    /// ([`BaselineEnvironmentFault`](crate::domain::harness_baseline::BaselineEnvironmentFault)).
    ///
    /// **Terminal, with remediation.** This looks byte-for-byte like
    /// [`PreExisting`](Self::PreExisting) and must not be treated like it: a
    /// pre-existing *code* defect is a verdict the gate actually reached and
    /// which predates the feature, so subtracting it leaves the rest of the
    /// gate's evidence intact. A gate that could not run reached no verdict at
    /// all, so subtracting it passes the step on nothing. The motivating
    /// incident (a missing `gdk-3.0`) exits **1**, not 127, so the exit-127 fast
    /// path cannot see it — only the classification can.
    Environment {
        /// One sentence naming what the machine is missing.
        reason: String,
        /// The concrete provisioning step; may be empty.
        remediation: String,
    },
    /// Green at the base, red now. **This feature broke it** — a verdict, and
    /// the rework loop is exactly the right place for it. No agent is needed to
    /// reach this conclusion; the measurement already made it.
    Regression,
    /// Red at the base and red now, but *differently*. New failures on top of a
    /// pre-existing one — a verdict, since something the feature did changed
    /// what the gate says.
    NewFailures,
    /// Nothing that covers this run's base commit measured this gate: no record
    /// at all, a record describing another commit, a record that never measured
    /// this gate, or one whose recorded command is not the command that just
    /// ran. **Not "it was green"** — an absence of evidence, which degrades to
    /// today's behaviour: a verdict, with C6's triage still available.
    NoBaseline,
}

/// What the caller must *do* about one gate, which is not the same question as
/// what the comparison determined.
///
/// This replaces the `is_attributable() -> bool` this type used to expose. The
/// boolean had exactly two answers — "fail the step" and "subtract it" — so the
/// moment a third became necessary the old shape would have quietly routed it
/// into whichever of the two it resembled, and
/// [`Environment`](GateDetermination::Environment) resembles the *subtract* one
/// precisely. A three-way return makes a caller that has not thought about the
/// third case fail to compile rather than fail to escalate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateOutcome<'a> {
    /// This feature answers for it: a verdict, feeding the rework loop.
    Attributable,
    /// Subtracted from the verdict, and named in the report so the subtraction
    /// is auditable.
    Excluded,
    /// The gate never ran here, so it proved nothing. Terminal `Environment`
    /// with the remediation the classifier produced — passing on this would be
    /// evidence-free, and retrying it spends an implement budget on something
    /// `s-implement` cannot reach.
    Unrunnable {
        reason: &'a str,
        remediation: &'a str,
    },
}

impl GateDetermination {
    /// What the caller must do about this gate. The **only** way to ask; see
    /// [`GateOutcome`] for why it is not a boolean.
    pub fn outcome(&self) -> GateOutcome<'_> {
        match self {
            GateDetermination::PreExisting => GateOutcome::Excluded,
            GateDetermination::Environment {
                reason,
                remediation,
            } => GateOutcome::Unrunnable {
                reason,
                remediation,
            },
            GateDetermination::Regression
            | GateDetermination::NewFailures
            | GateDetermination::NoBaseline => GateOutcome::Attributable,
        }
    }

    /// Whether C6's triage agent may still be consulted about this gate.
    ///
    /// This is the narrowing `docs/HARNESS_BASELINE.md` §2 asks for. Triage
    /// exists to tell an unprovisioned machine apart from a broken change, and
    /// a baseline answers most of that as a *measurement*:
    ///
    /// * [`PreExisting`](Self::PreExisting) never reaches the classifier at all
    ///   — it is subtracted before there is a failure to classify.
    /// * [`Environment`](Self::Environment) has already *been* classified, at
    ///   baseline-measurement time, and terminates before classification is
    ///   reached. Asking again would pay a second agent call for an answer
    ///   already on the record.
    /// * [`NewFailures`](Self::NewFailures) is answered too: the gate reached an
    ///   exit status at the base, so the machine can run it, and the *output
    ///   changed* under this feature's changes. There is no judgement left that
    ///   the comparison did not already make.
    /// * [`Regression`](Self::Regression) is the residue the doc names — green
    ///   at the base, red now, for an environmental reason that appeared
    ///   *during* the run (a disk filled, a service died, a registry went down).
    ///   That genuinely needs judgement, so the agent survives for it.
    /// * [`NoBaseline`](Self::NoBaseline) has no measurement to narrow with, so
    ///   it keeps today's behaviour exactly.
    ///
    /// Note what this does **not** do: it never *causes* a triage call. The
    /// reproduce-unchanged gate (`should_triage`) still has to fire first, so a
    /// first-sight regression is a plain verdict and costs no tokens — decision
    /// 44's "no agent needed to know this".
    pub fn allows_triage(&self) -> bool {
        matches!(
            self,
            GateDetermination::Regression | GateDetermination::NoBaseline
        )
    }
}

/// One gate as the live run observed it. Borrowed rather than owned because
/// every field is already sitting in the caller's `HarnessRun`.
#[derive(Debug, Clone, Copy)]
pub struct ObservedFailure<'a> {
    /// The gate's name — the join key against the baseline record.
    pub name: &'a str,
    /// The command as the user authored it. Compared against the baseline's,
    /// because a baseline measured with a *different* command is not a
    /// comparison at all, and only the string can show that.
    pub command: &'a str,
    /// `normalize_failure_fingerprint` over this gate's labelled failure block,
    /// built exactly the way the baseline producer built its own.
    pub fingerprint: &'a str,
}

/// One gate's determination, with the evidence it was reached on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateComparison {
    /// The gate's name, echoed so the caller can rejoin without re-zipping.
    pub name: String,
    pub determination: GateDetermination,
    /// What a *covering* record said about this gate, when there was one.
    /// `None` for [`GateDetermination::NoBaseline`]. Carried so the caller can
    /// name the exclusion — which commit, measured by which producer, when —
    /// without holding the record open a second time.
    pub baseline: Option<HarnessBaselineRun>,
}

/// Compare one red gate against the baseline.
///
/// `base_sha` is the commit **this run** forked from. It is checked against the
/// record's own ([`HarnessBaseline::covers`]) before anything is subtracted:
/// that field exists precisely so a stale measurement is detectable rather than
/// silently trusted, and skipping the check would compare this feature's
/// harness against a measurement of different code. An empty `base_sha` — the
/// merge-base would not resolve, or the run was cancelled before it could be
/// asked — can cover nothing, so it yields
/// [`NoBaseline`](GateDetermination::NoBaseline).
pub fn compare_gate(
    baseline: Option<&HarnessBaseline>,
    base_sha: &str,
    observed: &ObservedFailure<'_>,
) -> GateComparison {
    let unknown = || GateComparison {
        name: observed.name.to_string(),
        determination: GateDetermination::NoBaseline,
        baseline: None,
    };

    if base_sha.trim().is_empty() {
        return unknown();
    }
    // `harness(name)` is the only way to ask, and it answers `None` for a gate
    // that was never measured — so a record holding no measurement of this gate
    // says nothing rather than saying "fine" (HB2a).
    let Some(measured) = baseline
        .filter(|b| b.covers(base_sha))
        .and_then(|b| b.harness(observed.name))
    else {
        return unknown();
    };

    // A different command is a different question. The record keeps the command
    // string for exactly this check: `npm test` at the base tells us nothing
    // about `npm run test:ci` at the tip, however alike the two names are.
    if measured.command.trim() != observed.command.trim() {
        return unknown();
    }

    let determination = if measured.exit_ok {
        // Rung 1 answered it. Nothing a fingerprint could say changes a gate
        // that passed at the base and fails now.
        GateDetermination::Regression
    } else if !measured.fingerprint.is_empty() && measured.fingerprint == observed.fingerprint {
        // Rung 2. The empty guard is not pedantry: a red gate owes a
        // fingerprint, and a red record without one is a shape we do not
        // understand — matching it against an equally empty live fingerprint
        // would subtract a failure on the strength of two blanks.
        //
        // Same failure both sides — but *what* the failure was decides whether
        // it may be subtracted at all. The subtraction applies to a **verdict**
        // the gate reached: a pre-existing code defect predates the feature, and
        // removing it leaves the rest of the gate's evidence standing. A gate
        // that could not run reached no verdict, so there is no evidence to
        // leave standing and passing on it certifies nothing. Absent
        // classification reads as the former (see
        // `HarnessBaselineRun::environment`): only a positive `environment`
        // answer can terminate a run.
        match &measured.environment {
            Some(fault) => GateDetermination::Environment {
                reason: fault.reason.clone(),
                remediation: fault.remediation.clone(),
            },
            None => GateDetermination::PreExisting,
        }
    } else {
        GateDetermination::NewFailures
    };

    GateComparison {
        name: observed.name.to_string(),
        determination,
        baseline: Some(measured.clone()),
    }
}

/// Compare every red gate, preserving the order they ran in.
///
/// Order is preserved because it is the declared gate order (cheap gates
/// first), which is what the report renders and what the verdict reason names.
pub fn compare_gates(
    baseline: Option<&HarnessBaseline>,
    base_sha: &str,
    observed: &[ObservedFailure<'_>],
) -> Vec<GateComparison> {
    observed
        .iter()
        .map(|o| compare_gate(baseline, base_sha, o))
        .collect()
}

#[cfg(test)]
#[path = "../../tests/domain/harness_delta.rs"]
mod tests;

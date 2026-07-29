//! The harness baseline — what the project's gates said *before* the feature.
//!
//! Validate today asks "is the harness green?" and treats any non-zero exit as
//! this feature's verdict. Nothing establishes what the suite did on the base
//! commit, so a repository that was already red sends the run into a rework
//! loop for a defect it did not introduce. This record is the other half of
//! that subtraction: decision 44, `docs/HARNESS_BASELINE.md` HB2a.
//!
//! It is deliberately a **pure value object with pure operations**. The two
//! things anyone does with it — merging a fresh measurement into whatever is
//! already stored, and asking what a named gate did at the base — are policy
//! decisions, not I/O, so they live here where a test reaches them with no port
//! doubles at all (AGENTS.md §3, "Where a decision is allowed to live").
//! Persistence is one JSON column, `features.harness_baseline_json` (V37).
//!
//! ## The property the shape exists to protect
//!
//! **Absent is not green.** "No baseline was ever measured" and "every harness
//! passed at the base commit" are opposite answers, and mistaking the first for
//! the second inverts HB2c's whole decision table: a genuine regression would
//! read as pre-existing and be excluded from the verdict. Two things enforce
//! it structurally rather than by convention:
//!
//! * the column decodes to `Option<HarnessBaseline>`, and a missing/corrupt
//!   value decodes to `None` rather than to an empty record;
//! * there is no record-level "was it green?" accessor. Every question about a
//!   gate's status goes through [`HarnessBaseline::harness`], which can only
//!   answer for a harness that was actually measured, so a record holding no
//!   measurements answers nothing rather than answering "fine".

use serde::{Deserialize, Serialize};

/// Which producer measured a gate. The two have very different wall-clock
/// stories — the node runs at the head of the graph, hidden behind research;
/// the fallback runs on validate's failure path against a freshly provisioned
/// detached worktree with no `node_modules` and no `target/` — so a support
/// question about "why did this run take so long" needs to be able to tell
/// them apart after the fact.
///
/// Recorded per harness rather than per record: a partial re-measurement
/// merges into an existing record, so one record can legitimately hold gates
/// measured by both producers at different times, and a record-level field
/// would then be a lie about some of its own entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaselineProducer {
    /// The in-graph `baseline-harness` command node at the head of the
    /// starters (HB2b / P4.2a). The cheap path and the default: zero tokens,
    /// and its wall-clock hides behind research.
    Node,
    /// The verifier's lazy fallback (HB2b), measured on validate's failure
    /// path when no record existed. Minutes on a cold repo — which is exactly
    /// why it is not the default producer.
    Fallback,
}

/// Why a gate was red at the base **because it could not run on this machine**
/// — a missing system library, an absent toolchain, an unprovisioned service —
/// rather than because the code under it was broken.
///
/// The distinction is the whole reason this type exists. HB2c subtracts a gate
/// that is red at the base and identically red now, which is right for a
/// pre-existing *code* defect: the gate ran, reached a verdict, and that verdict
/// predates the feature. It is wrong for a gate that never ran, because then the
/// step passed on evidence that does not exist. The motivating incident was a
/// missing `gdk-3.0`, which exits **1**, not 127 — so the exit-127 fast path
/// cannot see it and only a classifier can tell the two apart.
///
/// The two strings are `TriageVerdict::Environment`'s own, carried verbatim so
/// `build_environment_message` renders the same text here as it does on every
/// other terminal environment failure: what to install, the failing command, and
/// a copy-pasteable reproduce line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineEnvironmentFault {
    /// One sentence naming what the machine is missing.
    pub reason: String,
    /// The concrete provisioning step, e.g. "install libgtk-3-dev". May be
    /// empty — the classifier is not obliged to know one, and a reason without
    /// a remedy is still worth more than a silent pass.
    pub remediation: String,
}

/// One gate's measurement at the base commit.
///
/// Mirrors `HarnessRun` in `driver/verifier.rs` field for field on the three
/// that identify a run (`name`, the command, and what it said), so HB2c's
/// subtraction and HB7's rendering are a comparison rather than a translation
/// layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessBaselineRun {
    /// The gate's name — `default` when it came from the project's
    /// `test_command`, `prepare` for the prepare command. This is the merge
    /// key and the join key against the live `HarnessRun`.
    pub name: String,
    /// The command as the user authored it (not the `2>&1` wrapper). Recorded
    /// because a baseline measured with a *different* command than the one
    /// validate runs is not a comparison, and only the string can show that.
    pub command: String,
    /// Whether the command exited zero at the base commit.
    pub exit_ok: bool,
    /// `normalize_failure_fingerprint` over the failing output, **empty when
    /// `exit_ok`**. This is the cheap rung of HB2c's granularity ladder: an
    /// identical fingerprint before and after is a pre-existing failure, a
    /// different one is new failures atop it.
    #[serde(default)]
    pub fingerprint: String,
    /// `ArtifactStore` reference to the merged stdout+stderr — **never the
    /// output itself.** Harness output is megabytes and this record is read on
    /// every validate attempt; a baseline you cannot afford to read is not a
    /// baseline. `None` when the producer stored nothing (a green gate whose
    /// output nobody needs).
    #[serde(default)]
    pub output_ref: Option<String>,
    /// What the triage classifier said about this gate's *red* measurement, and
    /// only when it said "environment". `None` covers three different histories
    /// that all have to behave identically:
    ///
    /// * the gate was green, so there was nothing to classify;
    /// * the classifier answered `regression` — a genuine pre-existing code
    ///   defect, which is exactly what HB2c subtracts;
    /// * nothing classified it at all: the classifier could not be spawned, timed
    ///   out, or the record was written by a build that predates this field (the
    ///   column is JSON, so an older record simply omits it and decodes to
    ///   `None`).
    ///
    /// Collapsing all three onto `None` is deliberate, and it is the fail-safe
    /// direction. `Some` is the only value that can *terminate* a run, so a
    /// classifier that malfunctions withholds an escalation — it can never
    /// manufacture one. That mirrors `triage_harness_failure`'s own fallback to
    /// `TriageVerdict::Regression` on every spawn/timeout/cancel/parse failure.
    #[serde(default)]
    pub environment: Option<BaselineEnvironmentFault>,
    /// The test identifiers this gate's *red* measurement named, read out of its
    /// own output — the third rung of HB2c's granularity ladder.
    ///
    /// `None` means **no reading was obtained**, and it collapses four
    /// different histories that must behave identically: the gate was
    /// green so there was nothing to read; the extractor could not be spawned,
    /// timed out, or answered nothing parseable; or the record was written by a
    /// build that predates this field (the column is JSON, so an older record
    /// simply omits it). Every one of those degrades to rungs 1–2, i.e. to the
    /// behaviour before this field existed.
    ///
    /// It is deliberately **not** evidence. Whether the gate passed is the exit
    /// status and nothing else; this only enumerates what the output *named*,
    /// so a wrong or missing reading can narrow a retry's advice and can never
    /// turn a red gate green. Decision 44 rejects agent-produced evidence; this
    /// is an agent-produced *reading of* evidence the engine already owns.
    #[serde(default)]
    pub failing_tests: Option<Vec<String>>,
    /// Unix seconds at which *this gate* was measured.
    pub measured_at: i64,
    /// Which producer measured this gate.
    pub producer: BaselineProducer,
}

/// Everything measured at one base commit, for one feature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessBaseline {
    /// The commit the measurement was taken against. **The field most easily
    /// omitted and most expensive to have omitted**: a baseline taken against
    /// a different base commit is not evidence about this run, and without the
    /// sha there is no way to notice — the record would look perfectly valid
    /// while describing other code. Every consumer gates on
    /// [`covers`](HarnessBaseline::covers) before subtracting.
    pub base_sha: String,
    /// The gates measured, in the order they were run. A `Vec` and not a map
    /// because the run order is the declared gate order (cheap gates first)
    /// and HB7 renders it as such; lookup is by [`harness`](Self::harness).
    #[serde(default)]
    pub harnesses: Vec<HarnessBaselineRun>,
}

impl HarnessBaseline {
    /// A record with no measurements yet, for `base_sha`. Producers fill it
    /// through [`merge`](Self::merge); it is *not* a claim that nothing needed
    /// running.
    pub fn empty(base_sha: impl Into<String>) -> Self {
        Self {
            base_sha: base_sha.into(),
            harnesses: Vec::new(),
        }
    }

    /// What the named gate did at the base. `None` means **this gate was never
    /// measured** — not that it passed. The only way to ask about a gate's
    /// baseline status, so no caller can read a green answer out of a record
    /// that does not contain one.
    pub fn harness(&self, name: &str) -> Option<&HarnessBaselineRun> {
        self.harnesses.iter().find(|h| h.name == name)
    }

    /// Whether this record is evidence about `base_sha`. A consumer that
    /// subtracts without asking is comparing the feature's harness against a
    /// measurement of different code.
    pub fn covers(&self, base_sha: &str) -> bool {
        !self.base_sha.is_empty() && self.base_sha == base_sha
    }

    /// Fold a fresh measurement into whatever is already stored.
    ///
    /// **A partial write must merge, not clobber.** HB2b's lazy fallback
    /// measures the *one* gate that just went red, not the whole set; replacing
    /// the record with it would discard the node's measurement of every other
    /// gate and silently narrow the subtraction to one harness. So:
    ///
    /// * same `base_sha` → per-name upsert. `incoming`'s entries replace
    ///   same-named entries **in place** (a re-measurement of `lint` stays
    ///   where `lint` was, so the declared gate order survives), and names only
    ///   `incoming` knows about are appended.
    /// * different `base_sha` → `incoming` **replaces** the record whole. The
    ///   stored entries describe another commit; blending them would produce a
    ///   record whose own `base_sha` is false for half its contents, which is
    ///   worse than having no baseline at all.
    ///
    /// Both writer and reader are the same process against one SQLite file
    /// (decision 44), so this is an in-process read-modify-write and nothing
    /// here needs to survive a concurrent writer on another host.
    pub fn merge(existing: Option<Self>, incoming: Self) -> Self {
        let Some(mut merged) = existing else {
            return incoming;
        };
        if merged.base_sha != incoming.base_sha {
            return incoming;
        }
        for run in incoming.harnesses {
            match merged.harnesses.iter_mut().find(|h| h.name == run.name) {
                Some(slot) => *slot = run,
                None => merged.harnesses.push(run),
            }
        }
        merged
    }

    /// Decode the `features.harness_baseline_json` column.
    ///
    /// Every failure mode — NULL, empty string, unparseable JSON, a record
    /// written by a newer version naming a producer this build does not know —
    /// degrades to `None`, i.e. *no baseline*. That is the safe direction:
    /// the consequence of `None` is today's behaviour (no subtraction), while
    /// the consequence of inventing a record would be excluding a real
    /// regression from the verdict.
    pub fn from_column(raw: Option<&str>) -> Option<Self> {
        let raw = raw?.trim();
        if raw.is_empty() {
            return None;
        }
        serde_json::from_str(raw).ok()
    }

    /// Encode for the column. `None` in, `None` out — the absent record is
    /// stored as SQL NULL rather than as `"null"` or `"{}"`, so nothing
    /// downstream has to distinguish three spellings of nothing.
    pub fn to_column(value: Option<&Self>) -> Option<String> {
        value.and_then(|v| serde_json::to_string(v).ok())
    }
}

/// The `{{harness_baseline}}` prompt block: what this project's harness can
/// actually prove, and what it already said about the code before the feature
/// started.
///
/// # The failure this exists to stop
///
/// Both failed validate attempts in `f-1785157902856` cost a rework cycle for
/// the same reason: the spec's acceptance criteria named commands the harness
/// never ran, so they could not be shown MET however correct the
/// implementation was (`docs/HARNESS_BASELINE.md` §1). The spec prompt already
/// handled a *blank* command; what it had no way to state was the **positive**
/// fact — these gates, these commands, and this is what they said. Guessing at
/// it from `test_command` alone became wrong the moment harnesses became plural
/// (HB5): a project whose validation gates select `lint` and `unit` runs
/// neither `test_command` nor one command.
///
/// # Why the empty case is worded as hard as it is
///
/// With no gates resolved, *every* criterion phrased against a command is
/// unprovable, and that is knowable here — at spec time, for the price of a
/// paragraph — instead of after the whole implement budget is gone and only if
/// the validate agent then picks `environment` over `fail`. So the block does
/// not merely omit a command list: it says nothing will run, says what that
/// means for a criterion, and names the settings that would change it.
///
/// Pure over the two values the caller already holds, so every wording decision
/// above is assertable without a driver.
pub fn render_harness_briefing(
    gates: &[crate::domain::verifier::ResolvedHarness],
    baseline: Option<&HarnessBaseline>,
) -> String {
    if gates.is_empty() {
        return "## What this project's harness can prove — NOTHING\n\
                No validation gate is configured for this project, so the orchestrator will \
                execute **no command at all** before judging the finished work. The only \
                evidence available to the validator is a reading of the diff.\n\n\
                That is an absence of evidence, not a passing result. A criterion that \
                requires a command to be run can therefore never be shown MET, however \
                correct the implementation is — and no amount of re-implementation can \
                change that, because the missing piece is a project setting (the test \
                command, or the harnesses selected as validation gates), not the code.\n\n\
                So: raise this as the **first Open Question**, and phrase every acceptance \
                criterion as something reviewable in the diff by eye. Do not assert a \
                criterion the harness cannot evidence.\n"
            .to_string();
    }

    let mut out = String::from(
        "## What this project's harness can prove\n\
         These are the **only** commands the orchestrator executes when it validates the \
         finished work, run in this order, each as its own gate:\n\n",
    );
    for gate in gates {
        let measured = baseline.and_then(|b| b.harness(&gate.name));
        let status = match measured {
            // Named as "before this feature started" rather than "at the base",
            // because the reader needs the attribution, not the git vocabulary.
            Some(run) if run.exit_ok => {
                "passed against this repository before this feature started".to_string()
            }
            Some(_) => "**already failing** against this repository before this feature \
                        started — this gate's output cannot evidence a new criterion until \
                        that pre-existing failure is dealt with"
                .to_string(),
            // Absent is not green (HB2a). Say so, rather than leaving a blank
            // that reads as a pass.
            None => "not measured before this feature started, so nothing is known about \
                     what it says on this repository"
                .to_string(),
        };
        out.push_str(&format!(
            "- `{}` → `{}`\n  - {}\n",
            gate.name, gate.command, status
        ));
    }
    out.push_str(
        "\nWrite every acceptance criterion so it can be judged from those commands' output \
         plus a reading of the diff. A criterion requiring anything they do not run can \
         never be shown MET — it fails validation forever and burns the whole rework \
         budget. If the feature genuinely needs a gate the list above does not cover, say \
         so in **Open Questions** and phrase the criterion against what *is* run; do not \
         assert it.\n",
    );
    out
}

/// Whether validate's failure path should measure a baseline **itself** — the
/// lazy fallback of HB2b, the producer that makes the subtraction
/// unconditional rather than a privilege of the workflows that happen to carry
/// a `baseline-harness` node.
///
/// Pure, and deliberately so: "should we spend minutes of wall-clock measuring
/// a baseline right now" is a policy decision, and AGENTS.md §3 puts those here
/// rather than inside the `async fn` that would also do the measuring. Every
/// input is a value the caller already holds.
///
/// The four conditions, and what each is protecting:
///
/// * **`harness_failed`** — the fallback fires *only* on the failure path. On a
///   green harness there is nothing to subtract from, and measuring anyway
///   would add minutes to every successful run to answer a question nobody
///   asked. This is an argument rather than an implicit precondition precisely
///   so a test can hold everything else fixed and prove the green path measures
///   nothing.
/// * **a non-empty `base_sha`** — a measurement that cannot say *which commit*
///   it describes is not evidence (see [`HarnessBaseline::base_sha`]). If the
///   merge-base would not resolve, the honest answer is no baseline, not a
///   baseline against an unknown commit.
/// * **a non-empty `gates`** — nothing gates this step, so nothing failed that
///   a baseline could excuse.
/// * **no covering measurement already** — a record that
///   [`covers`](HarnessBaseline::covers) this base *and* already holds every
///   gate in `gates` answers the question. Re-measuring would burn the same
///   minutes on the second validate attempt of the same run, which is exactly
///   what caching the fallback's own write exists to prevent. A record covering
///   a *different* sha does not count: it describes other code.
///
/// Note what is **not** here: whether the baseline was green or red. A stored
/// measurement is an answer either way, and re-measuring a gate that was
/// already measured at this commit cannot produce new information.
pub fn fallback_baseline_needed(
    harness_failed: bool,
    base_sha: &str,
    existing: Option<&HarnessBaseline>,
    gates: &[String],
) -> bool {
    if !harness_failed || base_sha.trim().is_empty() || gates.is_empty() {
        return false;
    }
    let Some(existing) = existing.filter(|b| b.covers(base_sha)) else {
        return true;
    };
    gates.iter().any(|g| existing.harness(g).is_none())
}

/// The gate a baseline measurement found unrunnable, named so the caller can
/// build the terminal message without searching the record a second time.
///
/// Borrowed rather than owned: every field is already a `String` on the record,
/// and the only thing anyone does with this is render it into
/// `build_environment_message`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnrunnableBaselineGate<'a> {
    /// The gate's name, for the log line — the message itself names the
    /// command, which is what a reproduce line needs.
    pub name: &'a str,
    /// The command as the user authored it.
    pub command: &'a str,
    /// One sentence naming what the machine is missing, verbatim from the
    /// classifier.
    pub reason: &'a str,
    /// The concrete provisioning step; may be empty.
    pub remediation: &'a str,
}

/// Does this baseline measurement permit the run to continue? `Some` means no,
/// and names the gate that says so (HB9).
///
/// # Why the baseline halts on this and not on a red gate
///
/// A gate that is simply **red** at the base completes the node and the run goes
/// on — that is the whole point of measuring a baseline, and halting there would
/// restate the misattribution the subtraction exists to remove (decision 44). A
/// gate the classifier said is red *because this machine cannot run it* is the
/// opposite case: it produced no evidence at the base and will produce none at
/// validate either, so every token spent between here and there buys nothing.
/// [`GateDetermination::Environment`](crate::domain::harness_delta::GateDetermination::Environment)
/// already terminates the run for exactly this gate — this asks the same
/// question at the head of the graph instead of after the implement budget is
/// spent, which is invariant I1 of `docs/HARNESS_BASELINE.md`.
///
/// Note what only this producer can see. `command -v` (HB1/HB4) catches a
/// missing **binary** before the run starts; the motivating incident was a
/// missing **library**, where `cargo` resolves fine and the *build* is what
/// fails, exiting 1. The baseline measurement is the first point at which that
/// is detectable at all.
///
/// # One unrunnable gate among green ones still halts
///
/// A gate the user selected as gating cannot produce evidence, so continuing
/// means the feature ships unverified on that dimension while looking verified.
/// HB1 makes the same call when one probed binary of several fails to resolve.
///
/// # The direction it fails in
///
/// Only a *positive* classification returns `Some`. A record that was never
/// classified, one the classifier called a regression, and one written by a
/// build that predates the field all decode to `None` on
/// [`HarnessBaselineRun::environment`] and are indistinguishable here — so a
/// malfunctioning classifier withholds a halt and can never manufacture one,
/// which is the same asymmetry `compare_gate` reads the field under.
///
/// A gate recorded **green** is likewise never a halt, however it is
/// classified. `measure_gates` classifies only red gates, so a green gate
/// carrying a fault is a shape nothing here wrote; refusing to act on it is the
/// safe reading of a record we do not understand.
///
/// # Only the first
///
/// The message this feeds carries a reproduce line, which means nothing for two
/// commands at once. The exit-127 fast path and the validate-time escalation
/// each name a single gate for the same reason.
///
/// The input is the gates **this measurement** produced, not the whole stored
/// record: a gate some earlier producer measured is not this node's finding, and
/// validate's own comparison is what answers for it.
pub fn unrunnable_baseline_gate(
    measured: &[HarnessBaselineRun],
) -> Option<UnrunnableBaselineGate<'_>> {
    measured.iter().filter(|run| !run.exit_ok).find_map(|run| {
        run.environment
            .as_ref()
            .map(|fault| UnrunnableBaselineGate {
                name: &run.name,
                command: &run.command,
                reason: &fault.reason,
                remediation: &fault.remediation,
            })
    })
}

#[cfg(test)]
#[path = "../../tests/domain/harness_baseline.rs"]
mod tests;

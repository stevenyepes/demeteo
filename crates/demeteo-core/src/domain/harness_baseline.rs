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

#[cfg(test)]
#[path = "../../tests/domain/harness_baseline.rs"]
mod tests;

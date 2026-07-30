use serde::{Deserialize, Deserializer, Serialize};

use crate::domain::models::{EffortLevel, WorktreeStrategy};

pub mod verdict;

#[cfg(test)]
#[path = "../../../tests/domain/verifier/harness_resolution_tests.rs"]
mod harness_resolution_tests;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerifierConfig {
    /// Agent kind for the verifier. `None` = same as the step's agent_kind.
    pub agent_kind: Option<String>,
    /// Model for the verifier turn. `None` = same as the step's model.
    /// The verifier's job — interpret harness output into one verdict
    /// JSON object — is a small-model task; setting a cheap model here
    /// cuts the per-attempt cost of every retry loop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Effort for the verifier turn. `None` = [`EffortLevel::VERIFIER_DEFAULT`]
    /// (low) — deliberately *not* the step's inherited effort, since this turn
    /// runs on every retry and only has to read harness output into a verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<EffortLevel>,
    /// Instructions injected as the verifier's prompt preamble.
    pub instructions: String,
    /// Names of the harnesses that gate this step (e.g. `["lint", "unit"]`),
    /// run **in declared order, each as its own command** with its own exit
    /// status and its own labelled output block. Empty (the default, and what
    /// all seven starters declare) hands the decision to the rest of the
    /// resolution chain — see [`resolve_harnesses`].
    ///
    /// Accepts the singular `harness_name` spelling this field replaced, in
    /// all three of its shapes (`null`, `"lint"`, and the list), so every
    /// workflow authored against the old field keeps parsing byte-for-byte
    /// unchanged. `null` and `""` both mean "not declared", not "a harness
    /// with an empty name".
    #[serde(
        default,
        alias = "harness_name",
        deserialize_with = "deserialize_harness_names",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub harness_names: Vec<String>,
    /// JSON key whose value must be `"pass"` or `"fail"`. Default: `"verdict"`.
    #[serde(default = "default_verdict_key")]
    pub verdict_key: String,
}

impl VerifierConfig {
    /// Human label for this verifier's harness selection — what the agent
    /// session is titled with. Falls back to [`DEFAULT_HARNESS_NAME`] when the
    /// step declares nothing, which is what the singular field's
    /// `unwrap_or("default")` produced.
    pub fn harness_label(&self) -> String {
        if self.harness_names.is_empty() {
            DEFAULT_HARNESS_NAME.to_string()
        } else {
            self.harness_names.join(", ")
        }
    }
}

fn default_verdict_key() -> String {
    "verdict".to_string()
}

/// Accept the three shapes a workflow may spell the harness selection in:
/// `null` (every shipped starter), one name, or an ordered list. Blank names
/// are dropped rather than resolved, since `""` can never match a key in the
/// project's `harnesses` map and silently falling through is what the old
/// `Option<String>` did.
fn deserialize_harness_names<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(Option<String>),
        Many(Vec<String>),
    }

    let names = match OneOrMany::deserialize(d)? {
        OneOrMany::One(None) => Vec::new(),
        OneOrMany::One(Some(name)) => vec![name],
        OneOrMany::Many(names) => names,
    };
    Ok(names.into_iter().filter(|n| !n.trim().is_empty()).collect())
}

/// The harness name a project-wide `test_command` runs under when no named
/// harness was selected — the label the singular field's fallback produced.
pub const DEFAULT_HARNESS_NAME: &str = "default";

/// One harness resolved to the command that will run it, and the wall-clock
/// ceiling *that command* may consume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHarness {
    /// The gate's name, as declared. What a failure is attributed to.
    pub name: String,
    /// The shell command to run, resolved from the project's `harnesses` map
    /// or its `test_command`.
    pub command: String,
    /// This command's own deadline, in seconds.
    ///
    /// **Per harness, not per step** (S10). `wall_cap_s` answers "how long may
    /// *one* command take", so N harnesses get N ceilings — it is deliberately
    /// **not** divided among them, because dividing would make a gate's
    /// deadline depend on how many *other* gates a workflow happens to declare,
    /// and a suite that passes alone would start timing out the moment someone
    /// adds a lint gate beside it. The consequence, stated rather than hidden:
    /// **the sum is unbounded** — a step declaring five harnesses may spend
    /// five ceilings before any agent turn. The step-level bound on that is the
    /// number of gates the author declares, and the escape hatch, if one is
    /// ever needed, is an opt-in `stop_on_first_failure` rather than a smaller
    /// per-command deadline.
    pub deadline_s: u64,
}

/// Decide **which harnesses gate this step**, most-specific-wins, and resolve
/// each to its command. Pure: the whole policy is decidable in a unit test with
/// no port double, and the `async fn` that runs the commands only executes what
/// this returned.
///
/// The chain, in order (mirrors how decision 5 (planner) and decision 37
/// (effort) resolve theirs):
///
/// 1. **the step's `verifier.harness_names`** — the workflow author was
///    explicit, so nothing else gets a say. A declared name the project's
///    `harnesses` map does not define falls back to `test_command` under the
///    declared name (what the singular field did), at most once no matter how
///    many unknown names are declared.
/// 2. **the project's selected validation gates**
///    ([`WorktreeStrategy::validation_gates`]) — the tier that makes the
///    `harnesses` map reachable at all, since **all seven starters declare no
///    harness**. Ticking `lint` there gates every workflow with no forking.
///    Names that no longer exist in the map are dropped (a stale selection is
///    not an authored declaration); if that empties the tier, resolution falls
///    through rather than gating on nothing.
/// 3. **the project's `test_command`** — today's fallback, unchanged, run under
///    the name [`DEFAULT_HARNESS_NAME`].
///
/// **Tier 2 is not additive.** "Always *also* run these" would make it
/// impossible to narrow, and would produce the surprise where a workflow pinned
/// to `unit` still runs the 20-minute integration suite. Most specific wins
/// outright.
///
/// An empty result means *nothing is configured* — an absence of evidence, not
/// a pass (S12).
pub fn resolve_harnesses(
    declared: &[String],
    strategy: &WorktreeStrategy,
    deadline_s: u64,
) -> Vec<ResolvedHarness> {
    // Tier 1 — the workflow author named them.
    if !declared.is_empty() {
        let resolved = resolve_named(declared, strategy, deadline_s, true);
        if !resolved.is_empty() {
            return resolved;
        }
    }

    // Tier 2 — the project's selected validation gates.
    if let Some(gates) = strategy.validation_gates.as_deref() {
        let resolved = resolve_named(gates, strategy, deadline_s, false);
        if !resolved.is_empty() {
            return resolved;
        }
    }

    // Tier 3 — the project's `test_command`.
    match strategy.test_command.as_deref().map(str::trim) {
        Some(cmd) if !cmd.is_empty() => vec![ResolvedHarness {
            name: DEFAULT_HARNESS_NAME.to_string(),
            command: cmd.to_string(),
            deadline_s,
        }],
        _ => Vec::new(),
    }
}

/// Which harnesses will judge this run's finished work.
///
/// The union of every step in the workflow that carries a `verifier`, each
/// resolved through [`resolve_harnesses`] — the same chain that step will
/// itself resolve through when it runs — deduplicated by name, in
/// first-declared order.
///
/// This is what the `{{harness_baseline}}` prompt block is built from (HB2c),
/// and its two obvious shortcuts are both wrong. Asking the *project* alone
/// (`resolve_harnesses(&[], …)`) lies about a workflow whose validate step pins
/// its own gates; asking only the first verifier lies about a workflow with two.
/// Either way the spec author is told about commands that will not run, which
/// is the same class of failure as telling them about none — and it is the
/// failure that cost two rework cycles in `f-1785157902856`.
///
/// Pure, so the wording downstream is assertable without a driver: the caller's
/// only job is to fetch the steps and the strategy.
pub fn resolve_gating_harnesses(
    steps: &[crate::domain::models::StepConfig],
    strategy: &WorktreeStrategy,
    deadline_s: u64,
) -> Vec<ResolvedHarness> {
    let mut gates: Vec<ResolvedHarness> = Vec::new();
    for verifier in steps.iter().filter_map(|s| s.verifier.as_ref()) {
        for gate in resolve_harnesses(&verifier.harness_names, strategy, deadline_s) {
            if !gates.iter().any(|g| g.name == gate.name) {
                gates.push(gate);
            }
        }
    }
    gates
}

/// Resolve a list of harness names against the project's `harnesses` map,
/// preserving declared order and dropping repeats.
///
/// `fallback_to_test_command` is the difference between the two tiers that use
/// this: a name the *workflow author* wrote is honoured even when the map has
/// no entry for it (the singular field resolved such a name to `test_command`,
/// and silently running nothing instead would turn a typo into an ungated
/// step), whereas a name in the *project's* selection that no longer exists in
/// the map is just a stale tick and is dropped. The fallback is emitted at most
/// once, so three unknown names cannot run `test_command` three times.
fn resolve_named(
    names: &[String],
    strategy: &WorktreeStrategy,
    deadline_s: u64,
    fallback_to_test_command: bool,
) -> Vec<ResolvedHarness> {
    let mut out: Vec<ResolvedHarness> = Vec::new();
    let mut used_fallback = false;
    for name in names {
        let name = name.trim();
        if name.is_empty() || out.iter().any(|r| r.name == name) {
            continue;
        }
        let mapped = strategy
            .harnesses
            .as_ref()
            .and_then(|h| h.get(name))
            .map(|c| c.trim())
            .filter(|c| !c.is_empty());
        let command = match mapped {
            Some(cmd) => cmd.to_string(),
            None if fallback_to_test_command && !used_fallback => {
                match strategy.test_command.as_deref().map(str::trim) {
                    Some(cmd) if !cmd.is_empty() => {
                        used_fallback = true;
                        cmd.to_string()
                    }
                    _ => continue,
                }
            }
            None => continue,
        };
        out.push(ResolvedHarness {
            name: name.to_string(),
            command,
            deadline_s,
        });
    }
    out
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifierVerdict {
    Pass,
    Fail(String), // reason
}

/// Structured "fail" verdict. Beyond the human-readable reason, the
/// verifier is asked to name the failing tests and the files it believes
/// are implicated — that is what lets the retry loop re-run only the
/// subtasks that own those files instead of re-implementing everything.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VerdictFailure {
    /// Actionable reason, injected as retry feedback.
    pub reason: String,
    /// Test identifiers that failed (verbatim from the harness), if known.
    #[serde(default)]
    pub failing_tests: Vec<String>,
    /// Repo-relative paths the verifier believes must change to fix the
    /// failure, if known. Used to select which subtasks re-run.
    #[serde(default)]
    pub implicated_files: Vec<String>,
}

impl VerdictFailure {
    pub fn from_reason(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            failing_tests: Vec::new(),
            implicated_files: Vec::new(),
        }
    }

    /// Render the failure as retry-feedback prose. The structured lists
    /// are appended so a retried agent sees them even when the caller
    /// only threads a plain string through.
    pub fn to_feedback(&self) -> String {
        let mut out = self.reason.clone();
        if !self.failing_tests.is_empty() {
            out.push_str("\nFailing tests:\n");
            for t in &self.failing_tests {
                out.push_str(&format!("- {}\n", t));
            }
        }
        if !self.implicated_files.is_empty() {
            out.push_str("\nFiles implicated:\n");
            for f in &self.implicated_files {
                out.push_str(&format!("- {}\n", f));
            }
        }
        out
    }
}

/// Distinguishes a deliberate verdict failure from a verifier infrastructure
/// problem. Only `Verdict` failures feed back into the `on_failure` retry
/// loop — `Infrastructure` errors indicate a broken verifier setup (bad
/// agent config, timeout, unparseable output) that retrying the implementation
/// step will not fix.
#[derive(Debug)]
pub enum VerifierError {
    /// The verifier ran to completion and explicitly returned "fail",
    /// or the harness itself exited non-zero. Carries the structured
    /// failure to inject as retry feedback.
    Verdict(VerdictFailure),
    /// The verifier could not complete: spawn failure, timeout, parse error,
    /// cancelled, or an unrecognised verdict value. The inner string describes
    /// the infrastructure problem for the user.
    Infrastructure(String),
    /// The harness ran and failed, but triage (C6, D7) determined the cause is
    /// the *execution environment* not being provisioned — a missing system
    /// library / toolchain / binary / service, a permission or network problem —
    /// which editing source code cannot fix. Routed to
    /// [`StepOutcome::NonRetryable`](crate::adapters::step_executor::steps::StepOutcome)
    /// so it terminates immediately instead of burning the implement retry
    /// budget. The inner string is user-facing remediation (what to install /
    /// fix, the failing command, and how to reproduce it), not a stack trace.
    Environment(String),
    /// The user pressed Stop while a harness command was still running.
    ///
    /// Distinct from every variant above because it is not a failure: nothing
    /// was judged, nothing is broken, and nothing should be persisted as an
    /// error on the step. Callers map it to
    /// [`StepOutcome::Cancelled`](crate::adapters::step_executor::steps::StepOutcome).
    ///
    /// It needs its own variant because the harness is the one place a step
    /// spends unbounded wall-clock *outside* an agent turn, so the turn-level
    /// cancel plumbing never covered it — Stop appeared to do nothing until the
    /// command exited on its own.
    Cancelled,
}

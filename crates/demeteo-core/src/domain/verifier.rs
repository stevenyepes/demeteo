use serde::{Deserialize, Serialize};

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
    /// Instructions injected as the verifier's prompt preamble.
    pub instructions: String,
    /// Name of the harness to run (e.g. "lint", "integration"). If `None`, falls back to the project's default `test_command`.
    pub harness_name: Option<String>,
    /// JSON key whose value must be `"pass"` or `"fail"`. Default: `"verdict"`.
    #[serde(default = "default_verdict_key")]
    pub verdict_key: String,
}

fn default_verdict_key() -> String {
    "verdict".to_string()
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
}

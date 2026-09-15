//! Per-attempt history for step executions (PRD §5.3, task P1.8).
//!
//! One row per *dispatch* of a step by the execution driver — an
//! `on_failure` redirect loop, an environmental in-place retry, and a
//! manual retry each open a fresh attempt instead of overwriting the
//! step row. The `step_executions` row keeps its cumulative totals;
//! attempts carry their own deltas.

use serde::{Deserialize, Serialize};

use crate::domain::ids::StepExecutionId;

/// Failure classes as stored in `step_attempts.error_class` — the same
/// vocabulary the declarative retry policy (P1.10,
/// [`RetryPolicy`](crate::domain::models::workflow_v2::RetryPolicy)) is
/// keyed by. Kept as string constants rather than an enum column so the
/// table stays queryable without a decode step; writers use these
/// constants only.
pub mod error_class {
    /// The environment broke, not the implementation (agent blocked,
    /// spawn failure, worktree provisioning).
    pub const ENVIRONMENT: &str = "environment";
    /// A verifier/harness explicitly failed the work with structure.
    pub const VERDICT: &str = "verdict";
    /// Plain step failure (agent error, merge failure, …).
    pub const AGENT_FAILURE: &str = "agent_failure";
    /// Failed in a way retrying the implementation cannot fix.
    pub const NON_RETRYABLE: &str = "non_retryable";
}

/// One attempt row. `attempt_no` is 1-based and dense per step
/// execution; `(step_execution_id, attempt_no)` is UNIQUE.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepAttempt {
    pub step_execution_id: StepExecutionId,
    pub attempt_no: u32,
    /// `running` while in flight, then one of
    /// `completed | failed | cancelled | interrupted | redirected | parked`.
    ///
    /// `parked` is the attempt that stopped to ask a human and closed
    /// before waiting, so the wait is not billed to it. The column has no
    /// `CHECK`, so the vocabulary lives here and in V31's comment; V31 is
    /// applied and checksummed, so this is the copy that gets updated.
    pub status: String,
    /// This attempt's own spend (delta), not the step's running total.
    pub cost_usd: Option<f64>,
    pub tokens: Option<i64>,
    pub wall_clock_ms: Option<u64>,
    /// One of the [`error_class`] constants; `None` for non-failures.
    pub error_class: Option<String>,
    /// Normalized failure output (see
    /// `normalize_failure_fingerprint`), for "same failure again?"
    /// comparisons across attempts.
    pub failure_fingerprint: Option<String>,
    /// The retry-policy rule that answered this failure (P1.10), as
    /// `<class>.<strategy>` — e.g. `verdict.redirect`,
    /// `environment.in_place`. `None` for non-failure outcomes and for
    /// failures preempted by a cancel (no rule was applied).
    pub applied_rule: Option<String>,
    /// Workspace state at attempt start (P1.14):
    /// `<repo HEAD>:<dirty|clean>`. `None` when the probe failed or the
    /// row predates the column. On resume of an interrupted node, a
    /// mismatch against the live workspace surfaces as the Decision-14
    /// synthetic gate instead of blind re-execution.
    pub workspace_fingerprint: Option<String>,
    /// `<step_execution_id>#<attempt_no>#<fingerprint>` (P1.14) — the
    /// idempotency identity of this attempt's side effects; groundwork
    /// for `command` nodes' `idempotent: false` semantics (P3.5).
    pub idempotency_key: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

/// The answer to "why did this pipeline fail" for one step, computed from its
/// attempt history alone — no port, no I/O, so a caller can compute it inline
/// from whatever attempts it already has in hand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailureVerdict {
    /// [`StepAttempt::error_class`] of the slice's last attempt, unchanged.
    /// `None` when that attempt did not fail.
    pub error_class: Option<String>,
    /// [`StepAttempt::applied_rule`] of the slice's last attempt, unchanged.
    pub applied_rule: Option<String>,
    /// Whether the two most recent attempts that actually failed (a
    /// non-`None` `error_class`) share a `failure_fingerprint`. `Some(true)`
    /// means the same failure is repeating — stop retrying and go read.
    /// `Some(false)` means the fingerprints differ — more likely a flaky
    /// environment. `None` when fewer than two failed attempts exist in the
    /// slice, or when either of the two fingerprints being compared is
    /// itself `None` (the workspace probe failed on that attempt, so no
    /// comparison can be made).
    pub repeated_failure: Option<bool>,
    /// This attempt's own spend (the delta [`StepAttempt`] already carries),
    /// not the step's cumulative total.
    pub cost_usd: Option<f64>,
    pub tokens: Option<i64>,
    pub wall_clock_ms: Option<u64>,
}

/// The trailing slice of a log, capped to a byte budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogTail {
    pub text: String,
    /// `true` when `text` is shorter than the input because the input
    /// exceeded the budget it was cut to.
    pub truncated: bool,
    /// Bytes dropped from the front of the input. `0` when `truncated` is
    /// `false`.
    pub omitted_bytes: usize,
}

/// Byte budget [`tail_log`] cuts its output to. A failure explanation is
/// meant to be read in one glance alongside a [`FailureVerdict`], not paged
/// through, and 4 KiB of trailing log text is already generous for that —
/// most single-assertion or single-stack-trace failures fit in a fraction of
/// it. Kept separate from `prompt_budget::HARNESS_SECTION_BUDGET_BYTES`: that
/// one is an argv-size ceiling shared across a step's declared gates for a
/// harness-agent prompt, this one bounds a single tail for a human-facing
/// verdict, and nothing ties the two together.
pub const LOG_TAIL_BUDGET_BYTES: usize = 4 * 1024;

/// Compute why a step failed from its own attempt history — synchronous and
/// total, like everything in `domain/` (see [`crate::domain::harness_fingerprint::should_triage`]
/// for the sibling shape: a pure decision fn beside the data it decides
/// about).
pub fn explain_failure(attempts: &[StepAttempt]) -> FailureVerdict {
    let last = attempts.last();

    let mut recent_failures = attempts.iter().rev().filter(|a| a.error_class.is_some());
    let repeated_failure = match (recent_failures.next(), recent_failures.next()) {
        (Some(newest), Some(prior)) => {
            match (&newest.failure_fingerprint, &prior.failure_fingerprint) {
                (Some(a), Some(b)) => Some(a == b),
                _ => None,
            }
        }
        _ => None,
    };

    FailureVerdict {
        error_class: last.and_then(|a| a.error_class.clone()),
        applied_rule: last.and_then(|a| a.applied_rule.clone()),
        repeated_failure,
        cost_usd: last.and_then(|a| a.cost_usd),
        tokens: last.and_then(|a| a.tokens),
        wall_clock_ms: last.and_then(|a| a.wall_clock_ms),
    }
}

/// Cut `body` to its trailing `<= budget_bytes` bytes, moving the cut point
/// forward to the next line boundary so the result is always a clean suffix
/// — never a split UTF-8 character, never a partial line. Tail-only sibling
/// of `prompt_budget::window_harness_log`'s head+tail split: this feature
/// only ever wants the end of the log, so it doesn't reach for that budget's
/// omission banner either.
pub fn tail_log(body: &str, budget_bytes: usize) -> LogTail {
    if body.len() <= budget_bytes {
        return LogTail {
            text: body.to_string(),
            truncated: false,
            omitted_bytes: 0,
        };
    }

    let start = line_start_at_or_after(body, body.len() - budget_bytes);
    LogTail {
        text: body[start..].to_string(),
        truncated: true,
        omitted_bytes: start,
    }
}

/// The start of the first line that begins at or after `at`, falling back to
/// a char boundary when the suffix holds no line break at all.
fn line_start_at_or_after(s: &str, at: usize) -> usize {
    let at = ceil_char_boundary(s, at);
    match s[at..].find('\n') {
        Some(nl) => at + nl + 1,
        None => at,
    }
}

/// Smallest char boundary `>= at`. `str::ceil_char_boundary` is still
/// unstable.
fn ceil_char_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
#[path = "../../../tests/domain/models/step_attempt.rs"]
mod tests;

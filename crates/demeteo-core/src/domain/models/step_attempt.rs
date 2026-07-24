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
    /// `completed | failed | cancelled | interrupted | redirected`.
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
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

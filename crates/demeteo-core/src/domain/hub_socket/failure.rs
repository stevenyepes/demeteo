//! Which [`FailureReason`] a failure carries to the Hub (docs/HUB.md §5).
//!
//! A `step_progress` or `feature_status` row says only that something
//! failed. Why is in the rows the engine wrote just before it, on both
//! transports alike (`run_event_record` is the one translation the desktop
//! and the runner share): the `retry_decision` that answered the failure,
//! carrying the `error_class` its `step_attempts` row also holds; an
//! `env_not_ready` triage verdict; a failed `bootstrap_progress` phase; the
//! runner's `over_budget`. [`failure_reason`] reads those and answers one
//! enum variant. Their free text is read here, on the instance, and never
//! leaves it — only the variant does.

use demeteo_hub_protocol::FailureReason;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::domain::models::step_attempt::error_class;
use crate::ports::run_events::RunEvent;

/// How many offsets before a failure row its context is looked for. The
/// classifying row is written a statement or two before the failure; the
/// window only has to outlast the rows other Features append in between,
/// and bounds the read when a run's own log is long.
pub const FAILURE_CONTEXT_WINDOW: i64 = 512;

/// The wording every wall-clock cap an agent turn or a verifier trips is
/// reported in. [`failure_reason`] matches it in a `retry_decision`'s
/// reason, so a cap message that drops it reads as the class it failed
/// under instead of [`FailureReason::WallClockCap`].
pub const WALL_CLOCK_CAP_PHRASE: &str = "exceeded wall clock cap";

/// How the runner's `over_budget` row opens when the cap it passed was the
/// run's wall clock rather than its cost.
pub const OVER_WALL_CLOCK_CAP: &str = "wall clock";

/// Whether `row` may carry a reason at all, so the session reads context
/// for these rows only.
pub fn may_carry_failure(row: &RunEvent) -> bool {
    matches!(
        (row.kind.as_str(), status(row).as_deref()),
        ("step_progress", Some("failed")) | ("feature_status", Some("failed" | "cancelled"))
    )
}

/// The reason `failed` carries, given rows of the same run that precede it
/// in offset order. `None` for a row that is no failure, and where the
/// instance has no say:
///
/// - a Feature that failed without a Step failing or a bootstrap phase
///   failing in `before` — a desktop's shadow of a detached run logs only
///   its status, and reporting [`FailureReason::Other`] would claim a
///   classification the desktop never made;
/// - a cancel that no cap caused: a cancel is its own reason.
///
/// A Step's failure always has one: the engine ran it here, so a failure
/// with no classifying row is [`FailureReason::Other`].
pub fn failure_reason(failed: &RunEvent, before: &[RunEvent]) -> Option<FailureReason> {
    if !may_carry_failure(failed) {
        return None;
    }
    let context = Context::of(before.iter().filter(|row| row.offset < failed.offset));
    match failed.kind.as_str() {
        "step_progress" => Some(context.step_reason(&field(failed, "step_id")?)),
        _ if status(failed).as_deref() == Some("cancelled") => context.over_cap,
        _ => context.last_step_failure.or(context
            .bootstrap_failed
            .then_some(FailureReason::Environment)),
    }
}

/// One classifying answer, by the `error_class` it was written under and
/// the reason text beside it.
///
/// `usage_limit` is matched by its spelling: the class is the retry
/// policy's, and a class this build has no constant for still has to read
/// as itself rather than as [`FailureReason::Other`].
pub fn classified(class: &str, reason: &str) -> FailureReason {
    if class == "usage_limit" {
        return FailureReason::UsageLimit;
    }
    if reason.contains(WALL_CLOCK_CAP_PHRASE) {
        return FailureReason::WallClockCap;
    }
    match class {
        error_class::VERDICT => FailureReason::VerdictFailed,
        error_class::ENVIRONMENT => FailureReason::Environment,
        error_class::AGENT_FAILURE => FailureReason::AgentError,
        error_class::NON_RETRYABLE => FailureReason::NotRetryable,
        _ => FailureReason::Other,
    }
}

/// What the rows before a failure said, folded in order.
#[derive(Default)]
struct Context {
    /// The latest retry decision of each Step's current attempt, as
    /// `(error_class, reason)`. A Step that runs again starts clean, so a
    /// decision from an earlier attempt never answers a later failure.
    decisions: HashMap<String, (String, String)>,
    not_ready: HashSet<String>,
    last_step_failure: Option<FailureReason>,
    bootstrap_failed: bool,
    over_cap: Option<FailureReason>,
}

impl Context {
    fn of<'a>(rows: impl Iterator<Item = &'a RunEvent>) -> Self {
        let mut context = Self::default();
        for row in rows {
            context.observe(row);
        }
        context
    }

    fn observe(&mut self, row: &RunEvent) {
        let step = || field(row, "step_id");
        match row.kind.as_str() {
            "step_progress" => {
                let Some(step) = step() else { return };
                match status(row).as_deref() {
                    Some("failed") => self.last_step_failure = Some(self.step_reason(&step)),
                    Some("running" | "pending" | "completed") => {
                        self.decisions.remove(&step);
                        self.not_ready.remove(&step);
                    }
                    _ => {}
                }
            }
            "retry_decision" => {
                if let (Some(step), Some(class)) = (step(), field(row, "error_class")) {
                    let reason = field(row, "reason").unwrap_or_default();
                    self.decisions.insert(step, (class, reason));
                }
            }
            "env_not_ready" => {
                if let Some(step) = step() {
                    self.not_ready.insert(step);
                }
            }
            "bootstrap_progress" => {
                self.bootstrap_failed |= status(row).as_deref() == Some("failed");
            }
            "over_budget" => {
                let text = payload(row)
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_default();
                self.over_cap = Some(if text.starts_with(OVER_WALL_CLOCK_CAP) {
                    FailureReason::WallClockCap
                } else {
                    FailureReason::BudgetCap
                });
            }
            _ => {}
        }
    }

    /// A triage verdict that the machine is broken outranks the class the
    /// failure was then retried under — except a usage limit, which is the
    /// provider's answer and says more than "the environment".
    fn step_reason(&self, step: &str) -> FailureReason {
        let decided = self
            .decisions
            .get(step)
            .map(|(class, reason)| classified(class, reason));
        match decided {
            Some(FailureReason::UsageLimit) => FailureReason::UsageLimit,
            _ if self.not_ready.contains(step) => FailureReason::Environment,
            Some(reason) => reason,
            None => FailureReason::Other,
        }
    }
}

fn payload(row: &RunEvent) -> Option<Value> {
    serde_json::from_str(row.payload_json.as_deref()?).ok()
}

fn field(row: &RunEvent, name: &str) -> Option<String> {
    payload(row)?.get(name)?.as_str().map(str::to_owned)
}

fn status(row: &RunEvent) -> Option<String> {
    field(row, "status")
}

#[cfg(test)]
#[path = "../../../tests/domain/hub_socket_failure.rs"]
mod tests;

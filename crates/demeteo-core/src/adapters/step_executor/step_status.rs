//! One step transition: written to the row, then announced.
//!
//! Every caller here is reporting the *same* decision in two places — the
//! durable `step_executions` row and the live `StepProgress` event — and
//! the order between them is the contract (P1.13: the row is the truth,
//! the event is its push). Keeping both halves behind one call is what
//! stops a caller from emitting a transition it never persisted.
//!
//! The three bundles below are what the twelve-argument signature was
//! spelling out longhand: the pair of ports that always travel together,
//! the transition itself, and the cache-token telemetry that is always
//! read as a pair off the driver.

use crate::domain::ids::FeatureId;
use crate::domain::models::StepExecution;
use crate::ports::db::{FeatureRepository, StepExecutionPatch};
use crate::ports::notification::{DomainEvent, NotificationPort};

/// Where one transition lands: the row's home, the event's channel, and
/// the feature both are scoped to.
///
/// The two ports are never one without the other — a persisted status
/// nobody is told about and an announced status nobody persisted are both
/// bugs — and neither is addressable without the feature id, so all three
/// travel as one.
#[derive(Clone, Copy)]
pub(crate) struct StatusWriters<'a> {
    pub features: &'a dyn FeatureRepository,
    pub notif: &'a dyn NotificationPort,
    pub f_id: &'a FeatureId,
}

/// Prompt-cache telemetry for the turn that produced this transition:
/// tokens read from cache, and tokens written into it.
///
/// A pair because the live cache chip renders them together and because
/// every call site reads them together off `ExecutionDriver`'s
/// `last_cache_read` / `last_cache_creation`. `None` on both means the
/// transition carries no cache news — not "zero tokens".
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct CacheTokens {
    pub read: Option<u64>,
    pub creation: Option<u64>,
}

/// What one step row becomes, and what the matching event announces.
///
/// Built through the named constructors below rather than field-by-field:
/// each one fixes the `status` string against the payload that status is
/// allowed to carry, so a `completed` transition cannot arrive with an
/// error message and a `pending` one cannot arrive with an artifact.
///
/// `artifact_path: None` means **leave the column as it is**, not "clear
/// it" — see the patch construction in [`try_update_step_status`].
///
/// `error_message` is the one column where "leave it" is the wrong default and
/// the constructors never choose it: a row that ran again and succeeded still
/// rendering the previous attempt's failure is a `completed` step the Output
/// tab paints red. Every other node kind clears it by hand in its own
/// completion path, so the clearing belongs to the status rather than to
/// whoever remembered.
pub(crate) struct StepTransition {
    pub status: &'static str,
    pub cost_usd: f64,
    pub tokens: Option<i64>,
    pub wall_clock_secs: u64,
    pub artifact_path: Option<String>,
    pub error_message: Option<Option<String>>,
    pub cache: CacheTokens,
}

impl StepTransition {
    /// The step is starting. Carries the row's existing spend forward so
    /// a re-dispatch doesn't reset the totals a retry has accumulated.
    pub(crate) fn running(cost_usd: f64, tokens: Option<i64>, wall_clock_secs: u64) -> Self {
        Self {
            status: "running",
            cost_usd,
            tokens,
            wall_clock_secs,
            artifact_path: None,
            error_message: Some(None),
            cache: CacheTokens::default(),
        }
    }

    /// The step succeeded. `artifact_path` is `None` when the caller has
    /// nothing new to say about the column, which leaves whatever the
    /// step itself already wrote there intact.
    pub(crate) fn completed(
        cost_usd: f64,
        tokens: i64,
        wall_clock_secs: u64,
        artifact_path: Option<String>,
        cache: CacheTokens,
    ) -> Self {
        Self {
            status: "completed",
            cost_usd,
            tokens: Some(tokens),
            wall_clock_secs,
            artifact_path,
            error_message: Some(None),
            cache,
        }
    }

    /// The step failed terminally, or is failing on its way to a
    /// redirect. The message is the reason a user reads.
    pub(crate) fn failed(
        cost_usd: f64,
        tokens: Option<i64>,
        wall_clock_secs: u64,
        error_message: String,
        cache: CacheTokens,
    ) -> Self {
        Self {
            status: "failed",
            cost_usd,
            tokens,
            wall_clock_secs,
            artifact_path: None,
            error_message: Some(Some(error_message)),
            cache,
        }
    }

    /// A cancel arrived while the step was already failing. Distinct from
    /// `failed` so the run reads as stopped by the user, not by the code.
    pub(crate) fn interrupted(
        cost_usd: f64,
        tokens: i64,
        wall_clock_secs: u64,
        error_message: String,
        cache: CacheTokens,
    ) -> Self {
        Self {
            status: "interrupted",
            cost_usd,
            tokens: Some(tokens),
            wall_clock_secs,
            artifact_path: None,
            error_message: Some(Some(error_message)),
            cache,
        }
    }

    /// The step is parked back at the start line for an in-place retry —
    /// the run loop re-dispatches whatever it finds `pending`.
    pub(crate) fn pending(
        cost_usd: f64,
        tokens: i64,
        wall_clock_secs: u64,
        error_message: String,
        cache: CacheTokens,
    ) -> Self {
        Self {
            status: "pending",
            cost_usd,
            tokens: Some(tokens),
            wall_clock_secs,
            artifact_path: None,
            error_message: Some(Some(error_message)),
            cache,
        }
    }

    /// The scheduler decided this node will not run. The reason rides in
    /// `error_message` because that is what the dim-with-tooltip
    /// rendering reads (PRD §6.1).
    ///
    /// `status` is the scheduler's own vocabulary constant
    /// (`run_loop::schedule::STATUS_SKIPPED`) rather than a literal here:
    /// the read back out of the row (`schedule::node_state_for`) matches
    /// on that same const, and two copies of the word is how they drift.
    pub(crate) fn skipped(
        status: &'static str,
        cost_usd: f64,
        tokens: Option<i64>,
        wall_clock_secs: u64,
        error_message: String,
    ) -> Self {
        Self {
            status,
            cost_usd,
            tokens,
            wall_clock_secs,
            artifact_path: None,
            error_message: Some(Some(error_message)),
            cache: CacheTokens::default(),
        }
    }
}

impl crate::adapters::step_executor::driver::ExecutionDriver {
    /// The driver's pair of status ports, borrowed for one transition.
    pub(crate) fn status_writers(&self) -> StatusWriters<'_> {
        StatusWriters {
            features: self.features.as_ref(),
            notif: self.notif.as_ref(),
            f_id: &self.f_id,
        }
    }

    /// The cache telemetry the last turn reported, as the pair every
    /// transition carries.
    pub(crate) fn cache_tokens(&self) -> CacheTokens {
        CacheTokens {
            read: self.last_cache_read,
            creation: self.last_cache_creation,
        }
    }
}

/// Set a step execution to a final status (completed / failed / interrupted / awaiting_gate)
/// and emit the corresponding notification. Always sets cost_usd, tokens, wall_clock_secs
/// and the cache-token telemetry to the caller-provided values.
pub(crate) fn update_step_status(
    writers: StatusWriters<'_>,
    step_exec: &StepExecution,
    transition: StepTransition,
) {
    let _ = try_update_step_status(writers, step_exec, transition);
}

/// [`update_step_status`], but surfacing whether the **durable write** landed.
///
/// Almost every caller is reporting a transition it has already committed to
/// and has nothing useful to do about a repository error, which is why the
/// infallible form above exists. The exception is a caller whose *control
/// flow* depends on the write being visible to the next read — the scheduler's
/// skip persistence, which re-derives its state from these rows and would
/// otherwise re-decide the same skip forever.
///
/// The event is still emitted only after a successful write (the P1.13
/// ordering: the row is the truth, the event is its push).
pub(crate) fn try_update_step_status(
    writers: StatusWriters<'_>,
    step_exec: &StepExecution,
    transition: StepTransition,
) -> Result<(), String> {
    let StepTransition {
        status,
        cost_usd,
        tokens,
        wall_clock_secs,
        artifact_path,
        error_message,
        cache,
    } = transition;
    writers.features.step_update(
        &step_exec.id,
        &StepExecutionPatch {
            last_failure_fingerprint: None,
            iteration_count: None,
            status: Some(status.to_string()),
            cost_usd: Some(Some(cost_usd)),
            tokens: Some(tokens),
            wall_clock_secs: Some(Some(wall_clock_secs)),
            artifact_path: artifact_path.map(Some),
            artifact_paths: None,
            error_message,
            cache_read_input_tokens: Some(cache.read),
            cache_creation_input_tokens: Some(cache.creation),
        },
    )?;
    let _ = writers.notif.emit(&DomainEvent::StepProgress {
        feature_id: writers.f_id.clone(),
        step_id: step_exec.step_id.0.clone(),
        status: status.into(),
        cost_usd: Some(cost_usd),
        tokens,
        wall_clock_secs: Some(wall_clock_secs),
        cache_read_input_tokens: cache.read,
        cache_creation_input_tokens: cache.creation,
    });
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/step_executor/step_status.rs"]
mod step_status_tests;

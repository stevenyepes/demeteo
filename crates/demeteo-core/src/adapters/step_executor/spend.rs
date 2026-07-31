//! What a step has cost by the time one of its write paths reports it.

use std::time::Instant;

use super::step_status::CacheTokens;

/// What a step has consumed as of one moment, and when it started.
///
/// These travel together through every terminal write path — fail,
/// redirect, exhaust, park back at pending — because the row and the
/// event each report all of them at once. The dollars and the tokens are
/// not the *step's*: the driver accumulates them across the whole
/// feature, and a step's own share is only knowable as a difference. A
/// snapshot rather than a borrow, because these sites read the totals and
/// never advance them.
///
/// A near-identical bundle exists at `steps/sequence/context.rs` with
/// `&mut` fields, for the stages that *do* advance the totals. The
/// duplication is deliberate: a driver reaching into a step's context
/// types would be a worse coupling than a few repeated fields.
#[derive(Clone, Copy)]
pub(crate) struct StepSpend {
    pub cost: f64,
    pub tokens: i64,
    /// Prompt-cache telemetry for the turn that got the step here. Rides
    /// with the totals because the transition reports it in the same
    /// breath — and because a caller holding one and not the other is how
    /// the live cache chip goes blank mid-run.
    pub cache: CacheTokens,
    /// When the driver started this step — the wall clock every
    /// terminal row reports beside the totals above.
    pub start: Instant,
}

impl StepSpend {
    /// Whole seconds since the step started, as `wall_clock_secs` records
    /// them.
    pub(crate) fn wall_secs(&self) -> u64 {
        self.start.elapsed().as_secs()
    }
}

/// The feature-wide totals a stage advances, borrowed for the length of the
/// one call that spends against them.
///
/// The borrowed peer of [`StepSpend`]: a stage that *runs* an agent has to
/// fold what the turn cost back into the driver's running totals, so it needs
/// the `&mut` the snapshot deliberately does not give. `start` rides along
/// because every site that reports the totals reports wall-clock beside them.
///
/// This lives here rather than in either caller's module because both callers
/// are reached from `steps/sequence`, and a step importing another step's
/// context types is the coupling `steps/sequence/context.rs` argues against.
pub(crate) struct RunningSpend<'a> {
    pub cost: &'a mut f64,
    pub tokens: &'a mut i64,
    pub start: Instant,
}

/// The running totals one step's dispatch advances, owned by the run loop
/// for the length of one iteration.
///
/// The mutable peer of [`StepSpend`]: the handler writes into these
/// through [`NodeCtx`](super::registry::NodeCtx), and every terminal write
/// path afterwards reads the snapshot. They were four `&mut` locals passed
/// side by side, which is the same bundle without the name.
pub(crate) struct StepTotals {
    pub cost: f64,
    pub tokens: i64,
    /// Out-slot: what the last turn reported about the prompt cache. The
    /// live cache chip renders it, and the driver keeps the last value for
    /// the terminal transition.
    pub cache: CacheTokens,
}

impl StepTotals {
    /// The totals as of now, for a write path that reports them.
    pub(crate) fn snapshot(&self, start: Instant) -> StepSpend {
        StepSpend {
            cost: self.cost,
            tokens: self.tokens,
            cache: self.cache,
            start,
        }
    }
}

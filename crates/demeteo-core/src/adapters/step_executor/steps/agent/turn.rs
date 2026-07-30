//! One agent turn of an agent step, and what the step owes the run for it.
//!
//! An agent step streams two turns against the same session: the coding turn
//! that does the work, and — only when the first produced no readable
//! verdict — a strict-JSON correction. They pass the same nine arguments to
//! `stream_agent_turn` and differ in exactly two ways: whether the caller
//! wants per-delta progress events, and how the result is folded.
//!
//! Only the fold is testable here. The streaming half reaches `notif`,
//! `pricing`, `app_settings` and the cancel watch — four of the driver's
//! eighteen ports — so it stays a method on `ExecutionDriver` and stays
//! covered by `tests/e2e/step_executor.rs` alone. Splitting the fold out is
//! what makes the part with a decision in it reachable without a fixture.

use std::sync::Arc;

use crate::adapters::agent::event_stream::{stream_agent_turn, TurnResult};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::agent_event::AgentEvent;
use crate::domain::artifact::Artifact;
use crate::ports::agent_runtime::AgentSession;
use crate::ports::notification::DomainEvent;

use super::context::{AgentRunTarget, AgentSpend, AgentStepCtx, AgentWorktree};

/// What the step should do about the turn that just ended.
pub(crate) enum TurnDisposition {
    /// The agent answered: its reply text, and the artifacts it announced.
    Answered {
        text: String,
        produced: Vec<Artifact>,
    },
    /// Stop was pressed while the turn was streaming.
    Cancelled,
    /// The turn did not complete. Carries the outcome the step returns —
    /// `Failed` for the agent's own failure, `Environmental` for its box.
    Broken(StepOutcome),
}

/// Fold one completed turn into the feature's running totals.
///
/// Only a `Success` spends anything, so only a `Success` writes: an
/// interrupted or broken turn leaves both cache slots holding whatever the
/// previous turn of this step put there, which is what the UI's cache chip
/// should keep showing.
pub(crate) fn apply_turn_result(res: TurnResult, spend: &mut AgentSpend<'_>) -> TurnDisposition {
    match res {
        TurnResult::Interrupted => TurnDisposition::Cancelled,
        TurnResult::Failed(descriptive) => {
            TurnDisposition::Broken(StepOutcome::Failed(descriptive))
        }
        TurnResult::Environmental(descriptive) => {
            TurnDisposition::Broken(StepOutcome::Environmental(descriptive))
        }
        TurnResult::Success(outcome) => {
            *spend.cost += outcome.cost_usd;
            *spend.tokens += outcome.tokens;
            // Surface cache telemetry from the just-completed
            // turn on the out-params. The driver loop reads
            // these for the final `StepProgress` notification
            // + DB row update so the UI's "Saved $X by cache"
            // chip has fresh numbers.
            *spend.cache_read = Some(outcome.cache_read_input_tokens);
            *spend.cache_creation = Some(outcome.cache_creation_input_tokens);
            TurnDisposition::Answered {
                text: outcome.text,
                produced: outcome.produced_artifacts,
            }
        }
    }
}

impl ExecutionDriver {
    /// The step's coding turn: stream it, and relay every text delta to the
    /// UI as it arrives.
    pub(crate) async fn run_agent_turn(
        &self,
        session: &Arc<dyn AgentSession>,
        prompt: &str,
        ctx: AgentStepCtx<'_>,
        target: AgentRunTarget<'_>,
        wt: AgentWorktree<'_>,
        spend: &AgentSpend<'_>,
    ) -> TurnResult {
        let step_exec = ctx.step_exec;
        let timeouts = crate::application::timeouts::resolve_effective(self.app_settings.as_ref());

        stream_agent_turn(
            &**session,
            prompt,
            timeouts,
            Some(self.cancel_watch.clone()),
            wt.machine,
            &*self.exec,
            target.override_model.map(str::to_string),
            self.pricing.clone(),
            |event| {
                if let AgentEvent::Text { delta } = event {
                    let _ = self.notif.emit(&DomainEvent::AgentStream {
                        feature_id: self.f_id.clone(),
                        step_execution_id: step_exec.id.clone(),
                        content: delta.clone(),
                    });
                    let _ = self.notif.emit(&DomainEvent::StepProgress {
                        feature_id: self.f_id.clone(),
                        step_id: step_exec.step_id.0.clone(),
                        status: "running".into(),
                        cost_usd: Some(*spend.cost),
                        tokens: Some(*spend.tokens),
                        wall_clock_secs: Some(spend.start.elapsed().as_secs()),
                        cache_read_input_tokens: None,
                        cache_creation_input_tokens: None,
                    });
                }
            },
        )
        .await
    }

    /// A resumed turn against the same session with nothing relayed to the
    /// UI. The verdict correction is not the step's work and has no deltas
    /// worth streaming — one cheap resumed turn instead of a whole fresh
    /// verifier session.
    pub(crate) async fn run_silent_turn(
        &self,
        session: &Arc<dyn AgentSession>,
        prompt: &str,
        target: AgentRunTarget<'_>,
        wt: AgentWorktree<'_>,
    ) -> TurnResult {
        let timeouts = crate::application::timeouts::resolve_effective(self.app_settings.as_ref());

        stream_agent_turn(
            &**session,
            prompt,
            timeouts,
            Some(self.cancel_watch.clone()),
            wt.machine,
            &*self.exec,
            target.override_model.map(str::to_string),
            self.pricing.clone(),
            |_event| {},
        )
        .await
    }
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/agent/turn.rs"]
mod tests;

//! How an agent step ends, once there is nothing left to run.
//!
//! Three things can be true at the end of an agent step and they are checked
//! in a fixed order — Stop beats a failed turn, a failed turn beats the merge
//! — because each later check reports a *state of the branch* that an earlier
//! one has already made meaningless.
//!
//! Only the "what should this be called" half is a decision, and it lives in
//! [`crate::domain::artifact_capture`]. The rest is the row write and the
//! event that announces it, in that order.

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::artifact_capture::{missing_deliverables_message, MissingArtifact};
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

use super::context::{AgentSpend, AgentStepCtx};

/// Everything the stages before the end left for the end to judge.
pub(crate) struct StepClose<'a> {
    /// Stop was seen, either mid-turn or during the conflict pass.
    pub cancelled: bool,
    /// The agent's own failure, or its box's, if one happened.
    pub failed: Option<StepOutcome>,
    /// Whether the subtask branch reached the feature branch.
    pub merge: Result<(), String>,
    /// The step's primary deliverable reference, and all of them.
    pub artifact_path: Option<String>,
    pub artifact_paths: Vec<String>,
    /// Declared deliverables the turn never produced.
    pub missing: &'a [MissingArtifact],
    /// The agent's final reply, for the memory signal.
    pub text: &'a str,
}

impl ExecutionDriver {
    /// Decide the step's outcome and persist it.
    pub(crate) fn settle_agent_step(
        &self,
        ctx: AgentStepCtx<'_>,
        spend: &AgentSpend<'_>,
        close: StepClose<'_>,
    ) -> StepOutcome {
        let step_exec = ctx.step_exec;

        if close.cancelled || *self.cancel_watch.borrow() {
            let wall = spend.start.elapsed().as_secs();
            let _ = self.features.step_update(
                &step_exec.id,
                &StepExecutionPatch {
                    last_failure_fingerprint: None,
                    iteration_count: None,
                    status: Some("interrupted".to_string()),
                    cost_usd: Some(Some(*spend.cost)),
                    tokens: Some(Some(*spend.tokens)),
                    wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                    artifact_path: None,
                    artifact_paths: None,
                    error_message: Some(Some("Execution cancelled by user".to_string())),
                    cache_read_input_tokens: Some(*spend.cache_read),
                    cache_creation_input_tokens: Some(*spend.cache_creation),
                },
            );
            let _ = self.notif.emit(&DomainEvent::StepProgress {
                feature_id: self.f_id.clone(),
                step_id: step_exec.step_id.0.clone(),
                status: "interrupted".into(),
                cost_usd: Some(*spend.cost),
                tokens: Some(*spend.tokens),
                wall_clock_secs: Some(wall),
                cache_read_input_tokens: *spend.cache_read,
                cache_creation_input_tokens: *spend.cache_creation,
            });
            return StepOutcome::Cancelled;
        }

        if let Some(failed_outcome) = close.failed {
            return failed_outcome;
        }

        match close.merge {
            Ok(()) if !close.missing.is_empty() => {
                StepOutcome::Failed(missing_deliverables_message(close.missing))
            }
            Ok(()) => {
                let wall = spend.start.elapsed().as_secs();
                let _ = self.features.step_update(
                    &step_exec.id,
                    &StepExecutionPatch {
                        last_failure_fingerprint: None,
                        iteration_count: None,
                        status: Some("completed".to_string()),
                        cost_usd: Some(Some(*spend.cost)),
                        tokens: Some(Some(*spend.tokens)),
                        wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                        artifact_path: Some(close.artifact_path),
                        artifact_paths: Some(close.artifact_paths),
                        error_message: Some(None),
                        cache_read_input_tokens: Some(*spend.cache_read),
                        cache_creation_input_tokens: Some(*spend.cache_creation),
                    },
                );
                let _ = self.notif.emit(&DomainEvent::StepProgress {
                    feature_id: self.f_id.clone(),
                    step_id: step_exec.step_id.0.clone(),
                    status: "completed".into(),
                    cost_usd: Some(*spend.cost),
                    tokens: Some(*spend.tokens),
                    wall_clock_secs: Some(wall),
                    cache_read_input_tokens: *spend.cache_read,
                    cache_creation_input_tokens: *spend.cache_creation,
                });
                // Capture the agent's final summary as a signal for the
                // memory worker. Cap length to keep the queue lightweight.
                let summary = close.text.trim();
                if !summary.is_empty() {
                    let capped: String = summary.chars().take(4000).collect();
                    self.capture_signal(
                        Some(step_exec.id.0.clone()),
                        crate::domain::memory::SignalKind::AgentSummary,
                        format!(
                            "Step '{}' completed. Agent summary:\n{}",
                            step_exec.step_id.0, capped
                        ),
                    );
                }
                StepOutcome::Completed
            }
            Err(err) => StepOutcome::Failed(format!("agent step merge failed: {}", err)),
        }
    }
}

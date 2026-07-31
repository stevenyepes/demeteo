use async_trait::async_trait;

use crate::domain::ids::{FeatureId, GateDecisionId, StepExecutionId};
use crate::domain::models::GateDecision;
use crate::domain::run_control::{shadow_refusal, RunAction};
use crate::error::AppError;
use crate::paths;
use crate::ports::notification::DomainEvent;
use crate::ports::step_executor::GatePresenter;

use super::super::DagStepExecutor;
use super::lock_registry;

#[async_trait]
impl GatePresenter for DagStepExecutor {
    async fn gate_pending_for_run(&self, feature_id: &str) -> Result<Option<GateDecision>, String> {
        self.gates
            .pending_for_feature(&FeatureId::from(feature_id.to_string()))
    }

    async fn gate_decide(
        &self,
        step_execution_id: &str,
        decision: &str,
        feedback: Option<&str>,
    ) -> Result<(), AppError> {
        let se_id = StepExecutionId::from(step_execution_id.to_string());

        // Pre-flight guard: refuse to apply a gate decision while an
        // earlier step is still running. The UI also disables the
        // Approve / Redirect buttons in this case, but the backend
        // must enforce the rule because a stale `gate_required` event
        // can race the agent's final artifact write and surface a
        // decidable gate while a predecessor is still in flight.
        let step_exec = self
            .features
            .step_get(&se_id)
            .map_err(AppError::from)?
            .ok_or_else(|| {
                AppError::not_found(format!("Step execution not found: {}", step_execution_id))
            })?;
        self.assert_no_active_predecessors(&step_exec, "deciding this gate")?;

        // Refuse a shadow outright. This machine holds a read-only mirror
        // of a run a `demeteo-runner` owns: the decision belongs in *its*
        // DB, reached over the tunnel (`remote_decide_gate`). Writing it
        // here would upsert a row nothing reads and return `Ok` — the
        // driver spawn below is the only thing that would notice, and it
        // only logs its refusal, so the user saw an approval that silently
        // did nothing.
        if self
            .runner_owned_features()
            .contains(step_exec.feature_id.as_str())
        {
            return Err(AppError::validation(shadow_refusal(
                RunAction::DecideGate,
                &step_exec.feature_id.0,
            )));
        }

        // 1. Durable: write the decision to the DB. UPSERT so the call
        //    is idempotent whether or not a row already exists. This is
        //    the source of truth — everything below is a wakeup hint.
        self.gates
            .upsert_decision(&se_id, decision, feedback, paths::now_ms())
            .map_err(AppError::from)?;

        // Narrate the answer (P1.13): `gate_required` marked the wait,
        // this marks the human's decision, so the run-event log tells
        // both halves of the story. Emitted after the durable write —
        // the event never precedes the state it describes.
        let _ = self.notif.emit(&DomainEvent::GateDecided {
            feature_id: step_exec.feature_id.clone(),
            step_execution_id: se_id.clone(),
            decision: decision.to_string(),
            feedback: feedback.map(|s| s.to_string()),
        });

        let gd = GateDecision {
            id: GateDecisionId::from(format!("gd-{}", step_execution_id)),
            step_execution_id: se_id.clone(),
            decision: Some(decision.to_string()),
            feedback: feedback.map(|s| s.to_string()),
            created_at: paths::now_ms(),
        };

        // 2. Fast path: if the driver is alive and waiting on this
        //    step's waiter, deliver the decision in-memory. Missing
        //    waiter is *not* an error — the DB row will be picked up
        //    when the driver reconciles on its next startup.
        if let Some(waiter) = lock_registry(&self.gate_waiters)
            .get(step_execution_id)
            .cloned()
        {
            waiter.deliver(gd);
        }

        // 3. Self-healing: if the driver is dead (app restart, race,
        //    manual interruption), try to spawn one. The new driver
        //    will reconcile the decided gate on its first loop
        //    iteration. Best-effort: the decision is already durable
        //    in the DB, so a spawn failure (missing project, path
        //    probe failure, etc.) is logged but does NOT roll back
        //    the decision — the next legitimate operation will retry.
        if let Err(e) = self.ensure_driver_running(&step_exec.feature_id.0).await {
            eprintln!(
                "gate_decide: failed to ensure driver running for {}: {} \
                 (decision is durable; will retry on next operation)",
                step_exec.feature_id.0, e
            );
        }

        Ok(())
    }
}

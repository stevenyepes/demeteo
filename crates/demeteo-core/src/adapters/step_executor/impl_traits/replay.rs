use super::super::DagStepExecutor;
use crate::domain::ids::StepExecutionId;
use crate::domain::models::{StepConfig, StepExecution};
use crate::ports::db::{FeaturePatch, StepExecutionPatch};
use crate::ports::notification::DomainEvent;
use crate::ports::step_executor::StepExecutor;

/// The patch that rewinds one in-range step to `pending`.
///
/// A free function rather than an inline literal because it is the only
/// *decision* in the rewind — everything around it is I/O — and pinning it
/// needs neither a `DagStepExecutor` nor the twenty-odd ports one carries.
///
/// Spend (`cost_usd`, `tokens`, `wall_clock_secs`) is written back
/// unchanged: the run really did spend it, and a rewind is not a refund.
/// `iteration_count` is the opposite case and the reason this exists.
/// It is the redirect budget [`crate::adapters::step_executor::retry_policy::evaluate`]
/// reads, and it must reset: both callers are a human asking for another
/// go, and a node that carried its spent budget across the rewind would get
/// exactly one attempt before the policy answers `Exhausted`, never
/// reaching its `on_failure` target. That is a Retry button that visibly
/// re-runs the step and then fails it the same way, every time.
fn rewind_patch(step: &StepExecution) -> StepExecutionPatch {
    StepExecutionPatch {
        last_failure_fingerprint: None,
        iteration_count: Some(0),
        status: Some("pending".to_string()),
        cost_usd: step.cost_usd.map(Some),
        tokens: step.tokens.map(Some),
        wall_clock_secs: step.wall_clock_secs.map(Some),
        artifact_path: None,
        artifact_paths: Some(Vec::new()),
        error_message: Some(None),
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
    }
}

/// The patch that puts one step back as [`rewind_patch`] found it, when
/// arming the driver failed and the whole rewind has to be undone.
///
/// Restores the budget rather than leaving it zeroed: an arm that never
/// happened must not hand the run a set of retries it was never granted.
fn unwind_patch(original_status: &str, original_iterations: u32) -> StepExecutionPatch {
    StepExecutionPatch {
        last_failure_fingerprint: None,
        iteration_count: Some(original_iterations),
        status: Some(original_status.to_string()),
        cost_usd: None,
        tokens: None,
        wall_clock_secs: None,
        artifact_path: None,
        artifact_paths: None,
        error_message: None,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/replay_patch.rs"]
mod replay_patch_tests;

impl DagStepExecutor {
    /// Rewind a step (and its graph descendants) to `pending` and re-arm the
    /// driver.
    ///
    /// `clear_sequence_checkpoints` decides what happens to a sequence
    /// node's landed prefix in that range, and the two callers want opposite
    /// things. A **retry** is "carry on from where this broke" — the prefix
    /// is the point, and re-running twenty paid tasks to reach the one that
    /// failed is the behaviour checkpointing exists to prevent. A **replay**
    /// is an explicit redo, so it drops the prefix and the step runs its
    /// whole list again.
    ///
    /// Only the durable row is dropped, not the git ref pinning the prefix's
    /// commits: deleting that would need a resolved execution context (repo
    /// path + machine) purely to tidy up. The row is the authority — with it
    /// gone the resume reads "no checkpoint" and re-runs everything — and
    /// the ref name is deterministic, so the step's own completion path
    /// deletes it next time round.
    ///
    /// Every rewound node's `iteration_count` — the redirect budget the
    /// retry policy reads (`retry_policy::evaluate`) — resets to zero. Both
    /// callers are a human asking for another go, and a node that carried
    /// its spent budget across the rewind would get exactly one attempt and
    /// then exhaust, never reaching its `on_failure` target.
    pub(crate) async fn replay_steps_from(
        &self,
        execution_id: &str,
        new_model: Option<&str>,
        new_agent: Option<&str>,
        new_effort: Option<crate::domain::models::EffortLevel>,
        include_target: bool,
        clear_sequence_checkpoints: bool,
    ) -> Result<(), String> {
        let se_id = StepExecutionId::from(execution_id.to_string());
        let step_exec = self
            .features
            .step_get(&se_id)?
            .ok_or_else(|| format!("Step execution not found: {}", execution_id))?;

        let feature_id = &step_exec.feature_id;

        // Never replay a runner-owned shadow. This is the shared primitive
        // behind both `step_retry` and `replay_from_step`, and it calls
        // `start_execution_loop` directly rather than going through
        // `ensure_driver_running` — so its shadow guard never fires here.
        // Without this, replaying a detached run's step would rewind the
        // mirrored rows and arm a *second* driver, on this machine, against
        // a run the runner is still driving, in a worktree that only exists
        // on the runner's box. `step_retry` repeats this check to return a
        // typed validation error; this one is the backstop for every caller.
        if self.runner_owned_features().contains(feature_id.as_str()) {
            return Err(format!(
                "Feature '{}' is a read-only shadow of a run owned by a demeteo-runner; \
                 replay it on the runner, not here",
                feature_id.0
            ));
        }

        let feature = self
            .features
            .get(feature_id)?
            .ok_or_else(|| format!("Feature not found: {}", feature_id))?;

        // Cancel any in-flight execution and force-kill the old session
        if feature.status == "running" {
            self.feature_cancel(feature_id.as_str()).await?;
            let reg = self.registry.clone();
            let fid = feature_id.to_string();
            reg.kill(&fid).await;
            // Yield to let the old driver's cancel handler finish
            // writing its terminal state before we overwrite it.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        // Re-pin the feature-wide harness/model overrides (resolution tier 2)
        // before restarting the loop, so the replayed steps run with the
        // operator's chosen agent and model.
        //
        // Model patch rules:
        //   - explicit `new_model`            → set it.
        //   - harness changed, no model given → clear any existing model
        //     override, so the new harness resolves its own default model
        //     rather than inheriting a stale model that may not exist for it.
        //   - nothing given                   → leave the override untouched.
        //
        // Effort is re-pinned only when explicitly given. Unlike the model it
        // is harness-agnostic — the canonical ladder is clamped per agent in
        // the adapter — so switching harness is no reason to drop it.
        let agent_patch = new_agent.map(|a| Some(a.to_string()));
        let model_patch = match (new_agent, new_model) {
            (_, Some(m)) => Some(Some(m.to_string())),
            (Some(_), None) => Some(None),
            (None, None) => None,
        };
        let effort_patch = new_effort.map(Some);
        if agent_patch.is_some() || model_patch.is_some() || effort_patch.is_some() {
            self.features.update(
                feature_id,
                &FeaturePatch {
                    agent_kind: agent_patch,
                    model: model_patch,
                    effort: effort_patch,
                    ..Default::default()
                },
            )?;
        }

        let mut workflow_id = feature.workflow_id.clone();
        if workflow_id.is_none() {
            let step_execs = self.features.steps_for_feature(feature_id)?;
            let step_ids: Vec<String> = step_execs.iter().map(|s| s.step_id.0.clone()).collect();

            let workflows = self.workflows.list()?;
            for w in workflows {
                if let Some(version) = self.workflows.latest_version(&w.id)? {
                    if let Ok(steps) = serde_json::from_str::<Vec<StepConfig>>(&version.steps_json)
                    {
                        let w_step_ids: Vec<String> =
                            steps.iter().map(|s| s.id.0.clone()).collect();
                        if w_step_ids == step_ids {
                            self.features.update_workflow_id(feature_id, &w.id)?;
                            workflow_id = Some(w.id);
                            break;
                        }
                    }
                }
            }
        }

        let workflow_id = workflow_id.ok_or_else(|| {
            format!(
                "Workflow ID not found for feature {}. \
                 This legacy feature does not match any current workflow steps.",
                feature_id
            )
        })?;

        // The rewind set: the target plus its graph *descendants* (P1.12)
        // — for a v1 chain exactly the old `step_index >=` tail, and for
        // a DAG only the downstream cone, leaving independent branches'
        // results intact. Graph resolution misses (legacy feature, no
        // matching workflow) fall back to the index comparison.
        let reset_ids: Option<std::collections::HashSet<crate::domain::ids::StepId>> =
            self.resolve_feature_graph(feature_id).and_then(|graph| {
                graph.descendants(&step_exec.step_id).map(|d| {
                    let mut set: std::collections::HashSet<crate::domain::ids::StepId> =
                        d.into_iter().cloned().collect();
                    if include_target {
                        set.insert(step_exec.step_id.clone());
                    }
                    set
                })
            });

        let all_steps = self.features.steps_for_feature(feature_id)?;
        let mut patch_list: Vec<(StepExecutionId, (String, u32))> = Vec::new();
        for s in &all_steps {
            let is_in_range = match &reset_ids {
                Some(set) => set.contains(&s.step_id),
                None if include_target => s.step_index >= step_exec.step_index,
                None => s.step_index > step_exec.step_index,
            };

            if is_in_range {
                patch_list.push((s.id.clone(), (s.status.clone(), s.iteration_count)));
                self.features.step_update(&s.id, &rewind_patch(s))?;
                // Mirror the DB reset with a `StepProgress` event so
                // the frontend's local `steps` array reflects the
                // rewind without waiting for a full
                // `step_list_for_run` poll. Without this, the
                // timeline keeps rendering the "Retry Step" /
                // "Decide Gate" affordance for rows whose DB state
                // has already moved on (the UI staleness bug this
                // event exists to break). For gate rows, the
                // pending re-prompt will re-emit `awaiting_gate`
                // when the gate is re-entered by the driver.
                let _ = self.notif.emit(&DomainEvent::StepProgress {
                    feature_id: feature_id.clone(),
                    step_id: s.step_id.0.clone(),
                    status: "pending".into(),
                    cost_usd: s.cost_usd,
                    tokens: s.tokens,
                    wall_clock_secs: s.wall_clock_secs,
                    cache_read_input_tokens: None,
                    cache_creation_input_tokens: None,
                });
                if s.step_kind == "gate" {
                    let _ = self.gates.reset_for_step_execution(&s.id);
                }
                // A no-op for any node that never checkpointed, so this
                // needs no step-kind branch (and `sequence` answers to two
                // kind strings anyway).
                if clear_sequence_checkpoints {
                    let _ = self
                        .sequence_resume
                        .sequence_checkpoint_clear(feature_id, &s.step_id.0);
                }
            }
        }

        let prev_feature_status = feature.status.clone();
        self.features.update(
            feature_id,
            &FeaturePatch {
                status: Some("running".to_string()),
                total_cost: None,
                duration: None,
                ..Default::default()
            },
        )?;
        let _ = self.notif.emit(&DomainEvent::FeatureStatusChanged {
            feature_id: feature_id.clone(),
            status: "running".into(),
        });

        if let Err(e) = self
            .start_execution_loop(
                feature_id.as_str(),
                &feature.project_id.0,
                workflow_id.as_str(),
                &feature.title,
            )
            .await
        {
            for (sid, (original_status, original_iterations)) in &patch_list {
                let _ = self
                    .features
                    .step_update(sid, &unwind_patch(original_status, *original_iterations));
            }
            let _ = self.features.update(
                feature_id,
                &FeaturePatch {
                    status: Some(prev_feature_status.clone()),
                    total_cost: None,
                    duration: None,
                    ..Default::default()
                },
            );
            return Err(e);
        }

        Ok(())
    }
}

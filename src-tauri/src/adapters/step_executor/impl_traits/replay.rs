use super::super::DagStepExecutor;
use crate::domain::ids::StepExecutionId;
use crate::domain::models::StepConfig;
use crate::ports::db::{FeaturePatch, StepExecutionPatch};
use crate::ports::notification::DomainEvent;
use crate::ports::step_executor::StepExecutor;

impl DagStepExecutor {
    pub(crate) async fn replay_steps_from(
        &self,
        execution_id: &str,
        new_model: Option<&str>,
        include_target: bool,
    ) -> Result<(), String> {
        let se_id = StepExecutionId::from(execution_id.to_string());
        let step_exec = self
            .features
            .step_get(&se_id)?
            .ok_or_else(|| format!("Step execution not found: {}", execution_id))?;

        let feature_id = &step_exec.feature_id;
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

        if let Some(model) = new_model {
            self.features.update(
                feature_id,
                &FeaturePatch {
                    model: Some(Some(model.to_string())),
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

        let all_steps = self.features.steps_for_feature(feature_id)?;
        let mut patch_list: Vec<(StepExecutionId, String)> = Vec::new();
        for s in &all_steps {
            let is_in_range = if include_target {
                s.step_index >= step_exec.step_index
            } else {
                s.step_index > step_exec.step_index
            };

            if is_in_range {
                patch_list.push((s.id.clone(), s.status.clone()));
                self.features.step_update(
                    &s.id,
                    &StepExecutionPatch {
                        iteration_count: None,
                        status: Some("pending".to_string()),
                        cost_usd: s.cost_usd.map(Some),
                        tokens: s.tokens.map(Some),
                        wall_clock_secs: s.wall_clock_secs.map(Some),
                        artifact_path: None,
                        artifact_paths: Some(Vec::new()),
                        error_message: Some(None),
                    },
                )?;
                if s.step_kind == "gate" {
                    let _ = self.gates.reset_for_step_execution(&s.id);
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
            for (sid, original_status) in &patch_list {
                let _ = self.features.step_update(
                    sid,
                    &StepExecutionPatch {
                        iteration_count: None,
                        status: Some(original_status.clone()),
                        cost_usd: None,
                        tokens: None,
                        wall_clock_secs: None,
                        artifact_path: None,
                        artifact_paths: None,
                        error_message: None,
                    },
                );
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

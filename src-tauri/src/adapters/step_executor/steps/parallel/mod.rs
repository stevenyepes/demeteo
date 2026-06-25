use std::time::Instant;

use crate::adapters::step_executor::artifacts::{compute_git_diff, resolve_attached_artifacts};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::artifact::Artifact;
use crate::domain::models::{StepConfig, StepExecution};
use crate::paths;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

pub(crate) mod list_unmerged;
pub(crate) mod planner;
pub(crate) mod subtask;

use planner::{extract_subtask_dag, SubtaskDag};

impl ExecutionDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn handle_parallel_step(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
        step_index: usize,
        step_execs: &[StepExecution],
    ) -> StepOutcome {
        if *self.cancel_watch.borrow() {
            return StepOutcome::Cancelled;
        }

        let (planner_kind, override_model) = self.resolve_step_agent(step_conf);

        let machine_str = self
            .machine_id_opt
            .clone()
            .unwrap_or_else(|| "local".to_string());

        let base_sha = match self
            .exec
            .run_command(
                &machine_str,
                &format!(
                    "git -C {} rev-parse {}",
                    paths::shell_escape_posix(&self.target_dir),
                    paths::shell_escape_posix(&self.branch_name),
                ),
            )
            .await
        {
            Ok(s) => s.trim().to_string(),
            Err(_) => "HEAD".to_string(),
        };

        // 1. Planner pass: ask the planner agent for a subtask DAG.
        let dag = match self
            .run_planner_pass(
                step_exec,
                step_conf,
                accumulated_cost,
                accumulated_tokens,
                &planner_kind,
                &override_model,
                &machine_str,
                step_execs,
                step_index,
            )
            .await
        {
            Ok(d) => d,
            Err(e) => return StepOutcome::Failed(e),
        };

        let subtasks = dag.subtasks;
        eprintln!(
            "[parallel step] planner produced {} subtask(s)",
            subtasks.len()
        );

        // 2. Fan out: one worker per subtask.
        let mut subtask_artifacts = Vec::new();
        let mut all_artifact_refs = Vec::new();

        let is_legacy = step_conf.artifacts.as_ref().is_none_or(|d| d.is_empty());

        let subtasks_res = self
            .run_subtasks_loop(
                step_exec,
                step_conf,
                accumulated_cost,
                accumulated_tokens,
                step_start,
                step_index,
                step_execs,
                &subtasks,
                &machine_str,
                &base_sha,
                &planner_kind,
                &override_model,
                &mut all_artifact_refs,
                &mut subtask_artifacts,
            )
            .await;

        if let Err(step_err_msg) = subtasks_res {
            let is_cancelled = *self.cancel_watch.borrow();
            let status_str = if is_cancelled {
                "interrupted"
            } else {
                "failed"
            };
            let wall = step_start.elapsed().as_secs();
            let _ = self.features.step_update(
                &step_exec.id,
                &StepExecutionPatch {
                    iteration_count: None,
                    status: Some(status_str.to_string()),
                    cost_usd: Some(Some(*accumulated_cost)),
                    tokens: Some(Some(*accumulated_tokens)),
                    wall_clock_secs: Some(Some(wall)),
                    artifact_path: None,
                    artifact_paths: None,
                    error_message: Some(Some(step_err_msg.clone())),
                },
            );
            let _ = self.notif.emit(&DomainEvent::StepProgress {
                feature_id: self.f_id.clone(),
                step_id: step_exec.step_id.0.clone(),
                status: status_str.into(),
                cost_usd: Some(*accumulated_cost),
                tokens: Some(*accumulated_tokens),
                wall_clock_secs: Some(wall),
            });
            if is_cancelled {
                return StepOutcome::Cancelled;
            }
            return StepOutcome::Failed(step_err_msg);
        }

        // Run verifier check if configured on the parallel step
        if let Some(ref verifier_cfg) = step_conf.verifier {
            let (agent_kind, verifier_model) = self.resolve_step_agent(step_conf);
            let machine_str = self
                .machine_id_opt
                .clone()
                .unwrap_or_else(|| "local".to_string());

            if let Err(verdict_err) = self
                .run_verifier_logic(
                    step_exec,
                    verifier_cfg,
                    &self.target_dir,
                    &[],
                    accumulated_cost,
                    accumulated_tokens,
                    step_start,
                    &agent_kind,
                    &verifier_model,
                    &machine_str,
                )
                .await
            {
                let _ = self
                    .registry
                    .kill(&format!("{}-verifier", self.f_id.as_str()))
                    .await;
                return StepOutcome::Failed(verdict_err);
            }
        }

        // Write parallel step artifact summary
        let (artifact_path, artifact_paths) = if !is_legacy {
            let diff_ref = base_sha;
            let diff_body =
                compute_git_diff(&*self.exec, &machine_str, &self.target_dir, &diff_ref).await;
            let mut refs = Vec::new();
            if !diff_body.trim().is_empty() {
                let diff_artifact = Artifact {
                    name: "code-diff".into(),
                    mime: "text/x-diff".into(),
                    content: diff_body,
                    source: crate::domain::artifact::ArtifactSource::Diff {
                        base: diff_ref,
                        head: "WORKTREE".into(),
                        path_filter: None,
                    },
                };
                if let Ok(reference) =
                    self.artifacts
                        .put(&self.f_id_str, &step_exec.step_id.0, &diff_artifact)
                {
                    refs.push(reference);
                }
            }
            refs.extend(all_artifact_refs);
            let primary = refs.first().cloned();
            (primary, refs)
        } else {
            let mut art_path = self
                .app_local_data_dir
                .join("artifacts")
                .join(&self.f_id_str);
            let _ = std::fs::create_dir_all(&art_path);
            let file_name = format!("{}.md", step_exec.step_id.0);
            art_path.push(&file_name);
            let _ = std::fs::write(&art_path, subtask_artifacts.join("\n\n"));
            let art_path_str = art_path.to_string_lossy().to_string();
            (Some(art_path_str.clone()), vec![art_path_str])
        };

        let wall = step_start.elapsed().as_secs();
        let _ = self.features.step_update(
            &step_exec.id,
            &StepExecutionPatch {
                iteration_count: None,
                status: Some("completed".to_string()),
                cost_usd: Some(Some(*accumulated_cost)),
                tokens: Some(Some(*accumulated_tokens)),
                wall_clock_secs: Some(Some(wall)),
                artifact_path: Some(artifact_path),
                artifact_paths: Some(artifact_paths),
                error_message: Some(None),
            },
        );
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: "completed".into(),
            cost_usd: Some(*accumulated_cost),
            tokens: Some(*accumulated_tokens),
            wall_clock_secs: Some(wall),
        });
        StepOutcome::Completed
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_planner_pass(
        &self,
        _step_exec: &StepExecution,
        _step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        planner_kind: &str,
        override_model: &Option<String>,
        machine_str: &str,
        step_execs: &[StepExecution],
        step_index: usize,
    ) -> Result<SubtaskDag, String> {
        let planner_thread_id = format!("{}-planner", self.f_id_str);
        let feature_desc = self.base_ctx.get("feature_description").to_string();
        let repo_list = self.base_ctx.get("repo_list").to_string();
        let planner_prompt = format!(
            "You are a planning agent. Decompose the following feature into a small DAG of independent, parallelizable subtasks.\n\n\
             Feature: {feature_desc}\n\
             Repositories in scope: {repo_list}\n\n\
             Read any attached artifacts (e.g. the spec) for context. Then emit a single JSON object, in a ```json ... ``` fence, of the form:\n\
             {{\"subtasks\": [{{\"id\": \"sub-1\", \"title\": \"...\", \"description\": \"...\", \"files\": [\"src/foo.rs\"], \"test_command\": \"...\"}}]}}\n\n\
             Constraints:\n\
             - 2 to 5 subtasks. Aim for the smallest set that covers the work end-to-end.\n\
             - Subtask IDs must be kebab-case, unique, and stable.\n\
             - Each subtask's `files` list must be disjoint from the others — no shared ownership.\n\
             - If no decomposition makes sense (the work is small), return a single subtask with id `sub-1` that does the whole thing.\n",
        );
        let planner_prompt =
            resolve_attached_artifacts(&planner_prompt, step_execs, step_index, &*self.artifacts);

        let is_cli_agent = planner_kind == "opencode"
            || planner_kind == "hermes"
            || planner_kind == "claude-code"
            || planner_kind == "antigravity";

        let mut planner_env = crate::ports::agent_runtime::agent_base_env();
        if let Some(ref m) = override_model {
            // CLI agents take the model via a --model flag at spawn; only the
            // opencode-config-driven agents read OPENCODE_CONFIG_CONTENT.
            if !is_cli_agent {
                let config = format!(
                    r#"{{"$schema":"https://opencode.ai/config.json","model":"{}"}}"#,
                    m
                );
                planner_env.insert("OPENCODE_CONFIG_CONTENT".to_string(), config);
            }
        }

        // Resolve the actual executable name from the registered runtime
        // (e.g. kind "claude-code" → binary "claude"). Falls back to the
        // kind itself if no runtime is registered for it.
        let planner_binary = self
            .registry
            .runtime_for(planner_kind)
            .map(|r| r.binary().to_string())
            .unwrap_or_else(|| planner_kind.to_string());

        let planner_ctx = AgentContext {
            thread_id: planner_thread_id.clone(),
            machine_id: machine_str.to_string(),
            binary: planner_binary,
            args: vec![],
            env: planner_env,
            cwd: self.target_dir.clone(),
            model: override_model.clone(),
            title: Some("plan".to_string()),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
        };

        let mut cancel_watch = self.cancel_watch.clone();
        let spawn_res = tokio::select! {
            res = self.registry.get_or_spawn(&planner_thread_id, planner_kind, planner_ctx) => Some(res),
            _ = cancel_watch.changed() => None,
        };

        let planner_session = match spawn_res {
            Some(Ok(s)) => s,
            Some(Err(e)) => {
                return Err(format!("parallel step: planner spawn failed: {:?}", e));
            }
            None => {
                let _ = self.registry.kill(&planner_thread_id).await;
                return Err("parallel step: planner spawn cancelled".to_string());
            }
        };

        if let Some(ref model) = override_model {
            if !is_cli_agent {
                let _ = planner_session.set_config_option("model", model);
            }
        }

        const PLANNER_FAST_S: u64 = 180;
        const PLANNER_NORMAL_S: u64 = 300;
        const PLANNER_WALL_S: u64 = 900;

        let timeouts = crate::adapters::agent::event_stream::Timeouts {
            fast_timeout_s: PLANNER_FAST_S,
            normal_timeout_s: PLANNER_NORMAL_S,
            wall_cap_s: PLANNER_WALL_S,
        };

        // The planner's output is machine-consumed (parsed into a SubtaskDag),
        // but CLI agents stream free text and sometimes wrap or precede the JSON
        // with prose. Try once, and on a parse miss re-ask the *same* session
        // with a strict JSON-only correction prompt before giving up. The
        // session is kept alive across both turns so the retry has full context.
        const PLANNER_MAX_ATTEMPTS: usize = 2;
        let mut last_text = String::new();
        let mut parsed: Option<SubtaskDag> = None;

        for attempt in 0..PLANNER_MAX_ATTEMPTS {
            let prompt = if attempt == 0 {
                planner_prompt.clone()
            } else {
                "Your previous response could not be parsed as the required \
                 subtask DAG. Reply with ONLY a single JSON object — no prose, \
                 no markdown outside the fence — of the form:\n\
                 ```json\n\
                 {\"subtasks\": [{\"id\": \"sub-1\", \"title\": \"...\", \"description\": \"...\", \"files\": [\"src/foo.rs\"], \"test_command\": \"...\"}]}\n\
                 ```"
                    .to_string()
            };

            let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
                &*planner_session,
                &prompt,
                timeouts,
                Some(self.cancel_watch.clone()),
                machine_str,
                &*self.exec,
                |_event| {},
            )
            .await;

            last_text = match turn_res {
                crate::adapters::agent::event_stream::TurnResult::Interrupted => {
                    let _ = self.registry.kill(&planner_thread_id).await;
                    return Err("parallel step: planner cancelled".to_string());
                }
                crate::adapters::agent::event_stream::TurnResult::Failed(descriptive) => {
                    let _ = self.registry.kill(&planner_thread_id).await;
                    return Err(format!("parallel step: planner failed: {}", descriptive));
                }
                crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                    *accumulated_cost += outcome.cost_usd;
                    *accumulated_tokens += outcome.tokens;
                    outcome.text
                }
            };

            match extract_subtask_dag(&last_text) {
                Some(d) if !d.subtasks.is_empty() => {
                    parsed = Some(d);
                    break;
                }
                _ => {
                    eprintln!(
                        "[parallel step] planner attempt {}/{} produced no valid subtask DAG",
                        attempt + 1,
                        PLANNER_MAX_ATTEMPTS
                    );
                }
            }
        }

        let _ = self.registry.kill(&planner_thread_id).await;

        match parsed {
            Some(d) => Ok(d),
            None => Err(format!(
                "parallel step: planner did not return a valid subtask DAG after {} attempts. \
                 The agent's last response was: {}",
                PLANNER_MAX_ATTEMPTS,
                if last_text.len() > 500 {
                    format!("{}…(truncated)", &last_text[..500])
                } else {
                    last_text
                }
            )),
        }
    }
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/parallel.rs"]
mod tests;

use std::time::Instant;

use super::list_unmerged::list_unmerged_files;
use super::planner::PlannedSubtask;
use crate::adapters::step_executor::artifacts::{
    commit_worktree_changes, inject_artifact_contract, read_worktree_file,
    resolve_attached_artifacts, resolve_declared_artifacts, WorktreeSnapshot,
};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::agent_event::AgentEvent;
use crate::domain::artifact::Artifact;
use crate::domain::models::{StepConfig, StepExecution};
use crate::paths;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::notification::DomainEvent;

use crate::adapters::step_executor::steps::agent::{
    append_retry_feedback_section, format_retry_feedback_section, template_uses_retry_section,
};

impl ExecutionDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_subtasks_loop(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
        step_index: usize,
        step_execs: &[StepExecution],
        subtasks: &[PlannedSubtask],
        machine_str: &str,
        _base_sha: &str,
        planner_kind: &str,
        override_model: &Option<String>,
        all_artifact_refs: &mut Vec<String>,
        subtask_artifacts: &mut Vec<String>,
    ) -> Result<(), String> {
        let mut step_failed = false;
        let mut step_err_msg = String::new();
        let (retry_feedback, retry_iteration, retry_max) = match &self.retry_ctx {
            Some(rc) => (
                rc.feedback.clone(),
                rc.iteration.to_string(),
                rc.max.to_string(),
            ),
            None => (String::new(), String::new(), String::new()),
        };
        let is_legacy = step_conf.artifacts.as_ref().is_none_or(|d| d.is_empty());
        let decls: &[crate::domain::artifact::ArtifactDecl] =
            step_conf.artifacts.as_deref().unwrap_or(&[]);

        for (sub_idx, sub) in subtasks.iter().enumerate() {
            if *self.cancel_watch.borrow() {
                step_failed = true;
                step_err_msg = "Execution cancelled by user".to_string();
                break;
            }

            // Provision subtask worktree
            let wt_path = match self
                .git_ops
                .provision_subtask_worktree(
                    self.machine_id_opt.as_deref(),
                    &self.target_dir,
                    &self.branch_name,
                    &sub.id,
                )
                .await
            {
                Ok(p) => p,
                Err(e) => {
                    step_failed = true;
                    step_err_msg = format!(
                        "parallel subtask worktree provision failed ({}): {}",
                        sub.id, e
                    );
                    break;
                }
            };

            // Snapshot the subtask worktree's dirty state BEFORE the worker runs.
            let subtask_snapshot =
                WorktreeSnapshot::capture(&*self.exec, machine_str, &wt_path).await;

            // Apply artifact-scope chmod fence before the worker spawns.
            // For `AllWrites` capture (the standard `s-implement`
            // parallel step) this is a no-op. For constrained captures
            // it restricts the worker to the declared artifact paths
            // plus project-level extra writable paths (e.g. `target/`
            // for a partition that runs `cargo test`).
            let writable_paths = crate::adapters::worktree::git_ops::scope::derive_writable_paths(
                step_conf.artifacts.as_ref(),
                &self.extra_writable_paths,
            );
            if let Err(e) = self
                .git_ops
                .apply_artifact_scope(self.machine_id_opt.as_deref(), &wt_path, &writable_paths)
                .await
            {
                step_failed = true;
                step_err_msg = format!(
                    "parallel subtask {} artifact scope setup failed: {}",
                    sub.id, e
                );
                break;
            }

            let other_files: Vec<String> = subtasks
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != sub_idx)
                .flat_map(|(_, s)| s.files.clone())
                .collect();
            let other_files_str = other_files.join(", ");
            let sub_files_str = sub.files.join(", ");
            // Render the worker prompt template.
            // `retry_note` (per-subtask, from the planner's retry pass) takes
            // priority over the global `retry_feedback` so each worker only
            // sees guidance relevant to its own file ownership.
            let effective_retry_feedback = sub
                .retry_note
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&retry_feedback);
            // Build the retry ctx used for `{{retry_feedback_section}}` so
            // the formatted block also reflects the subtask-specific note.
            let effective_retry_ctx =
                sub.retry_note
                    .as_ref()
                    .filter(|s| !s.trim().is_empty())
                    .and_then(|note| {
                        self.retry_ctx.as_ref().map(|rc| {
                            crate::adapters::step_executor::driver::RetryContext {
                                feedback: note.clone(),
                                iteration: rc.iteration,
                                max: rc.max,
                            }
                        })
                    })
                    .or_else(|| self.retry_ctx.clone());
            let sub_template = step_conf.prompt_template.as_deref().unwrap_or("");
            let sub_retry_section =
                format_retry_feedback_section(effective_retry_ctx.as_ref());
            let sub_uses_retry_section = template_uses_retry_section(sub_template);
            let sub_prompt = self
                .base_ctx
                .clone()
                .set("subtask_description", &sub.description)
                .set("subtask_files", &sub_files_str)
                .set("other_subtask_files", &other_files_str)
                .set("partition_id", &sub.id)
                .set("retry_feedback_section", &sub_retry_section)
                .set("retry_feedback", effective_retry_feedback)
                .set("iteration", &retry_iteration)
                .set("max_iterations", &retry_max)
                .render(sub_template);
            let sub_prompt = if sub_prompt.trim().is_empty() {
                format!(
                    "Subtask: {}. Files: {}. Code inside: {}",
                    sub.title, sub_files_str, wt_path
                )
            } else {
                resolve_attached_artifacts(
                    &sub_prompt,
                    step_execs,
                    step_index,
                    &*self.artifacts,
                    &self.steps,
                )
            };
            let sub_prompt =
                inject_artifact_contract(&sub_prompt, if is_legacy { None } else { Some(decls) });
            // Surface retry feedback to the worker regardless of whether
            // the step's `prompt_template` references
            // `{{retry_feedback_section}}`. Matches the agent step
            // pattern — auto-append only as safety net.
            let sub_prompt = if sub_uses_retry_section {
                sub_prompt
            } else {
                append_retry_feedback_section(sub_prompt, effective_retry_ctx.as_ref())
            };

            // Copy any external artifact paths referenced in path manifests into
            // the worktree so opencode's `external_directory: deny` doesn't block
            // the agent from reading them.
            let sub_prompt =
                crate::adapters::step_executor::artifacts::materialize_external_artifact_paths(
                    &sub_prompt,
                    &wt_path,
                );

            let agent_kind = planner_kind.to_string();
            let sub_thread_id = format!("{}-{}", self.f_id_str, sub.id);
            let mut worker_env = crate::ports::agent_runtime::agent_base_env();
            // CLI agents: pass model via --model flag, not OPENCODE_CONFIG_CONTENT.
            if let Some(ref m) = override_model {
                if agent_kind == "opencode"
                    || agent_kind == "hermes"
                    || agent_kind == "claude-code"
                    || agent_kind == "antigravity"
                {
                    // CLI mode: model passed as --model flag at spawn
                } else {
                    let config = format!(
                        r#"{{"$schema":"https://opencode.ai/config.json","model":"{}"}}"#,
                        m
                    );
                    worker_env.insert("OPENCODE_CONFIG_CONTENT".to_string(), config);
                }
            }
            let binary = self
                .registry
                .runtime_for(&agent_kind)
                .map(|r| r.binary().to_string())
                .unwrap_or_else(|| agent_kind.clone());
            let ctx = AgentContext {
                thread_id: sub_thread_id.clone(),
                machine_id: machine_str.to_string(),
                binary,
                args: vec![],
                env: worker_env,
                cwd: wt_path.clone(),
                model: override_model.clone(),
                title: Some(sub.title.clone()),
                agent_exec: self.agent_exec.clone(),
                exec: self.exec.clone(),
                permissions: crate::domain::permission::PermissionProfile::all_allow(),
                bare_mode: agent_kind == "claude-code",
            };

            let spawn_fut = self.registry.get_or_spawn(&sub_thread_id, &agent_kind, ctx);
            let mut cancel_watch_spawn = self.cancel_watch.clone();
            let spawn_res = tokio::select! {
                res = spawn_fut => Some(res),
                _ = cancel_watch_spawn.changed() => None,
            };

            match spawn_res {
                Some(Ok(session)) => {
                    let is_cli_agent = agent_kind == "opencode"
                        || agent_kind == "hermes"
                        || agent_kind == "claude-code"
                        || agent_kind == "antigravity";
                    if !is_cli_agent {
                        if let Some(ref model) = override_model {
                            let info = session.session_info();
                            let applied = info
                                .config_options
                                .as_ref()
                                .and_then(|opts| opts.iter().find(|o| o.id == "model"))
                                .map(|o| o.current_value == *model)
                                .unwrap_or(false);
                            if !applied {
                                let _ = session.set_config_option("model", model);
                            }
                        }
                    }
                    let mut produced_artifacts: Vec<Artifact> = Vec::new();
                    let mut legacy_sub_content = String::new();

                    let timeouts =
                        crate::application::timeouts::resolve_effective(self.app_settings.as_ref());

                    let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
                        &*session,
                        &sub_prompt,
                        timeouts,
                        Some(self.cancel_watch.clone()),
                        machine_str,
                        &*self.exec,
                        override_model.clone(),
                        self.pricing.clone(),
                        |event| {
                            if let AgentEvent::Text { delta } = event {
                                if is_legacy {
                                    legacy_sub_content.push_str(delta);
                                }
                                let _ = self.notif.emit(&DomainEvent::AgentStream {
                                    feature_id: self.f_id.clone(),
                                    step_execution_id: step_exec.id.clone(),
                                    content: delta.clone(),
                                });
                                let _ = self.notif.emit(&DomainEvent::StepProgress {
                                    feature_id: self.f_id.clone(),
                                    step_id: step_exec.step_id.0.clone(),
                                    status: "running".into(),
                                    cost_usd: Some(*accumulated_cost),
                                    tokens: Some(*accumulated_tokens),
                                    wall_clock_secs: Some(step_start.elapsed().as_secs()),
                                    cache_read_input_tokens: None,
                                    cache_creation_input_tokens: None,
                                });
                            }
                        },
                    )
                    .await;

                    match turn_res {
                        crate::adapters::agent::event_stream::TurnResult::Interrupted => {
                            step_failed = true;
                            step_err_msg = "Execution cancelled by user".to_string();
                        }
                        crate::adapters::agent::event_stream::TurnResult::Failed(descriptive) => {
                            step_failed = true;
                            step_err_msg = format!(
                                "parallel subtask agent error ({}): {}",
                                sub.id, descriptive
                            );
                        }
                        crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                            *accumulated_cost += outcome.cost_usd;
                            *accumulated_tokens += outcome.tokens;
                            produced_artifacts = outcome.produced_artifacts;
                        }
                    }

                    if step_failed {
                        crate::adapters::agent::event_stream::cleanup_subtask(
                            &self.registry,
                            &self.git_ops,
                            self.machine_id_opt.as_deref(),
                            &self.target_dir,
                            &self.branch_name,
                            &sub.id,
                            &sub_thread_id,
                        )
                        .await;
                        break;
                    }

                    if is_legacy {
                        subtask_artifacts
                            .push(format!("### {}\n\n{}", sub.title, legacy_sub_content));
                    } else {
                        let always: Vec<&str> = decls
                            .iter()
                            .filter_map(|d| match &d.capture {
                                crate::domain::artifact::ArtifactCapture::LastWriteTo { path } => {
                                    Some(path.as_str())
                                }
                                _ => None,
                            })
                            .collect();
                        let mut changed = subtask_snapshot
                            .delta(&*self.exec, machine_str, &wt_path, &always, &[])
                            .await;
                        if changed.is_empty() {
                            if let Ok(git_diff_files) = self
                                .exec
                                .run_command(
                                    machine_str,
                                    &format!(
                                        "git -C {} diff --name-only {}",
                                        paths::shell_escape_posix(&wt_path),
                                        paths::shell_escape_posix(&self.branch_name),
                                    ),
                                )
                                .await
                            {
                                changed = git_diff_files
                                    .lines()
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                            }
                        }
                        for rel_path in changed {
                            let name = std::path::Path::new(&rel_path)
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("artifact")
                                .to_string();
                            if let Some(content) =
                                read_worktree_file(&*self.exec, machine_str, &wt_path, &rel_path)
                                    .await
                            {
                                produced_artifacts
                                    .push(Artifact::tool_write(name, rel_path, content));
                            }
                        }

                        // Post-step diff guard. Reverts any writes
                        // outside the declared artifact paths *before*
                        // commit, so the bad changes never reach the
                        // feature branch via the merge below.
                        if let Ok(reverted) = self
                            .git_ops
                            .verify_and_revert_out_of_scope_writes(
                                self.machine_id_opt.as_deref(),
                                &wt_path,
                                &writable_paths,
                            )
                            .await
                        {
                            if !reverted.is_empty() {
                                step_failed = true;
                                step_err_msg = format!(
                                    "parallel subtask {} wrote outside declared artifacts; \
                                     reverted: {}",
                                    sub.id,
                                    reverted.join(", ")
                                );
                                self.capture_signal(
                                    Some(step_exec.id.0.clone()),
                                    crate::domain::memory::SignalKind::Retry,
                                    format!(
                                        "Subtask '{}' wrote outside declared artifacts; \
                                         reverted: {}. Stay inside the artifacts directory.",
                                        sub.id,
                                        reverted.join(", ")
                                    ),
                                );
                                break;
                            }
                        }

                        let _ = commit_worktree_changes(
                            &*self.exec,
                            machine_str,
                            &wt_path,
                            &format!("feat({}): {}", self.f_id.as_str(), sub.title,),
                            &self.artifact_subdir,
                            self.commit_artifacts,
                        )
                        .await;

                        let refs = resolve_declared_artifacts(
                            decls,
                            &produced_artifacts,
                            &self.artifacts,
                            &self.f_id_str,
                            &step_exec.step_id.0,
                        );
                        all_artifact_refs.extend(refs);
                    }

                    // Merge back
                    let mut merge_result = self
                        .git_ops
                        .merge_subtask(
                            self.machine_id_opt.as_deref(),
                            &wt_path,
                            &self.branch_name,
                            &sub.id,
                        )
                        .await;

                    if merge_result.is_err() {
                        // Handle merge conflicts via helper function
                        let conflict_res = self
                            .handle_subtask_conflict(
                                step_exec,
                                &*session,
                                machine_str,
                                &wt_path,
                                &sub.id,
                                override_model,
                                accumulated_cost,
                                accumulated_tokens,
                                step_start,
                            )
                            .await;

                        match conflict_res {
                            Ok(()) => {
                                // Try merging again
                                merge_result = self
                                    .git_ops
                                    .merge_subtask(
                                        self.machine_id_opt.as_deref(),
                                        &wt_path,
                                        &self.branch_name,
                                        &sub.id,
                                    )
                                    .await;
                            }
                            Err(conflict_err) => {
                                merge_result = Err(conflict_err);
                            }
                        }
                    }

                    if step_failed {
                        let _ = self.registry.kill(&sub_thread_id).await;
                        let _ = tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        let _ = self
                            .git_ops
                            .cleanup_subtask_worktree(
                                self.machine_id_opt.as_deref(),
                                &self.target_dir,
                                &self.branch_name,
                                &sub.id,
                            )
                            .await;
                        break;
                    }

                    if let Err(err) = merge_result {
                        let _ = self.notif.emit(&DomainEvent::ConflictDetected {
                            feature_id: self.f_id.clone(),
                            subtask_id: format!("{}_subtask_{}", self.branch_name, sub.id),
                        });
                        step_failed = true;
                        step_err_msg =
                            format!("parallel subtask merge failed ({}): {}", sub.id, err);
                        crate::adapters::agent::event_stream::cleanup_subtask(
                            &self.registry,
                            &self.git_ops,
                            self.machine_id_opt.as_deref(),
                            &self.target_dir,
                            &self.branch_name,
                            &sub.id,
                            &sub_thread_id,
                        )
                        .await;
                        break;
                    }
                }
                Some(Err(e)) => {
                    step_failed = true;
                    step_err_msg =
                        format!("parallel subtask agent spawn failed ({}): {:?}", sub.id, e);
                    crate::adapters::agent::event_stream::cleanup_subtask(
                        &self.registry,
                        &self.git_ops,
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        &self.branch_name,
                        &sub.id,
                        &sub_thread_id,
                    )
                    .await;
                    break;
                }
                None => {
                    step_failed = true;
                    step_err_msg = "Execution cancelled by user".to_string();
                    crate::adapters::agent::event_stream::cleanup_subtask(
                        &self.registry,
                        &self.git_ops,
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        &self.branch_name,
                        &sub.id,
                        &sub_thread_id,
                    )
                    .await;
                    break;
                }
            }

            // Cleanup worktree (success path)
            crate::adapters::agent::event_stream::cleanup_subtask(
                &self.registry,
                &self.git_ops,
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                &sub.id,
                &sub_thread_id,
            )
            .await;
        }

        if step_failed {
            Err(step_err_msg)
        } else {
            Ok(())
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_subtask_conflict(
        &self,
        step_exec: &StepExecution,
        session: &dyn crate::ports::agent_runtime::AgentSession,
        machine_str: &str,
        wt_path: &str,
        sub_id: &str,
        override_model: &Option<String>,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
    ) -> Result<(), String> {
        let merge_back_cmd = format!(
            "git -C {} merge {}",
            paths::shell_escape_posix(wt_path),
            paths::shell_escape_posix(&self.branch_name)
        );
        let _ = self.exec.run_command(machine_str, &merge_back_cmd).await;

        let unmerged = list_unmerged_files(&*self.exec, machine_str, wt_path).await;
        if unmerged.is_empty() {
            return Ok(());
        }

        let files_list = unmerged
            .iter()
            .map(|f| format!("- {} ({})", f.path, f.kind))
            .collect::<Vec<_>>()
            .join("\n");
        let conflict_prompt = format!(
            "We encountered a merge conflict while merging the latest changes from the feature branch '{}' into your workspace.\n\
             Please resolve the conflicts in the following files:\n\
             {}\n\n\
             Ensure you edit these files to remove conflict markers (<<<<<<<, =======, >>>>>>>) and integrate the changes correctly. \
             Make sure all code builds and passes tests. Once done, let me know.",
            self.branch_name, files_list
        );

        let timeouts = crate::application::timeouts::resolve_effective(self.app_settings.as_ref());

        let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
            session,
            &conflict_prompt,
            timeouts,
            Some(self.cancel_watch.clone()),
            machine_str,
            &*self.exec,
            override_model.clone(),
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
                        cost_usd: Some(*accumulated_cost),
                        tokens: Some(*accumulated_tokens),
                        wall_clock_secs: Some(step_start.elapsed().as_secs()),
                        cache_read_input_tokens: None,
                        cache_creation_input_tokens: None,
                    });
                }
            },
        )
        .await;

        let mut conflict_failed = None;
        let mut conflict_cancelled = false;

        match turn_res {
            crate::adapters::agent::event_stream::TurnResult::Interrupted => {
                conflict_cancelled = true;
            }
            crate::adapters::agent::event_stream::TurnResult::Failed(descriptive) => {
                conflict_failed = Some(descriptive);
            }
            crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                *accumulated_cost += outcome.cost_usd;
                *accumulated_tokens += outcome.tokens;
            }
        }

        if conflict_cancelled || *self.cancel_watch.borrow() {
            return Err("Execution cancelled by user".to_string());
        }
        if let Some(failed_msg) = conflict_failed {
            return Err(format!(
                "parallel subtask agent error during conflict resolution ({}): {}",
                sub_id, failed_msg
            ));
        }

        // Verify conflicts are resolved.
        let still_unmerged = list_unmerged_files(&*self.exec, machine_str, wt_path).await;
        if still_unmerged.is_empty() {
            let commit_resolved = self
                .exec
                .run_command(
                    machine_str,
                    &format!(
                        "git -C {} commit -am \"Resolve merge conflicts with {}\"",
                        paths::shell_escape_posix(wt_path),
                        paths::shell_escape_posix(&self.branch_name)
                    ),
                )
                .await;
            if commit_resolved.is_ok() {
                Ok(())
            } else {
                Err("Failed to commit merge conflict resolution".to_string())
            }
        } else {
            Err(format!(
                "Agent failed to resolve merge conflicts in: {:?}",
                still_unmerged.iter().map(|f| &f.path).collect::<Vec<_>>()
            ))
        }
    }
}

use std::time::Instant;

use tokio_stream::StreamExt;

use crate::adapters::step_executor::artifacts::{
    compute_git_diff, inject_artifact_contract, read_worktree_file, resolve_attached_artifacts,
    resolve_declared_artifacts, WorktreeSnapshot,
};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::agent_event::AgentEvent;
use crate::domain::artifact::Artifact;
use crate::domain::models::{StepConfig, StepExecution};
use crate::paths;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

impl ExecutionDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn handle_agent_step(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
        step_index: usize,
        step_execs: &[StepExecution],
    ) -> StepOutcome {
        let feature = self.features.get(&self.f_id).ok().flatten();
        let override_agent = feature.as_ref().and_then(|f| f.agent_kind.clone());
        let override_model = feature.as_ref().and_then(|f| f.model.clone());

        let agent_kind = override_agent
            .or_else(|| step_conf.agent_kind.clone())
            .unwrap_or_else(|| "opencode".to_string());

        let (gate_decision, gate_feedback) =
            crate::adapters::step_executor::artifacts::get_latest_gate_decision(
                &*self.gates,
                self.f_id.as_str(),
            );

        let prompt = self
            .base_ctx
            .clone()
            .set("gate_feedback", &gate_feedback)
            .set("gate_decision", &gate_decision)
            .render(step_conf.prompt_template.as_deref().unwrap_or(""));
        let prompt = resolve_attached_artifacts(&prompt, step_execs, step_index);

        // Inject the machine-readable artifact contract into the prompt
        // so the agent knows exactly what files to produce.
        let is_legacy = step_conf.artifacts.as_ref().is_none_or(|d| d.is_empty());
        let decls: &[crate::domain::artifact::ArtifactDecl] =
            step_conf.artifacts.as_deref().unwrap_or(&[]);
        let prompt = inject_artifact_contract(&prompt, if is_legacy { None } else { Some(decls) });

        let machine_str = self
            .machine_id_opt
            .clone()
            .unwrap_or_else(|| "local".to_string());
        let mut agent_env = crate::ports::agent_runtime::agent_base_env();
        // CLI agents (opencode, hermes): pass model via --model flag, not OPENCODE_CONFIG_CONTENT.
        // ACP agents: pass model via OPENCODE_CONFIG_CONTENT.
        if let Some(ref m) = override_model {
            if agent_kind == "opencode" || agent_kind == "hermes" {
                // CLI mode: model passed as --model flag at spawn; no config content needed.
            } else {
                let config = format!(
                    r#"{{"$schema":"https://opencode.ai/config.json","model":"{}"}}"#,
                    m
                );
                agent_env.insert("OPENCODE_CONFIG_CONTENT".to_string(), config);
            }
        }

        let subtask_id = format!("step-{}", step_exec.step_id.0);
        let wt_path = match self.git_ops.provision_subtask_worktree(
            self.machine_id_opt.as_deref(),
            &self.target_dir,
            &self.branch_name,
            &subtask_id,
        ) {
            Ok(p) => p,
            Err(e) => {
                return StepOutcome::Failed(format!(
                    "agent step worktree provision failed ({}): {}",
                    subtask_id, e
                ));
            }
        };

        let ctx = AgentContext {
            thread_id: self.f_id_str.clone(),
            machine_id: machine_str.clone(),
            binary: agent_kind.clone(),
            args: vec![],
            env: agent_env,
            cwd: wt_path.clone(),
            model: override_model.clone(),
            title: Some(step_conf.title.clone()),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
        };

        if *self.cancel_watch.borrow() {
            let _ = self.git_ops.cleanup_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                &subtask_id,
            );
            return StepOutcome::Cancelled;
        }

        // Snapshot the worktree's dirty state BEFORE the agent runs.
        let worktree_snapshot = WorktreeSnapshot::capture(&*self.exec, &machine_str, &wt_path);

        let worktree_base_ref = self
            .exec
            .run_command(
                &machine_str,
                &format!(
                    "git -C {} rev-parse {}",
                    paths::shell_escape_posix(&self.target_dir),
                    paths::shell_escape_posix(&self.branch_name),
                ),
            )
            .map(|s| s.trim().to_string())
            .ok();

        let spawn_fut = self
            .registry
            .get_or_spawn(self.f_id.as_str(), &agent_kind, ctx);
        let mut cancel_watch_spawn = self.cancel_watch.clone();
        let spawn_res = tokio::select! {
            res = spawn_fut => Some(res),
            _ = cancel_watch_spawn.changed() => None,
        };

        let outcome = match spawn_res {
            Some(Ok(session)) => {
                let is_cli_agent = agent_kind == "opencode" || agent_kind == "hermes";
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
                            let mut config_ok = false;
                            if session.set_config_option("model", model).is_ok() {
                                let info2 = session.session_info();
                                let really_applied = info2
                                    .config_options
                                    .as_ref()
                                    .and_then(|opts| opts.iter().find(|o| o.id == "model"))
                                    .map(|o| o.current_value == *model)
                                    .unwrap_or(false);
                                if really_applied {
                                    config_ok = true;
                                }
                            }
                            if !config_ok {
                                let descriptive = format!(
                                    "Model '{}' could not be applied to the agent session.",
                                    model
                                );
                                return StepOutcome::Failed(descriptive);
                            }
                        }
                    }
                }

                let mut produced_artifacts: Vec<Artifact> = Vec::new();
                let mut text_buffer = String::new();
                let hb = session.stderr_heartbeat();
                let mut stream = session.prompt(&prompt);
                let mut cancel_watch = self.cancel_watch.clone();
                let mut first_event_seen = false;
                let mut latest_cost = 0.0;
                let mut latest_tokens = 0;

                const FAST_TIMEOUT_S: u64 = 180;
                const NORMAL_TIMEOUT_S: u64 = 180;
                const WALL_CAP_S: u64 = 600;

                let fast_sleep = tokio::time::sleep(std::time::Duration::from_secs(FAST_TIMEOUT_S));
                let normal_sleep =
                    tokio::time::sleep(std::time::Duration::from_secs(NORMAL_TIMEOUT_S));
                let wall_sleep = tokio::time::sleep(std::time::Duration::from_secs(WALL_CAP_S));
                tokio::pin!(fast_sleep);
                tokio::pin!(normal_sleep);
                tokio::pin!(wall_sleep);

                let mut run_failed = None;
                let mut run_cancelled = false;

                loop {
                    tokio::select! {
                        event_opt = stream.next() => {
                            let event = match event_opt {
                                Some(ev) => ev,
                                None => break,
                            };
                            first_event_seen = true;

                            let now = tokio::time::Instant::now();
                            let next_fast = now + std::time::Duration::from_secs(FAST_TIMEOUT_S);
                            let next_normal = now + std::time::Duration::from_secs(NORMAL_TIMEOUT_S);
                            fast_sleep.as_mut().reset(next_fast);
                            normal_sleep.as_mut().reset(next_normal);

                            match event {
                                AgentEvent::Text { delta } => {
                                    let _ = self.notif.emit(&DomainEvent::AgentStream {
                                        feature_id: self.f_id.clone(),
                                        step_execution_id: step_exec.id.clone(),
                                        content: delta.clone(),
                                    });
                                    let _ = self.notif.emit(&DomainEvent::StepProgress {
                                        feature_id: self.f_id.clone(),
                                        step_id: step_exec.step_id.0.clone(),
                                        status: "running".into(),
                                        cost_usd: Some(*accumulated_cost + latest_cost),
                                        tokens: Some(*accumulated_tokens + latest_tokens),
                                        wall_clock_secs: Some(step_start.elapsed().as_secs()),
                                    });
                                    let is_tool_breadcrumb =
                                        delta.starts_with("[tool ") || delta.starts_with("[tool:");
                                    if !is_tool_breadcrumb {
                                        text_buffer.push_str(&delta);
                                    }
                                }
                                AgentEvent::ArtifactProduced { artifact } => {
                                    produced_artifacts.push(artifact);
                                }
                                AgentEvent::Usage { input_tokens, output_tokens, cost_usd } => {
                                    if let Some(c) = cost_usd {
                                        latest_cost = c;
                                    }
                                    latest_tokens = (input_tokens + output_tokens) as i64;
                                }
                                AgentEvent::TurnComplete { .. } => break,
                                AgentEvent::Error { message, .. } => {
                                    let descriptive = format_agent_error_message(&message, &machine_str, &*self.exec);
                                    run_failed = Some(StepOutcome::Failed(descriptive));
                                    break;
                                }
                                _ => {}
                            }
                        }
                        _ = &mut fast_sleep => {
                            if !first_event_seen {
                                fast_sleep.as_mut().reset(
                                    tokio::time::Instant::now() + std::time::Duration::from_secs(FAST_TIMEOUT_S),
                                );
                                continue;
                            }
                            if hb.as_ref().is_some_and(|h| h.last_activity_ago_ms() > FAST_TIMEOUT_S * 1000) {
                                let descriptive = format_agent_error_message(
                                    &format!("Agent blocked: no output for {}s (stdout and stderr both silent)", FAST_TIMEOUT_S),
                                    &machine_str, &*self.exec,
                                );
                                run_failed = Some(StepOutcome::Failed(descriptive));
                                break;
                            }
                            fast_sleep.as_mut().reset(
                                tokio::time::Instant::now() + std::time::Duration::from_secs(FAST_TIMEOUT_S),
                            );
                        }
                        _ = &mut normal_sleep => {
                            if let Some(ref h) = hb {
                                  if h.last_activity_ago_ms() < NORMAL_TIMEOUT_S * 1000 {
                                      normal_sleep.as_mut().reset(
                                          tokio::time::Instant::now() + std::time::Duration::from_secs(NORMAL_TIMEOUT_S),
                                      );
                                      continue;
                                  }
                            }
                            let descriptive = format_agent_error_message(
                                &format!("Agent response timed out (no output for {}s)", NORMAL_TIMEOUT_S),
                                &machine_str, &*self.exec,
                            );
                            run_failed = Some(StepOutcome::Failed(descriptive));
                            break;
                        }
                        _ = &mut wall_sleep => {
                            let elapsed = step_start.elapsed().as_secs();
                            run_failed = Some(StepOutcome::Failed(format!(
                                "Agent step exceeded wall clock cap ({}s / {}s elapsed)",
                                WALL_CAP_S, elapsed,
                            )));
                            break;
                        }
                        _ = cancel_watch.changed() => {
                            if *cancel_watch.borrow() {
                                let _ = session.cancel();
                                run_cancelled = true;
                                break;
                            }
                        }
                    }
                }

                *accumulated_cost += latest_cost;
                *accumulated_tokens += latest_tokens;

                if run_cancelled || *self.cancel_watch.borrow() {
                    let wall = step_start.elapsed().as_secs();
                    let _ = self.features.step_update(
                        &step_exec.id,
                        &StepExecutionPatch {
                            iteration_count: None,
                            status: Some("interrupted".to_string()),
                            cost_usd: Some(Some(*accumulated_cost)),
                            tokens: Some(Some(*accumulated_tokens)),
                            wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                            artifact_path: None,
                            artifact_paths: None,
                            error_message: Some(Some("Execution cancelled by user".to_string())),
                        },
                    );
                    let _ = self.notif.emit(&DomainEvent::StepProgress {
                        feature_id: self.f_id.clone(),
                        step_id: step_exec.step_id.0.clone(),
                        status: "interrupted".into(),
                        cost_usd: Some(*accumulated_cost),
                        tokens: Some(*accumulated_tokens),
                        wall_clock_secs: Some(wall),
                    });
                    StepOutcome::Cancelled
                } else if let Some(failed_outcome) = run_failed {
                    failed_outcome
                } else {
                    // Process artifacts from the worktree path.
                    if !is_legacy {
                        let always: Vec<&str> = decls
                            .iter()
                            .filter_map(|d| match &d.capture {
                                crate::domain::artifact::ArtifactCapture::LastWriteTo { path } => {
                                    Some(path.as_str())
                                }
                                _ => None,
                            })
                            .collect();
                        let changed = worktree_snapshot.delta(
                            &*self.exec,
                            &machine_str,
                            &wt_path,
                            &always,
                            &[],
                        );
                        for rel_path in changed {
                            let name = std::path::Path::new(&rel_path)
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("artifact")
                                .to_string();
                            if let Some(content) =
                                read_worktree_file(&*self.exec, &machine_str, &wt_path, &rel_path)
                            {
                                produced_artifacts
                                    .push(Artifact::tool_write(name, rel_path, content));
                            }
                        }
                    }

                    if !is_legacy {
                        let diff_ref = worktree_base_ref.as_deref().unwrap_or("HEAD");
                        let diff_body =
                            compute_git_diff(&*self.exec, &machine_str, &wt_path, diff_ref);
                        if !diff_body.trim().is_empty() {
                            let diff_name = "code-diff".to_string();
                            produced_artifacts.push(Artifact {
                                name: diff_name,
                                mime: "text/x-diff".into(),
                                content: diff_body,
                                source: crate::domain::artifact::ArtifactSource::Diff {
                                    base: diff_ref.to_string(),
                                    head: "WORKTREE".to_string(),
                                    path_filter: None,
                                },
                            });
                        }
                    }

                    // Commit worktree changes.
                    let _ = crate::adapters::step_executor::artifacts::commit_worktree_changes(
                        &*self.exec,
                        &machine_str,
                        &wt_path,
                        &format!("feat({}): {}", self.f_id.as_str(), step_conf.title),
                    );

                    // Run verifier check if configured
                    if let Some(ref verifier_cfg) = step_conf.verifier {
                        if let Err(verdict_err) = self
                            .run_verifier_logic(
                                step_exec,
                                verifier_cfg,
                                &wt_path,
                                &produced_artifacts,
                                accumulated_cost,
                                accumulated_tokens,
                                step_start,
                                &agent_kind,
                                &override_model,
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

                    // Merge subtask branch back (in the worktree, isolated).
                    let mut merge_result = self.git_ops.merge_subtask(
                        self.machine_id_opt.as_deref(),
                        &wt_path,
                        &self.branch_name,
                        &subtask_id,
                    );

                    // Resolve conflicts if merge failed.
                    if let Err(ref e) = merge_result {
                        // Try to resolve merge conflict using the agent.
                        let merge_back_cmd = format!(
                            "git -C {} merge {}",
                            paths::shell_escape_posix(&wt_path),
                            paths::shell_escape_posix(&self.branch_name)
                        );
                        let _ = self.exec.run_command(&machine_str, &merge_back_cmd);

                        let unmerged = list_unmerged_files(&*self.exec, &machine_str, &wt_path);
                        if !unmerged.is_empty() {
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

                            let mut conflict_stream = session.prompt(&conflict_prompt);
                            let mut cancel_watch_conflict = self.cancel_watch.clone();
                            let mut first_event_seen = false;
                            let mut latest_conflict_cost = 0.0;
                            let mut latest_conflict_tokens = 0;

                            let fast_sleep =
                                tokio::time::sleep(std::time::Duration::from_secs(FAST_TIMEOUT_S));
                            let normal_sleep = tokio::time::sleep(std::time::Duration::from_secs(
                                NORMAL_TIMEOUT_S,
                            ));
                            let wall_sleep =
                                tokio::time::sleep(std::time::Duration::from_secs(WALL_CAP_S));
                            tokio::pin!(fast_sleep);
                            tokio::pin!(normal_sleep);
                            tokio::pin!(wall_sleep);

                            let mut conflict_failed = None;
                            let mut conflict_cancelled = false;

                            loop {
                                tokio::select! {
                                    event_opt = conflict_stream.next() => {
                                        let event = match event_opt {
                                            Some(ev) => ev,
                                            None => break,
                                        };
                                        first_event_seen = true;

                                        let now = tokio::time::Instant::now();
                                        let next_fast = now + std::time::Duration::from_secs(FAST_TIMEOUT_S);
                                        let next_normal = now + std::time::Duration::from_secs(NORMAL_TIMEOUT_S);
                                        fast_sleep.as_mut().reset(next_fast);
                                        normal_sleep.as_mut().reset(next_normal);

                                        match event {
                                            AgentEvent::Text { delta } => {
                                                let _ = self.notif.emit(&DomainEvent::AgentStream {
                                                    feature_id: self.f_id.clone(),
                                                    step_execution_id: step_exec.id.clone(),
                                                    content: delta.clone(),
                                                });
                                                let _ = self.notif.emit(&DomainEvent::StepProgress {
                                                    feature_id: self.f_id.clone(),
                                                    step_id: step_exec.step_id.0.clone(),
                                                    status: "running".into(),
                                                    cost_usd: Some(*accumulated_cost + latest_conflict_cost),
                                                    tokens: Some(*accumulated_tokens + latest_conflict_tokens),
                                                    wall_clock_secs: Some(step_start.elapsed().as_secs()),
                                                });
                                            }
                                            AgentEvent::Usage { input_tokens, output_tokens, cost_usd } => {
                                                if let Some(c) = cost_usd {
                                                    latest_conflict_cost = c;
                                                }
                                                latest_conflict_tokens = (input_tokens + output_tokens) as i64;
                                            }
                                            AgentEvent::TurnComplete { .. } => break,
                                            AgentEvent::Error { message, .. } => {
                                                let descriptive = format_agent_error_message(&message, &machine_str, &*self.exec);
                                                conflict_failed = Some(StepOutcome::Failed(descriptive));
                                                break;
                                            }
                                            _ => {}
                                        }
                                    }
                                    _ = &mut fast_sleep => {
                                        if !first_event_seen {
                                            fast_sleep.as_mut().reset(
                                                tokio::time::Instant::now() + std::time::Duration::from_secs(FAST_TIMEOUT_S),
                                            );
                                            continue;
                                        }
                                        if hb.as_ref().is_some_and(|h| h.last_activity_ago_ms() > FAST_TIMEOUT_S * 1000) {
                                            let descriptive = format_agent_error_message(
                                                &format!("Agent blocked: no output for {}s (stdout and stderr both silent)", FAST_TIMEOUT_S),
                                                &machine_str, &*self.exec,
                                            );
                                            conflict_failed = Some(StepOutcome::Failed(descriptive));
                                            break;
                                        }
                                        fast_sleep.as_mut().reset(
                                            tokio::time::Instant::now() + std::time::Duration::from_secs(FAST_TIMEOUT_S),
                                        );
                                    }
                                    _ = &mut normal_sleep => {
                                        if let Some(ref h) = hb {
                                            if h.last_activity_ago_ms() < NORMAL_TIMEOUT_S * 1000 {
                                                normal_sleep.as_mut().reset(
                                                    tokio::time::Instant::now() + std::time::Duration::from_secs(NORMAL_TIMEOUT_S),
                                                );
                                                continue;
                                            }
                                        }
                                        let descriptive = format_agent_error_message(
                                            &format!("Agent response timed out (no output for {}s)", NORMAL_TIMEOUT_S),
                                            &machine_str, &*self.exec,
                                        );
                                        conflict_failed = Some(StepOutcome::Failed(descriptive));
                                        break;
                                    }
                                    _ = &mut wall_sleep => {
                                        conflict_failed = Some(StepOutcome::Failed("Agent conflict resolution exceeded wall clock cap".to_string()));
                                        break;
                                    }
                                    _ = cancel_watch_conflict.changed() => {
                                        if *cancel_watch_conflict.borrow() {
                                            let _ = session.cancel();
                                            conflict_cancelled = true;
                                            break;
                                        }
                                    }
                                }
                            }

                            *accumulated_cost += latest_conflict_cost;
                            *accumulated_tokens += latest_conflict_tokens;

                            if conflict_cancelled || *self.cancel_watch.borrow() {
                                run_cancelled = true;
                            } else if let Some(failed_outcome) = conflict_failed {
                                run_failed = Some(failed_outcome);
                            } else {
                                // Verify conflicts are resolved.
                                let still_unmerged =
                                    list_unmerged_files(&*self.exec, &machine_str, &wt_path);
                                if still_unmerged.is_empty() {
                                    let commit_resolved = self.exec.run_command(
                                        &machine_str,
                                        &format!(
                                            "git -C {} commit -am \"Resolve merge conflicts with {}\"",
                                            paths::shell_escape_posix(&wt_path),
                                            paths::shell_escape_posix(&self.branch_name)
                                        ),
                                    );
                                    if commit_resolved.is_ok() {
                                        merge_result = self.git_ops.merge_subtask(
                                            self.machine_id_opt.as_deref(),
                                            &wt_path,
                                            &self.branch_name,
                                            &subtask_id,
                                        );
                                    } else {
                                        merge_result =
                                            Err("Failed to commit merge conflict resolution"
                                                .to_string());
                                    }
                                } else {
                                    merge_result = Err(format!(
                                        "Agent failed to resolve merge conflicts in: {:?}",
                                        still_unmerged.iter().map(|f| &f.path).collect::<Vec<_>>()
                                    ));
                                }
                            }
                        } else {
                            merge_result = Err(format!("agent step merge failed: {}", e));
                        }
                    }

                    if run_cancelled || *self.cancel_watch.borrow() {
                        let wall = step_start.elapsed().as_secs();
                        let _ = self.features.step_update(
                            &step_exec.id,
                            &StepExecutionPatch {
                                iteration_count: None,
                                status: Some("interrupted".to_string()),
                                cost_usd: Some(Some(*accumulated_cost)),
                                tokens: Some(Some(*accumulated_tokens)),
                                wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                                artifact_path: None,
                                artifact_paths: None,
                                error_message: Some(Some(
                                    "Execution cancelled by user".to_string(),
                                )),
                            },
                        );
                        let _ = self.notif.emit(&DomainEvent::StepProgress {
                            feature_id: self.f_id.clone(),
                            step_id: step_exec.step_id.0.clone(),
                            status: "interrupted".into(),
                            cost_usd: Some(*accumulated_cost),
                            tokens: Some(*accumulated_tokens),
                            wall_clock_secs: Some(wall),
                        });
                        StepOutcome::Cancelled
                    } else if let Some(failed_outcome) = run_failed {
                        failed_outcome
                    } else {
                        match merge_result {
                            Ok(()) => {
                                // Resolve artifacts: either declared (new) or text-dump (legacy)
                                let (artifact_path, artifact_paths) = if !is_legacy {
                                    let refs = resolve_declared_artifacts(
                                        decls,
                                        &produced_artifacts,
                                        &self.artifacts,
                                        &self.f_id_str,
                                        &step_exec.step_id.0,
                                    );
                                    let primary = if step_conf.kind == "parallel" {
                                        refs.iter()
                                            .find(|r| {
                                                r.contains("code-diff") || r.ends_with(".diff")
                                            })
                                            .cloned()
                                            .or_else(|| refs.first().cloned())
                                    } else {
                                        refs.first().cloned()
                                    };
                                    (primary, refs)
                                } else {
                                    let mut art_path = self
                                        .app_local_data_dir
                                        .join("artifacts")
                                        .join(&self.f_id_str);
                                    let _ = std::fs::create_dir_all(&art_path);
                                    let file_name = format!("{}.md", step_exec.step_id.0);
                                    art_path.push(&file_name);
                                    let _ = std::fs::write(&art_path, &text_buffer);
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
                                        wall_clock_secs: Some(wall).map(|_v| Some(wall)),
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
                            Err(err) => {
                                StepOutcome::Failed(format!("agent step merge failed: {}", err))
                            }
                        }
                    }
                }
            }
            Some(Err(e)) => {
                let descriptive =
                    format_agent_error_message(&e.to_string(), &machine_str, &*self.exec);
                StepOutcome::Failed(descriptive)
            }
            None => StepOutcome::Cancelled,
        };

        // Cleanup temporary worktree in all cases.
        let _ = self.git_ops.cleanup_subtask_worktree(
            self.machine_id_opt.as_deref(),
            &self.target_dir,
            &self.branch_name,
            &subtask_id,
        );

        let _ = self.registry.kill(self.f_id.as_str()).await;
        let _ = self
            .registry
            .kill(&format!("{}-verifier", self.f_id.as_str()))
            .await;

        outcome
    }
}

pub(crate) fn format_agent_error_message(
    message: &str,
    machine_id: &str,
    exec: &dyn crate::ports::execution::ExecutionPort,
) -> String {
    if message.contains("OpenCode service failure")
        || message.contains("timed out")
        || message.contains("no output")
        || message.is_empty()
    {
        // Fetch last 100 lines of remote log
        if let Ok(logs) = exec.run_command(machine_id, "tail -n 100 /tmp/opencode_run.log") {
            if logs.contains("FreeUsageLimitError") || logs.contains("Rate limit exceeded") {
                return "OpenCode Rate Limit Exceeded: The free model rate limit was reached. Please try changing the model to a different model (e.g. 'opencode/big-pickle') or try again later.".to_string();
            }
            if logs.contains("CreditLimitError")
                || logs.contains("Insufficient funds")
                || logs.contains("credits limit")
                || logs.contains("insufficient balance")
            {
                return "OpenCode Credit Limit Exceeded: You have run out of credits or reached your usage quota. Please verify your billing/credits on OpenCode or switch to a free model.".to_string();
            }
            // Fallback search through last lines
            for line in logs.lines().rev() {
                if line.contains("FreeUsageLimitError") || line.contains("Rate limit exceeded") {
                    return "OpenCode Rate Limit Exceeded: The free model rate limit was reached. Please try changing the model to a different model (e.g. 'opencode/big-pickle') or try again later.".to_string();
                }
                if line.contains("CreditLimitError") || line.contains("credits limit") {
                    return "OpenCode Credit Limit Exceeded: You have run out of credits or reached your usage quota. Please verify your billing/credits on OpenCode or switch to a free model.".to_string();
                }
            }
        }
    }
    message.to_string()
}

struct ConflictFile {
    path: String,
    kind: String,
}

fn list_unmerged_files(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
) -> Vec<ConflictFile> {
    let raw = match exec.run_command(
        machine_id,
        &format!(
            "git -C {} status --porcelain --untracked-files=no",
            paths::shell_escape_posix(repo_dir)
        ),
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    raw.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            if line.len() < 3 {
                return None;
            }
            let xy = &line[..2];
            let path = line[3..].trim().to_string();
            let kind = match xy {
                "UU" | "AA" | "DD" => "both-modified".to_string(),
                "UA" => "added-by-them".to_string(),
                "AU" => "added-by-us".to_string(),
                "UD" => "deleted-by-them".to_string(),
                "DU" => "deleted-by-us".to_string(),
                _ => return None,
            };
            Some(ConflictFile { path, kind })
        })
        .collect()
}

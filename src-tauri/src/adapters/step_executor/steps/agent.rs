use std::time::Instant;

use tokio_stream::StreamExt;

use crate::adapters::step_executor::artifacts::{compute_git_diff, inject_artifact_contract, read_worktree_file, resolve_attached_artifacts, resolve_declared_artifacts, WorktreeSnapshot};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::artifact::Artifact;
use crate::domain::agent_event::AgentEvent;
use crate::domain::models::{StepConfig, StepExecution};
use crate::ports::agent_runtime::AgentContext;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;
use crate::paths;

impl ExecutionDriver {
    pub(crate) async fn handle_agent_step(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
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

        let (gate_decision, gate_feedback) = crate::adapters::step_executor::artifacts::get_latest_gate_decision(
            &*self.gates,
            self.f_id.as_str(),
        );

        let prompt = self.base_ctx.clone()
            .set("gate_feedback", &gate_feedback)
            .set("gate_decision", &gate_decision)
            .render(step_conf.prompt_template.as_deref().unwrap_or(""));
        let prompt = resolve_attached_artifacts(&prompt, step_execs, step_index);

        // Inject the machine-readable artifact contract into the prompt
        // so the agent knows exactly what files to produce.
        let is_legacy = step_conf.artifacts.as_ref().map_or(true, |d| d.is_empty());
        let decls: &[crate::domain::artifact::ArtifactDecl] = step_conf.artifacts.as_deref().unwrap_or(&[]);
        let prompt = inject_artifact_contract(&prompt, if is_legacy { None } else { Some(decls) });

        let machine_str = self.machine_id_opt.clone().unwrap_or_else(|| "local".to_string());
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
        let ctx = AgentContext {
            thread_id: self.f_id_str.clone(),
            machine_id: machine_str.clone(),
            binary: agent_kind.clone(),
            args: vec![],
            env: agent_env,
            cwd: self.target_dir.clone(),
            model: override_model.clone(),
            title: Some(step_conf.title.clone()),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
        };

        if *self.cancel_watch.borrow() {
            return StepOutcome::Cancelled;
        }

        // Snapshot the worktree's dirty state BEFORE the agent runs.
        // The delta at step end tells us which files the agent actually
        // created/modified during this step, isolated from any state
        // that was already dirty from a prior step. This replaces the
        // old `git diff <base>..HEAD` strategy, which missed every
        // file the agent wrote because the agent does not commit.
        let worktree_snapshot = WorktreeSnapshot::capture(
            &*self.exec,
            &machine_str,
            &self.target_dir,
        );

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

        let spawn_fut = self.registry.get_or_spawn(self.f_id.as_str(), &agent_kind, ctx);
        let mut cancel_watch_spawn = self.cancel_watch.clone();
        let spawn_res = tokio::select! {
            res = spawn_fut => Some(res),
            _ = cancel_watch_spawn.changed() => {
                if *cancel_watch_spawn.borrow() { None } else { None }
            }
        };

        match spawn_res {
            Some(Ok(session)) => {
                // Verify the model from session/new — fall back to
                // set_config_option if the agent didn't apply it.
                // For CLI agents (opencode, hermes), set_config_option is a no-op
                // and the model is passed as --model at spawn time, so we skip
                // post-spawn verification for them.
                let is_cli_agent = agent_kind == "opencode" || agent_kind == "hermes";
                if !is_cli_agent {
                if let Some(ref model) = override_model {
                    let info = session.session_info();
                    let applied = info.config_options.as_ref().and_then(|opts|
                        opts.iter().find(|o| o.id == "model")
                    ).map(|o| o.current_value == *model).unwrap_or(false);
                    if !applied {
                        eprintln!(
                            "[agent step] model '{}' not applied in session/new (current={:?}), trying set_config_option",
                            model,
                            info.config_options.as_ref().and_then(|opts|
                                opts.iter().find(|o| o.id == "model").map(|o| &o.current_value)
                            )
                        );
                        let config_ok = match session.set_config_option("model", model) {
                            Ok(_) => {
                                let info2 = session.session_info();
                                let really_applied = info2.config_options.as_ref().and_then(|opts|
                                    opts.iter().find(|o| o.id == "model")
                                ).map(|o| o.current_value == *model).unwrap_or(false);
                                if really_applied {
                                    println!("[agent step] set_config_option model to '{}' confirmed in session_info", model);
                                    true
                                } else {
                                    eprintln!(
                                        "[agent step] set_config_option returned Ok but model '{}' STILL not applied (current={:?})",
                                        model,
                                        info2.config_options.as_ref().and_then(|opts|
                                            opts.iter().find(|o| o.id == "model").map(|o| &o.current_value)
                                        )
                                    );
                                    false
                                }
                            }
                            Err(e) => {
                                eprintln!("[agent step] set_config_option model to '{}' failed: {}", model, e);
                                false
                            }
                        };
                        if !config_ok {
                            let _ = self.registry.kill(self.f_id.as_str()).await;
                            let descriptive = format!(
                                "Model '{}' could not be applied to the agent session. \
                                 The agent rejected the model selection via set_config_option. \
                                 Try selecting a different model, or check that the model is valid for this provider.",
                                model
                            );
                            return StepOutcome::Failed(descriptive);
                        }
                    } else {
                        println!("[agent step] model '{}' confirmed in session_info after spawn", model);
                    }
                }
                } // end if !is_cli_agent
                // Collect ArtifactProduced events emitted by the runtime
                // during the agent turn. In legacy mode (no declarations)
                // we fall back to the old text-dump approach.
                // In declared mode we accumulate text and synthesize
                // ArtifactProduced events at TurnComplete so the
                // orchestrator can match them against declarations.
                let mut produced_artifacts: Vec<Artifact> = Vec::new();
                let mut text_buffer = String::new();
                let hb = session.stderr_heartbeat();
                let mut stream = session.prompt(&prompt);
                let mut cancel_watch = self.cancel_watch.clone();
                let mut first_event_seen = false;

                const FAST_TIMEOUT_S: u64 = 180;
                const NORMAL_TIMEOUT_S: u64 = 180;
                const WALL_CAP_S: u64 = 600;

                let fast_sleep = tokio::time::sleep(std::time::Duration::from_secs(FAST_TIMEOUT_S));
                let normal_sleep = tokio::time::sleep(std::time::Duration::from_secs(NORMAL_TIMEOUT_S));
                let wall_sleep = tokio::time::sleep(std::time::Duration::from_secs(WALL_CAP_S));
                tokio::pin!(fast_sleep);
                tokio::pin!(normal_sleep);
                tokio::pin!(wall_sleep);

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
                                    // Always emit stream events for the UI
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
                                        wall_clock_secs: Some(step_start.elapsed().as_secs()),
                                    });
                                    // Accumulate text for artifact synthesis at TurnComplete.
                                        // In legacy mode this becomes the text-dump file;
                                        // in declared mode it is used to synthesize
                                        // ArtifactProduced events for ByName declarations.
                                        //
                                        // Tool-use breadcrumbs (formatted as Text events by
                                        // the agent adapters for UI streaming) are excluded
                                        // from artifact accumulation so downstream steps
                                        // don't see "[tool write ...] Wrote file." noise.
                                        let is_tool_breadcrumb =
                                            delta.starts_with("[tool ") || delta.starts_with("[tool:");
                                        if !is_tool_breadcrumb {
                                            text_buffer.push_str(&delta);
                                        }
                                    }
                                AgentEvent::ArtifactProduced { artifact } => {
                                    produced_artifacts.push(artifact);
                                }
                                AgentEvent::Usage { cost_usd, .. } => {
                                    if let Some(c) = cost_usd {
                                        *accumulated_cost += c;
                                    }
                                }
                                AgentEvent::TurnComplete { .. } => break,
                                AgentEvent::Error { message, .. } => {
                                    let _ = self.registry.kill(self.f_id.as_str()).await;
                                    let descriptive = format_agent_error_message(&message, &machine_str, &*self.exec);
                                    return StepOutcome::Failed(descriptive);
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
                            if hb.as_ref().map_or(false, |h| h.last_activity_ago_ms() > FAST_TIMEOUT_S * 1000) {
                                eprintln!("[agent step] Fast timeout ({}s): both stdout and stderr silent — agent is blocked.", FAST_TIMEOUT_S);
                                let _ = self.registry.kill(self.f_id.as_str()).await;
                                let descriptive = format_agent_error_message(
                                    &format!("Agent blocked: no output for {}s (stdout and stderr both silent)", FAST_TIMEOUT_S),
                                    &machine_str, &*self.exec,
                                );
                                return StepOutcome::Failed(descriptive);
                            }
                            fast_sleep.as_mut().reset(
                                tokio::time::Instant::now() + std::time::Duration::from_secs(FAST_TIMEOUT_S),
                            );
                        }
                        _ = &mut normal_sleep => {
                            let elapsed = step_start.elapsed().as_secs();
                            eprintln!("[agent step] Agent silent timeout of {}s reached (wall: {}s).", NORMAL_TIMEOUT_S, elapsed);
                            let _ = self.registry.kill(self.f_id.as_str()).await;
                            let descriptive = format_agent_error_message(
                                &format!("Agent response timed out (no output for {}s)", NORMAL_TIMEOUT_S),
                                &machine_str, &*self.exec,
                            );
                            return StepOutcome::Failed(descriptive);
                        }
                        _ = &mut wall_sleep => {
                            let elapsed = step_start.elapsed().as_secs();
                            eprintln!("[agent step] Wall clock cap of {}s reached (elapsed: {}s).", WALL_CAP_S, elapsed);
                            let _ = self.registry.kill(self.f_id.as_str()).await;
                            return StepOutcome::Failed(format!(
                                "Agent step exceeded wall clock cap ({}s / {}s elapsed)",
                                WALL_CAP_S, elapsed,
                            ));
                        }
                        _ = cancel_watch.changed() => {
                            if *cancel_watch.borrow() {
                                let _ = session.cancel();
                                break;
                            }
                        }
                    }
                }

                if *self.cancel_watch.borrow() {
                    let wall = step_start.elapsed().as_secs();
                    let _ = self.features.step_update(&step_exec.id, &StepExecutionPatch {
        iteration_count: None,
                        status: Some("interrupted".to_string()),
                        cost_usd: Some(*accumulated_cost).map(|v| Some(v)),
                        wall_clock_secs: Some(wall).map(|v| Some(wall)),
                        artifact_path: None,
                        artifact_paths: None,
                        error_message: Some(Some("Execution cancelled by user".to_string())),
                    });
                    let _ = self.notif.emit(&DomainEvent::StepProgress {
                        feature_id: self.f_id.clone(),
                        step_id: step_exec.step_id.0.clone(),
                        status: "interrupted".into(),
                        cost_usd: Some(*accumulated_cost),
                        wall_clock_secs: Some(wall),
                    });
                    let _ = self.registry.kill(self.f_id.as_str()).await;
                    return StepOutcome::Cancelled;
                }

                // Detect the files the agent actually wrote during this
                // step. The strategy: compare the worktree's `git status
                // --porcelain` against the snapshot taken before the
                // agent ran, plus any path explicitly named in a
                // `LastWriteTo` declaration (so refining a previous
                // step's artifact still produces the latest body).
                //
                // The previous strategy used `git diff <base>..HEAD`,
                // which was always empty because the agent writes
                // files to the working tree without committing.
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
                        &self.target_dir,
                        &always,
                        &[],
                    );
                    for rel_path in changed {
                        let name = std::path::Path::new(&rel_path)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("artifact")
                            .to_string();
                        if let Some(content) = read_worktree_file(
                            &*self.exec,
                            &machine_str,
                            &self.target_dir,
                            &rel_path,
                        ) {
                            produced_artifacts
                                .push(Artifact::tool_write(name, rel_path, content));
                        }
                    }
                }

                // Compute the unified diff against the worktree's
                // base ref and synthesise a diff artifact. Empty for
                // text-only steps (research, spec, critic, validate
                // when the agent only writes .md) and useful for
                // implement-style steps where the agent touches code.
                //
                // Per the user-facing rule "always show a code diff",
                // we attach the diff for every non-legacy step,
                // regardless of whether code was changed. An empty
                // diff still renders harmlessly in the diff viewer.
                if !is_legacy {
                    let diff_ref = worktree_base_ref.as_deref().unwrap_or("HEAD");
                    let diff_body =
                        compute_git_diff(&*self.exec, &machine_str, &self.target_dir, diff_ref);
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

                // Resolve artifacts: either declared (new) or text-dump (legacy)
                let (artifact_path, artifact_paths) = if !is_legacy {
                    let refs = resolve_declared_artifacts(
                        decls,
                        &produced_artifacts,
                        &self.artifacts,
                        &self.f_id_str,
                        &step_exec.step_id.0,
                    );
                    // For implement-style steps (parallel kind, or any
                    // step with a `code-diff` artifact), the primary
                    // view is the unified diff so the user sees the
                    // change summary first. For text-only steps, the
                    // primary is the first detected file (research
                    // report, spec, critic review, validation report).
                    let primary = if step_conf.kind == "parallel" {
                        refs.iter()
                            .find(|r| r.contains("code-diff") || r.ends_with(".diff"))
                            .cloned()
                            .or_else(|| refs.first().cloned())
                    } else {
                        refs.first().cloned()
                    };
                    (primary, refs)
                } else {
                    let mut art_path = self.app_local_data_dir.join("artifacts").join(&self.f_id_str);
                    let _ = std::fs::create_dir_all(&art_path);
                    let file_name = format!("{}.md", step_exec.step_id.0);
                    art_path.push(&file_name);
                    let _ = std::fs::write(&art_path, &text_buffer);
                    let art_path_str = art_path.to_string_lossy().to_string();
                    (Some(art_path_str.clone()), vec![art_path_str])
                };

                let wall = step_start.elapsed().as_secs();
                let _ = self.features.step_update(&step_exec.id, &StepExecutionPatch {
        iteration_count: None,
                    status: Some("completed".to_string()),
                    cost_usd: Some(*accumulated_cost).map(|v| Some(v)),
                    wall_clock_secs: Some(wall).map(|v| Some(wall)),
                    artifact_path: Some(artifact_path),
                    artifact_paths: Some(artifact_paths),
                    error_message: Some(None),
                });
                let _ = self.notif.emit(&DomainEvent::StepProgress {
                    feature_id: self.f_id.clone(),
                    step_id: step_exec.step_id.0.clone(),
                    status: "completed".into(),
                    cost_usd: Some(*accumulated_cost),
                    wall_clock_secs: Some(wall),
                });
                let _ = self.registry.kill(self.f_id.as_str()).await;
                StepOutcome::Completed
            }
            Some(Err(e)) => {
                let descriptive = format_agent_error_message(&e.to_string(), &machine_str, &*self.exec);
                StepOutcome::Failed(descriptive)
            }
            None => StepOutcome::Cancelled,
        }
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

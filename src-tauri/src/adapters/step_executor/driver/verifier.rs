use super::ExecutionDriver;
use crate::domain::agent_event::AgentEvent;
use crate::domain::models::StepExecution;
use crate::paths;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::notification::DomainEvent;
use std::time::Instant;
use tokio_stream::StreamExt;

impl ExecutionDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_verifier_logic(
        &self,
        step_exec: &StepExecution,
        verifier_cfg: &crate::domain::verifier::VerifierConfig,
        wt_path: &str,
        produced_artifacts: &[crate::domain::artifact::Artifact],
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
        default_agent_kind: &str,
        override_model: &Option<String>,
        machine_str: &str,
    ) -> Result<(), String> {
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: "verifying".into(),
            cost_usd: Some(*accumulated_cost),
            tokens: Some(*accumulated_tokens),
            wall_clock_secs: Some(step_start.elapsed().as_secs()),
        });

        let feature = self.features.get(&self.f_id).ok().flatten();
        let mut harnesses = None;
        if let Some(ref f) = feature {
            if let Ok(Some(settings)) = self.projects.get_settings(&f.project_id) {
                harnesses = settings.worktree_strategy.harnesses;
            }
        }

        // Resolve the harness command: an explicitly named harness from project
        // settings takes priority, otherwise fall back to the project's detected
        // `test_command`. If neither is available we run no command and let the
        // verifier agent decide purely from the instructions and artifacts.
        let harness_name = verifier_cfg
            .harness_name
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let harness_cmd = verifier_cfg
            .harness_name
            .as_ref()
            .and_then(|name| harnesses.as_ref().and_then(|h| h.get(name)))
            .cloned()
            .or_else(|| {
                feature.as_ref().and_then(|f| {
                    self.projects
                        .get_settings(&f.project_id)
                        .ok()
                        .flatten()
                        .and_then(|s| s.worktree_strategy.test_command.clone())
                })
            });

        let harness_output = match harness_cmd {
            Some(ref cmd) => {
                let harness_run_cmd =
                    format!("cd {} && {}", paths::shell_escape_posix(wt_path), cmd);
                let out = match self.exec.run_command(machine_str, &harness_run_cmd).await {
                    Ok(out) => out,
                    Err(out) => out,
                };
                Some(out)
            }
            None => None,
        };

        let mut produced_artifacts_summary = String::new();
        for art in produced_artifacts {
            produced_artifacts_summary.push_str(&format!("- File/Artifact: {}\n", art.name));
        }

        let harness_section = match (&harness_cmd, &harness_output) {
            (Some(cmd), Some(output)) => format!(
                "We ran the test harness '{}' with the command '{}'.\n\
                 The output of the test command was:\n\
                 ```\n\
                 {}\n\
                 ```\n",
                harness_name, cmd, output,
            ),
            _ => "No test harness was configured or detected for this project, so no test \
                  command was run. Base your verdict on the instructions and the produced \
                  artifacts below.\n"
                .to_string(),
        };

        let verifier_prompt = format!(
            "You are a verifier agent performing a verification task.\n\n\
             Instructions:\n\
             {}\n\n\
             {}\n\
             We also produced/modified the following files/artifacts:\n\
             {}\n\n\
             Please analyze the available information and artifacts, then provide a JSON object containing the verification verdict.\n\
             The JSON object must have a key '{}' with the value either \"pass\" or \"fail\".\n\
             For example: {{ \"{}\": \"pass\" }} or {{ \"{}\": \"fail\", \"reason\": \"...\" }}.\n\
             Do not output any other text or code blocks outside the JSON.",
            verifier_cfg.instructions,
            harness_section,
            produced_artifacts_summary,
            verifier_cfg.verdict_key,
            verifier_cfg.verdict_key,
            verifier_cfg.verdict_key,
        );

        let verifier_agent_kind = verifier_cfg
            .agent_kind
            .clone()
            .unwrap_or_else(|| default_agent_kind.to_string());

        let mut agent_env = crate::ports::agent_runtime::agent_base_env();
        if let Some(ref m) = override_model {
            if verifier_agent_kind != "opencode"
                && verifier_agent_kind != "hermes"
                && verifier_agent_kind != "claude-code"
                && verifier_agent_kind != "antigravity"
            {
                let config = format!(
                    r#"{{"$schema":"https://opencode.ai/config.json","model":"{}"}}"#,
                    m
                );
                agent_env.insert("OPENCODE_CONFIG_CONTENT".to_string(), config);
            }
        }

        let verifier_thread_id = format!("{}-verifier", self.f_id_str);
        let verifier_binary = self
            .registry
            .runtime_for(&verifier_agent_kind)
            .map(|r| r.binary().to_string())
            .unwrap_or_else(|| verifier_agent_kind.clone());
        let verifier_ctx = AgentContext {
            thread_id: verifier_thread_id.clone(),
            machine_id: machine_str.to_string(),
            binary: verifier_binary,
            args: vec![],
            env: agent_env,
            cwd: wt_path.to_string(),
            model: override_model.clone(),
            title: Some(format!("Verify: {}", harness_name)),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            permissions: crate::domain::permission::PermissionProfile::all_allow(),
        };

        let spawn_fut =
            self.registry
                .get_or_spawn(&verifier_thread_id, &verifier_agent_kind, verifier_ctx);
        let mut cancel_watch_spawn = self.cancel_watch.clone();
        let spawn_res = tokio::select! {
            res = spawn_fut => Some(res),
            _ = cancel_watch_spawn.changed() => None,
        };

        let session = match spawn_res {
            Some(Ok(session)) => session,
            Some(Err(e)) => return Err(format!("Verifier spawn failed: {}", e)),
            None => return Err("Verifier spawn cancelled".to_string()),
        };

        let mut text_buffer = String::new();
        let hb = session.stderr_heartbeat();
        let mut stream = session.prompt(&verifier_prompt);
        let mut cancel_watch = self.cancel_watch.clone();
        let mut first_event_seen = false;

        const VERIFIER_TIMEOUT_S: u64 = 180;
        let fast_sleep = tokio::time::sleep(std::time::Duration::from_secs(VERIFIER_TIMEOUT_S));
        let normal_sleep = tokio::time::sleep(std::time::Duration::from_secs(VERIFIER_TIMEOUT_S));
        let wall_sleep = tokio::time::sleep(std::time::Duration::from_secs(VERIFIER_TIMEOUT_S * 2));
        tokio::pin!(fast_sleep);
        tokio::pin!(normal_sleep);
        tokio::pin!(wall_sleep);

        let mut run_failed = None;
        let mut run_cancelled = false;
        let mut latest_cost = 0.0;
        let mut latest_tokens = 0;

        loop {
            tokio::select! {
                event_opt = stream.next() => {
                    let event = match event_opt {
                        Some(ev) => ev,
                        None => break,
                    };
                    first_event_seen = true;

                    let now = tokio::time::Instant::now();
                    let next_fast = now + std::time::Duration::from_secs(VERIFIER_TIMEOUT_S);
                    let next_normal = now + std::time::Duration::from_secs(VERIFIER_TIMEOUT_S);
                    fast_sleep.as_mut().reset(next_fast);
                    normal_sleep.as_mut().reset(next_normal);

                    match event {
                        AgentEvent::Text { delta } => {
                            let _ = self.notif.emit(&DomainEvent::AgentStream {
                                feature_id: self.f_id.clone(),
                                step_execution_id: step_exec.id.clone(),
                                content: delta.clone(),
                            });
                            text_buffer.push_str(&delta);
                        }
                        AgentEvent::Usage { input_tokens, output_tokens, cost_usd } => {
                            if let Some(c) = cost_usd {
                                latest_cost = c;
                            }
                            latest_tokens = (input_tokens + output_tokens) as i64;
                        }
                        AgentEvent::TurnComplete { .. } => break,
                        AgentEvent::Error { message, .. } => {
                            run_failed = Some(format!("Verifier agent error: {}", message));
                            break;
                        }
                        _ => {}
                    }
                }
                _ = &mut fast_sleep => {
                    if !first_event_seen {
                        fast_sleep.as_mut().reset(
                            tokio::time::Instant::now() + std::time::Duration::from_secs(VERIFIER_TIMEOUT_S),
                        );
                        continue;
                    }
                    if hb.as_ref().is_some_and(|h| h.last_activity_ago_ms() > VERIFIER_TIMEOUT_S * 1000) {
                        run_failed = Some("Verifier blocked: no output (stdout and stderr silent)".to_string());
                        break;
                    }
                    fast_sleep.as_mut().reset(
                        tokio::time::Instant::now() + std::time::Duration::from_secs(VERIFIER_TIMEOUT_S),
                    );
                }
                _ = &mut normal_sleep => {
                    if let Some(ref h) = hb {
                        if h.last_activity_ago_ms() < VERIFIER_TIMEOUT_S * 1000 {
                            normal_sleep.as_mut().reset(
                                tokio::time::Instant::now() + std::time::Duration::from_secs(VERIFIER_TIMEOUT_S),
                            );
                            continue;
                        }
                    }
                    run_failed = Some("Verifier response timed out".to_string());
                    break;
                }
                _ = &mut wall_sleep => {
                    run_failed = Some("Verifier exceeded wall clock cap".to_string());
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

        let _ = self.registry.kill(&verifier_thread_id).await;

        *accumulated_cost += latest_cost;
        *accumulated_tokens += latest_tokens;

        if run_cancelled || *self.cancel_watch.borrow() {
            return Err("Verifier cancelled by user".to_string());
        }

        if let Some(err) = run_failed {
            return Err(err);
        }

        let mut parsed_val: Option<serde_json::Value> = None;

        // Find all indices of '{' and '}' in the text_buffer
        let brace_starts: Vec<usize> = text_buffer.match_indices('{').map(|(i, _)| i).collect();
        let brace_ends: Vec<usize> = text_buffer.match_indices('}').map(|(i, _)| i).collect();

        // Search from the end to find the most recent valid JSON block containing the verdict key
        'outer: for &s in brace_starts.iter().rev() {
            for &e in brace_ends.iter().rev() {
                if e > s {
                    let candidate = &text_buffer[s..=e];
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(candidate) {
                        if val.is_object() && val.get(&verifier_cfg.verdict_key).is_some() {
                            parsed_val = Some(val);
                            break 'outer;
                        }
                    }
                }
            }
        }

        let val = match parsed_val {
            Some(v) => v,
            None => {
                // Fall back to original extraction logic if robust parsing fails, to provide a clear error message.
                let start = text_buffer.find('{');
                let end = text_buffer.rfind('}');
                let json_str = if let (Some(s), Some(e)) = (start, end) {
                    if s < e {
                        &text_buffer[s..=e]
                    } else {
                        text_buffer.trim()
                    }
                } else {
                    text_buffer.trim()
                };
                serde_json::from_str(json_str).map_err(|e| {
                    format!(
                        "Failed to parse verifier output JSON: {} (raw: {})",
                        e, json_str
                    )
                })?
            }
        };

        let verdict_str = val
            .get(&verifier_cfg.verdict_key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                format!(
                    "Verifier output missing verdict key '{}'",
                    verifier_cfg.verdict_key
                )
            })?;

        match verdict_str.to_lowercase().as_str() {
            "pass" => Ok(()),
            "fail" => {
                let reason = val
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Verifier check failed (no reason provided)");
                Err(reason.to_string())
            }
            other => Err(format!("Invalid verifier verdict: '{}'", other)),
        }
    }
}

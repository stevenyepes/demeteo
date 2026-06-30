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
    ) -> Result<(), crate::domain::verifier::VerifierError> {
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: "verifying".into(),
            cost_usd: Some(*accumulated_cost),
            tokens: Some(*accumulated_tokens),
            wall_clock_secs: Some(step_start.elapsed().as_secs()),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
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

        // Run the harness and capture both the output and whether it succeeded.
        // A non-zero exit code is an objective signal — we fail immediately
        // without invoking the verifier agent, so the agent can't "pass" a
        // broken build. The exit status from `run_command`: Ok → exit 0,
        // Err → non-zero exit or I/O error (both are failures).
        let harness_result: Option<(String, bool)> = match harness_cmd {
            Some(ref cmd) => {
                // Restore write permissions before running the harness. The
                // capability scope fence (`apply_artifact_scope`) ran before the
                // agent spawned and left most of the worktree `a-w`. The agent is
                // done at this point, so the fence has served its purpose. Build
                // tools (cargo, npm, tsc) need to write to target/, node_modules/,
                // etc. — without this they fail with EPERM, not a real test failure.
                let _ = self
                    .exec
                    .run_command(
                        machine_str,
                        &format!(
                            "chmod -R u+w {} 2>/dev/null || true",
                            paths::shell_escape_posix(wt_path)
                        ),
                    )
                    .await;
                let harness_run_cmd =
                    format!("cd {} && {}", paths::shell_escape_posix(wt_path), cmd);
                match self.exec.run_command(machine_str, &harness_run_cmd).await {
                    Ok(out) => Some((out, true)),
                    Err(out) => Some((out, false)),
                }
            }
            None => None,
        };

        // Hard gate: if the harness exited non-zero, fail as a Verdict so the
        // reason feeds back into the retry loop. The verifier agent is skipped —
        // its job is interpretation, not override of an objective exit code.
        if let Some((ref out, false)) = harness_result {
            let truncated: String = out.chars().take(2000).collect();
            return Err(crate::domain::verifier::VerifierError::Verdict(format!(
                "test harness exited with failure:\n{}",
                truncated
            )));
        }

        let mut produced_artifacts_summary = String::new();
        for art in produced_artifacts {
            produced_artifacts_summary.push_str(&format!("- File/Artifact: {}\n", art.name));
        }

        let harness_section = match (&harness_cmd, &harness_result) {
            (Some(cmd), Some((output, _))) => format!(
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
            bare_mode: verifier_agent_kind == "claude-code",
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
            Some(Err(e)) => {
                return Err(crate::domain::verifier::VerifierError::Infrastructure(
                    format!("Verifier spawn failed: {}", e),
                ))
            }
            None => {
                return Err(crate::domain::verifier::VerifierError::Infrastructure(
                    "Verifier spawn cancelled".to_string(),
                ))
            }
        };

        let mut text_buffer = String::new();
        let hb = session.stderr_heartbeat();
        let mut stream = session.prompt(&verifier_prompt);
        let mut cancel_watch = self.cancel_watch.clone();
        let mut first_event_seen = false;

        let verifier_timeouts =
            crate::application::timeouts::resolve_effective(self.app_settings.as_ref());
        let fast_s = verifier_timeouts.fast_timeout_s;
        let normal_s = verifier_timeouts.normal_timeout_s;
        let wall_s = verifier_timeouts.wall_cap_s;
        let fast_sleep = tokio::time::sleep(std::time::Duration::from_secs(fast_s));
        let normal_sleep = tokio::time::sleep(std::time::Duration::from_secs(normal_s));
        let wall_sleep = tokio::time::sleep(std::time::Duration::from_secs(wall_s));
        tokio::pin!(fast_sleep);
        tokio::pin!(normal_sleep);
        tokio::pin!(wall_sleep);

        let mut run_failed = None;
        let mut run_cancelled = false;
        let mut usage_acc = crate::domain::usage::UsageAccumulator::new(override_model.clone());

        loop {
            tokio::select! {
                event_opt = stream.next() => {
                    let event = match event_opt {
                        Some(ev) => ev,
                        None => break,
                    };
                    first_event_seen = true;

                    let now = tokio::time::Instant::now();
                    let next_fast = now + std::time::Duration::from_secs(fast_s);
                    let next_normal = now + std::time::Duration::from_secs(normal_s);
                    fast_sleep.as_mut().reset(next_fast);
                    normal_sleep.as_mut().reset(next_normal);

                    match &event {
                        AgentEvent::Text { delta } => {
                            let _ = self.notif.emit(&DomainEvent::AgentStream {
                                feature_id: self.f_id.clone(),
                                step_execution_id: step_exec.id.clone(),
                                content: delta.clone(),
                            });
                            text_buffer.push_str(delta);
                        }
                        AgentEvent::TurnComplete { .. } => break,
                        AgentEvent::Error { message, .. } => {
                            run_failed = Some(format!("Verifier agent error: {}", message));
                            break;
                        }
                        _ => {}
                    }

                    usage_acc.ingest_event(&event);
                }
                _ = &mut fast_sleep => {
                    if !first_event_seen {
                        fast_sleep.as_mut().reset(
                            tokio::time::Instant::now() + std::time::Duration::from_secs(fast_s),
                        );
                        continue;
                    }
                    if hb.as_ref().is_some_and(|h| h.last_activity_ago_ms() > fast_s * 1000) {
                        run_failed = Some("Verifier blocked: no output (stdout and stderr silent)".to_string());
                        break;
                    }
                    fast_sleep.as_mut().reset(
                        tokio::time::Instant::now() + std::time::Duration::from_secs(fast_s),
                    );
                }
                _ = &mut normal_sleep => {
                    if let Some(ref h) = hb {
                        if h.last_activity_ago_ms() < normal_s * 1000 {
                            normal_sleep.as_mut().reset(
                                tokio::time::Instant::now() + std::time::Duration::from_secs(normal_s),
                            );
                            continue;
                        }
                    }
                    run_failed = Some("Verifier response timed out".to_string());
                    break;
                }
                _ = &mut wall_sleep => {
                    run_failed = Some(format!(
                        "Verifier exceeded wall clock cap ({}s)",
                        wall_s
                    ));
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

        usage_acc.finalize_arc(&self.pricing);
        *accumulated_cost += usage_acc.cost_usd();
        *accumulated_tokens += usage_acc.tokens();

        if run_cancelled || *self.cancel_watch.borrow() {
            return Err(crate::domain::verifier::VerifierError::Infrastructure(
                "Verifier cancelled by user".to_string(),
            ));
        }

        if let Some(err) = run_failed {
            return Err(crate::domain::verifier::VerifierError::Infrastructure(err));
        }

        // Strip extended-thinking tags before JSON parsing — verifier agents
        // using thinking mode emit <think>…</think> as raw text and the parser
        // would otherwise trip over them or include them in the JSON search.
        let text_buffer = crate::domain::text::strip_think_tags(&text_buffer);

        // Walk forward through every {…} span. For each balanced span:
        //   - Valid JSON with the verdict key → record it, skip past the span.
        //   - Valid JSON without the verdict key → step forward by 1 so inner
        //     nested objects are independently evaluated (handles models that
        //     wrap the verdict in an outer object like {"result": {"verdict":"pass"}}).
        //   - Malformed JSON → skip past the span to avoid O(n²) re-parsing.
        let mut parsed_val: Option<serde_json::Value> = None;
        let bytes = text_buffer.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'{' {
                if let Some(close) = find_matching_close_brace(bytes, i) {
                    match serde_json::from_str::<serde_json::Value>(&text_buffer[i..=close]) {
                        Ok(val)
                            if val.is_object() && val.get(&verifier_cfg.verdict_key).is_some() =>
                        {
                            parsed_val = Some(val);
                            i = close + 1;
                            continue;
                        }
                        Ok(_) => {
                            // Valid JSON but no verdict key at top level; step
                            // forward by 1 so inner objects get evaluated.
                        }
                        Err(_) => {
                            // Balanced braces but not valid JSON; skip the span.
                            i = close + 1;
                            continue;
                        }
                    }
                }
            }
            i += 1;
        }

        let val = match parsed_val {
            Some(v) => v,
            None => {
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
                    crate::domain::verifier::VerifierError::Infrastructure(format!(
                        "Failed to parse verifier output JSON: {} (raw: {})",
                        e, json_str
                    ))
                })?
            }
        };

        let verdict_str = val
            .get(&verifier_cfg.verdict_key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::domain::verifier::VerifierError::Infrastructure(format!(
                    "Verifier output missing verdict key '{}'",
                    verifier_cfg.verdict_key
                ))
            })?;

        match verdict_str.to_lowercase().as_str() {
            "pass" => {
                tracing::info!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    "verifier verdict: pass"
                );
                Ok(())
            }
            "fail" => {
                let reason = val
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Verifier check failed (no reason provided)");
                tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    reason = %reason,
                    "verifier verdict: fail"
                );
                Err(crate::domain::verifier::VerifierError::Verdict(
                    reason.to_string(),
                ))
            }
            other => {
                tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    verdict = %other,
                    "verifier infrastructure error: unrecognised verdict"
                );
                Err(crate::domain::verifier::VerifierError::Infrastructure(
                    format!("Invalid verifier verdict: '{}'", other),
                ))
            }
        }
    }
}

/// Find the index of the `}` that closes the `{` at `start` in `bytes`,
/// correctly skipping over string literals (including escaped characters).
/// Returns `None` if the braces are unbalanced.
fn find_matching_close_brace(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut in_str = false;
    let mut escaped = false;
    for (offset, &b) in bytes[start..].iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_str {
            match b {
                b'\\' => escaped = true,
                b'"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

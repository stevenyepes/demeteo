use super::ExecutionDriver;
use crate::domain::agent_event::AgentEvent;
use crate::domain::models::StepExecution;
use crate::paths;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::notification::DomainEvent;
use std::time::Instant;
use tokio_stream::StreamExt;

impl ExecutionDriver {
    /// Resolve and run the project's prepare command + test harness inside
    /// `wt_path`, and return the formatted "harness results" section for an
    /// agent prompt.
    ///
    /// This is the harness-first primitive: it runs **before** any agent
    /// turn, so a red harness fails the step objectively at zero token
    /// cost, and a green harness's output is injected into the single
    /// validate turn instead of paying for the agent to re-run the same
    /// commands (which the capability chmod fence would block with EPERM
    /// anyway — build tools need to write `target/`, `node_modules/`, …).
    ///
    /// Errors:
    /// * prepare or harness exits non-zero → [`VerifierError::Verdict`]
    ///   with the output tail as the actionable reason (feeds the
    ///   on_failure retry loop).
    pub(crate) async fn run_harness_first(
        &self,
        verifier_cfg: &crate::domain::verifier::VerifierConfig,
        wt_path: &str,
        machine_str: &str,
    ) -> Result<String, crate::domain::verifier::VerifierError> {
        let feature = self.features.get(&self.f_id).ok().flatten();
        let settings = feature
            .as_ref()
            .and_then(|f| self.projects.get_settings(&f.project_id).ok().flatten());
        let harnesses = settings
            .as_ref()
            .and_then(|s| s.worktree_strategy.harnesses.clone());
        let prepare_command = settings
            .as_ref()
            .and_then(|s| s.worktree_strategy.prepare_command.clone());

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
                settings
                    .as_ref()
                    .and_then(|s| s.worktree_strategy.test_command.clone())
            });

        // Idempotent write-restore. Fresh worktrees are writable, but a
        // retried step may run in a worktree the fence already touched.
        if prepare_command.is_some() || harness_cmd.is_some() {
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
        }

        // Run the prepare/harness commands in the *same* shell mode the coding
        // agent ran in, with the worktree as an explicit cwd (D2 — never rely on
        // ambient state). On a machine flagged `use_login_shell`, the agent was
        // spawned under an interactive login shell so `mise`/`asdf`/`nvm`-shimmed
        // toolchains are on `PATH`; the harness gate must see the identical
        // environment, or a project whose `npm test`/`pytest` the agent ran fine
        // would fail here with "command not found" only on remote.
        let opts = self.harness_shell_options(machine_str, wt_path);

        if let Some(ref cmd) = prepare_command {
            if let Err(out) = self
                .exec
                .run_command_with(machine_str, cmd, opts.clone())
                .await
            {
                // A transport failure (unreachable machine, dropped channel,
                // drain timeout) is not a red build — surface it as
                // Infrastructure (non-retryable) instead of a Verdict that
                // would pointlessly re-run the same command. See C0.2 / D3.
                if is_transport_failure(&out) {
                    return Err(crate::domain::verifier::VerifierError::Infrastructure(
                        format!("prepare command '{}' could not run: {}", cmd, out),
                    ));
                }
                let truncated = tail_chars(&out, 2000);
                return Err(crate::domain::verifier::VerifierError::Verdict(
                    crate::domain::verifier::VerdictFailure::from_reason(format!(
                        "prepare command '{}' exited with failure:\n{}",
                        cmd, truncated
                    )),
                ));
            }
        }

        let harness_result: Option<(String, bool)> = match harness_cmd {
            Some(ref cmd) => match self
                .exec
                .run_command_with(machine_str, cmd, opts.clone())
                .await
            {
                Ok(out) => Some((out, true)),
                // A transport failure is infrastructure, not a red harness —
                // don't gate a Verdict on it (C0.2 / D3).
                Err(out) if is_transport_failure(&out) => {
                    return Err(crate::domain::verifier::VerifierError::Infrastructure(
                        format!("test harness '{}' could not run: {}", cmd, out),
                    ))
                }
                Err(out) => Some((out, false)),
            },
            None => None,
        };

        // Hard gate: non-zero exit is objective — fail without any agent
        // involvement so nothing can "pass" a broken build.
        if let Some((ref out, false)) = harness_result {
            let truncated = tail_chars(out, 2000);
            return Err(crate::domain::verifier::VerifierError::Verdict(
                crate::domain::verifier::VerdictFailure::from_reason(format!(
                    "test harness exited with failure:\n{}",
                    truncated
                )),
            ));
        }

        Ok(match (&harness_cmd, &harness_result) {
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
        })
    }

    /// Build the [`ShellOptions`](crate::ports::execution::ShellOptions) the
    /// prepare/test harness runs under: the worktree as an explicit cwd, and
    /// login mode mirroring the coding agent's spawn. Resolving the machine's
    /// `use_login_shell` here (not baking a `cd … &&` string) is the D2
    /// "explicit context" move — the harness gate and the agent see the same
    /// `PATH`, so a toolchain the agent used never silently vanishes for the
    /// gate on a remote box. A missing/unknown machine (or `local`) resolves to
    /// a plain non-login shell, matching the historical default.
    fn harness_shell_options(
        &self,
        machine_str: &str,
        wt_path: &str,
    ) -> crate::ports::execution::ShellOptions {
        let use_login = crate::infrastructure::worktree::machine_resolver::resolve_machine(
            &*self.machines,
            machine_str,
        )
        .ok()
        .and_then(|m| m.use_login_shell)
        .unwrap_or(false);
        crate::ports::execution::ShellOptions {
            login_shell: use_login,
            // Interactive matches `spawn_interactive`: mise/asdf/nvm activate in
            // `~/.bashrc` behind the non-interactive guard, so only `-i` puts
            // their shims on PATH. Off when not a login shell.
            interactive: use_login,
            cwd: Some(wt_path.to_string()),
            env: Default::default(),
        }
    }

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

        // Resolve + run prepare and harness via the shared harness-first
        // primitive. A non-zero exit propagates as a Verdict failure
        // before any verifier agent spawns.
        let harness_name = verifier_cfg
            .harness_name
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let harness_section = self
            .run_harness_first(verifier_cfg, wt_path, machine_str)
            .await?;

        let produced_artifacts_summary = format_produced_artifacts_summary(produced_artifacts);

        let verifier_prompt = format!(
            "You are a verifier agent performing a verification task.\n\n\
             Instructions:\n\
             {}\n\n\
             {}\n\
             We also produced/modified the following files/artifacts:\n\
             {}\n\n\
             Please analyze the available information and artifacts, then provide a JSON object containing the verification verdict.\n\
             The JSON object must have a key '{}' with the value either \"pass\" or \"fail\".\n\
             On \"fail\", also include:\n\
             - \"reason\": a concise, actionable description naming exactly what to fix\n\
             - \"failing_tests\": an array of failing test identifiers, verbatim from the harness output ([] if none)\n\
             - \"implicated_files\": an array of repo-relative file paths that most likely must change to fix the failure ([] if unknown)\n\
             For example: {{ \"{}\": \"pass\" }} or {{ \"{}\": \"fail\", \"reason\": \"...\", \"failing_tests\": [\"...\"], \"implicated_files\": [\"src/foo.rs\"] }}.\n\
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

        // Verifier-specific model override. Interpreting harness output
        // into one verdict object is a small-model job; a cheap model
        // here cuts the recurring cost of every retry loop.
        let verifier_model: Option<String> = verifier_cfg
            .model
            .clone()
            .or_else(|| override_model.clone());

        let mut agent_env =
            crate::ports::agent_runtime::agent_base_env(self.exec.as_ref(), machine_str).await;
        if let Some(ref m) = verifier_model {
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
            model: verifier_model.clone(),
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
        let mut usage_acc = crate::domain::usage::UsageAccumulator::new(verifier_model.clone());

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

        match parse_verdict_text(&text_buffer, &verifier_cfg.verdict_key) {
            ParsedVerdict::Pass => {
                tracing::info!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    "verifier verdict: pass"
                );
                Ok(())
            }
            ParsedVerdict::Fail(failure) => {
                tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    reason = %failure.reason,
                    "verifier verdict: fail"
                );
                Err(crate::domain::verifier::VerifierError::Verdict(failure))
            }
            ParsedVerdict::Missing(desc) => {
                tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    desc = %desc,
                    "verifier infrastructure error: unusable verdict"
                );
                Err(crate::domain::verifier::VerifierError::Infrastructure(desc))
            }
        }
    }
}

/// Result of scanning free text for a verdict JSON object.
pub(crate) enum ParsedVerdict {
    Pass,
    Fail(crate::domain::verifier::VerdictFailure),
    /// No JSON object carrying the verdict key was found, or its value
    /// was neither "pass" nor "fail". The string describes the problem.
    Missing(String),
}

/// Scan `raw_text` (a full agent turn's text output) for a JSON object
/// carrying `verdict_key`. Tolerates prose around the JSON, fenced code
/// blocks, extended-thinking tags, and verdicts nested one level deep.
///
/// Shared by the dedicated verifier turn (parallel steps) and the
/// harness-first single-turn validate path (agent steps), so both parse
/// the wire contract identically.
pub(crate) fn parse_verdict_text(raw_text: &str, verdict_key: &str) -> ParsedVerdict {
    // Strip extended-thinking tags before JSON parsing — agents using
    // thinking mode emit <think>…</think> as raw text and the parser
    // would otherwise trip over them or include them in the JSON search.
    let text_buffer = crate::domain::text::strip_think_tags(raw_text);

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
                    Ok(val) if val.is_object() && val.get(verdict_key).is_some() => {
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
            match serde_json::from_str(json_str) {
                Ok(v) => v,
                Err(e) => {
                    return ParsedVerdict::Missing(format!(
                        "Failed to parse verifier output JSON: {} (raw: {})",
                        e,
                        tail_chars(json_str, 500)
                    ))
                }
            }
        }
    };

    let Some(verdict_str) = val.get(verdict_key).and_then(|v| v.as_str()) else {
        return ParsedVerdict::Missing(format!(
            "Verifier output missing verdict key '{}'",
            verdict_key
        ));
    };

    match verdict_str.to_lowercase().as_str() {
        "pass" => ParsedVerdict::Pass,
        "fail" => {
            let reason = val
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("Verifier check failed (no reason provided)");
            let string_list = |key: &str| -> Vec<String> {
                val.get(key)
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default()
            };
            ParsedVerdict::Fail(crate::domain::verifier::VerdictFailure {
                reason: reason.to_string(),
                failing_tests: string_list("failing_tests"),
                implicated_files: string_list("implicated_files"),
            })
        }
        other => ParsedVerdict::Missing(format!("Invalid verifier verdict: '{}'", other)),
    }
}

/// Build the "we also produced/modified the following files/artifacts"
/// section of the verifier prompt. For `ToolWrite`-sourced artifacts
/// (the common case: a report the step's own agent turn wrote via
/// `LastWriteTo`, e.g. `validation-report.md`), point the verifier at
/// the actual worktree-relative path and tell it to `Read` the file —
/// its `cwd` is the same worktree, so the path resolves directly. Without
/// this, the verifier only ever saw a bare artifact name it had no way
/// to locate, so its judgment was effectively limited to the harness
/// output plus generic instructions — none of the rich analysis the
/// step's own agent turn produced (critic-issue cross-checks, security
/// audit findings, etc.) ever reached the verdict.
///
/// Other artifact sources (`Diff`, `AgentText`, …) fall back to the
/// bare-name line — a `Diff` artifact in particular is never written to
/// disk in the worktree, so there's no path to point at.
fn format_produced_artifacts_summary(
    produced_artifacts: &[crate::domain::artifact::Artifact],
) -> String {
    let mut summary = String::new();
    for art in produced_artifacts {
        match &art.source {
            crate::domain::artifact::ArtifactSource::ToolWrite { path } => {
                summary.push_str(&format!(
                    "- `{}` (artifact: {}) — use your Read tool to inspect the full content\n",
                    path, art.name
                ));
            }
            _ => {
                summary.push_str(&format!("- File/Artifact: {}\n", art.name));
            }
        }
    }
    summary
}

/// Whether an `ExecutionPort` error string denotes a *transport* failure
/// (the machine could not be reached, the channel broke, or the drain timed
/// out) rather than a *command* failure (it ran and exited non-zero). Keyed
/// off the [`TRANSPORT_ERROR_PREFIX`](crate::ports::execution::TRANSPORT_ERROR_PREFIX)
/// contract (C0.2, `docs/EXECUTION_CONSISTENCY_PLAN.md`) so the verifier can
/// route transport failures to `Infrastructure` (non-retryable) instead of a
/// `Verdict` that would re-run a build that never actually failed.
fn is_transport_failure(err: &str) -> bool {
    err.starts_with(crate::ports::execution::TRANSPORT_ERROR_PREFIX)
}

/// Truncate `s` to at most `max` characters, keeping the *tail* rather
/// than the head. Build/test failures are almost always at the end of
/// the output (the failing assertion, the panic message, the compiler
/// error) — a long build log's useful signal is at the bottom. Keeping
/// the head instead (the previous behavior) surfaced the install/build
/// banner and truncated away exactly the information the retry loop
/// needs to act on.
fn tail_chars(s: &str, max: usize) -> String {
    let total = s.chars().count();
    if total <= max {
        return s.to_string();
    }
    let skip = total - max;
    s.chars().skip(skip).collect()
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

#[cfg(test)]
mod produced_artifacts_summary_tests {
    use super::format_produced_artifacts_summary;
    use crate::domain::artifact::{Artifact, ArtifactSource};

    #[test]
    fn tool_write_artifact_points_at_its_worktree_path() {
        let arts = vec![Artifact::tool_write(
            "validation-report",
            "artifacts/validation-report.md",
            "Overall: READY TO SHIP".to_string(),
        )];
        let summary = format_produced_artifacts_summary(&arts);
        assert!(
            summary.contains("artifacts/validation-report.md"),
            "expected the worktree-relative path, got: {summary}"
        );
        assert!(
            summary.contains("Read"),
            "expected an instruction to Read the file, got: {summary}"
        );
    }

    #[test]
    fn non_tool_write_artifact_falls_back_to_bare_name() {
        let arts = vec![Artifact {
            name: "code-diff".to_string(),
            mime: "text/x-diff".into(),
            content: "diff --git a/x b/x".to_string(),
            source: ArtifactSource::Diff {
                base: "abc123".to_string(),
                head: "WORKTREE".to_string(),
                path_filter: None,
            },
        }];
        let summary = format_produced_artifacts_summary(&arts);
        assert!(summary.contains("File/Artifact: code-diff"));
    }

    #[test]
    fn empty_input_produces_empty_summary() {
        assert_eq!(format_produced_artifacts_summary(&[]), "");
    }

    #[test]
    fn multiple_artifacts_each_get_their_own_line() {
        let arts = vec![
            Artifact::tool_write("validation-report", "artifacts/validation-report.md", "x"),
            Artifact::tool_write("critic-review", "artifacts/critic-review.md", "y"),
        ];
        let summary = format_produced_artifacts_summary(&arts);
        assert_eq!(summary.lines().count(), 2);
        assert!(summary.contains("artifacts/validation-report.md"));
        assert!(summary.contains("artifacts/critic-review.md"));
    }
}

#[cfg(test)]
mod tail_chars_tests {
    use super::tail_chars;

    #[test]
    fn returns_input_unchanged_when_under_limit() {
        assert_eq!(tail_chars("short output", 2000), "short output");
    }

    #[test]
    fn returns_input_unchanged_when_exactly_at_limit() {
        let s = "x".repeat(2000);
        assert_eq!(tail_chars(&s, 2000), s);
    }

    #[test]
    fn keeps_the_tail_not_the_head() {
        // The failing assertion lives at the end of a long build log —
        // the truncated output must contain it, not the install banner.
        let head = "npm install banner...\n".repeat(200);
        let tail =
            "\nFAIL src/foo.test.ts\n  ✕ should do the thing\nAssertionError: expected 1 to be 2";
        let full = format!("{head}{tail}");
        let max = tail.chars().count();
        let truncated = tail_chars(&full, max);
        assert_eq!(
            truncated, tail,
            "expected exactly the tail (no banner leakage) when max == tail length"
        );
    }

    #[test]
    fn truncated_length_matches_max() {
        let s = "a".repeat(5000);
        let truncated = tail_chars(&s, 2000);
        assert_eq!(truncated.chars().count(), 2000);
    }

    #[test]
    fn respects_char_boundaries_with_multibyte_content() {
        // Every char is 3 bytes (multi-byte UTF-8); a byte-oriented slice
        // (e.g. naive `s[s.len() - max..]`) would panic mid-character.
        let s = "€".repeat(3000);
        let truncated = tail_chars(&s, 2000);
        assert_eq!(truncated.chars().count(), 2000);
        assert!(truncated.chars().all(|c| c == '€'));
    }
}

#[cfg(test)]
mod is_transport_failure_tests {
    use super::is_transport_failure;
    use crate::ports::execution::TRANSPORT_ERROR_PREFIX;

    #[test]
    fn transport_prefixed_error_is_transport() {
        let err = format!("{}Timed out after the transport wall cap (1800s)", TRANSPORT_ERROR_PREFIX);
        assert!(is_transport_failure(&err));
    }

    #[test]
    fn command_failure_is_not_transport() {
        // The non-zero-exit path ("Command failed (...)") carries no prefix,
        // so it stays a Verdict (a real red build the retry loop should act on).
        assert!(!is_transport_failure(
            "Command failed (exit code: 1): cd src-tauri && cargo test"
        ));
    }
}

#[cfg(test)]
mod parse_verdict_text_tests {
    use super::{parse_verdict_text, ParsedVerdict};

    #[test]
    fn pass_verdict_amid_prose() {
        let text = "Report written to artifacts/validation-report.md.\n\n{ \"verdict\": \"pass\" }";
        assert!(matches!(
            parse_verdict_text(text, "verdict"),
            ParsedVerdict::Pass
        ));
    }

    #[test]
    fn fail_verdict_carries_structured_fields() {
        let text = r#"Done. {"verdict": "fail", "reason": "auth test broken", "failing_tests": ["auth::login_works"], "implicated_files": ["src/auth.rs"]}"#;
        match parse_verdict_text(text, "verdict") {
            ParsedVerdict::Fail(vf) => {
                assert_eq!(vf.reason, "auth test broken");
                assert_eq!(vf.failing_tests, vec!["auth::login_works"]);
                assert_eq!(vf.implicated_files, vec!["src/auth.rs"]);
            }
            _ => panic!("expected fail verdict"),
        }
    }

    #[test]
    fn fail_without_lists_defaults_to_empty() {
        let text = r#"{"verdict": "fail", "reason": "nope"}"#;
        match parse_verdict_text(text, "verdict") {
            ParsedVerdict::Fail(vf) => {
                assert!(vf.failing_tests.is_empty());
                assert!(vf.implicated_files.is_empty());
            }
            _ => panic!("expected fail verdict"),
        }
    }

    #[test]
    fn nested_verdict_object_is_found() {
        let text = r#"{"result": {"verdict": "pass"}}"#;
        assert!(matches!(
            parse_verdict_text(text, "verdict"),
            ParsedVerdict::Pass
        ));
    }

    #[test]
    fn missing_verdict_reports_missing() {
        assert!(matches!(
            parse_verdict_text("all good, ship it!", "verdict"),
            ParsedVerdict::Missing(_)
        ));
    }

    #[test]
    fn invalid_verdict_value_reports_missing() {
        assert!(matches!(
            parse_verdict_text(r#"{"verdict": "maybe"}"#, "verdict"),
            ParsedVerdict::Missing(_)
        ));
    }

    #[test]
    fn think_tags_are_stripped_before_parsing() {
        let text = "<think>{\"verdict\": \"fail\"} draft</think>{\"verdict\": \"pass\"}";
        assert!(matches!(
            parse_verdict_text(text, "verdict"),
            ParsedVerdict::Pass
        ));
    }
}

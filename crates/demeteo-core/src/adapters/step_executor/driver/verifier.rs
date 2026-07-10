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
    ///   on_failure retry loop) — **unless** the failure reproduces the
    ///   previous attempt's failure unchanged, in which case a triage agent
    ///   (C6) may reclassify it as [`VerifierError::Environment`] (terminal).
    /// * a transport failure → [`VerifierError::Infrastructure`].
    pub(crate) async fn run_harness_first(
        &self,
        step_exec: &StepExecution,
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
                return Err(self
                    .classify_harness_failure(step_exec, machine_str, wt_path, cmd, &out)
                    .await);
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
        // involvement so nothing can "pass" a broken build. On a *persistent*
        // (reproduces-unchanged) failure the triage agent may reclassify this
        // as an environment problem rather than a code regression (C6).
        if let Some((ref out, false)) = harness_result {
            let cmd = harness_cmd.as_deref().unwrap_or("test harness");
            return Err(self
                .classify_harness_failure(step_exec, machine_str, wt_path, cmd, out)
                .await);
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

    /// Turn a non-transport prepare/harness failure into the right
    /// [`VerifierError`](crate::domain::verifier::VerifierError) (C6/D7).
    ///
    /// On first sight — or when the failing output *changed* from the previous
    /// attempt — it is a plain
    /// [`Verdict`](crate::domain::verifier::VerifierError::Verdict) that feeds
    /// the `on_failure` retry loop, and we persist a normalized fingerprint of
    /// the output so the *next* attempt can tell whether it reproduced. When it
    /// reproduces unchanged (persistent), a triage agent decides regression vs.
    /// environment; only a confident `environment` verdict escalates to the
    /// terminal
    /// [`Environment`](crate::domain::verifier::VerifierError::Environment).
    /// **Every other outcome** — regression, agent spawn/timeout/parse failure,
    /// unknown category — falls safe back to `Verdict`, so a broken triage can
    /// only ever *withhold* the remaining retries, never wrongly terminate a
    /// real regression.
    async fn classify_harness_failure(
        &self,
        step_exec: &StepExecution,
        machine_str: &str,
        wt_path: &str,
        cmd: &str,
        output: &str,
    ) -> crate::domain::verifier::VerifierError {
        let current_fp = normalize_failure_fingerprint(output, wt_path);
        let persistent = should_triage(step_exec.last_failure_fingerprint.as_deref(), &current_fp);

        // Persist the fingerprint for the next attempt's comparison. Harmless
        // on the terminal path; load-bearing on the retry path (the retry lands
        // back in this same step row via `on_failure`, and the driver reloads
        // `step_exec` fresh each dispatch, so the value is visible next time).
        let _ = self.features.step_update(
            &step_exec.id,
            &crate::ports::db::StepExecutionPatch {
                last_failure_fingerprint: Some(Some(current_fp)),
                ..Default::default()
            },
        );

        let truncated = tail_chars(output, 2000);
        let verdict = crate::domain::verifier::VerifierError::Verdict(
            crate::domain::verifier::VerdictFailure::from_reason(format!(
                "command '{}' exited with failure:\n{}",
                cmd, truncated
            )),
        );

        if !persistent {
            // First sight, or the error changed across the retry — treat it as
            // ongoing progress and let the implement loop keep working. No
            // triage call on attempt 1 (C6.2 DoD).
            return verdict;
        }

        // Reproduced unchanged → consult the classifier. Any non-`environment`
        // answer falls back to `verdict`.
        match self
            .triage_harness_failure(machine_str, wt_path, cmd, output)
            .await
        {
            TriageVerdict::Environment {
                reason,
                remediation,
            } => {
                let msg =
                    build_environment_message(machine_str, wt_path, cmd, &reason, &remediation);
                self.notify_environment_not_ready(step_exec, &msg);
                tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    cmd = %cmd,
                    "harness failure triaged as environment — terminating without further retries"
                );
                crate::domain::verifier::VerifierError::Environment(msg)
            }
            TriageVerdict::Regression => verdict,
        }
    }

    /// Spawn a small classifier agent to decide regression vs. environment for
    /// a *persistent* harness failure. Reuses the verifier's cheap-model
    /// plumbing. Fails safe: any spawn/timeout/cancel/parse error returns
    /// [`TriageVerdict::Regression`], so a broken triage can only ever withhold
    /// an escalation, never manufacture one.
    async fn triage_harness_failure(
        &self,
        machine_str: &str,
        wt_path: &str,
        cmd: &str,
        output: &str,
    ) -> TriageVerdict {
        let agent_kind = self
            .feature_agent_kind
            .clone()
            .or_else(|| self.default_agent_kind.clone())
            .unwrap_or_else(|| "claude-code".to_string());
        let model = self
            .feature_model
            .clone()
            .or_else(|| self.default_model.clone());

        let prompt = build_triage_prompt(machine_str, wt_path, cmd, &tail_chars(output, 4000));

        // Every supported agent is a CLI runtime that takes its model via the
        // `--model` flag in `build_args` from `ctx.model` below.
        let agent_env =
            crate::ports::agent_runtime::agent_base_env(self.exec.as_ref(), machine_str).await;

        let thread_id = format!("{}-triage", self.f_id_str);
        let binary = self
            .registry
            .runtime_for(&agent_kind)
            .map(|r| r.binary().to_string())
            .unwrap_or_else(|| agent_kind.clone());
        let ctx = AgentContext {
            thread_id: thread_id.clone(),
            machine_id: machine_str.to_string(),
            binary,
            args: vec![],
            env: agent_env,
            cwd: wt_path.to_string(),
            model,
            title: Some("Triage harness failure".to_string()),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            permissions: crate::domain::permission::PermissionProfile::all_allow(),
            bare_mode: agent_kind == "claude-code",
        };

        let spawn_fut = self.registry.get_or_spawn(&thread_id, &agent_kind, ctx);
        let mut cancel_spawn = self.cancel_watch.clone();
        let session = match tokio::select! {
            res = spawn_fut => Some(res),
            _ = cancel_spawn.changed() => None,
        } {
            Some(Ok(session)) => session,
            _ => return TriageVerdict::Regression,
        };

        let timeouts = crate::application::timeouts::resolve_effective(self.app_settings.as_ref());
        let idle_s = timeouts.normal_timeout_s;
        let wall_s = timeouts.wall_cap_s;
        let idle_sleep = tokio::time::sleep(std::time::Duration::from_secs(idle_s));
        let wall_sleep = tokio::time::sleep(std::time::Duration::from_secs(wall_s));
        tokio::pin!(idle_sleep);
        tokio::pin!(wall_sleep);

        let mut text = String::new();
        let mut stream = session.prompt(&prompt);
        let mut cancel_watch = self.cancel_watch.clone();

        let verdict = loop {
            tokio::select! {
                ev = stream.next() => {
                    idle_sleep
                        .as_mut()
                        .reset(tokio::time::Instant::now() + std::time::Duration::from_secs(idle_s));
                    match ev {
                        Some(AgentEvent::Text { delta }) => text.push_str(&delta),
                        Some(AgentEvent::TurnComplete { .. }) | None => break parse_triage_text(&text),
                        Some(AgentEvent::Error { .. }) => break TriageVerdict::Regression,
                        Some(_) => {}
                    }
                }
                _ = &mut idle_sleep => break TriageVerdict::Regression,
                _ = &mut wall_sleep => break TriageVerdict::Regression,
                _ = cancel_watch.changed() => {
                    if *cancel_watch.borrow() {
                        let _ = session.cancel();
                        break TriageVerdict::Regression;
                    }
                }
            }
        };

        let _ = self.registry.kill(&thread_id).await;
        verdict
    }

    /// Persist + emit the terminal environment-not-ready signal (C6.3), fired
    /// *immediately* on triage (no wasted retries first). Mirrors the
    /// `RetryBudgetExhausted` persistence path so the bell shows it after a
    /// refresh, plus a live event for the toast.
    fn notify_environment_not_ready(&self, step_exec: &StepExecution, message: &str) {
        if let Ok(Some(feature)) = self.features.get(&self.f_id) {
            let notification = crate::domain::models::Notification {
                id: format!("notif-{}", crate::paths::now_ms()),
                project_id: feature.project_id.0.clone(),
                feature_id: self.f_id.0.clone(),
                kind: crate::domain::models::NotificationKind::EnvironmentNotReady,
                message: message.to_string(),
                feature_url: Some(format!(
                    "/projects/{}/features/{}",
                    feature.project_id.0, self.f_id.0
                )),
                read: false,
                created_at: crate::paths::now_ms(),
            };
            let _ = self.notifications.add(notification);
        }
        let _ = self.notif.emit(&DomainEvent::EnvironmentNotReady {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            reason: message.to_string(),
        });
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
            .run_harness_first(step_exec, verifier_cfg, wt_path, machine_str)
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

        // Every supported agent is a CLI runtime that takes its model via the
        // `--model` flag in `build_args` from `ctx.model` below.
        let agent_env =
            crate::ports::agent_runtime::agent_base_env(self.exec.as_ref(), machine_str).await;

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

/// Outcome of the harness-failure triage classifier (C6/D7).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TriageVerdict {
    /// The change under test is broken — editing source can fix it. Stays on
    /// the existing `Verdict` retry path.
    Regression,
    /// The execution environment is not provisioned (missing lib/toolchain/
    /// service, permission, network). Editing source cannot fix it → terminal.
    Environment { reason: String, remediation: String },
}

/// Whether a reproduced-unchanged failure should be handed to the triage
/// classifier: only when this attempt's fingerprint exactly matches the prior
/// attempt's persisted one. A first failure (`None`) or a *changed* fingerprint
/// is ongoing progress — no triage (C6.2).
pub(crate) fn should_triage(prior_fingerprint: Option<&str>, current_fingerprint: &str) -> bool {
    prior_fingerprint == Some(current_fingerprint)
}

/// Normalize a failing harness/prepare output into a fingerprint that is
/// stable across retries of the *same* failure while still differing for a
/// genuinely different one (C6.2). Conservative: mask only known-volatile
/// spans — the absolute worktree path (which carries the per-run subtask id)
/// and long numeric runs (epoch/timestamps/ids of ≥6 digits) — and nothing
/// else, so two runs of the same missing-lib failure fingerprint-**match**
/// while a different regression error still differs.
///
/// The gate is only a cheap pre-filter: a false match costs at most one triage
/// call (the agent still makes the real regression/environment call), so this
/// leans toward *matching* volatile-only differences rather than risk missing
/// a genuine reproduction.
pub(crate) fn normalize_failure_fingerprint(output: &str, wt_path: &str) -> String {
    let mut s = output.to_string();
    if !wt_path.is_empty() {
        s = s.replace(wt_path, "<WT>");
    }
    let masked = mask_long_digit_runs(&s);
    // Drop trailing whitespace per line so cosmetic reflow doesn't perturb the
    // fingerprint, but keep line structure (don't collapse everything).
    masked
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Replace every maximal run of ≥6 ASCII digits with `<N>`. Six is above
/// typical line numbers / exit codes / version components (which we keep) and
/// at or below epoch seconds (10) / epoch millis (13) / long run-ids, which are
/// the volatile spans we want to mask.
fn mask_long_digit_runs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut digits = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            flush_digit_run(&mut out, &mut digits);
            out.push(c);
        }
    }
    flush_digit_run(&mut out, &mut digits);
    out
}

fn flush_digit_run(out: &mut String, digits: &mut String) {
    if digits.is_empty() {
        return;
    }
    if digits.len() >= 6 {
        out.push_str("<N>");
    } else {
        out.push_str(digits);
    }
    digits.clear();
}

/// Build the classifier prompt. It asks for exactly one JSON object so
/// [`parse_triage_text`] can lift the verdict out of any surrounding prose.
fn build_triage_prompt(machine: &str, wt_path: &str, cmd: &str, output_tail: &str) -> String {
    format!(
        "You are a build-failure triage classifier. A verification harness command was run \
         inside a project worktree and it FAILED. Classify the *cause* of the failure as \
         exactly one of:\n\
         - \"regression\": the code change under test is broken — a compile/type error, a \
           failing assertion, a lint the change introduced. Editing the source code can fix it.\n\
         - \"environment\": the execution machine is not provisioned — a missing system library \
           (e.g. pkg-config cannot find a dev package), a missing toolchain or binary (command \
           not found), a missing service, or a permission/network fault. Editing source code \
           CANNOT fix it; the machine must be provisioned.\n\n\
         If uncertain, prefer \"regression\" (it is always safe to let the implementer retry).\n\n\
         The failing command was:\n{}\n\n\
         It ran on machine '{}' in worktree '{}'.\n\n\
         The tail of its output was:\n```\n{}\n```\n\n\
         Respond with ONLY a JSON object and no other text:\n\
         {{ \"category\": \"regression\" | \"environment\", \"reason\": \"one concise sentence\", \
         \"remediation\": \"for environment: the exact provisioning step, e.g. 'install \
         libgtk-3-dev'; for regression: an empty string\" }}",
        cmd, machine, wt_path, output_tail,
    )
}

/// Scan a classifier agent's turn text for the triage JSON object. Tolerates
/// prose, code fences, and extended-thinking tags around the JSON, mirroring
/// [`parse_verdict_text`]'s tolerance. Any failure to find a usable
/// `"category"` defaults to [`TriageVerdict::Regression`] (fail-safe).
pub(crate) fn parse_triage_text(raw_text: &str) -> TriageVerdict {
    let text = crate::domain::text::strip_think_tags(raw_text);
    let bytes = text.as_bytes();
    let mut found: Option<serde_json::Value> = None;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(close) = find_matching_close_brace(bytes, i) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text[i..=close]) {
                    if val.is_object() && val.get("category").is_some() {
                        found = Some(val);
                        i = close + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }

    let Some(val) = found else {
        return TriageVerdict::Regression;
    };
    let category = val
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("regression")
        .to_lowercase();
    if category == "environment" {
        let reason = val
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("The execution environment is not provisioned for this command.")
            .to_string();
        let remediation = val
            .get("remediation")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        TriageVerdict::Environment {
            reason,
            remediation,
        }
    } else {
        TriageVerdict::Regression
    }
}

/// Build the user-facing environment-not-ready message (C6.3): the triage
/// reason + remediation plus the concrete context the orchestrator holds — the
/// exact failing command, the target machine, and a copy-pasteable reproduce
/// line — so the failure says *what* ran, *where*, and *how to reproduce/fix*.
pub(crate) fn build_environment_message(
    machine: &str,
    wt_path: &str,
    cmd: &str,
    reason: &str,
    remediation: &str,
) -> String {
    let reproduce = if machine.is_empty() || machine == "local" {
        format!("  cd {} && {}", wt_path, cmd)
    } else {
        format!("  ssh {}\n  cd {} && {}", machine, wt_path, cmd)
    };
    let remediation_line = if remediation.trim().is_empty() {
        String::new()
    } else {
        format!("\nRemediation: {}\n", remediation.trim())
    };
    format!(
        "Environment not ready — this failure is not something editing the code can fix.\n\n\
         {}\n{}\nFailing command: {}\nMachine: {}\nReproduce:\n{}\n",
        reason.trim(),
        remediation_line,
        cmd,
        if machine.is_empty() { "local" } else { machine },
        reproduce,
    )
}

#[cfg(test)]
mod triage_tests {
    use super::{
        build_environment_message, normalize_failure_fingerprint, parse_triage_text, should_triage,
        TriageVerdict,
    };

    // ── fingerprint normalization: the load-bearing part (C6.2) ──────────────

    #[test]
    fn same_failure_differing_only_in_worktree_and_timestamp_fingerprint_matches() {
        // Two attempts of the *same* missing-lib failure whose logs differ only
        // in the per-run worktree path and an epoch-ms timestamp MUST
        // fingerprint-match, so triage actually fires (the under-normalization
        // guard the DoD's differing-output fixture does not catch).
        let wt1 = "/home/u/.demeteo/wt/se-feat-s-impl-1699999999999";
        let wt2 = "/home/u/.demeteo/wt/se-feat-s-impl-1700000000000";
        let log = |wt: &str, ts: &str| {
            format!(
                "error: The system library 'gdk-3.0' was not found\n  building {}/build.rs\n  \
                 at epoch {}\n",
                wt, ts
            )
        };
        let a = normalize_failure_fingerprint(&log(wt1, "1699999999999"), wt1);
        let b = normalize_failure_fingerprint(&log(wt2, "1700000000000"), wt2);
        assert_eq!(a, b, "volatile-only differences must fingerprint-match");
    }

    #[test]
    fn genuinely_different_errors_fingerprint_differently() {
        // Over-normalization guard: a different regression error on the retry
        // must NOT read as "same" (or we'd triage real progress).
        let wt = "/tmp/wt";
        let a = normalize_failure_fingerprint("error[E0308]: mismatched types in auth.rs\n", wt);
        let b = normalize_failure_fingerprint("error: test payments::refund panicked\n", wt);
        assert_ne!(a, b);
    }

    #[test]
    fn short_numbers_and_versions_are_preserved() {
        // We must NOT mask line numbers / exit codes / version components, or
        // distinct failures would collapse together.
        let wt = "";
        let a = normalize_failure_fingerprint("gdk-3.0 not found (exit 1) at line 42\n", wt);
        assert!(a.contains("gdk-3.0"));
        assert!(a.contains("exit 1"));
        assert!(a.contains("line 42"));
    }

    // ── the persistence gate (C6.2) ─────────────────────────────────────────

    #[test]
    fn first_failure_does_not_trigger_triage() {
        assert!(!should_triage(None, "fp"));
    }

    #[test]
    fn changed_failure_does_not_trigger_triage() {
        assert!(!should_triage(Some("old"), "new"));
    }

    #[test]
    fn reproduced_failure_triggers_triage() {
        assert!(should_triage(Some("same"), "same"));
    }

    // ── classifier parsing, fail-safe to Regression ─────────────────────────

    #[test]
    fn parses_environment_verdict() {
        let raw = r#"{"category":"environment","reason":"gdk-3.0 dev package missing","remediation":"install libgtk-3-dev"}"#;
        match parse_triage_text(raw) {
            TriageVerdict::Environment {
                reason,
                remediation,
            } => {
                assert!(reason.contains("gdk-3.0"));
                assert_eq!(remediation, "install libgtk-3-dev");
            }
            _ => panic!("expected environment"),
        }
    }

    #[test]
    fn parses_regression_verdict() {
        let raw = r#"prose... {"category":"regression","reason":"broken test","remediation":""}"#;
        assert_eq!(parse_triage_text(raw), TriageVerdict::Regression);
    }

    #[test]
    fn environment_verdict_amid_prose_and_think_tags() {
        let raw = "<think>maybe env?</think>My verdict:\n{ \"category\": \"environment\", \"reason\": \"no compiler\", \"remediation\": \"install rustc\" }";
        assert!(matches!(
            parse_triage_text(raw),
            TriageVerdict::Environment { .. }
        ));
    }

    #[test]
    fn unparseable_or_unknown_defaults_to_regression() {
        // Fail-safe: a broken/garbage classifier answer must never terminate a
        // real regression — it falls back to the retry path.
        assert_eq!(
            parse_triage_text("I could not decide."),
            TriageVerdict::Regression
        );
        assert_eq!(
            parse_triage_text(r#"{"category":"banana"}"#),
            TriageVerdict::Regression
        );
    }

    // ── remediation message (C6.3) ──────────────────────────────────────────

    #[test]
    fn remote_message_has_ssh_reproduce_line_and_context() {
        let msg = build_environment_message(
            "gpu-box",
            "/home/u/wt/feat",
            "cd src-tauri && cargo test",
            "The system library 'gdk-3.0' was not found",
            "install libgtk-3-dev",
        );
        assert!(msg.contains("ssh gpu-box"));
        assert!(msg.contains("cd /home/u/wt/feat && cd src-tauri && cargo test"));
        assert!(msg.contains("Failing command: cd src-tauri && cargo test"));
        assert!(msg.contains("Machine: gpu-box"));
        assert!(msg.contains("install libgtk-3-dev"));
    }

    #[test]
    fn local_message_omits_ssh_line() {
        let msg = build_environment_message(
            "local",
            "/home/u/wt/feat",
            "cargo test",
            "missing lib",
            "install it",
        );
        assert!(!msg.contains("ssh "));
        assert!(msg.contains("cd /home/u/wt/feat && cargo test"));
    }
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
        let err = format!(
            "{}Timed out after the transport wall cap (1800s)",
            TRANSPORT_ERROR_PREFIX
        );
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

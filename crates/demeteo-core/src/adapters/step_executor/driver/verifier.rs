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

        // Run the prepare/harness commands under an interactive login shell with
        // the worktree as an explicit cwd, so the user's `PATH` (and any
        // `mise`/`asdf`/`nvm` shims) is established exactly as it is for the
        // agent — otherwise a project whose `npm test`/`pytest`/`cargo test` the
        // agent ran fine fails here with "command not found" on remote only.
        let opts = self.harness_shell_options(wt_path);

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
    /// prepare/test harness runs under: the worktree as an explicit cwd (D2 —
    /// never rely on ambient state) under an **interactive login shell**,
    /// unconditionally.
    ///
    /// A prepare/test command is user-authored shell (`cargo test`, `npm test`,
    /// `pytest`) whose binaries live on the *user's* `PATH`, which only a login
    /// shell's profile establishes — and only an *interactive* one activates
    /// `mise`/`asdf`/`nvm` shims, which hide behind the standard `~/.bashrc`
    /// non-interactive guard. So the harness always needs the same shell the
    /// agent probe already hardcodes (`ShellOptions::login_interactive`).
    ///
    /// This deliberately does **not** consult the machine's `use_login_shell`
    /// flag. That flag is only reachable through the SSH adapter — i.e. an
    /// *attached* run, where the desktop app drives commands over the wire. A
    /// **detached** run executes inside `demeteo-runner` on the target box
    /// itself, which registers its project as `compute_type: "local"`; `"local"`
    /// is a sentinel that short-circuits the DB lookup and yields a synthetic
    /// machine whose `use_login_shell` is hardcoded `None` (see
    /// `machine_resolver::local_machine`). Gating on the flag therefore forced
    /// every detached harness through a bare non-login `sh -c` no matter what
    /// the user had ticked in the UI, and a bare `cargo` in the harness command
    /// died with "cargo: not found" — while the *implement* step sailed through,
    /// because the agent binary is resolved to an absolute path up front and so
    /// never needed `PATH` at all.
    fn harness_shell_options(&self, wt_path: &str) -> crate::ports::execution::ShellOptions {
        crate::ports::execution::ShellOptions {
            cwd: Some(wt_path.to_string()),
            ..crate::ports::execution::ShellOptions::login_interactive()
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
        // Fast path: the shell could not find a binary the harness command
        // itself invokes (exit 127). That is objectively an environment gap —
        // the code never ran, so no amount of editing it can help. Escalate
        // straight to the terminal `Environment` error rather than spending a
        // `Verdict` retry (which re-runs the agent against a gate that cannot
        // pass) plus a triage agent turn to reach the same conclusion on the
        // *next* attempt. This skips `should_triage`'s reproduce-unchanged
        // requirement on purpose: a 127 is deterministic, not flaky.
        if let Some(missing) = detect_missing_command(cmd, output) {
            let msg = build_environment_message(
                machine_str,
                wt_path,
                cmd,
                &format!(
                    "The shell could not find `{}` on PATH (exit 127), so the command never ran.",
                    missing
                ),
                &format!(
                    "Install `{}` on this machine, or make it available on the login shell's PATH \
                     (e.g. add it to ~/.profile / ~/.bashrc, or expose it via mise/asdf/nvm). \
                     The harness runs under an interactive login shell, so anything an \
                     interactive `ssh` session can run, it can run too.",
                    missing
                ),
            );
            self.notify_environment_not_ready(step_exec, &msg);
            tracing::warn!(
                feature_id = %self.f_id,
                step_id = %step_exec.step_id.0,
                cmd = %cmd,
                missing = %missing,
                "harness command not found on PATH — terminating without retries"
            );
            return crate::domain::verifier::VerifierError::Environment(msg);
        }

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
    let text_buffer = crate::domain::text::strip_think_tags(raw_text);
    let parsed_val = crate::domain::text::find_json_object_with_key(raw_text, verdict_key);

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

/// Detect "the shell could not find a binary the harness command invokes"
/// (exit 127) and return the missing command's name.
///
/// Recognizes the three shell diagnostics we can actually receive — dash/`sh`
/// (`sh: 1: cargo: not found`), bash (`bash: line 1: cargo: command not found`),
/// and zsh (`zsh: command not found: cargo`) — because the exit code itself is
/// not reliably in the error string: the local adapter formats
/// `Command failed (exit code: Some(127)): …` but the SSH adapter substitutes
/// the remote stderr for the code whenever stderr is non-empty.
///
/// Guarded against false positives by requiring the missing name to appear as a
/// token of `cmd`. A test that merely *prints* "command not found" in its output
/// therefore stays a normal `Verdict`, and only a binary the harness genuinely
/// tries to run escalates. The cost of that guard is an indirect invocation
/// (`make test` shelling out to a missing `cargo`) not matching — that falls
/// through to the existing triage path, which reaches the same verdict one
/// attempt later.
fn detect_missing_command(cmd: &str, output: &str) -> Option<String> {
    let invoked = |name: &str| -> bool {
        !name.is_empty()
            && !name.contains(char::is_whitespace)
            && cmd
                .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')'))
                .any(|tok| tok == name)
    };

    output.lines().map(str::trim).find_map(|line| {
        // Scan *within* the line rather than anchoring to its end: the SSH
        // adapter embeds the remote stderr mid-string (`Command failed (sh: 1:
        // cargo: not found): bash -l -i -c …`), so the diagnostic is not the
        // line's suffix.
        //
        // zsh (`command not found: npm`) is matched first because it names the
        // binary *after* the marker while carrying the bash marker's text as a
        // prefix — checking bash first would mis-extract the shell's own name.
        let raw = if let Some((_, rest)) = line.split_once("command not found: ") {
            rest.split_whitespace().next()?
        } else if let Some(i) = line.find(": command not found") {
            line[..i].rsplit(':').next()?
        } else {
            let i = line.find(": not found")?;
            line[..i].rsplit(':').next()?
        };

        // Strip the punctuation an adapter's own wrapper can leave glued to the
        // name (`… command not found: npm): bash -l …`); no real binary ends in
        // one of these.
        let name = raw.trim().trim_end_matches([')', ':', ',', '.', '\'', '"']);

        invoked(name).then(|| name.to_string())
    })
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
    let Some(val) = crate::domain::text::find_json_object_with_key(raw_text, "category") else {
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
#[path = "../../../../tests/infrastructure/step_executor/verifier/triage_tests.rs"]
mod triage_tests;

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/verifier/produced_artifacts_summary_tests.rs"]
mod produced_artifacts_summary_tests;

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/verifier/tail_chars_tests.rs"]
mod tail_chars_tests;

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/verifier/is_transport_failure_tests.rs"]
mod is_transport_failure_tests;

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/verifier/parse_verdict_text_tests.rs"]
mod parse_verdict_text_tests;

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/verifier/detect_missing_command_tests.rs"]
mod detect_missing_command_tests;

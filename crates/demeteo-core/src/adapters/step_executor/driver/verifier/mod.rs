use super::ExecutionDriver;
use crate::adapters::step_executor::artifacts::materialize_external_artifact_paths;
use crate::adapters::step_executor::driver::verifier::environment::notify_environment_not_ready;
use crate::adapters::step_executor::harness_shell::{
    harness_ceiling_s, harness_shell_options, run_harness_command,
};
use crate::adapters::step_executor::spend::RunningSpend;
use crate::domain::agent_event::AgentEvent;
use crate::domain::harness_attribution::HarnessFailureSet;
use crate::domain::harness_failure::{classify_exec_failure, HarnessExecFailure};
use crate::domain::harness_outcome::{merge_stderr_into_stdout, HarnessOutcome, HarnessRun};
use crate::domain::harness_remediation::build_timeout_message;
use crate::domain::models::StepExecution;
use crate::domain::verifier::verdict::{
    build_verifier_prompt, format_produced_artifacts_summary, parse_verdict_text, ParsedVerdict,
};
use crate::paths;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::notification::DomainEvent;
use tokio_stream::StreamExt;

mod adjudication;
pub(crate) mod environment;

/// Who runs the verifier turn, and where.
///
/// The four values the caller has already resolved for the step and never
/// splits: the machine and the worktree address the same one place, and the
/// harness/model pair is the answer `resolve_step_agent` gave once at the top
/// of the step. A verifier that ran on a different machine from the tasks it
/// judges, or under a model the step did not ask for, would be a bug that
/// four loose arguments make easy to write.
pub(crate) struct VerifierTarget<'a> {
    pub machine: &'a str,
    pub wt_path: &'a str,
    pub agent_kind: &'a str,
    pub override_model: Option<&'a str>,
}

impl ExecutionDriver {
    /// Persist complete harness logs for the validator without making its argv
    /// exceed the prompt budget. A failed artifact write leaves the bounded
    /// evidence intact; it must not turn a green harness into a failed step.
    pub(crate) fn store_harness_logs(
        &self,
        step_exec: &StepExecution,
        outcome: &HarnessOutcome,
    ) -> String {
        let references = outcome
            .full_log_artifacts()
            .into_iter()
            .filter_map(|artifact| {
                let name = artifact
                    .name
                    .strip_prefix("harness-")
                    .unwrap_or(&artifact.name);
                match self
                    .artifacts
                    .put(&self.f_id_str, &step_exec.step_id.0, &artifact)
                {
                    Ok(reference) => Some((name.to_string(), reference)),
                    Err(error) => {
                        tracing::warn!(
                            feature_id = %self.f_id,
                            step_id = %step_exec.step_id.0,
                            harness = %name,
                            %error,
                            "failed to store complete harness output"
                        );
                        None
                    }
                }
            })
            .collect::<Vec<_>>();
        outcome.render_full_log_references(&references)
    }

    /// Resolve and run the project's prepare command + every harness that gates
    /// this step inside `wt_path`, and return the formatted "harness results"
    /// section for an agent prompt.
    ///
    /// This is the harness-first primitive: it runs **before** any agent
    /// turn, so a red harness fails the step objectively at zero token
    /// cost, and a green harness's output is injected into the single
    /// validate turn instead of paying for the agent to re-run the same
    /// commands (which the capability chmod fence would block with EPERM
    /// anyway — build tools need to write `target/`, `node_modules/`, …).
    ///
    /// *Which* harnesses gate the step is a policy decision, resolved by
    /// [`resolve_harnesses`](crate::domain::verifier::resolve_harnesses) — pure,
    /// synchronous, and not spelled here. This function only executes what that
    /// returned, **in order, each as its own command with its own deadline and
    /// its own labelled output block**.
    ///
    /// **Every resolved harness runs, even after one fails** (HB5). If lint and
    /// unit are both red the user wants both: stopping at the first turns one
    /// wasted rework cycle into two, which is the thing
    /// [`docs/HARNESS_BASELINE.md`] exists to prevent. Only the three failure
    /// modes that end the *step* short-circuit the loop — a dead machine, an
    /// expired deadline, and Stop — because none of them can produce a verdict
    /// on the gates that have not run yet either.
    ///
    /// Errors:
    /// * prepare or any harness exits non-zero → [`VerifierError::Verdict`]
    ///   naming **each** failing gate, with its output tail as the actionable
    ///   reason (feeds the on_failure retry loop) — **unless** the failure
    ///   reproduces the previous attempt's failure unchanged, in which case a
    ///   triage agent (C6) may reclassify it as
    ///   [`VerifierError::Environment`] (terminal).
    /// * a transport failure → [`VerifierError::Infrastructure`].
    /// * a timeout → [`VerifierError::Environment`] (terminal, with remediation).
    /// * cancellation → [`VerifierError::Cancelled`].
    ///
    /// Returns a [`HarnessOutcome`] rather than a pre-rendered string, so
    /// "a harness ran and here is its output" and "no harness exists" cannot be
    /// confused by a caller — see that type's docs for the bug that forced it.
    pub(crate) async fn run_harness_first(
        &self,
        step_exec: &StepExecution,
        verifier_cfg: &crate::domain::verifier::VerifierConfig,
        wt_path: &str,
        machine_str: &str,
    ) -> Result<HarnessOutcome, crate::domain::verifier::VerifierError> {
        let feature = self.features.get(&self.f_id).ok().flatten();
        let settings = feature
            .as_ref()
            .and_then(|f| self.projects.get_settings(&f.project_id).ok().flatten());
        let prepare_command = settings
            .as_ref()
            .and_then(|s| s.worktree_strategy.prepare_command.clone());

        let resolved = settings
            .as_ref()
            .map(|s| {
                crate::domain::verifier::resolve_harnesses(
                    &verifier_cfg.harness_names,
                    &s.worktree_strategy,
                    harness_ceiling_s(self.app_settings.as_ref()),
                )
            })
            .unwrap_or_default();

        // Idempotent write-restore. Fresh worktrees are writable, but a
        // retried step may run in a worktree the fence already touched.
        if prepare_command.is_some() || !resolved.is_empty() {
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
        let opts = harness_shell_options(self.app_settings.as_ref(), wt_path);

        if let Some(ref cmd) = prepare_command {
            match run_harness_command(
                self.exec.as_ref(),
                self.cancel_watch.clone(),
                machine_str,
                cmd,
                opts.clone(),
            )
            .await
            {
                None => return Err(crate::domain::verifier::VerifierError::Cancelled),
                Some(Ok(_)) => {}
                Some(Err(out)) => match classify_exec_failure(&out) {
                    // A transport failure (unreachable machine, dropped
                    // channel, drain timeout) is not a red build — surface it
                    // as Infrastructure (non-retryable) instead of a Verdict
                    // that would pointlessly re-run the same command. See
                    // C0.2 / D3.
                    HarnessExecFailure::Transport => {
                        return Err(crate::domain::verifier::VerifierError::Infrastructure(
                            format!("prepare command '{}' could not run: {}", cmd, out),
                        ))
                    }
                    // Abandoned at the ceiling: the command never reached a
                    // verdict, so retrying the *code* cannot help. Terminal,
                    // with remediation, exactly like the exit-127 path.
                    HarnessExecFailure::Timeout => {
                        let msg = build_timeout_message(
                            machine_str,
                            wt_path,
                            cmd,
                            harness_ceiling_s(self.app_settings.as_ref()),
                        );
                        notify_environment_not_ready(&self.environment_signal(), step_exec, &msg);
                        return Err(crate::domain::verifier::VerifierError::Environment(msg));
                    }
                    // A failing prepare is never subtracted. `measure_gates`
                    // records **no gate** when the baseline's own prepare
                    // fails, so there is nothing on record that could excuse
                    // this — and that is the right asymmetry: a worktree that
                    // cannot be made runnable is not a pre-existing test
                    // failure, it is a step that never got to run one.
                    HarnessExecFailure::NonZeroExit => {
                        return Err(self
                            .classify_harness_failures(
                                step_exec,
                                machine_str,
                                wt_path,
                                &HarnessFailureSet {
                                    attributable: &[HarnessRun {
                                        name: "prepare".to_string(),
                                        cmd: cmd.clone(),
                                        output: out,
                                    }],
                                    excluded: &[],
                                    triage_allowed: true,
                                    // A prepare command runs no tests, so there
                                    // is nothing for rung 3 to name.
                                    failing_tests: &[],
                                },
                            )
                            .await);
                    }
                },
            }
        }

        let mut ran: Vec<HarnessRun> = Vec::new();
        let mut failed: Vec<HarnessRun> = Vec::new();
        for harness in &resolved {
            // Each harness carries **its own** deadline (see
            // `ResolvedHarness::deadline_s`): the ceiling is per command, so it
            // is applied N times rather than divided N ways.
            let opts = crate::ports::execution::ShellOptions {
                timeout: Some(std::time::Duration::from_secs(harness.deadline_s)),
                ..opts.clone()
            };
            // Merge stderr into stdout for the harness run. The port contract
            // is "stdout on success, stdout+stderr on failure" (D3) — correct
            // for a port, wrong for this caller, because a *green* suite that
            // reports on stderr would hand the validate agent an empty output
            // block that the prompt then calls authoritative. `steps/command.rs`
            // solves the identical problem the identical way; the two callers
            // of `harness_shell_options` should not disagree about this either.
            //
            // The exit status survives (it is the subshell's last command's),
            // so the pass/fail gate below is unchanged. The newlines matter: a
            // command whose last line is a `#` comment would otherwise swallow
            // the closing paren.
            let result = run_harness_command(
                self.exec.as_ref(),
                self.cancel_watch.clone(),
                machine_str,
                &merge_stderr_into_stdout(&harness.command),
                opts,
            )
            .await;
            let run = |output: String| HarnessRun {
                name: harness.name.clone(),
                cmd: harness.command.clone(),
                output,
            };
            match result {
                None => return Err(crate::domain::verifier::VerifierError::Cancelled),
                Some(Ok(out)) => ran.push(run(out)),
                Some(Err(out)) => match classify_exec_failure(&out) {
                    // A transport failure is infrastructure, not a red harness —
                    // don't gate a Verdict on it (C0.2 / D3). It also ends the
                    // loop: the machine is gone, so the remaining gates cannot
                    // reach a verdict either.
                    HarnessExecFailure::Transport => {
                        return Err(crate::domain::verifier::VerifierError::Infrastructure(
                            format!(
                                "test harness '{}' ({}) could not run: {}",
                                harness.name, harness.command, out
                            ),
                        ))
                    }
                    HarnessExecFailure::Timeout => {
                        let msg = build_timeout_message(
                            machine_str,
                            wt_path,
                            &harness.command,
                            harness.deadline_s,
                        );
                        notify_environment_not_ready(&self.environment_signal(), step_exec, &msg);
                        return Err(crate::domain::verifier::VerifierError::Environment(msg));
                    }
                    // The one shape that is a verdict — record it and keep
                    // going, so a run that broke two gates reports two.
                    HarnessExecFailure::NonZeroExit => failed.push(run(out)),
                },
            }
        }

        // Hard gate: a non-zero exit is objective — but *whose* defect it is is
        // not, and that is what HB2c decides here before anything fails.
        if !failed.is_empty() {
            return self
                .adjudicate_red_gates(step_exec, machine_str, wt_path, &resolved, ran, &failed)
                .await;
        }

        Ok(HarnessOutcome::from_runs(ran))
    }

    pub(crate) async fn run_verifier_logic(
        &self,
        step_exec: &StepExecution,
        verifier_cfg: &crate::domain::verifier::VerifierConfig,
        target: VerifierTarget<'_>,
        produced_artifacts: &[crate::domain::artifact::Artifact],
        spend: RunningSpend<'_>,
    ) -> Result<(), crate::domain::verifier::VerifierError> {
        let VerifierTarget {
            machine: machine_str,
            wt_path,
            agent_kind: default_agent_kind,
            override_model,
        } = target;
        let RunningSpend {
            cost: accumulated_cost,
            tokens: accumulated_tokens,
            start: step_start,
        } = spend;
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
        let harness_label = verifier_cfg.harness_label();
        let harness_outcome = self
            .run_harness_first(step_exec, verifier_cfg, wt_path, machine_str)
            .await?;
        let harness_section = materialize_external_artifact_paths(
            &format!(
                "{}{}",
                harness_outcome.render_section(),
                self.store_harness_logs(step_exec, &harness_outcome)
            ),
            wt_path,
            self.exec.as_ref(),
            machine_str,
        )
        .await;

        let verifier_prompt = build_verifier_prompt(
            &verifier_cfg.instructions,
            &harness_section,
            &format_produced_artifacts_summary(produced_artifacts),
            &verifier_cfg.verdict_key,
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
            .or_else(|| override_model.map(str::to_string));

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
            // Same reasoning as `verifier_model`: turning harness output into
            // one verdict object is a small job that runs on every retry, so
            // it defaults low instead of inheriting the run's effort. A
            // `VerifierConfig` may still ask for more.
            effort: Some(
                verifier_cfg
                    .effort
                    .unwrap_or(crate::domain::models::EffortLevel::VERIFIER_DEFAULT),
            ),
            title: Some(format!("Verify: {}", harness_label)),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            permissions: crate::domain::permission::PermissionProfile::all_allow(),
            bare_mode: true,
            // The verifier reads artifacts/files on demand, so it keeps its
            // full toolset — but interpreting an already-run harness into
            // one verdict object should never take dozens of round trips.
            // Anti-runaway only; a tripped cap fails through the normal
            // error path and the retry ladder owns recovery.
            tool_allowlist: None,
            max_turns: Some(25),
            // Reads artifacts on demand, then emits one verdict object.
            max_budget_usd: self.role_max_budget_usd(Self::BUDGET_FRACTION_VERIFIER),
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

                    usage_acc.ingest_event(&event);

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
            ParsedVerdict::Environment(reason) => {
                tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    reason = %reason,
                    "verifier verdict: environment — the project is not configured to evidence \
                     these criteria, so no amount of re-implementation can satisfy them"
                );
                Err(crate::domain::verifier::VerifierError::Environment(reason))
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

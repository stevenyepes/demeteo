use super::ExecutionDriver;
use crate::adapters::step_executor::driver::verifier::environment::{
    command_never_ran_error, notify_environment_not_ready,
};
use crate::adapters::step_executor::harness_shell::{
    harness_ceiling_s, harness_shell_options, run_harness_command,
};
use crate::domain::agent_event::AgentEvent;
use crate::domain::harness_attribution::{
    split_by_determination, HarnessFailureSet, SubtractedFailures,
};
use crate::domain::harness_failure::{classify_exec_failure, HarnessExecFailure};
use crate::domain::harness_fingerprint::normalize_failure_fingerprint;
use crate::domain::harness_outcome::{
    build_exclusion_note, build_failure_reason, combined_failure_output, merge_stderr_into_stdout,
    HarnessOutcome, HarnessRun,
};
use crate::domain::harness_remediation::{build_environment_message, build_timeout_message};
use crate::domain::harness_triage::{
    build_triage_prompt, parse_triage_text, triage_decision, TriageDecision, TriageVerdict,
};
use crate::domain::models::StepExecution;
use crate::domain::text::tail_chars;
use crate::domain::verifier::verdict::{
    build_verifier_prompt, format_produced_artifacts_summary, parse_verdict_text, ParsedVerdict,
};
use crate::paths;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::notification::DomainEvent;
use std::time::Instant;
use tokio_stream::StreamExt;

pub(crate) mod environment;

impl ExecutionDriver {
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
            // The never-ran fast path runs on the **unsubtracted** set and
            // before the baseline is consulted at all. A binary the shell
            // cannot find — or a script this worktree does not define — is
            // terminal whether or not it was equally missing at the base: the
            // code never ran, so there is nothing for a subtraction to be
            // evidence about, and "pre-existing" would quietly pass a step that
            // tested nothing. Decision 44's table says so in its own row — 127
            // is terminal `Environment`, no baseline column — and a missing
            // script is that same row reached through exit 1.
            if let Some(err) = command_never_ran_error(
                &self.environment_signal(),
                step_exec,
                machine_str,
                wt_path,
                &failed,
            ) {
                return Err(err);
            }

            // HB2b's lazy fallback, on the *failure* path only. A gate that
            // just went red is the one case where knowing what it did at the
            // base is worth minutes of wall-clock — the alternative is a rework
            // cycle at $14.63 and 11M tokens. On a green harness this is not
            // reached at all, which is deliberate: there is nothing to subtract
            // from, and every successful run would otherwise pay for it.
            //
            // It measures and records; it decides nothing, and it returns `()`
            // so it cannot influence the verdict computed below however badly
            // it goes.
            //
            // Raced against cancellation for the same reason every other
            // command here is: dropping the future is what stops the work, and
            // a Stop must not wait out a cold `npm install`. A cancelled race
            // leaves `base_sha` empty, which is *no evidence* — so a Stop
            // yields today's verdict rather than a subtraction taken on half a
            // measurement.
            let mut cancel_watch = self.cancel_watch.clone();
            let cancelled = async move {
                if cancel_watch.wait_for(|c| *c).await.is_err() {
                    std::future::pending::<()>().await;
                }
            };
            let base_sha = tokio::select! {
                biased;
                _ = cancelled => String::new(),
                sha = async {
                    let sha = self.resolve_base_sha().await.unwrap_or_default();
                    self.measure_fallback_baseline(
                        &step_exec.step_id.0,
                        machine_str,
                        &sha,
                        &resolved,
                        &failed,
                    )
                    .await;
                    sha
                } => sha,
            };

            // Re-read the record *after* the fallback: on a workflow with no
            // baseline node it is the write that just happened, and reading
            // before it would make the fallback a producer with no consumer.
            let baseline = self
                .features
                .get(&self.f_id)
                .ok()
                .flatten()
                .and_then(|f| f.harness_baseline);

            let subtracted = self
                .subtract_pre_existing(baseline.as_ref(), &base_sha, wt_path, machine_str, &failed)
                .await;

            // Red at the base **because the gate could not run here**. It looks
            // identical to a pre-existing failure — same command, same output,
            // same fingerprint — and subtracting it would pass the step on a
            // gate that verified nothing. The baseline's own classification
            // (taken at measurement time, before a line was implemented) is what
            // tells the two apart; the motivating incident exited 1, not 127, so
            // the fast path above cannot.
            if let Some(gate) = subtracted.unrunnable {
                let msg = build_environment_message(
                    machine_str,
                    wt_path,
                    &gate.run.cmd,
                    &gate.reason,
                    &gate.remediation,
                );
                notify_environment_not_ready(&self.environment_signal(), step_exec, &msg);
                tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    harness = %gate.run.name,
                    base_sha = %base_sha,
                    "the gate was red at the base because this machine cannot run it — \
                     terminating rather than excusing a gate that proved nothing"
                );
                return Err(crate::domain::verifier::VerifierError::Environment(msg));
            }

            let SubtractedFailures {
                attributable,
                excluded,
                triage_allowed,
                new_failing_tests,
                ..
            } = subtracted;

            if attributable.is_empty() {
                // Every red gate was already red, identically, before this
                // feature existed. It does not fail the step — and the
                // exclusion travels into the prompt so the report names it,
                // because a subtraction the user cannot audit is one they will
                // not trust the first time it is wrong.
                tracing::info!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    excluded = excluded.len(),
                    base_sha = %base_sha,
                    "every failing gate was red at the base with the identical failure — \
                     excluded from this step's verdict"
                );
                return Ok(HarnessOutcome::from_runs_with_exclusions(ran, excluded));
            }

            return Err(self
                .classify_harness_failures(
                    step_exec,
                    machine_str,
                    wt_path,
                    &HarnessFailureSet {
                        attributable: &attributable,
                        excluded: &excluded,
                        triage_allowed,
                        failing_tests: &new_failing_tests,
                    },
                )
                .await);
        }

        Ok(HarnessOutcome::from_runs(ran))
    }

    /// Split the red gates into the ones this feature answers for and the ones
    /// the baseline excuses, and say whether C6's classifier has anything left
    /// to add.
    ///
    /// Both decisions are pure and in `domain/`, reachable from a test with no
    /// port doubles (AGENTS.md §3): per gate,
    /// [`compare_gates`](crate::domain::harness_delta::compare_gates); for the
    /// step,
    /// [`split_by_determination`](crate::domain::harness_attribution::split_by_determination),
    /// whose header carries the positional rules the fold enforces. What is
    /// left here is paying for the comparison.
    ///
    /// Async only because of rung 3: naming *which* failures a differently-red
    /// gate added takes a reading of two outputs, and
    /// [`compare_gates_with_extraction`](crate::adapters::step_executor::failing_tests::compare_gates_with_extraction)
    /// is what decides the narrow set of gates that is worth paying for. Nothing
    /// it returns can move a gate between attributable and excluded.
    async fn subtract_pre_existing(
        &self,
        baseline: Option<&crate::domain::harness_baseline::HarnessBaseline>,
        base_sha: &str,
        wt_path: &str,
        machine_str: &str,
        failed: &[HarnessRun],
    ) -> SubtractedFailures {
        let comparisons =
            crate::adapters::step_executor::failing_tests::compare_gates_with_extraction(
                &crate::adapters::step_executor::failing_tests::DriverExtractor {
                    driver: self,
                    machine: machine_str,
                    wt_path,
                },
                baseline,
                base_sha,
                wt_path,
                failed,
            )
            .await;

        split_by_determination(failed, &comparisons, base_sha)
    }

    /// Turn one or more non-transport prepare/harness failures into the right
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
    ///
    /// HB2c narrows *when* the classifier is consulted without touching that
    /// fail-safe: a gate whose baseline already settled regression-vs-
    /// environment as a measurement skips it
    /// ([`HarnessFailureSet::triage_allowed`]), and gates the baseline excused
    /// never arrive here at all — they are named in the verdict reason instead,
    /// so the rework loop does not go looking for a defect the feature did not
    /// cause.
    ///
    /// With several gates red, the *fingerprint* and the *verdict reason* cover
    /// all of them — the retry loop needs every failure or it fixes one and
    /// rediscovers the next — while the never-ran fast path and the triage agent
    /// are asked about a single gate each: the first that never ran, and the
    /// first that failed, respectively. Both of those answer "is this
    /// machine able to run this command", which is a per-command question, and
    /// both attach a copy-pasteable reproduce line that only means anything for
    /// one command.
    async fn classify_harness_failures(
        &self,
        step_exec: &StepExecution,
        machine_str: &str,
        wt_path: &str,
        set: &HarnessFailureSet<'_>,
    ) -> crate::domain::verifier::VerifierError {
        let failures = set.attributable;
        let Some(primary) = failures.first() else {
            // Unreachable by construction (callers only call this with at least
            // one failure), but returning a verdict beats an `unwrap` in a
            // production path.
            return crate::domain::verifier::VerifierError::Verdict(
                crate::domain::verifier::VerdictFailure::from_reason(
                    "the harness reported a failure with no failing command".to_string(),
                ),
            );
        };

        if let Some(err) = command_never_ran_error(
            &self.environment_signal(),
            step_exec,
            machine_str,
            wt_path,
            failures,
        ) {
            return err;
        }

        // Fingerprint the whole failing set, not just the first gate: two
        // attempts that fail the same lint but different tests are *not* the
        // same failure, and triaging them as one would hand the classifier a
        // reproduction that never happened.
        let current_fp = normalize_failure_fingerprint(&combined_failure_output(failures), wt_path);
        let decision = triage_decision(
            step_exec.last_failure_fingerprint.as_deref(),
            &current_fp,
            set.triage_allowed,
        );

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

        // Rung 3 rides on the *existing* carrier into the retry loop rather than
        // a parallel channel: `failing_tests` is what `RetryContext` threads into
        // a rework template's `{{failing_tests}}`, so a scoped delta becomes one
        // ticket per new failure instead of a re-derivation of the whole gate.
        // The reason is unchanged — the scope is added to the evidence, never
        // substituted for it, so an empty list costs the reader nothing.
        let verdict = crate::domain::verifier::VerifierError::Verdict(
            crate::domain::verifier::VerdictFailure {
                reason: format!(
                    "{}{}",
                    build_failure_reason(failures),
                    build_exclusion_note(set.excluded),
                ),
                failing_tests: set.failing_tests.to_vec(),
                implicated_files: Vec::new(),
            },
        );

        match decision {
            TriageDecision::NotReproduced => return verdict,
            TriageDecision::SettledByBaseline => {
                tracing::debug!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    harness = %primary.name,
                    "the baseline already answered regression-vs-environment for this gate — \
                     not consulting the triage classifier"
                );
                return verdict;
            }
            TriageDecision::Consult => {}
        }

        let cmd = primary.cmd.as_str();
        match self
            .triage_harness_failure(machine_str, wt_path, cmd, &primary.output)
            .await
        {
            TriageVerdict::Environment {
                reason,
                remediation,
            } => {
                let msg =
                    build_environment_message(machine_str, wt_path, cmd, &reason, &remediation);
                notify_environment_not_ready(&self.environment_signal(), step_exec, &msg);
                tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    harness = %primary.name,
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
    ///
    /// `pub(crate)` because HB2c's baseline producer calls it too, through
    /// [`BaselineTriage`](crate::adapters::step_executor::baseline::BaselineTriage),
    /// to classify a gate that was already red at the base. Sharing the one
    /// classifier is the point: a second implementation would be free to drift,
    /// and the direction it drifted in would decide whether a run terminates.
    pub(crate) async fn triage_harness_failure(
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
            // Classification, not reasoning work — pinned low rather than
            // inheriting the run's effort (which may be `max`).
            effort: Some(crate::domain::models::EffortLevel::TRIAGE),
            title: Some("Triage harness failure".to_string()),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            permissions: crate::domain::permission::PermissionProfile::all_allow(),
            bare_mode: agent_kind == "claude-code",
            // The classifier's entire input is inlined in the prompt (the
            // harness output tail) and its entire output is one JSON
            // object — no tool definitions in context, no agentic loop.
            tool_allowlist: Some(vec![]),
            max_turns: Some(2),
            // Cheapest role turn: one classification over inlined output.
            max_budget_usd: self.role_max_budget_usd(Self::BUDGET_FRACTION_TRIAGE),
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
        override_model: Option<&str>,
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
        let harness_label = verifier_cfg.harness_label();
        let harness_section = self
            .run_harness_first(step_exec, verifier_cfg, wt_path, machine_str)
            .await?
            .render_section();

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
            bare_mode: verifier_agent_kind == "claude-code",
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

use super::ExecutionDriver;
use crate::domain::agent_event::AgentEvent;
use crate::domain::harness_failure::{
    build_missing_task_message, classify_exec_failure, detect_missing_command, detect_missing_task,
    HarnessExecFailure,
};
use crate::domain::harness_fingerprint::normalize_failure_fingerprint;
use crate::domain::harness_outcome::{
    build_exclusion_note, build_exclusion_reason, build_failure_reason, combined_failure_output,
    merge_stderr_into_stdout, ExcludedRun, HarnessOutcome, HarnessRun,
};
use crate::domain::harness_remediation::build_environment_message;
use crate::domain::harness_triage::{
    build_triage_prompt, parse_triage_text, recover_unbraced_object, triage_decision,
    TriageDecision, TriageVerdict,
};
use crate::domain::models::StepExecution;
use crate::domain::text::tail_chars;
use crate::paths;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::notification::DomainEvent;
use std::time::Instant;
use tokio_stream::StreamExt;

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
                    self.harness_ceiling_s(),
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
        let opts = self.harness_shell_options(wt_path);

        if let Some(ref cmd) = prepare_command {
            match self
                .run_harness_command(machine_str, cmd, opts.clone())
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
                            self.harness_ceiling_s(),
                        );
                        self.notify_environment_not_ready(step_exec, &msg);
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
            let result = self
                .run_harness_command(
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
                        self.notify_environment_not_ready(step_exec, &msg);
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
            if let Some(err) =
                self.command_never_ran_error(step_exec, machine_str, wt_path, &failed)
            {
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
                self.notify_environment_not_ready(step_exec, &msg);
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
    /// The decision itself is [`compare_gates`](crate::domain::harness_delta::compare_gates)
    /// — pure, in `domain/`, reachable from a test with no port doubles
    /// (AGENTS.md §3). What is left here is joining its answer back onto the
    /// runs and wording the exclusion, neither of which is a policy choice.
    ///
    /// `triage_allowed` is taken from the **first attributable** gate, because
    /// that is the single gate `classify_harness_failures` asks the classifier
    /// about: gating on a determination reached for some *other* gate would ask
    /// the agent a question the baseline already answered, or withhold one it
    /// did not.
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
        use crate::domain::harness_delta::GateOutcome;

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

        let mut attributable = Vec::new();
        let mut excluded = Vec::new();
        let mut unrunnable: Option<UnrunnableGate> = None;
        let mut triage_allowed = None;
        let mut new_failing_tests: Vec<String> = Vec::new();
        for (run, cmp) in failed.iter().zip(&comparisons) {
            match cmp.determination.outcome() {
                GateOutcome::Attributable => {
                    triage_allowed.get_or_insert(cmp.determination.allows_triage());
                    // Rung 3's scope, unioned across the gates this feature
                    // answers for. Empty for every gate rung 3 could not narrow,
                    // which is what keeps a partial reading from claiming to be
                    // the whole failure set: the verdict reason still carries
                    // every gate's output either way.
                    for name in cmp.determination.new_failures() {
                        if !new_failing_tests.contains(name) {
                            new_failing_tests.push(name.clone());
                        }
                    }
                    attributable.push(run.clone());
                }
                GateOutcome::Excluded => excluded.push(ExcludedRun {
                    run: run.clone(),
                    reason: build_exclusion_reason(base_sha, cmp.baseline.as_ref()),
                }),
                // The **first** one only: the message carries a reproduce line,
                // which means nothing for more than one command — the same
                // reason the 127 fast path and the classifier each name a single
                // gate.
                GateOutcome::Unrunnable {
                    reason,
                    remediation,
                } => {
                    unrunnable.get_or_insert_with(|| UnrunnableGate {
                        run: run.clone(),
                        reason: reason.to_string(),
                        remediation: remediation.to_string(),
                    });
                }
            }
        }
        SubtractedFailures {
            attributable,
            excluded,
            // No attributable gate ⇒ nothing will be classified, so the flag is
            // moot; `true` keeps it from reading as a suppression that was never
            // decided.
            triage_allowed: triage_allowed.unwrap_or(true),
            unrunnable,
            new_failing_tests,
        }
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
    ///
    /// `pub(crate)` because the `command` node type (P3.5) runs
    /// user-authored shell for the same reason under the same
    /// constraints — sharing the decision beats re-deriving it there.
    ///
    /// # Deadline
    ///
    /// The options carry the run's `wall_cap_s` as an explicit
    /// [`timeout`](crate::ports::execution::ShellOptions::timeout). Without one
    /// the harness was the only unbounded wait in a step: `wall_cap_s` itself is
    /// enforced inside `stream_agent_turn`, and the harness runs *before* any
    /// turn starts, so a command that never exits hung the step until the app
    /// restarted. It reuses the existing user-configurable cap rather than
    /// introducing a second knob — a harness is bounded by the same "how long
    /// may one step take" answer an agent turn is.
    ///
    /// The `command` node overrides this with its own `spec.timeout`, which is
    /// why this is a default rather than a floor — and so does each resolved
    /// harness, which carries the same ceiling as *its own*
    /// [`deadline_s`](crate::domain::verifier::ResolvedHarness::deadline_s).
    /// The cap answers "how long may one command take", so N gates get N
    /// ceilings rather than a slice each; the sum a step may spend is therefore
    /// unbounded in the number of gates its author declared. See that field for
    /// why dividing would be worse.
    pub(crate) fn harness_shell_options(
        &self,
        wt_path: &str,
    ) -> crate::ports::execution::ShellOptions {
        crate::ports::execution::ShellOptions {
            cwd: Some(wt_path.to_string()),
            timeout: Some(std::time::Duration::from_secs(self.harness_ceiling_s())),
            ..crate::ports::execution::ShellOptions::login_interactive()
        }
    }

    /// The wall-clock ceiling one prepare/harness command may consume, in
    /// seconds. Read through the same resolver every agent-turn call site uses,
    /// so one preferences change moves both.
    pub(crate) fn harness_ceiling_s(&self) -> u64 {
        crate::application::timeouts::resolve_effective(self.app_settings.as_ref()).wall_cap_s
    }

    /// Run one prepare/harness command, racing it against cancellation.
    ///
    /// Dropping the run future is what actually stops the work — the local
    /// adapter kills the command's process group on drop — so the `biased`
    /// select is the mechanism, not just a status check. Mirrors what
    /// `steps/command.rs` already does for the `command` node type: both are
    /// user-authored shell built from [`harness_shell_options`], and they must
    /// not disagree about whether Stop works.
    async fn run_harness_command(
        &self,
        machine_str: &str,
        cmd: &str,
        opts: crate::ports::execution::ShellOptions,
    ) -> Option<Result<String, String>> {
        let mut cancel_watch = self.cancel_watch.clone();
        let cancelled = async move {
            // `wait_for` also resolves — as `Err` — when the sender is dropped.
            // That is "nobody can cancel this any more", not "this was
            // cancelled", so park forever and let the command decide the
            // outcome rather than killing a healthy step during teardown.
            if cancel_watch.wait_for(|c| *c).await.is_err() {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            biased;
            _ = cancelled => None,
            r = self.exec.run_command_with(machine_str, cmd, opts) => Some(r),
        }
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

        if let Some(err) = self.command_never_ran_error(step_exec, machine_str, wt_path, failures) {
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
                self.notify_environment_not_ready(step_exec, &msg);
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

    /// The exit-127 fast path: the shell could not find a binary a harness
    /// command itself invokes. That is objectively an environment gap — the
    /// code never ran, so no amount of editing it can help. Escalate straight
    /// to the terminal `Environment` error rather than spending a `Verdict`
    /// retry (which re-runs the agent against a gate that cannot pass) plus a
    /// triage agent turn to reach the same conclusion on the *next* attempt.
    /// This skips `should_triage`'s reproduce-unchanged requirement on purpose:
    /// a 127 is deterministic, not flaky.
    ///
    /// A method rather than inline code because HB2c gave it a second caller:
    /// `run_harness_first` asks it about the **unsubtracted** failure set,
    /// before the baseline is consulted, so a missing binary stays terminal
    /// even when it was equally missing at the base. `None` means no failure
    /// here names a binary the shell could not find.
    fn missing_command_error(
        &self,
        step_exec: &StepExecution,
        machine_str: &str,
        wt_path: &str,
        failures: &[HarnessRun],
    ) -> Option<crate::domain::verifier::VerifierError> {
        let (failure, missing) = failures
            .iter()
            .find_map(|f| detect_missing_command(&f.cmd, &f.output).map(|m| (f, m)))?;
        let cmd = failure.cmd.as_str();
        let msg = build_environment_message(
            machine_str,
            wt_path,
            cmd,
            &format!(
                "The shell could not find `{}` on PATH (exit 127), so the command never ran.",
                missing
            ),
            &format!(
                "Make `{missing}` *discoverable* on this machine — installed is not enough, it \
                 has to be on the PATH of a fresh interactive login shell, which is what the \
                 harness runs commands under. Check it with:\n\
                 \x20 bash -l -i -c 'command -v {missing}'\n\
                 If that prints nothing, either export the tool's directory from ~/.profile or \
                 ~/.bashrc, or — if a version manager owns it (mise, asdf, nvm, pyenv, rbenv) — \
                 declare it in that manager's *global* config so every shell activates it, not \
                 just the directories that ask for it.",
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
        Some(crate::domain::verifier::VerifierError::Environment(msg))
    }

    /// The sibling of [`missing_command_error`](Self::missing_command_error) for
    /// the failures that *do* reach an exit code: a task runner that ran, but
    /// was asked for a script or target this worktree does not define.
    ///
    /// Same conclusion, same fail-safe direction, different remediation — see
    /// [`build_missing_task_message`] for why reusing the 127 path's wording
    /// would send the user after a package that was never missing.
    fn missing_task_error(
        &self,
        step_exec: &StepExecution,
        machine_str: &str,
        wt_path: &str,
        failures: &[HarnessRun],
    ) -> Option<crate::domain::verifier::VerifierError> {
        let (failure, missing) = failures
            .iter()
            .find_map(|f| detect_missing_task(&f.cmd, &f.output).map(|m| (f, m)))?;
        let cmd = failure.cmd.as_str();
        let msg = build_missing_task_message(machine_str, wt_path, cmd, &missing);
        self.notify_environment_not_ready(step_exec, &msg);
        tracing::warn!(
            feature_id = %self.f_id,
            step_id = %step_exec.step_id.0,
            cmd = %cmd,
            runner = %missing.runner,
            missing = %missing.name,
            "harness command named a {} this worktree does not define — terminating without retries",
            missing.noun()
        );
        Some(crate::domain::verifier::VerifierError::Environment(msg))
    }

    /// "The command never ran" — both shapes of it, in the order that gives the
    /// most specific remediation.
    ///
    /// A binary the shell could not find (exit 127) and a script/target the
    /// runner could not find (exit 1) are one category: the code was never
    /// exercised, so a `Verdict` would redirect an agent to fix something that
    /// was never tested, and it would reproduce identically on every retry until
    /// the budget ran out. Both therefore skip `should_triage`'s
    /// reproduce-unchanged requirement and terminate directly — neither is flaky.
    ///
    /// The 127 check goes first because it is the stronger claim: if the binary
    /// itself is absent, "your project's script list is wrong" would be a
    /// misdiagnosis of a machine that cannot run the tool at all.
    fn command_never_ran_error(
        &self,
        step_exec: &StepExecution,
        machine_str: &str,
        wt_path: &str,
        failures: &[HarnessRun],
    ) -> Option<crate::domain::verifier::VerifierError> {
        self.missing_command_error(step_exec, machine_str, wt_path, failures)
            .or_else(|| self.missing_task_error(step_exec, machine_str, wt_path, failures))
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

    /// Persist + emit the terminal environment-not-ready signal (C6.3), fired
    /// *immediately* on triage (no wasted retries first). Mirrors the
    /// `RetryBudgetExhausted` persistence path so the bell shows it after a
    /// refresh, plus a live event for the toast.
    ///
    /// `pub(crate)` because the baseline node terminates on the same signal
    /// (HB9): a gate that cannot run is the same news to the user whether the
    /// engine noticed it at the head of the graph or at validate, and it must
    /// arrive through the same channel — one that survives a refresh — rather
    /// than as a step error string only the node panel shows.
    pub(crate) fn notify_environment_not_ready(&self, step_exec: &StepExecution, message: &str) {
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

/// Result of scanning free text for a verdict JSON object.
#[derive(Debug)]
pub(crate) enum ParsedVerdict {
    Pass,
    Fail(crate::domain::verifier::VerdictFailure),
    /// The verifier judged the work unjudgeable: the criteria it could not
    /// satisfy demand something the *project* is not configured to do, not
    /// something the code got wrong.
    ///
    /// A third verdict rather than a flavour of `Fail`, because the two
    /// route to opposite places. `Fail` opens a rework loop — the right
    /// answer when an agent can fix what is broken. Nothing an agent writes
    /// can add a `build_command` to project settings, so routing this to
    /// `Fail` spends the entire retry budget re-implementing a feature that
    /// was already correct and ends no better informed. This terminates
    /// once, carrying remediation the user can act on.
    Environment(String),
    /// No JSON object carrying the verdict key was found, or its value was
    /// none of the three above. The string describes the problem.
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

    let val = match parsed_val.or_else(|| recover_unbraced_object(&text_buffer, verdict_key)) {
        Some(v) => v,
        None => {
            // Report against the *turn*, not against a span we stitched together
            // out of it. The old fallback parsed "first `{` in the turn" through
            // "last `}`" and surfaced serde's complaint about that span — which,
            // on a turn that quoted any code, meant the error described a random
            // brace in someone's TypeScript rather than the verdict. The turn's
            // tail is where a verdict is supposed to be, so that is what we show.
            return ParsedVerdict::Missing(format!(
                "No JSON object carrying the verdict key '{}' in the validate turn — the reply \
                 must end with a single JSON object. Turn ended with: {}",
                verdict_key,
                tail_chars(text_buffer.trim(), 300)
            ));
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
        // The verifier can only reach this by being *told* to in its
        // instructions (the shipped validate step is), so an older
        // workflow's verifier can never produce it by accident.
        "environment" => {
            let reason = val
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("The project is not configured to evidence this step's criteria.");
            ParsedVerdict::Environment(reason.to_string())
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

/// The verdict contract appended to a single-turn validate prompt.
///
/// Pure so the *set of verdicts offered* is assertable without building a
/// driver. That set is the whole point: `environment` lived only in the
/// verifier's prose instructions while this menu offered pass and fail, so an
/// agent that correctly judged a criterion unprovable still had to answer
/// `fail` — and `fail` opens a rework loop that re-implements a feature whose
/// defect is a project setting (S13).
pub(crate) fn verdict_contract(verdict_key: &str) -> String {
    format!(
        "After writing your report artifact, END your reply with a single JSON \
         object (no other JSON after it). Choose exactly one of:\n\
         {{ \"{key}\": \"pass\" }}\n\
         or\n\
         {{ \"{key}\": \"fail\", \"reason\": \"what exactly to fix\", \
         \"failing_tests\": [\"test id\"], \"implicated_files\": [\"src/foo.rs\"] }}\n\
         or\n\
         {{ \"{key}\": \"environment\", \"reason\": \"which command is missing and \
         which project setting configures it\" }}\n\n\
         Use `environment` — NOT `fail` — when the criteria you could not confirm \
         are ones this project is not configured to evidence, rather than ones the \
         implementation got wrong. `fail` sends the work back to be \
         re-implemented; nothing an agent writes can add a missing test command, \
         so `fail` there burns the entire rework budget and ends no better \
         informed.",
        key = verdict_key,
    )
}

/// A red gate whose baseline says the gate **could not run on this machine**,
/// so its failure is not a verdict about anything.
///
/// Distinct from [`ExcludedRun`] on purpose, and the distinction is the defect
/// HB2c shipped with: both are "not this feature's fault", and they have
/// opposite consequences. An excluded gate is subtracted and the step passes on
/// what the *other* gates proved; an unrunnable one proved nothing, so passing
/// on it is evidence-free and the run has to stop with remediation instead.
struct UnrunnableGate {
    run: HarnessRun,
    /// The classifier's sentence, as recorded at baseline-measurement time.
    reason: String,
    /// Its provisioning step; may be empty.
    remediation: String,
}

/// Everything the subtraction concluded about one harness pass.
///
/// A struct rather than the tuple this used to return because there are now
/// three destinations a red gate can reach, and a positional tuple of two
/// vectors plus two more fields is exactly where a caller starts binding the
/// wrong one.
struct SubtractedFailures {
    /// The gates this feature answers for — a verdict, feeding the rework loop.
    attributable: Vec<HarnessRun>,
    /// The gates the baseline excused as pre-existing.
    excluded: Vec<ExcludedRun>,
    /// Whether C6's classifier may still be consulted about `attributable`.
    triage_allowed: bool,
    /// The first gate the baseline says cannot run here. `Some` short-circuits
    /// everything else: it is terminal, so no verdict and no subtraction is
    /// reached.
    unrunnable: Option<UnrunnableGate>,
    /// Rung 3's scope across `attributable`: the individual tests failing now
    /// that were not failing at the base.
    ///
    /// **Advisory, and additive.** It travels into
    /// [`VerdictFailure::failing_tests`](crate::domain::verifier::VerdictFailure::failing_tests)
    /// so the rework producer can write one ticket per genuinely new failure
    /// instead of re-deriving a whole gate — the `{{failing_tests}}` a rework
    /// template already renders. Empty is the common case and means *unscoped*,
    /// never "nothing failed": the verdict reason carries every failing gate's
    /// output regardless, so nothing is hidden by an extraction that read
    /// nothing.
    new_failing_tests: Vec<String>,
}

/// The red gates of one harness pass, split by who answers for them.
///
/// One struct rather than three parameters because they are three views of a
/// single decision and are meaningless apart: excluded gates without the
/// attributable ones cannot be reported, and `triage_allowed` is a property of
/// the first attributable gate. Bundling also keeps
/// `classify_harness_failures` under the argument ceiling without reaching for
/// `too_many_arguments`, which AGENTS.md §3 calls a review trigger rather than
/// a fix.
pub(crate) struct HarnessFailureSet<'a> {
    /// The gates this feature answers for. Non-empty at every call site: a set
    /// with nothing attributable never reaches classification.
    pub attributable: &'a [HarnessRun],
    /// The gates the baseline excused, carried so the verdict names them —
    /// otherwise the rework loop is handed a failure list that silently omits
    /// half of what the user can see in the log.
    pub excluded: &'a [ExcludedRun],
    /// Whether C6's triage classifier may still be consulted — see
    /// [`GateDetermination::allows_triage`](crate::domain::harness_delta::GateDetermination::allows_triage).
    pub triage_allowed: bool,
    /// Rung 3's scope: the individual tests among `attributable` that were not
    /// failing at the base. Empty means unscoped, and the verdict then reads
    /// exactly as it did before rung 3 existed.
    pub failing_tests: &'a [String],
}

/// User-facing remediation for a harness command that hit its ceiling.
///
/// A command that produces no exit status inside a generous wall-clock budget
/// is overwhelmingly a runner left in **watch mode**, not a slow suite — and
/// that is a configuration defect no retry can resolve, so the message leads
/// with it. `scripts.test` is very often `vitest` or `jest --watch`, and
/// detection used to emit a bare `npm test` for any repo with a root
/// `package.json` — so this was the default path for a large class of projects,
/// not an exotic one. HB3 now reads the script and either corrects it or
/// declines to emit it, which shrinks the population that reaches here to
/// hand-written commands and watch-mode forms detection does not recognise. It
/// does not empty it: this message is still the only thing standing between a
/// user and a silent half-hour.
fn build_timeout_message(machine_str: &str, wt_path: &str, cmd: &str, ceiling_s: u64) -> String {
    build_environment_message(
        machine_str,
        wt_path,
        cmd,
        &format!(
            "The command produced no exit status within {}s and was abandoned, so nothing was \
             tested. This is not a verdict on the code — the suite never finished running.",
            ceiling_s
        ),
        "The usual cause is a test runner left in **watch mode**, which never exits: `vitest` \
         (use `vitest run`), `jest --watch` (use `jest --ci`), `cargo watch`. Check what the \
         command actually resolves to — for an `npm test` that is the `scripts.test` entry in \
         `package.json` — and change the project's test command to the one-shot form. If the \
         suite is genuinely slower than the ceiling, raise the wall-clock cap in preferences \
         instead.",
    )
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/verifier/triage_tests.rs"]
mod triage_tests;

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/verifier/produced_artifacts_summary_tests.rs"]
mod produced_artifacts_summary_tests;

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/verifier/harness_outcome_tests.rs"]
mod harness_outcome_tests;

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/verifier/parse_verdict_text_tests.rs"]
mod parse_verdict_text_tests;

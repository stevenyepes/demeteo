//! Adjudicating a red harness pass: whose defect is it, and does it end the run?
//!
//! `run_harness_first` establishes the objective half — which gates exited
//! non-zero — and stops there. Everything after that is contested, and this is
//! where it is settled: the never-ran fast path, HB2b's lazy baseline measured
//! against a cancellation race, HB2c's subtraction, and C6's classifier.
//!
//! The decisions are all elsewhere and all pure —
//! [`harness_failure`](crate::domain::harness_failure) for "did it ever run",
//! [`harness_delta`](crate::domain::harness_delta) per gate,
//! [`harness_attribution`](crate::domain::harness_attribution) for the step,
//! [`harness_triage`](crate::domain::harness_triage) for whether the classifier
//! is consulted and how its answer is read. What is left here needs
//! `ExecutionDriver` and genuinely reads it: `cancel_watch`, `resolve_base_sha`,
//! `measure_fallback_baseline`, `features`, `registry`, `f_id` and the
//! failing-tests extractor.
//!
//! The `tokio::select!` against `measure_fallback_baseline` is the only complex
//! thing on the page, and it is a *mechanism*, not a policy: dropping the future
//! is what stops the work.

use super::ExecutionDriver;
use crate::adapters::step_executor::driver::verifier::environment::{
    command_never_ran_error, notify_environment_not_ready,
};
use crate::domain::agent_event::AgentEvent;
use crate::domain::harness_attribution::{
    split_by_determination, HarnessFailureSet, SubtractedFailures,
};
use crate::domain::harness_fingerprint::normalize_failure_fingerprint;
use crate::domain::harness_outcome::{
    build_exclusion_note, build_failure_reason, combined_failure_output, HarnessOutcome, HarnessRun,
};
use crate::domain::harness_remediation::build_environment_message;
use crate::domain::harness_triage::{
    build_triage_prompt, parse_triage_text, triage_decision, TriageDecision, TriageVerdict,
};
use crate::domain::models::StepExecution;
use crate::domain::text::tail_chars;
use crate::domain::verifier::ResolvedHarness;
use crate::ports::agent_runtime::AgentContext;
use tokio_stream::StreamExt;

impl ExecutionDriver {
    /// Decide what a non-empty set of red gates means for the step.
    ///
    /// Reached only when at least one gate exited non-zero, and it answers with
    /// exactly one of the three outcomes a red pass can have: terminal
    /// `Environment` (the command never ran, or the baseline says this machine
    /// cannot run it), a `Verdict` feeding the rework loop, or — when every red
    /// gate was already red identically at the base — an `Ok` carrying the
    /// exclusions so the report can name them.
    ///
    /// `ran` is taken by value because the green gates are only needed on that
    /// last outcome, where they render beside the exclusions.
    pub(super) async fn adjudicate_red_gates(
        &self,
        step_exec: &StepExecution,
        machine_str: &str,
        wt_path: &str,
        resolved: &[ResolvedHarness],
        ran: Vec<HarnessRun>,
        failed: &[HarnessRun],
    ) -> Result<HarnessOutcome, crate::domain::verifier::VerifierError> {
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
            failed,
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
                    resolved,
                    failed,
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
            .subtract_pre_existing(baseline.as_ref(), &base_sha, wt_path, machine_str, failed)
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
    pub(super) async fn classify_harness_failures(
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
            bare_mode: true,
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
}

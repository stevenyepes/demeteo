//! Measuring the harness baseline — the two producers of the HB2a record
//! (`docs/HARNESS_BASELINE.md` HB2b, decision 44).
//!
//! Validate today asks "is the harness green?" and treats any non-zero exit as
//! this feature's verdict, so a repository that was already red sends the run
//! into a rework loop for a defect it did not introduce. The record this module
//! produces is the other half of that subtraction; HB2c is what reads it and
//! changes an outcome. **Nothing here changes a verdict.**
//!
//! # Two producers, one shape
//!
//! 1. **The in-graph node** — `baseline-harness`, a zero-token `command` node at
//!    the head of the Standard and Refactor starters (P4.2a). Cheap and the
//!    default: its wall-clock hides behind research.
//! 2. **The lazy fallback** — fired from `run_harness_first`'s *failure* path
//!    when no stored record covers the run's base. This is what makes the
//!    subtraction unconditional instead of a privilege of the two starters that
//!    carry the node.
//!
//! Both funnel through [`measure_gates`], so a record cannot depend on which
//! producer wrote it beyond the [`BaselineProducer`] stamped on each gate.
//!
//! # Where the decisions live
//!
//! *Whether* to measure a fallback baseline is
//! [`fallback_baseline_needed`](crate::domain::harness_baseline::fallback_baseline_needed)
//! — pure, in `domain/`, reachable from a test with no port doubles. *Which*
//! harnesses gate a step is
//! [`resolve_harnesses`](crate::domain::verifier::resolve_harnesses), likewise
//! pure and, critically, **the same function validate resolves through**: a
//! baseline measured over a different set of gates than validate runs is worse
//! than no baseline. What is left here is execution, and [`measure_gates`] is a
//! free function over the one port it needs rather than a method on
//! `ExecutionDriver`, so it is reachable from a test that stubs an
//! `ExecutionPort` and nothing else (AGENTS.md §3).
//!
//! # The direction this fails in
//!
//! An absent baseline degrades to today's behaviour. A **fabricated** one
//! inverts HB2c's table: a gate wrongly recorded as red-at-base excuses a real
//! regression. So every ambiguity resolves toward recording nothing —
//! a transport failure, a timeout, and a failed `prepare_command` all record
//! *no gate at all* rather than a red one, and the whole fallback returns `()`
//! so it is structurally incapable of changing the verdict it runs beside.

use crate::adapters::step_executor::driver::verifier::{
    classify_exec_failure, harness_block, merge_stderr_into_stdout, HarnessExecFailure, HarnessRun,
    TriageVerdict,
};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::artifact::{Artifact, ArtifactSource};
use crate::domain::harness_baseline::{
    fallback_baseline_needed, BaselineEnvironmentFault, BaselineProducer, HarnessBaseline,
    HarnessBaselineRun,
};
use crate::domain::verifier::ResolvedHarness;
use crate::ports::execution::{ExecutionPort, ShellOptions};

/// One gate as measured: the record to persist, plus the output that record
/// only *references*.
///
/// The two are separated because they have opposite lifetimes. Harness output
/// is megabytes and the record is re-read on every validate attempt, so the
/// output goes to the [`ArtifactStore`](crate::ports::artifact_store::ArtifactStore)
/// and the record keeps a reference — "a baseline you cannot afford to read is
/// not a baseline" (HB2a). Keeping them apart here also means [`measure_gates`]
/// needs no artifact store, and so needs no second port double to test.
#[derive(Debug, Clone)]
pub(crate) struct MeasuredGate {
    /// The record for this gate. `output_ref` is `None` until the caller has
    /// stored the output.
    pub run: HarnessBaselineRun,
    /// Merged stdout+stderr, verbatim.
    pub output: String,
}

/// Answers "was this gate red because the code is broken, or because this
/// machine cannot run it?" for one red baseline measurement.
///
/// A trait rather than a call straight into `ExecutionDriver` because the
/// classification is the *only* thing [`measure_gates`] needs beyond an
/// `ExecutionPort`, and taking a driver for it would drag twenty-odd unread
/// ports into every test of this function (AGENTS.md §3 names constructing an
/// `ExecutionDriver` in a test as the shape to avoid). The production
/// implementation is [`DriverTriage`], which is a thin forward to C6's existing
/// `triage_harness_failure` — there is deliberately no second classifier.
///
/// # Why the classification happens *here*
///
/// C6 reaches the same question through `should_triage`, which requires the
/// failure to have reproduced unchanged across two attempts. Its cheapest
/// possible detection is therefore one full rework cycle — ~$14 and 11M tokens
/// in the run `docs/HARNESS_BASELINE.md` §1 records — and if the implementer
/// perturbs the output in between, the fingerprint comparison resets and it is
/// two. At baseline-measurement time the run is at the head of the graph and
/// **no implement budget has been spent at all**. Same agent, same prompt, same
/// fail-safe; one cycle earlier.
#[async_trait::async_trait]
pub(crate) trait BaselineTriage: Send + Sync {
    /// Classify one gate's red measurement. Must fail safe: every spawn,
    /// timeout, cancellation and parse failure owes
    /// [`TriageVerdict::Regression`], because `Regression` is what leaves the
    /// gate subtractable and the run unaffected.
    async fn classify(&self, harness: &ResolvedHarness, output: &str) -> TriageVerdict;
}

/// Where a measurement is being taken, and on whose behalf.
///
/// One struct rather than five parameters because they always travel together
/// and none of them means anything alone: a machine without a worktree, or a
/// provenance stamp without a time, is not a coherent input. Bundling is also
/// what keeps [`measure_gates`] under the argument ceiling without reaching for
/// the `too_many_arguments` allow, which AGENTS.md §3 calls a review trigger
/// rather than a fix.
pub(crate) struct BaselineSite<'a> {
    /// The `ExecutionPort` machine id.
    pub machine: &'a str,
    /// The already-provisioned worktree the commands run in. The caller owns
    /// its lifecycle, teardown included.
    pub wt_path: &'a str,
    /// The step whose artifact space the outputs are stored under.
    pub step_id: &'a str,
    /// The commit being measured — recorded verbatim on the record, because a
    /// baseline that cannot name its commit is not evidence.
    pub base_sha: &'a str,
    /// Which producer is asking. Stamped per gate.
    pub producer: BaselineProducer,
}

/// Run `prepare_command` and then every resolved harness inside an
/// already-provisioned worktree, and report what each said.
///
/// The caller owns the worktree — provisioning it, and tearing it down on every
/// path including this function's failure paths.
///
/// **A failed `prepare_command` measures nothing.** Returning the harness
/// results anyway would be the single most dangerous thing this module could
/// do: a suite run without its install step fails for reasons that have nothing
/// to do with the base commit, and every such gate would be recorded as
/// red-at-base — which is precisely the shape that excuses a real regression in
/// HB2c. An empty result degrades to today's behaviour; a fabricated one
/// silently disables the verdict.
///
/// Each harness carries **its own** deadline
/// ([`ResolvedHarness::deadline_s`], HB5): the ceiling is per command, applied
/// N times rather than divided N ways.
///
/// A gate whose command could not reach an exit status — the machine went away,
/// or the deadline expired — is **omitted**, not recorded as red, for the same
/// reason `run_harness_first` refuses to call either a verdict: a command that
/// never finished is not a failing command.
///
/// # Every red gate is classified, once
///
/// A red measurement is handed to `triage` before it is recorded, and what it
/// says lands on
/// [`HarnessBaselineRun::environment`](crate::domain::harness_baseline::HarnessBaselineRun::environment).
/// HB2c then *reads* that field rather than re-asking, so the cost is one agent
/// call per red gate per measurement — not per validate attempt — and a green
/// baseline costs nothing at all, because there is no failure to classify.
///
/// It changes no verdict here, exactly like everything else in this module: the
/// record grows a field, and `compare_gate` decides what the field means.
pub(crate) async fn measure_gates(
    exec: &dyn ExecutionPort,
    triage: &dyn BaselineTriage,
    site: &BaselineSite<'_>,
    prepare_command: Option<&str>,
    harnesses: &[ResolvedHarness],
    opts: ShellOptions,
    measured_at: i64,
) -> Vec<MeasuredGate> {
    let (machine, wt_path) = (site.machine, site.wt_path);
    if let Some(cmd) = prepare_command.map(str::trim).filter(|c| !c.is_empty()) {
        if exec
            .run_command_with(machine, &merge_stderr_into_stdout(cmd), opts.clone())
            .await
            .is_err()
        {
            tracing::warn!(
                machine = %machine,
                wt_path = %wt_path,
                cmd = %cmd,
                "baseline prepare command failed — recording no baseline rather than a \
                 measurement taken without it"
            );
            return Vec::new();
        }
    }

    let mut measured = Vec::new();
    for harness in harnesses {
        let opts = ShellOptions {
            timeout: Some(std::time::Duration::from_secs(harness.deadline_s)),
            ..opts.clone()
        };
        // Same `( … ) 2>&1` wrap the live harness uses (D3): the port yields
        // stdout only on success, and a green suite that reports on stderr
        // would otherwise be fingerprinted against an empty string.
        let result = exec
            .run_command_with(machine, &merge_stderr_into_stdout(&harness.command), opts)
            .await;

        let (exit_ok, output) = match result {
            Ok(out) => (true, out),
            Err(out) => match classify_exec_failure(&out) {
                HarnessExecFailure::NonZeroExit => (false, out),
                // No exit status ⇒ no evidence. Recording this gate as red at
                // the base would excuse whatever it does at the tip.
                HarnessExecFailure::Transport | HarnessExecFailure::Timeout => {
                    tracing::warn!(
                        machine = %machine,
                        harness = %harness.name,
                        "baseline harness produced no exit status — omitting the gate"
                    );
                    continue;
                }
            },
        };

        // Only a red gate is classified. A green one has nothing to classify,
        // and paying an agent call to be told so would make every healthy
        // repository fund the unhealthy case.
        let environment = if exit_ok {
            None
        } else {
            match triage.classify(harness, &output).await {
                TriageVerdict::Environment {
                    reason,
                    remediation,
                } => {
                    tracing::warn!(
                        machine = %machine,
                        harness = %harness.name,
                        reason = %reason,
                        "the gate was already red at the base because this machine cannot run \
                         it — recording that, so validate terminates instead of passing on a \
                         gate that proved nothing"
                    );
                    Some(BaselineEnvironmentFault {
                        reason,
                        remediation,
                    })
                }
                // Every failure mode of the classifier lands here (see
                // `BaselineTriage::classify`), and that is the safe direction:
                // no fault recorded ⇒ HB2c reads the gate as a pre-existing
                // defect ⇒ subtracted, which is the behaviour with no
                // classification at all.
                TriageVerdict::Regression => None,
            }
        };

        measured.push(MeasuredGate {
            run: HarnessBaselineRun {
                name: harness.name.clone(),
                command: harness.command.clone(),
                exit_ok,
                // Fingerprinted over the same labelled block the live failure
                // path fingerprints (`combined_failure_output`), so HB2c
                // compares two strings built the same way rather than two
                // shapes that merely look similar. Empty when green: there is
                // no failure to fingerprint.
                fingerprint: if exit_ok {
                    String::new()
                } else {
                    crate::adapters::step_executor::driver::verifier::normalize_failure_fingerprint(
                        &harness_block(&harness.name, &harness.command, &output),
                        wt_path,
                    )
                },
                output_ref: None,
                environment,
                measured_at,
                producer: site.producer,
            },
            output,
        });
    }
    measured
}

/// The production [`BaselineTriage`]: C6's classifier, reached through the
/// driver, asked about a gate in the worktree the measurement ran in.
///
/// It holds borrows rather than owning anything because the measurement it
/// serves is a single `await` inside `record_harness_baseline` — and because a
/// classifier that outlived the site it was built for could be asked about a
/// worktree that no longer exists (the fallback producer tears its own down as
/// soon as the measurement returns).
struct DriverTriage<'a> {
    driver: &'a ExecutionDriver,
    machine: &'a str,
    wt_path: &'a str,
}

#[async_trait::async_trait]
impl BaselineTriage for DriverTriage<'_> {
    async fn classify(&self, harness: &ResolvedHarness, output: &str) -> TriageVerdict {
        // Deliberately the same function `classify_harness_failures` calls: one
        // classifier, one prompt, one fail-safe. A second implementation here
        // would be free to drift, and the direction it drifted in would decide
        // whether a run terminates.
        self.driver
            .triage_harness_failure(self.machine, self.wt_path, &harness.command, output)
            .await
    }
}

impl ExecutionDriver {
    /// The commit this run forked from — the one a baseline is evidence
    /// *about*.
    ///
    /// The fork point, not the feature branch's tip: the tip carries this
    /// feature's own commits, so measuring it would compare the work against
    /// itself. Resolved once per failure path and handed to both consumers —
    /// the producer that measures a fallback and the subtraction that checks
    /// [`HarnessBaseline::covers`] — because two resolutions that could
    /// disagree would silently disable the subtraction rather than fail.
    ///
    /// `None` on any failure (no settings, no merge-base, dead transport).
    /// Callers treat that as *no baseline evidence*, which is today's
    /// behaviour — never as a green base.
    pub(crate) async fn resolve_base_sha(&self) -> Option<String> {
        let feature = self.features.get(&self.f_id).ok().flatten()?;
        let settings = self
            .projects
            .get_settings(&feature.project_id)
            .ok()
            .flatten()?;
        let sha = self
            .git_ops
            .merge_base(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &settings.worktree_strategy.default_branch,
                &self.branch_name,
            )
            .await;
        if sha.is_none() {
            tracing::warn!(
                feature_id = %self.f_id,
                "no merge-base for the feature branch — this run has no baseline evidence"
            );
        }
        sha
    }

    /// Render the `{{harness_baseline}}` prompt block for this run: which gates
    /// will judge the finished work, and what each already said about this
    /// repository.
    ///
    /// The gate list is resolved through
    /// [`resolve_harnesses`](crate::domain::verifier::resolve_harnesses) — the
    /// same chain validate itself resolves through — over the declarations of
    /// **every step in this workflow that carries a verifier**, deduplicated by
    /// name. Asking the project alone would be wrong for a workflow whose
    /// validate step pins its own gates, and telling `s-spec` about gates that
    /// will not run is the same class of lie as telling it about none of them.
    ///
    /// The wording is [`render_harness_briefing`], which is pure and lives in
    /// `domain/`; what happens here is only the two lookups it cannot do.
    /// Anything unreadable yields an empty block rather than a guess: a prompt
    /// section that describes a harness this project does not have is worse
    /// than no section.
    pub(crate) fn render_harness_briefing(
        &self,
        feature: Option<&crate::domain::models::Feature>,
    ) -> String {
        let Some(feature) = feature else {
            return String::new();
        };
        let Some(settings) = self
            .projects
            .get_settings(&feature.project_id)
            .ok()
            .flatten()
        else {
            return String::new();
        };

        let gates = crate::domain::verifier::resolve_gating_harnesses(
            &self.steps,
            &settings.worktree_strategy,
            self.harness_ceiling_s(),
        );

        crate::domain::harness_baseline::render_harness_briefing(
            &gates,
            feature.harness_baseline.as_ref(),
        )
    }

    /// Measure `wt_path` at `base_sha` and fold the result into the feature's
    /// stored record. Shared by both producers.
    ///
    /// Returns the gates that were measured, for the caller's own reporting.
    /// An empty return is "nothing was measured" — never "everything passed".
    ///
    /// The record is merged rather than set
    /// ([`HarnessBaseline::merge`]): the fallback measures only the gates that
    /// just went red, and clobbering would discard the node's measurement of
    /// every other gate.
    pub(crate) async fn record_harness_baseline(
        &self,
        site: &BaselineSite<'_>,
        prepare_command: Option<&str>,
        harnesses: &[ResolvedHarness],
    ) -> Vec<HarnessBaselineRun> {
        let measured = measure_gates(
            self.exec.as_ref(),
            &DriverTriage {
                driver: self,
                machine: site.machine,
                wt_path: site.wt_path,
            },
            site,
            prepare_command,
            harnesses,
            self.harness_shell_options(site.wt_path),
            crate::paths::now_secs() as i64,
        )
        .await;

        if measured.is_empty() {
            return Vec::new();
        }

        let runs: Vec<HarnessBaselineRun> = measured
            .into_iter()
            .map(|gate| {
                let output_ref = self.store_baseline_output(site.step_id, &gate);
                HarnessBaselineRun {
                    output_ref,
                    ..gate.run
                }
            })
            .collect();

        let record = HarnessBaseline {
            base_sha: site.base_sha.to_string(),
            harnesses: runs.clone(),
        };
        if let Err(e) = self.features.merge_harness_baseline(&self.f_id, &record) {
            tracing::warn!(
                feature_id = %self.f_id,
                error = %e,
                "failed to persist the harness baseline — the run continues without one"
            );
        }
        runs
    }

    /// Persist one gate's output. A store failure degrades to `None`: the
    /// measurement itself is still worth recording, and `output_ref` is
    /// documented as optional precisely so a missing log cannot cost us a
    /// baseline.
    fn store_baseline_output(&self, step_id: &str, gate: &MeasuredGate) -> Option<String> {
        let artifact = Artifact {
            name: format!("baseline-{}", gate.run.name),
            mime: "text/plain".to_string(),
            content: gate.output.clone(),
            source: ArtifactSource::AgentText,
        };
        match self.artifacts.put(&self.f_id_str, step_id, &artifact) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!(
                    feature_id = %self.f_id,
                    harness = %gate.run.name,
                    error = %e,
                    "failed to store baseline harness output"
                );
                None
            }
        }
    }

    /// The in-graph producer: the `baseline-harness` node's body (P4.2a).
    ///
    /// The caller has already provisioned a worktree off the **feature
    /// branch** and owns its teardown. At the head of the graph that branch
    /// still points at the base commit, because nothing has been implemented
    /// yet — which is exactly why the node is only valid in that position. We
    /// record the sha we **actually measured** rather than assuming it, so a
    /// node someone later drags halfway down the graph produces a record whose
    /// `base_sha` visibly does not cover the run's base instead of a plausible
    /// lie.
    ///
    /// # It records a verdict; it does not judge one
    ///
    /// A gate that is simply **red** here completes the step. That is the whole
    /// purpose of the baseline: a repository whose suite was already failing is
    /// not this feature's defect, and failing the run at its first node would
    /// restate exactly the misattribution HB2 exists to remove — before a
    /// single line has been written.
    ///
    /// Two things do end the run, and both are the same statement: **this
    /// machine cannot produce evidence about this project.**
    ///
    /// * No measurement at all — a `prepare_command` that fails, or gates that
    ///   never reach an exit status. The worktree can never be made runnable.
    /// * A measurement whose classifier said the gate was red *because it could
    ///   not run here* (HB9). That gate reached an exit status but proved
    ///   nothing, and it will prove nothing at validate either — where the same
    ///   answer already terminates the run, after the entire implement budget.
    ///   Asking here costs zero implement budget, which is the point.
    ///
    /// The decision itself is
    /// [`unrunnable_baseline_gate`](crate::domain::harness_baseline::unrunnable_baseline_gate)
    /// — pure, in `domain/`, reachable from a test with no port doubles
    /// (AGENTS.md §3). What is left here is the notification and the
    /// [`StepOutcome`](super::steps::StepOutcome) it maps onto.
    ///
    /// Returns the same `(outcome, artifact refs)` pair the authored-command
    /// path does, so `handle_command_step` treats the two identically.
    pub(crate) async fn run_baseline_node(
        &self,
        step_exec: &crate::domain::models::StepExecution,
        step_conf: &crate::domain::models::StepConfig,
        machine_str: &str,
        wt_path: &str,
    ) -> (super::steps::StepOutcome, Vec<String>) {
        use super::steps::StepOutcome;

        if *self.cancel_watch.borrow() {
            return (StepOutcome::Cancelled, Vec::new());
        }

        let Some(base_sha) = self
            .git_ops
            .head_sha(self.machine_id_opt.as_deref(), wt_path)
            .await
        else {
            return (
                StepOutcome::Environmental(format!(
                    "baseline node could not resolve the commit it was measuring \
                     (`git rev-parse HEAD` in {wt_path} produced nothing). A measurement \
                     that cannot name its commit is not evidence, so none was recorded."
                )),
                Vec::new(),
            );
        };

        let Some(settings) = self
            .features
            .get(&self.f_id)
            .ok()
            .flatten()
            .and_then(|f| self.projects.get_settings(&f.project_id).ok().flatten())
        else {
            return (
                StepOutcome::Environmental(
                    "baseline node could not read the project's settings, so it does not \
                     know which harnesses to measure."
                        .to_string(),
                ),
                Vec::new(),
            );
        };

        // The same chain validate resolves through, fed the same way: a node
        // may declare `verifier.harness_names` to pin its gates, and otherwise
        // falls through to the project's selection and then its `test_command`
        // — which is what every shipped starter does, and is what keeps the two
        // measuring the same set.
        let declared: &[String] = step_conf
            .verifier
            .as_ref()
            .map(|v| v.harness_names.as_slice())
            .unwrap_or(&[]);
        let harnesses = crate::domain::verifier::resolve_harnesses(
            declared,
            &settings.worktree_strategy,
            self.harness_ceiling_s(),
        );

        let prepare = settings.worktree_strategy.prepare_command.as_deref();
        if harnesses.is_empty() && prepare.is_none_or(str::is_empty) {
            // Nothing is configured. An absence of evidence, not a pass — and
            // not a failure either: a project with no harness is a valid one,
            // it just gets no subtraction. S12's rule applies to how the record
            // reads, and an unwritten record reads as "never measured".
            let refs = self
                .store_baseline_note(
                    &step_exec.step_id.0,
                    "No harness is configured for this project, so nothing was measured. \
                     This is an absence of evidence, not a passing result.",
                )
                .into_iter()
                .collect();
            return (StepOutcome::Completed, refs);
        }

        let runs = self
            .record_harness_baseline(
                &BaselineSite {
                    machine: machine_str,
                    wt_path,
                    step_id: &step_exec.step_id.0,
                    base_sha: &base_sha,
                    producer: BaselineProducer::Node,
                },
                prepare,
                &harnesses,
            )
            .await;

        if *self.cancel_watch.borrow() {
            return (StepOutcome::Cancelled, Vec::new());
        }

        if runs.is_empty() {
            // `measure_gates` records nothing when the prepare command fails or
            // when no gate reached an exit status. Both are the same terminal
            // answer: the machine cannot produce a measurement of this project,
            // which no amount of implementing will change (HB2c's `prepare`
            // row).
            return (
                StepOutcome::Environmental(build_unmeasurable_message(
                    machine_str,
                    wt_path,
                    prepare,
                    &harnesses,
                )),
                Vec::new(),
            );
        }

        let refs: Vec<String> = runs.iter().filter_map(|r| r.output_ref.clone()).collect();

        // A gate that could not run on this machine. The record already knows
        // it — the classifier answered when the gate was measured, moments ago
        // — and validate would reach the identical conclusion from the identical
        // field, only after the whole implement budget has been spent. So say it
        // here, where nothing has been spent at all.
        //
        // The artifact references survive into the failure: the gate's output is
        // the evidence for the remediation, and a terminal step whose Output tab
        // is blank is the opposite of what it is for.
        if let Some(gate) = crate::domain::harness_baseline::unrunnable_baseline_gate(&runs) {
            let msg = crate::adapters::step_executor::driver::verifier::build_environment_message(
                machine_str,
                wt_path,
                gate.command,
                gate.reason,
                gate.remediation,
            );
            self.notify_environment_not_ready(step_exec, &msg);
            tracing::warn!(
                feature_id = %self.f_id,
                step_id = %step_exec.step_id.0,
                harness = %gate.name,
                base_sha = %base_sha,
                "a gate that validation depends on cannot run on this machine — ending the run \
                 at the head of the graph rather than after the implement budget"
            );
            return (StepOutcome::Environmental(msg), refs);
        }

        (StepOutcome::Completed, refs)
    }

    /// Store a short human-readable note as the node's output artifact, so the
    /// node panel's Output tab is never blank for an attempt that ran.
    fn store_baseline_note(&self, step_id: &str, body: &str) -> Option<String> {
        let artifact = Artifact {
            name: "baseline-summary".to_string(),
            mime: "text/plain".to_string(),
            content: body.to_string(),
            source: ArtifactSource::AgentText,
        };
        self.artifacts.put(&self.f_id_str, step_id, &artifact).ok()
    }

    /// The lazy fallback: validate's harness just went red and nothing on
    /// record says what those gates did at the base, so measure it here.
    ///
    /// **Returns `()`, and that is the point.** A baseline mechanism may
    /// withhold an improvement; it may never invent a failure. With no value to
    /// return there is nothing the caller could branch on, so every way this
    /// can go wrong — an unresolvable merge-base, a worktree that cannot be
    /// made, a prepare that fails, a dead transport — leaves the verdict
    /// exactly as it is today. This mirrors how C6's triage fails safe.
    ///
    /// Only the gates that **actually failed** are measured. They are still
    /// resolved through
    /// [`resolve_harnesses`](crate::domain::verifier::resolve_harnesses), so
    /// each command is byte-identical to the one validate just ran; the filter
    /// only drops the gates that were green, which need no baseline because
    /// nothing is being subtracted from them. That is also the partial write
    /// [`HarnessBaseline::merge`] was designed around.
    ///
    /// Cost, stated plainly: on a cold repo this is `prepare_command` plus a
    /// suite, i.e. minutes — which is why it fires only on the failure path,
    /// where the alternative is a rework cycle at $14.63 and 11M tokens.
    ///
    /// `base_sha` is supplied by the caller rather than resolved here, because
    /// HB2c's subtraction needs the *same* commit to check
    /// [`covers`](HarnessBaseline::covers) against. Two independent resolutions
    /// of "the base" that could disagree is precisely the bug `covers` exists
    /// to catch, and it would show up as a fallback that re-measures on every
    /// attempt while the subtraction never fires. An empty string is accepted
    /// and measures nothing: `fallback_baseline_needed` refuses a base it
    /// cannot name.
    pub(crate) async fn measure_fallback_baseline(
        &self,
        step_id: &str,
        machine_str: &str,
        base_sha: &str,
        resolved: &[ResolvedHarness],
        failed: &[HarnessRun],
    ) {
        let Some(feature) = self.features.get(&self.f_id).ok().flatten() else {
            return;
        };
        let Some(settings) = self
            .projects
            .get_settings(&feature.project_id)
            .ok()
            .flatten()
        else {
            return;
        };

        let gates: Vec<ResolvedHarness> = resolved
            .iter()
            .filter(|r| failed.iter().any(|f| f.name == r.name))
            .cloned()
            .collect();
        let names: Vec<String> = gates.iter().map(|g| g.name.clone()).collect();

        if !fallback_baseline_needed(
            !failed.is_empty(),
            base_sha,
            feature.harness_baseline.as_ref(),
            &names,
        ) {
            return;
        }

        let worktree_id = format!("{}-baseline", self.f_id_str);
        let cache_dir = crate::paths::feature_cache_dir(&self.target_dir, &self.branch_name);
        let wt_path = match self
            .git_ops
            .provision_detached_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                base_sha,
                &worktree_id,
                Some(&cache_dir),
            )
            .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    feature_id = %self.f_id,
                    error = %e,
                    "could not provision the detached baseline worktree — no baseline measured"
                );
                return;
            }
        };

        self.record_harness_baseline(
            &BaselineSite {
                machine: machine_str,
                wt_path: &wt_path,
                step_id,
                base_sha,
                producer: BaselineProducer::Fallback,
            },
            settings.worktree_strategy.prepare_command.as_deref(),
            &gates,
        )
        .await;

        // Torn down on every path — this one included, where the measurement
        // itself came back red. The worktree is disposable by construction
        // (detached, no branch), so leaving one behind is pure debris.
        let _ = self
            .git_ops
            .cleanup_detached_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &worktree_id,
            )
            .await;
    }
}

/// The message a baseline node fails with when the project's commands cannot
/// be measured at all — a `prepare_command` that exits non-zero, or gates that
/// never reached an exit status.
///
/// Reuses `build_environment_message` so the reproduce line, the machine, and
/// the shape of the text are identical to every other terminal environment
/// failure the engine produces (C6.3). Authoring a parallel wording here would
/// drift out of agreement with the one the user has already learned to read.
fn build_unmeasurable_message(
    machine: &str,
    wt_path: &str,
    prepare: Option<&str>,
    harnesses: &[ResolvedHarness],
) -> String {
    let cmd = prepare
        .map(str::to_string)
        .or_else(|| harnesses.first().map(|h| h.command.clone()))
        .unwrap_or_default();
    crate::adapters::step_executor::driver::verifier::build_environment_message(
        machine,
        wt_path,
        &cmd,
        "The project's configured commands could not be measured on this machine: either \
         the prepare command failed, or no harness produced an exit status. Nothing was \
         recorded, because a suite measured without its install step is not evidence about \
         the base commit.",
        // The settings panel (HB6) states the same two facts before a run is
        // ever paid for, so the sentence lives in one place and both sites read
        // it — a second copy would drift out of agreement with this one.
        crate::adapters::step_executor::preflight::FRESH_CHECKOUT_REMEDIATION,
    )
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/step_executor/baseline_tests.rs"]
mod baseline_tests;

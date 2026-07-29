//! Measuring the harness baseline — the in-graph producer of the HB2a record
//! (`docs/HARNESS_BASELINE.md` HB2b / P4.2a, decision 44).
//!
//! Validate today asks "is the harness green?" and treats any non-zero exit as
//! this feature's verdict, so a repository that was already red sends the run
//! into a rework loop for a defect it did not introduce. The record this module
//! produces is the other half of that subtraction; HB2c is what reads it and
//! changes an outcome. **Nothing here changes a verdict.**
//!
//! # The producer
//!
//! `baseline-harness`, a zero-token `command` node at the head of the Standard
//! and Refactor starters (P4.2a). Cheap and the default: its wall-clock hides
//! behind research, which in `f-1785157902856` ran ~31 minutes before implement
//! started.
//!
//! A second producer — a lazy fallback on validate's failure path, for the
//! graphs this node cannot reach — funnels through the same [`measure_gates`],
//! which is why each gate carries its own [`BaselineProducer`] rather than the
//! record carrying one.
//!
//! # Where the decisions live
//!
//! *Which* harnesses gate a step is
//! [`resolve_harnesses`](crate::domain::verifier::resolve_harnesses) — pure,
//! and critically **the same function validate resolves through**: a baseline
//! measured over a different set of gates than validate runs is worse than no
//! baseline. What is left here is execution, and [`measure_gates`] is a free
//! function over the one port it needs rather than a method on
//! `ExecutionDriver`, so it is reachable from a test that stubs an
//! `ExecutionPort` and nothing else (AGENTS.md §3).
//!
//! # The direction this fails in
//!
//! An absent baseline degrades to today's behaviour. A **fabricated** one
//! inverts HB2c's table: a gate wrongly recorded as red-at-base excuses a real
//! regression. So every ambiguity resolves toward recording nothing —
//! a transport failure, a timeout, and a failed `prepare_command` all record
//! *no gate at all* rather than a red one.

use crate::adapters::step_executor::driver::verifier::{
    classify_exec_failure, harness_block, merge_stderr_into_stdout, HarnessExecFailure,
};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::artifact::{Artifact, ArtifactSource};
use crate::domain::harness_baseline::{BaselineProducer, HarnessBaseline, HarnessBaselineRun};
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
pub(crate) async fn measure_gates(
    exec: &dyn ExecutionPort,
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
                measured_at,
                producer: site.producer,
            },
            output,
        });
    }
    measured
}

impl ExecutionDriver {
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
    /// # It records; it does not judge
    ///
    /// A red gate here completes the step. That is the whole purpose of the
    /// baseline: a repository whose suite was already failing is not this
    /// feature's defect, and failing the run at its first node would restate
    /// exactly the misattribution HB2 exists to remove — before a single line
    /// has been written. What *does* end the run is an environment that cannot
    /// produce a measurement at all: a `prepare_command` that fails means the
    /// worktree can never be made runnable, and no amount of implementing
    /// changes that.
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

        let refs = runs.iter().filter_map(|r| r.output_ref.clone()).collect();
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
        "Run the command below in a *fresh* checkout — that is what this step gets, with no \
         `node_modules` and no `target/`. If it needs an install step, set the project's \
         prepare command; if it hangs, it is most likely a watch-mode runner, which never \
         exits.",
    )
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/step_executor/baseline_tests.rs"]
mod baseline_tests;

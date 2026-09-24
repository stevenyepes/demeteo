//! The in-graph producer: the `baseline-harness` node's body (P4.2a), and the
//! checkout it measures in.
//!
//! *Where* it measures is
//! [`baseline_node_site`](crate::domain::harness_baseline::baseline_node_site)
//! — pure, in `domain/`. What is left here is the detached checkout that
//! answer may ask for, and its teardown.

use super::BaselineSite;
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::harness_shell::harness_ceiling_s;
use crate::domain::artifact::{Artifact, ArtifactSource};
use crate::domain::harness_baseline::{
    baseline_node_answer, baseline_node_site, baseline_node_verdict, BaselineNodeAnswer,
    BaselineProducer, HarnessBaselineRun, NodeSite,
};
use crate::domain::verifier::ResolvedHarness;

/// A node measurement, and where it was actually taken — which is not the
/// node's own worktree when the run's fork point is not its head.
struct NodeMeasurement {
    runs: Vec<HarnessBaselineRun>,
    /// Where the gates ran, which decides whether a failure ends the run
    /// ([`baseline_node_answer`]). Only [`NodeSite::InPlace`] can, so a
    /// terminal message's reproduce line always names the node's own worktree,
    /// never a fork-point checkout that has already been torn down.
    site: NodeSite,
}

impl ExecutionDriver {
    /// The in-graph producer: the `baseline-harness` node's body (P4.2a).
    ///
    /// The caller has already provisioned a worktree off the **feature
    /// branch** and owns its teardown. At the head of an ordinary run that
    /// branch still points at the fork point, and the gates run there. When it
    /// does not — a fix run cut from a pull request's head, or a node dragged
    /// halfway down the graph — they run in a detached checkout of the fork
    /// point instead ([`baseline_node_site`]). Either way the record names the
    /// sha **actually measured**, so a node that could not resolve a fork point
    /// produces a record whose `base_sha` visibly does not cover the run's base
    /// rather than a plausible lie.
    ///
    /// # It records a verdict; it does not judge one
    ///
    /// What a measurement means for the run is [`baseline_node_answer`] —
    /// pure, in `domain/`, reachable from a test with no port doubles
    /// (AGENTS.md §3), and where the reasoning for every answer lives. What is
    /// left here is the notification, the wordings, and the
    /// [`StepOutcome`](crate::adapters::step_executor::steps::StepOutcome) each
    /// answer maps onto.
    ///
    /// Returns the same `(outcome, artifact refs)` pair the authored-command
    /// path does, so `handle_command_step` treats the two identically.
    pub(crate) async fn run_baseline_node(
        &self,
        step_exec: &crate::domain::models::StepExecution,
        step_conf: &crate::domain::models::StepConfig,
        machine_str: &str,
        wt_path: &str,
    ) -> (
        crate::adapters::step_executor::steps::StepOutcome,
        Vec<String>,
    ) {
        use crate::adapters::step_executor::steps::StepOutcome;

        if *self.cancel_watch.borrow() {
            return (StepOutcome::Cancelled, Vec::new());
        }

        let Some(head_sha) = self
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
            harness_ceiling_s(self.app_settings.as_ref()),
        );

        let prepare = settings.worktree_strategy.prepare_command.as_deref();
        if crate::domain::harness_baseline::nothing_to_measure(&harnesses, prepare) {
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

        let NodeMeasurement { runs, site } = self
            .measure_node_site(
                &BaselineSite {
                    machine: machine_str,
                    wt_path,
                    step_id: &step_exec.step_id.0,
                    base_sha: &head_sha,
                    producer: BaselineProducer::Node,
                },
                prepare,
                &harnesses,
            )
            .await;

        if *self.cancel_watch.borrow() {
            return (StepOutcome::Cancelled, Vec::new());
        }

        let mut refs: Vec<String> = runs.iter().filter_map(|r| r.output_ref.clone()).collect();

        match baseline_node_answer(&site, baseline_node_verdict(&runs)) {
            BaselineNodeAnswer::EndUnmeasurable => (
                StepOutcome::Environmental(build_unmeasurable_message(
                    machine_str,
                    wt_path,
                    prepare,
                    &harnesses,
                )),
                Vec::new(),
            ),
            // The artifact references survive into the failure: the gate's
            // output is the evidence for the remediation, and a terminal step
            // whose Output tab is blank is the opposite of what it is for.
            BaselineNodeAnswer::EndUnrunnable(gate) => {
                let msg = crate::domain::harness_remediation::build_environment_message(
                    machine_str,
                    wt_path,
                    gate.command,
                    gate.reason,
                    gate.remediation,
                );
                crate::adapters::step_executor::driver::verifier::environment::notify_environment_not_ready(
                    &self.environment_signal(),
                    step_exec,
                    &msg,
                );
                tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    harness = %gate.name,
                    base_sha = %head_sha,
                    "a gate that validation depends on cannot run on this machine — ending the run \
                     at the head of the graph rather than after the implement budget"
                );
                (StepOutcome::Environmental(msg), refs)
            }
            BaselineNodeAnswer::CompletedWithoutBase => {
                tracing::info!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    site = ?site,
                    "the run's fork point could not be measured here — continuing with no \
                     baseline to subtract"
                );
                refs.extend(self.store_baseline_note(
                    &step_exec.step_id.0,
                    "The run's fork point could not be measured on this machine. No \
                     pre-existing failure can be subtracted on the strength of this \
                     measurement. This is an absence of evidence about the base, not a \
                     verdict on the branch this run validates.",
                ));
                (StepOutcome::Completed, refs)
            }
            BaselineNodeAnswer::CompletedWithUnrunnableGate => {
                refs.extend(self.store_baseline_note(
                    &step_exec.step_id.0,
                    "A gate at the run's fork point could not run on this machine. Its \
                     failure cannot be subtracted; measurements of other gates at the fork \
                     point remain available for comparison. This is not a verdict on the \
                     branch this run validates.",
                ));
                (StepOutcome::Completed, refs)
            }
            BaselineNodeAnswer::Completed => (StepOutcome::Completed, refs),
        }
    }

    /// Measure `in_place` — the node's own worktree at its head — or, when the
    /// run's fork point is another commit, a detached checkout of that commit.
    ///
    /// The fork point is resolved through
    /// [`resolve_base_sha`](Self::resolve_base_sha), the same call the
    /// subtraction makes, so the record's `base_sha` is the one it will ask
    /// [`covers`](crate::domain::harness_baseline::HarnessBaseline::covers)
    /// about. A checkout that cannot be made measures in place, and counts as
    /// [`NodeSite::InPlace`] — the head is what the run validates, so the
    /// terminal answers apply. The record will not be matched and a red run's
    /// fallback replaces it, but the early `Unrunnable` halt still fires, and
    /// that is worth the gate run alone.
    ///
    /// Its worktree id is not the fallback's `{f_id}-baseline`: provisioning
    /// clears whatever occupies the path first, so sharing one would let either
    /// producer delete the other's checkout mid-measurement.
    async fn measure_node_site(
        &self,
        in_place: &BaselineSite<'_>,
        prepare: Option<&str>,
        harnesses: &[ResolvedHarness],
    ) -> NodeMeasurement {
        let fork_point = self.resolve_base_sha().await;
        let NodeSite::ForkPoint(fork_sha) =
            baseline_node_site(in_place.base_sha, fork_point.as_deref())
        else {
            return self.measure_in_place(in_place, prepare, harnesses).await;
        };

        let worktree_id = format!("{}-baseline-node", self.f_id_str);
        let cache_dir = crate::paths::feature_cache_dir(&self.target_dir, &self.branch_name);
        let detached = match self
            .git_ops
            .provision_detached_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &fork_sha,
                &worktree_id,
                Some(&cache_dir),
            )
            .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    feature_id = %self.f_id,
                    fork_point = %fork_sha,
                    error = %e,
                    "could not check out the fork point for the baseline node — measuring its \
                     head instead, which the subtraction will not match"
                );
                return self.measure_in_place(in_place, prepare, harnesses).await;
            }
        };

        let runs = self
            .record_harness_baseline(
                &BaselineSite {
                    wt_path: &detached,
                    base_sha: &fork_sha,
                    ..*in_place
                },
                prepare,
                harnesses,
            )
            .await;

        // Torn down before anything reads the result, so a red measurement and
        // a cancelled one leave nothing behind either.
        let _ = self
            .git_ops
            .cleanup_detached_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &worktree_id,
            )
            .await;

        NodeMeasurement {
            runs,
            site: NodeSite::ForkPoint(fork_sha),
        }
    }

    async fn measure_in_place(
        &self,
        site: &BaselineSite<'_>,
        prepare: Option<&str>,
        harnesses: &[ResolvedHarness],
    ) -> NodeMeasurement {
        NodeMeasurement {
            runs: self.record_harness_baseline(site, prepare, harnesses).await,
            site: NodeSite::InPlace,
        }
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
    crate::domain::harness_remediation::build_environment_message(
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
        crate::domain::harness_preflight::verdict::FRESH_CHECKOUT_REMEDIATION,
    )
}

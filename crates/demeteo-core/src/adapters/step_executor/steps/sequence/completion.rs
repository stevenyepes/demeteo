//! What a `sequence` step hands downstream once every stage has passed: the
//! summary diff, the artifact references, and the completed row.

use crate::adapters::step_executor::artifacts::compute_git_diff;
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::artifact::Artifact;
use crate::domain::sequence::progress::StepTally;
use crate::domain::sequence::sha::Sha;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

use super::context::{RunTarget, StepCtx, StepSpend};

impl ExecutionDriver {
    /// Summary artifact: the whole feature's diff, computed from the
    /// fork point rather than this attempt's base, so a retry's critic
    /// reviews the complete change and not just the incremental fix.
    ///
    /// Two-dot range against `target_dir` rather than a single-ref
    /// `git diff`: `target_dir` sits on the default branch for the
    /// whole run (the feature branch is only ever a ref), so a
    /// single-ref diff would compare the default branch's working tree
    /// against the range start and render the implementation as
    /// additions that exist in commits but not on disk — which reads as
    /// "the code was committed then reverted".
    pub(crate) async fn collect_step_refs(
        &self,
        step: StepCtx<'_>,
        target: RunTarget<'_>,
        base_sha: &Sha,
        tally: &StepTally,
    ) -> Vec<String> {
        let step_exec = step.step_exec;
        let machine_str = target.machine;

        let diff_ref = match self.resolve_fork_point_ref(machine_str).await {
            Some(fork_point) => format!("{}..{}", fork_point, self.branch_name),
            None => format!("{}..{}", base_sha, self.branch_name),
        };
        let diff_body =
            compute_git_diff(&*self.exec, machine_str, &self.target_dir, &diff_ref).await;
        let mut refs = Vec::new();
        if !diff_body.trim().is_empty() {
            let diff_artifact = Artifact {
                name: "code-diff".into(),
                mime: "text/x-diff".into(),
                content: diff_body,
                source: crate::domain::artifact::ArtifactSource::Diff {
                    base: base_sha.to_string(),
                    head: self.branch_name.clone(),
                    path_filter: None,
                },
            };
            if let Ok(reference) =
                self.artifacts
                    .put(&self.f_id_str, &step_exec.step_id.0, &diff_artifact)
            {
                refs.push(reference);
            }
        }
        refs.extend(tally.artifact_refs().iter().cloned());
        // A reference is a stable path, so the store's own listing can name
        // the artifact this attempt just wrote — `list_for_step` on the
        // resumed-whole-list path returns the previous attempt's `code-diff`
        // at the same path the `put` above returns. Keep the first mention.
        {
            let mut seen = std::collections::HashSet::new();
            refs.retain(|r| seen.insert(r.clone()));
        }
        refs
    }

    /// Mark the step completed and announce it.
    pub(crate) fn mark_step_completed(
        &self,
        step: StepCtx<'_>,
        spend: &StepSpend<'_>,
        refs: Vec<String>,
    ) -> StepOutcome {
        let step_exec = step.step_exec;
        let (cost, tokens) = (*spend.cost, *spend.tokens);
        let wall = spend.start.elapsed().as_secs();
        let primary = refs.first().cloned();
        let _ = self.features.step_update(
            &step_exec.id,
            &StepExecutionPatch {
                last_failure_fingerprint: None,
                iteration_count: None,
                status: Some("completed".to_string()),
                cost_usd: Some(Some(cost)),
                tokens: Some(Some(tokens)),
                wall_clock_secs: Some(Some(wall)),
                artifact_path: Some(primary),
                artifact_paths: Some(refs),
                error_message: Some(None),
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        );
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: "completed".into(),
            cost_usd: Some(cost),
            tokens: Some(tokens),
            wall_clock_secs: Some(wall),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        });
        StepOutcome::Completed
    }
}

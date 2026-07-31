use crate::domain::ids::FeatureId;
use crate::domain::models::StepExecution;
use crate::domain::run_control::active_predecessor_refusal;
use crate::error::AppError;

use super::super::DagStepExecutor;

impl DagStepExecutor {
    /// Read what [`active_predecessor_refusal`] needs — the feature's step
    /// rows and its ancestor set — and obey the answer. Used by `step_retry`
    /// and `gate_decide`.
    ///
    /// The ancestor set comes from the feature's pinned workflow version
    /// migrated to the v2 graph (P1.12). A resolution miss hands `None` to the
    /// policy, which has its own fallback.
    pub(crate) fn assert_no_active_predecessors(
        &self,
        target: &StepExecution,
        intent: &str,
    ) -> Result<(), AppError> {
        let siblings = self
            .features
            .steps_for_feature(&target.feature_id)
            .map_err(AppError::from)?;

        let ancestors: Option<std::collections::HashSet<crate::domain::ids::StepId>> = self
            .resolve_feature_graph(&target.feature_id)
            .and_then(|graph| {
                graph
                    .ancestors(&target.step_id)
                    .map(|set| set.into_iter().cloned().collect())
            });

        match active_predecessor_refusal(target, &siblings, ancestors.as_ref(), intent) {
            Some(refusal) => Err(AppError::validation(refusal)),
            None => Ok(()),
        }
    }

    /// Best-effort resolution of a feature's scheduling graph: pinned
    /// workflow version → its schema-v2 definition (stored document, or the
    /// migration of its step list) → graph. Any miss (no workflow,
    /// unbuildable graph) yields `None` and callers fall back to v1 index
    /// ordering.
    pub(super) fn resolve_feature_graph(
        &self,
        feature_id: &FeatureId,
    ) -> Option<crate::domain::workflow_graph::WorkflowGraph> {
        let feature = self.features.get(feature_id).ok().flatten()?;
        let wf_id = feature.workflow_id?;
        let version = self
            .resolve_pinned_version(feature_id.as_str(), &wf_id)
            .ok()?;
        let def = version.definition(wf_id.as_str());
        crate::domain::workflow_graph::WorkflowGraph::build(&def).ok()
    }
}

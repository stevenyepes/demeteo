//! Persisting the implementation report ([`crate::domain::sequence::report`]).
//!
//! Free functions over the one port they need (AGENTS.md §3). The store is the
//! driver's own, host-side on every transport, so a fragment written by a
//! ticket that ran over SSH or on the runner lands where the step's completion
//! reads it back.

use crate::domain::artifact::{Artifact, ArtifactSource};
use crate::domain::sequence::report::{
    fragment_name, is_fragment_ref, render_implementation_report, TicketReport, REPORT_NAME,
};
use crate::ports::artifact_store::ArtifactStore;

/// Store one ticket's fragment. Best-effort: a missing fragment costs the
/// report a section, never the ticket its commit.
pub(crate) fn record_ticket_report(
    store: &dyn ArtifactStore,
    feature_id: &str,
    step_id: &str,
    report: &TicketReport,
) {
    let content = match serde_json::to_string(report) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(feature_id, step_id, error = %e, "could not serialize ticket report");
            return;
        }
    };
    let artifact = Artifact {
        name: fragment_name(&report.ticket_id),
        mime: "application/json".into(),
        content,
        source: ArtifactSource::AgentText,
    };
    if let Err(e) = store.put(feature_id, step_id, &artifact) {
        tracing::warn!(feature_id, step_id, error = %e, "could not store ticket report");
    }
}

/// Render every fragment the step holds into `implementation-report.md` and
/// return its reference, or `None` when there is nothing to render.
pub(crate) fn store_implementation_report(
    store: &dyn ArtifactStore,
    feature_id: &str,
    step_id: &str,
) -> Option<String> {
    let refs = store.list_for_step(feature_id, step_id).ok()?;
    let fragments: Vec<TicketReport> = refs
        .iter()
        .filter(|r| is_fragment_ref(r))
        .filter_map(|r| store.get(r).ok())
        .filter_map(|body| serde_json::from_str(&body).ok())
        .collect();
    let body = render_implementation_report(&fragments)?;
    store
        .put(
            feature_id,
            step_id,
            &Artifact::agent_text(REPORT_NAME, body),
        )
        .map_err(|e| {
            tracing::warn!(feature_id, step_id, error = %e, "could not store implementation report")
        })
        .ok()
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/sequence/report.rs"]
mod tests;

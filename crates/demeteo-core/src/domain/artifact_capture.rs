//! Whether a declared deliverable arrived, and what to call it when it did
//! not.
//!
//! A sibling of [`artifact`](crate::domain::artifact), not an addition to it:
//! that module is the *model* — what a declaration is, how it captures — and
//! this is the *decision* the executor makes about one, in the same way
//! `harness_baseline` and `harness_delta` pair.
//!
//! The two renderings below describe the same condition and are deliberately
//! **not** unified. They are read by different consumers at different points:
//! [`missing_deliverables_message`] fails the step and is what the UI shows
//! on the failed row, while [`note_undelivered_artifacts`] is appended to a
//! verdict a downstream step will read as feedback. Collapsing them into one
//! string is a behaviour change, not a cleanup.

/// A declared artifact (`ByName` / `LastWriteTo`) that the agent's turn
/// produced no matching output for. Surfaced by
/// `resolve_declared_artifacts` so the step executor can **fail** the
/// step with an actionable message instead of silently marking it
/// `completed` with an empty deliverable (the "green step, no plan
/// artifact" misconfiguration class). `detail` is a human hint about
/// what the capture expected (the path or name).
#[derive(Debug, Clone)]
pub(crate) struct MissingArtifact {
    pub name: String,
    pub detail: String,
}

/// Why a step that merged cleanly still failed.
///
/// The step ran to a clean merge, but a declared deliverable
/// (`ByName` / `LastWriteTo`) never materialised — fail instead of
/// marking a green step with an empty artifact. This is the visible
/// signal for the "agent ran but produced no plan/spec/report"
/// misconfiguration class (bad model/tooling, a project `opencode.json`
/// that blocks writes, agent wrote to the wrong path). The driver
/// persists this message as the step's `error_message`, which the UI
/// renders on the failed step, and routes it through `on_failure` retry.
///
/// Naming every missing deliverable, and its `detail`, is what makes the
/// message actionable: "no plan was produced" is not something a user can
/// act on, "'plan' (artifacts/plan.md)" is.
pub(crate) fn missing_deliverables_message(missing: &[MissingArtifact]) -> String {
    let deliverables = missing
        .iter()
        .map(|m| format!("'{}' ({})", m.name, m.detail))
        .collect::<Vec<_>>()
        .join(", ");
    let plural = if missing.len() == 1 {
        "declared artifact was"
    } else {
        "declared artifacts were"
    };
    format!(
        "The step completed but {count} {plural} never produced: {deliverables}. \
         The agent ran but did not write its required deliverable — it may have \
         failed, written to a different path, or been blocked by its model/config \
         or the project's `opencode.json` (MCP servers, tool permissions). \
         Nothing downstream can consume this step.",
        count = missing.len(),
        plural = plural,
        deliverables = deliverables,
    )
}

/// Append a note about undelivered artifacts to a *failing verdict's* reason.
///
/// A verdict failure returns before the ordinary declared-artifact check, and
/// deliberately keeps doing so: the verdict is the more actionable outcome and
/// its reason is what the rework step reads. But the step that consumes this
/// one attaches the report by name, so "rejected, and there is no report to
/// read" has to reach that step somehow — silently dropping it is how a
/// verdict-failed validate came to look identical to one that never wrote its
/// deliverable (S14).
///
/// Returns `reason` unchanged when nothing is missing, which is the common case.
pub(crate) fn note_undelivered_artifacts(reason: &str, missing: &[MissingArtifact]) -> String {
    if missing.is_empty() {
        return reason.to_string();
    }
    let deliverables = missing
        .iter()
        .map(|m| format!("'{}' ({})", m.name, m.detail))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{reason}\n\nNote for the next attempt: this step also failed to produce \
         {deliverables}, so no report is attached for the step that reads it. The \
         verdict above is the whole of the feedback available."
    )
}

#[cfg(test)]
#[path = "../../tests/domain/artifact_capture.rs"]
mod tests;

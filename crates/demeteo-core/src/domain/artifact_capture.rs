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

use crate::domain::artifact::{Artifact, ArtifactCapture, ArtifactDecl, ArtifactSource};

/// A catch-all capture whose producer is not an agent event, so nothing in the
/// turn's output can match it. The variant exists only so the caller can pick
/// the word in its own diagnostic; the wording is the adapter's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnwiredCapture {
    Diff,
    Worktree,
}

/// What one declaration's capture found in the turn's output.
#[derive(Debug)]
pub(crate) enum CaptureOutcome<'a> {
    /// The declaration matched. The caller stores it and records the reference.
    Store(&'a Artifact),
    /// Nothing to do for this declaration in this pass — either a catch-all
    /// handled elsewhere (`AllWrites`, `ChangedFiles`) or one whose producer is
    /// not an agent event at all, which the `Some` names.
    Skip(Option<UnwiredCapture>),
    /// A declared deliverable never materialised.
    Missing(MissingArtifact),
}

/// Resolve one declaration against the artifacts an agent turn produced.
///
/// D3: a declared capture (e.g. `LastWriteTo`) that matched no
/// `ArtifactProduced` event is a **surfaced diagnostic, not a silent skip** —
/// this is the signal that the step "succeeded" but its declared deliverable
/// never materialised. Only `ByName` / `LastWriteTo` can reach
/// [`Missing`](CaptureOutcome::Missing); every catch-all skips instead, because
/// an empty result is legitimate for them.
///
/// The two matchers deliberately look at **opposite ends** of the turn:
///
/// * `LastWriteTo` takes the **last** write to the path — the agent's final
///   version of a file it revised, not its first draft.
/// * `ByName` matches the artifact's own name, or its name with the extension
///   stripped through [`Path::file_stem`](std::path::Path::file_stem), so a
///   declaration for `b` matches a produced `a/b.md`.
pub(crate) fn resolve_capture<'a>(
    decl: &ArtifactDecl,
    produced: &'a [Artifact],
) -> CaptureOutcome<'a> {
    let matched: Option<&Artifact> = match &decl.capture {
        ArtifactCapture::ByName { name } => produced
            .iter()
            .find(|a| a.name == *name || strip_extension(&a.name).is_some_and(|s| s == *name)),
        ArtifactCapture::LastWriteTo { path } => produced
            .iter()
            .rfind(|a| matches!(&a.source, ArtifactSource::ToolWrite { path: p } if p == path)),
        // The `AllWrites` catch-all emits one artifact per unique path; see
        // `all_writes_selection`.
        ArtifactCapture::AllWrites => return CaptureOutcome::Skip(None),
        // `ChangedFiles` artifacts are detected directly via git diff by the
        // agent step and added to the produced list there. They are named by
        // their file basenames, so they can be matched here by name if needed.
        ArtifactCapture::ChangedFiles { .. } => return CaptureOutcome::Skip(None),
        // Derived at materialisation time by `GitOpsHelper`. No agent event
        // matches them.
        ArtifactCapture::Diff { .. } => {
            return CaptureOutcome::Skip(Some(UnwiredCapture::Diff));
        }
        // Synthesised by the executor from branch/machine state. No agent event
        // matches them.
        ArtifactCapture::Worktree { .. } => {
            return CaptureOutcome::Skip(Some(UnwiredCapture::Worktree));
        }
    };

    match matched {
        Some(artifact) => CaptureOutcome::Store(artifact),
        None => CaptureOutcome::Missing(MissingArtifact {
            name: decl.name.clone(),
            detail: match &decl.capture {
                ArtifactCapture::LastWriteTo { path } => {
                    format!("expected a write to `{}`", path)
                }
                ArtifactCapture::ByName { name } => {
                    format!("expected an artifact named `{}`", name)
                }
                // Unreachable — the other captures return above — but keep it
                // total so a future capture kind is handled.
                _ => "no matching agent output".to_string(),
            },
        }),
    }
}

/// The `AllWrites` catch-all: every unique `ToolWrite` path, **first** artifact
/// per path, in the order the turn produced them.
///
/// The opposite end from `LastWriteTo`'s `rfind`, and deliberately so: the
/// catch-all is an inventory of what the step touched, where the first write is
/// the one that establishes the path.
///
/// Empty when no declaration asks for it.
pub(crate) fn all_writes_selection<'a>(
    declarations: &[ArtifactDecl],
    produced: &'a [Artifact],
) -> Vec<&'a Artifact> {
    if !declarations
        .iter()
        .any(|d| matches!(d.capture, ArtifactCapture::AllWrites))
    {
        return Vec::new();
    }
    let mut seen_paths = std::collections::HashSet::new();
    produced
        .iter()
        .filter(|artifact| match &artifact.source {
            ArtifactSource::ToolWrite { path } => seen_paths.insert(path.clone()),
            _ => false,
        })
        .collect()
}

fn strip_extension(name: &str) -> Option<String> {
    std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

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

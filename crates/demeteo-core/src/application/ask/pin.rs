//! Pin an Ask Canvas message into the `ArtifactStore`, list what a thread has
//! pinned, and export a canvas independently of pinning.
//!
//! `pin_canvas` and `export_canvas` both go through [`load`](super::load) to
//! get the same [`AskMessageView`](super::AskMessageView) `parse_ask_turn`
//! already builds on every read, rather than re-deriving a turn from raw
//! message text a second time.

use serde::{Deserialize, Serialize};

use crate::domain::ask_canvas::{build_pinned_canvas_snapshot, PinnedCanvasSnapshot};
use crate::domain::ids::AskThreadId;
use crate::state::AppContext;

const SCOPE: &str = "ask-canvas";

/// Freeze a message's canvas into the artifact store, under a name derived
/// only from `message_id` — so re-pinning the same message overwrites the
/// same path rather than appending a duplicate to [`list_pinned`].
pub fn pin_canvas(
    ctx: &AppContext,
    thread_id: &AskThreadId,
    message_id: &str,
) -> Result<String, String> {
    pin_canvas_at(ctx, thread_id, message_id, crate::paths::now_ms())
}

/// [`pin_canvas`] with the pin's timestamp supplied rather than sampled, so a
/// test can drive the pin and the export at one `pinned_at` and compare the
/// two [`PinnedCanvasSnapshot`](crate::domain::ask_canvas::PinnedCanvasSnapshot)s
/// whole.
fn pin_canvas_at(
    ctx: &AppContext,
    thread_id: &AskThreadId,
    message_id: &str,
    pinned_at: i64,
) -> Result<String, String> {
    let snapshot = snapshot_for(ctx, thread_id, message_id, pinned_at)?;
    let artifact = crate::domain::artifact::Artifact::pinned_ask_canvas(
        thread_id.as_str(),
        message_id,
        &snapshot,
    )?;
    ctx.artifact_store.put(SCOPE, thread_id.as_str(), &artifact)
}

/// One row of a thread's pinned list.
///
/// `title` and `pinned_at` are `Option` because they come from a *second*
/// read of the snapshot body, which the enumeration in [`list_pinned`] does
/// not need and cannot vouch for: a truncated or hand-edited file still
/// occupies the scope directory and still has a `path` the viewer can open,
/// so it degrades to a path-only row instead of removing itself from the
/// list. `path` is the only field the viewer requires.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PinnedCanvasEntry {
    /// The `ArtifactStore` reference `artifact_body` reopens.
    pub path: String,
    /// The snapshot's `canvas.title` — the only thing that tells two pins
    /// apart on screen, since the artifact's name is a bare `message_id`.
    pub title: Option<String>,
    pub pinned_at: Option<i64>,
}

/// Every canvas pinned for this thread. Order is whatever `list_for_step`
/// returns — not guaranteed chronological.
///
/// Enumeration stays `list_for_step`; the per-entry `get` only *describes*
/// what it found. A read or parse failure on one entry is therefore not the
/// list's failure: one corrupt file must not make every other pin
/// unreachable, so it yields a [`PinnedCanvasEntry`] with `title` and
/// `pinned_at` unset rather than an `Err`.
pub fn list_pinned(
    ctx: &AppContext,
    thread_id: &AskThreadId,
) -> Result<Vec<PinnedCanvasEntry>, String> {
    let references = ctx
        .artifact_store
        .list_for_step(SCOPE, thread_id.as_str())?;
    Ok(references
        .into_iter()
        .map(|path| describe(ctx, path))
        .collect())
}

fn describe(ctx: &AppContext, path: String) -> PinnedCanvasEntry {
    let snapshot = ctx
        .artifact_store
        .get(&path)
        .ok()
        .and_then(|body| serde_json::from_str::<PinnedCanvasSnapshot>(&body).ok());
    match snapshot {
        Some(snapshot) => PinnedCanvasEntry {
            path,
            title: Some(snapshot.canvas.title),
            pinned_at: Some(snapshot.pinned_at),
        },
        None => PinnedCanvasEntry {
            path,
            title: None,
            pinned_at: None,
        },
    }
}

/// Drop every canvas this thread pinned, so a deleted thread leaves no
/// artifacts behind. This scope has no other deleter and the pinned list is
/// read-only — there is no unpin control — so the two paths that end a
/// thread's life are the only ones that can ever remove an entry:
/// [`super::delete`], and
/// [`projects::delete_workspace`](crate::application::projects::delete_workspace),
/// which reaches the same rows through SQLite's cascade.
pub fn clear_pins(ctx: &AppContext, thread_id: &AskThreadId) -> Result<(), String> {
    ctx.artifact_store.clear_step(SCOPE, thread_id.as_str())
}

/// The same snapshot [`pin_canvas`] would freeze, pretty-printed — with no
/// `ArtifactStore::put` call, so exporting never touches the pinned list.
pub fn export_canvas(
    ctx: &AppContext,
    thread_id: &AskThreadId,
    message_id: &str,
) -> Result<String, String> {
    export_canvas_at(ctx, thread_id, message_id, crate::paths::now_ms())
}

/// [`export_canvas`]'s seam, paired with [`pin_canvas_at`].
fn export_canvas_at(
    ctx: &AppContext,
    thread_id: &AskThreadId,
    message_id: &str,
    pinned_at: i64,
) -> Result<String, String> {
    let snapshot = snapshot_for(ctx, thread_id, message_id, pinned_at)?;
    serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())
}

fn snapshot_for(
    ctx: &AppContext,
    thread_id: &AskThreadId,
    message_id: &str,
    pinned_at: i64,
) -> Result<crate::domain::ask_canvas::PinnedCanvasSnapshot, String> {
    let detail = super::load(ctx, thread_id)?;
    let view = detail
        .messages
        .iter()
        .find(|v| v.message.id == message_id)
        .ok_or_else(|| format!("no such message: {message_id}"))?;
    build_pinned_canvas_snapshot(thread_id.as_str(), &view.message, &view.turn, pinned_at)
}

#[cfg(test)]
#[path = "../../../tests/application/ask/pin.rs"]
mod tests;

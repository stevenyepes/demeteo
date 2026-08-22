//! Tauri surface for the interview (`docs/PRD_DISCOVERY.md` §4).
//!
//! Thin by construction: every decision below is made in
//! [`demeteo_core::application::discovery`]. A turn returns as soon as the
//! user's message is stored and reaches the surface after that through the
//! three events [`turn`] declares, which is what §4.3 asks for — leaving
//! mid-interview is the case the feature is built for, so nothing here waits
//! for a turn to finish.
//!
//! There is no `discovery_delete` here: `commands::tickets` owns that name,
//! because the refusal it can return is a Ticket rule (§8.4).

use crate::domain::attachment::AttachedFile;
use crate::domain::ids::{DiscoveryId, ProjectId};
use crate::domain::models::{Discovery, DiscoveryMessage};
use crate::state::AppContext;
use demeteo_core::application::attachments::StagedAttachmentInput;
use demeteo_core::application::discovery::decompose::{
    self, proposal::DecomposeProposal, DecomposeApply,
};
use demeteo_core::application::discovery::{
    self, turn, DiscoveryDetail, DiscoverySummary, NewDiscovery,
};
use demeteo_core::application::tickets::DiscoveryBoard;
use tauri::{Emitter, State};

/// Every Discovery in a project, with the two numbers its card renders that
/// the row does not carry.
#[tauri::command]
pub fn discovery_list(
    ctx: State<'_, AppContext>,
    project_id: String,
) -> Result<Vec<DiscoverySummary>, String> {
    discovery::list_for_project(&ctx, &project_id)
}

#[tauri::command]
pub fn discovery_get(
    ctx: State<'_, AppContext>,
    discovery_id: String,
) -> Result<DiscoveryDetail, String> {
    discovery::get(&ctx, &DiscoveryId::from(discovery_id))
}

#[tauri::command]
pub fn discovery_create(
    ctx: State<'_, AppContext>,
    input: NewDiscovery,
) -> Result<Discovery, String> {
    discovery::create(&ctx, input)
}

/// Send the user's turn and start the interviewer's.
///
/// The returned message is the user's own, already persisted, so the
/// transcript can render it without waiting for the answer. `Err` therefore
/// means the turn was never accepted — a closed Discovery, empty text, or one
/// already taking a turn. A failure to *set the turn up* arrives as an error
/// status event instead; [`turn::send`] says why.
#[tauri::command]
pub async fn discovery_send_turn(
    ctx: State<'_, AppContext>,
    app: tauri::AppHandle,
    discovery_id: String,
    text: String,
) -> Result<DiscoveryMessage, String> {
    turn::send(
        &ctx,
        &DiscoveryId::from(discovery_id),
        text,
        move |event, payload| {
            let _ = app.emit(event, payload);
        },
    )
    .await
}

/// Stage one file on a Discovery (§4.6). `bytes` carries the content when the
/// webview has a `File` handle but no path on disk, exactly as
/// `feature_add_attachment` does.
#[tauri::command]
pub fn discovery_add_attachment(
    ctx: State<'_, AppContext>,
    discovery_id: String,
    source_path: String,
    mime: Option<String>,
    source_filename: Option<String>,
    bytes: Option<Vec<u8>>,
) -> Result<AttachedFile, String> {
    discovery::attachments::stage(
        &ctx,
        &DiscoveryId::from(discovery_id),
        StagedAttachmentInput {
            source_path,
            mime,
            source_filename,
            bytes,
        },
    )
}

#[tauri::command]
pub fn discovery_remove_attachment(
    ctx: State<'_, AppContext>,
    discovery_id: String,
    attachment_id: String,
) -> Result<(), String> {
    discovery::attachments::unstage(&ctx, &DiscoveryId::from(discovery_id), &attachment_id)
}

#[tauri::command]
pub async fn discovery_cancel_turn(
    ctx: State<'_, AppContext>,
    discovery_id: String,
) -> Result<(), String> {
    discovery::cancel_turn(&ctx, &DiscoveryId::from(discovery_id)).await
}

#[tauri::command]
pub async fn discovery_close(
    ctx: State<'_, AppContext>,
    discovery_id: String,
) -> Result<(), String> {
    discovery::close(&ctx, &DiscoveryId::from(discovery_id)).await
}

#[tauri::command]
pub fn discovery_reopen(ctx: State<'_, AppContext>, discovery_id: String) -> Result<(), String> {
    discovery::reopen(&ctx, &DiscoveryId::from(discovery_id))
}

/// Hand back the checkouts of Discoveries nobody has taken a turn in lately
/// (§4.6). Returns the ids it reclaimed.
#[tauri::command]
pub async fn discovery_reclaim_idle_worktrees(
    ctx: State<'_, AppContext>,
    project_id: String,
    idle_after_ms: i64,
) -> Result<Vec<String>, String> {
    discovery::worktree::reclaim_idle(&ctx, &ProjectId::from(project_id), idle_after_ms).await
}

/// Ask the interviewer for a plan and hand back what applying it would change
/// (§5.2). Nothing is written; §5.3's review comes first.
///
/// The pass streams through the same events a turn does, so the surface can
/// show the agent working, but the proposal comes back on the call: there is
/// nothing to render until it is whole.
#[tauri::command]
pub async fn discovery_decompose(
    ctx: State<'_, AppContext>,
    app: tauri::AppHandle,
    discovery_id: String,
) -> Result<DecomposeProposal, String> {
    decompose::run(
        &ctx,
        &DiscoveryId::from(discovery_id),
        move |event, payload| {
            let _ = app.emit(event, payload);
        },
    )
    .await
}

/// Forget the pass waiting for review without applying any of it.
///
/// Explicit, and not what closing the review does: a proposal is billed work,
/// so leaving the modal keeps it and this is the press that says otherwise.
#[tauri::command]
pub fn discovery_discard_proposal(
    ctx: State<'_, AppContext>,
    discovery_id: String,
) -> Result<(), String> {
    decompose::discard(&ctx, &DiscoveryId::from(discovery_id))
}

/// Land the changes the user checked, and return the board they leave behind.
#[tauri::command]
pub fn discovery_apply_decomposition(
    ctx: State<'_, AppContext>,
    input: DecomposeApply,
) -> Result<DiscoveryBoard, String> {
    decompose::apply(&ctx, input)
}

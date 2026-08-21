//! Tauri surface for a Discovery's Tickets (`docs/PRD_DISCOVERY.md` §6, §7).
//!
//! Thin by construction: every decision below is made in
//! [`demeteo_core::application::tickets`], and the read side returns the
//! tickets and their derived board from one call so the graph and the board
//! cannot disagree (§9.2).

use crate::domain::attachment::AttachedFile;
use crate::domain::ids::{DiscoveryId, TicketId};
use crate::domain::models::{Feature, Ticket};
use crate::state::AppContext;
use demeteo_core::application::tickets::{self, DiscoveryBoard};
use tauri::State;

#[tauri::command]
pub fn discovery_board(
    ctx: State<'_, AppContext>,
    discovery_id: String,
) -> Result<DiscoveryBoard, String> {
    tickets::board(&ctx, &DiscoveryId::from(discovery_id))
}

/// What the ticket's agent will be told (§7.2), rendered before anything is
/// started so the user can read it in the editor (`DISCOVERY_UI_SPEC.md` §5.8).
#[tauri::command]
pub fn ticket_briefing(ctx: State<'_, AppContext>, ticket_id: String) -> Result<String, String> {
    tickets::briefing_for(&ctx, &TicketId::from(ticket_id))
}

#[tauri::command]
pub async fn ticket_start(
    ctx: State<'_, AppContext>,
    ticket_id: String,
) -> Result<Feature, String> {
    tickets::launch::start(&ctx, &TicketId::from(ticket_id)).await
}

#[tauri::command]
pub async fn ticket_force_start(
    ctx: State<'_, AppContext>,
    ticket_id: String,
    reason: String,
) -> Result<Feature, String> {
    tickets::launch::force_start(&ctx, &TicketId::from(ticket_id), &reason).await
}

#[tauri::command]
pub fn ticket_drop(
    ctx: State<'_, AppContext>,
    ticket_id: String,
    reason: String,
) -> Result<Ticket, String> {
    tickets::launch::drop_ticket(&ctx, &TicketId::from(ticket_id), &reason)
}

/// Stage one file on a Ticket (§9.3). `bytes` carries the content when the
/// webview has a `File` handle but no path on disk, exactly as
/// `feature_add_attachment` does.
#[tauri::command]
pub fn ticket_add_attachment(
    ctx: State<'_, AppContext>,
    ticket_id: String,
    source_path: String,
    mime: Option<String>,
    source_filename: Option<String>,
    bytes: Option<Vec<u8>>,
) -> Result<AttachedFile, String> {
    tickets::attachments::stage(
        &ctx,
        &TicketId::from(ticket_id),
        &source_path,
        mime.as_deref(),
        source_filename.as_deref(),
        bytes,
    )
}

#[tauri::command]
pub fn ticket_remove_attachment(
    ctx: State<'_, AppContext>,
    ticket_id: String,
    attachment_id: String,
) -> Result<(), String> {
    tickets::attachments::unstage(&ctx, &TicketId::from(ticket_id), &attachment_id)
}

#[tauri::command]
pub fn discovery_delete(ctx: State<'_, AppContext>, discovery_id: String) -> Result<(), String> {
    tickets::delete_discovery(&ctx, &DiscoveryId::from(discovery_id))
}

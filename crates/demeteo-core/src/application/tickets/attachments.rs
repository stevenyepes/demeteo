//! A Ticket's own staged attachments (§9.3) — the screenshot the ticket's
//! implementer needs, not the one the interviewer was shown.
//!
//! The bytes are stored the moment they are staged, keyed by the **Ticket**
//! id, because a Ticket outlives the modal that dropped the file: a plan left
//! for a week and resumed on another machine has nowhere else to have kept
//! them. Why that needs no second store and no migration is on
//! [`crate::application::attachments::stage_on_owner`], which this and the
//! Discovery's equivalent both go through.

use crate::application::attachments::{
    stage_on_owner, staged_batch_for, unstage_from_owner, Staged, StagedAttachmentInput,
};
use crate::domain::attachment::AttachedFile;
use crate::domain::ids::TicketId;
use crate::domain::models::Ticket;
use crate::ports::discovery::TicketPatch;
use crate::state::AppContext;

/// Stage one file on a Ticket. Idempotent on content: re-staging the same
/// bytes returns the existing entry rather than a second chip.
pub fn stage(
    ctx: &AppContext,
    ticket_id: &TicketId,
    source_path: &str,
    mime: Option<&str>,
    source_filename: Option<&str>,
    bytes: Option<Vec<u8>>,
) -> Result<AttachedFile, String> {
    let ticket = super::load(ctx, ticket_id)?;
    match stage_on_owner(
        ctx.attachments.as_ref(),
        ticket_id.as_str(),
        "ticket",
        ticket.attachments,
        StagedAttachmentInput {
            source_path: source_path.to_string(),
            mime: mime.map(str::to_string),
            source_filename: source_filename.map(str::to_string),
            bytes,
        },
    )? {
        Staged::Unchanged(file) => Ok(file),
        Staged::Added { file, manifest } => {
            write_manifest(ctx, ticket_id, manifest)?;
            Ok(file)
        }
    }
}

/// Drop one staged entry and its bytes. Idempotent.
pub fn unstage(ctx: &AppContext, ticket_id: &TicketId, attachment_id: &str) -> Result<(), String> {
    let ticket = super::load(ctx, ticket_id)?;
    match unstage_from_owner(
        ctx.attachments.as_ref(),
        ticket_id.as_str(),
        &ticket.attachments,
        attachment_id,
    ) {
        Some(manifest) => write_manifest(ctx, ticket_id, manifest),
        None => Ok(()),
    }
}

/// Hand the whole staged batch to the launch.
///
/// **This corrects `docs/PRD_DISCOVERY.md` §9.3**, which says the entries are
/// committed "through `feature_add_attachment` when it starts". They are not,
/// and must not be: `FeatureLaunch::staged_attachments` persists the batch
/// before the driver is spawned, whereas post-launch attach calls race the
/// agent's first turn — the user attaches a screenshot and the agent answers
/// that nothing was attached. Restoring the §9.3 wording would restore that
/// race.
pub fn staged_for_launch(
    ctx: &AppContext,
    ticket: &Ticket,
) -> Result<Vec<StagedAttachmentInput>, String> {
    staged_batch_for(
        ctx.attachments.as_ref(),
        ticket.id.as_str(),
        &ticket.attachments,
    )
}

fn write_manifest(
    ctx: &AppContext,
    ticket_id: &TicketId,
    attachments: Vec<AttachedFile>,
) -> Result<(), String> {
    ctx.tickets.update(
        ticket_id,
        &TicketPatch {
            attachments: Some(attachments),
            ..Default::default()
        },
        crate::paths::now_ms(),
    )
}

//! A Ticket's own staged attachments (§9.3) — the screenshot the ticket's
//! implementer needs, not the one the interviewer was shown.
//!
//! The bytes are stored the moment they are staged, keyed by the **Ticket**
//! id, because a Ticket outlives the modal that dropped the file: a plan left
//! for a week and resumed on another machine has nowhere else to have kept
//! them. The store's key is a plain string with no feature dimension in it
//! (see [`crate::ports::attachment_store::AttachmentStore`]), so this needs no
//! second store and no migration.
//!
//! Validation mirrors the per-feature path in
//! [`crate::application::attachments`] by calling the same predicates; it does
//! not call [`commit_attachment_inner`] itself, which asserts a `features` row
//! exists — the state §9.3 says a Ticket is precisely not in until it starts.
//!
//! [`commit_attachment_inner`]: crate::application::attachments::commit_attachment_inner

use crate::application::attachments::{
    is_supported_attachment, resolve_mime, StagedAttachmentInput, MAX_ATTACHMENTS_PER_FEATURE,
    MAX_ATTACHMENT_BYTES,
};
use crate::domain::attachment::{
    compute_sha256_hex, ext_for_mime, resolved_ext, sanitize_attachment_filename, AttachedFile,
};
use crate::domain::ids::TicketId;
use crate::domain::models::Ticket;
use crate::ports::discovery::TicketPatch;
use crate::state::AppContext;
use std::path::Path;

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
    let bytes = read_bytes(source_path, bytes)?;

    let sha256 = compute_sha256_hex(&bytes);
    if let Some(existing) = ticket.attachments.iter().find(|a| a.sha256 == sha256) {
        return Ok(existing.clone());
    }
    if ticket.attachments.len() >= MAX_ATTACHMENTS_PER_FEATURE {
        return Err(format!(
            "ticket already has {} attachments (max {})",
            ticket.attachments.len(),
            MAX_ATTACHMENTS_PER_FEATURE
        ));
    }

    let src = std::path::PathBuf::from(source_path);
    let resolved_mime = resolve_mime(mime, source_filename, &src);
    let ext = match ext_for_mime(&resolved_mime) {
        Some(e) => e.to_string(),
        None => Path::new(source_filename.unwrap_or(source_path))
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_else(|| "bin".to_string()),
    };
    if !is_supported_attachment(&resolved_mime, &ext) {
        return Err(format!(
            "unsupported attachment type: mime={resolved_mime} ext={ext}"
        ));
    }

    ctx.attachments
        .write(ticket_id.as_str(), &sha256, &ext, &bytes)?;

    let id = format!("at-{}", crate::paths::new_id());
    let file = AttachedFile {
        id: id.clone(),
        name: sanitize_attachment_filename(source_filename.unwrap_or(&sha256)),
        mime: resolved_mime,
        sha256,
        size: bytes.len() as u64,
        source_filename: source_filename.unwrap_or(&id).to_string(),
    };

    let mut next = ticket.attachments;
    next.push(file.clone());
    write_manifest(ctx, ticket_id, next)?;
    Ok(file)
}

/// Drop one staged entry and its bytes. Idempotent.
pub fn unstage(ctx: &AppContext, ticket_id: &TicketId, attachment_id: &str) -> Result<(), String> {
    let ticket = super::load(ctx, ticket_id)?;
    let Some(target) = ticket.attachments.iter().find(|a| a.id == attachment_id) else {
        return Ok(());
    };
    let stored =
        ctx.attachments
            .lookup_path(ticket_id.as_str(), &target.sha256, &resolved_ext(target));
    let _ = ctx.attachments.delete(&stored.to_string_lossy());

    let next = ticket
        .attachments
        .iter()
        .filter(|a| a.id != attachment_id)
        .cloned()
        .collect();
    write_manifest(ctx, ticket_id, next)
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
    ticket
        .attachments
        .iter()
        .map(|a| {
            let stored =
                ctx.attachments
                    .lookup_path(ticket.id.as_str(), &a.sha256, &resolved_ext(a));
            let bytes = ctx.attachments.read(&stored.to_string_lossy())?;
            Ok(StagedAttachmentInput {
                source_path: String::new(),
                mime: Some(a.mime.clone()),
                source_filename: Some(a.source_filename.clone()),
                bytes: Some(bytes),
            })
        })
        .collect()
}

fn read_bytes(source_path: &str, bytes: Option<Vec<u8>>) -> Result<Vec<u8>, String> {
    let bytes = match bytes {
        Some(b) => b,
        None => {
            let src = std::path::PathBuf::from(source_path);
            let meta = std::fs::metadata(&src)
                .map_err(|e| format!("could not stat source file {source_path}: {e}"))?;
            if !meta.is_file() {
                return Err(format!("source path is not a regular file: {source_path}"));
            }
            std::fs::read(&src)
                .map_err(|e| format!("could not read source file {source_path}: {e}"))?
        }
    };
    if bytes.is_empty() {
        return Err("attachment bytes are empty".to_string());
    }
    if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "attachment too large: {} bytes (max {})",
            bytes.len(),
            MAX_ATTACHMENT_BYTES
        ));
    }
    Ok(bytes)
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

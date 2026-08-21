//! What the user handed the interviewer (§4.6).
//!
//! Owned by the Discovery, not by the turn that added it: §8.3 keeps an
//! interview open indefinitely, and a file that reached only one turn would
//! stop existing for the conversation the moment the next one was taken. The
//! store keys on the Discovery id, on the terms
//! [`crate::application::attachments::stage_on_owner`] states.
//!
//! What a Ticket's attachments are for is a different question — §9.3 gives
//! those to the agent that *implements* the ticket, and they are staged
//! separately in [`crate::application::tickets::attachments`].

use crate::application::attachments::{
    stage_on_owner, unstage_from_owner, Staged, StagedAttachmentInput,
};
use crate::domain::attachment::AttachedFile;
use crate::domain::ids::DiscoveryId;
use crate::domain::models::Discovery;
use crate::ports::discovery::DiscoveryPatch;
use crate::state::AppContext;

/// Stage one file on a Discovery. Idempotent on content.
pub fn stage(
    ctx: &AppContext,
    discovery_id: &DiscoveryId,
    file: StagedAttachmentInput,
) -> Result<AttachedFile, String> {
    let discovery = super::load(ctx, discovery_id)?;
    match stage_on_owner(
        ctx.attachments.as_ref(),
        discovery_id.as_str(),
        "discovery",
        discovery.attachments,
        file,
    )? {
        Staged::Unchanged(file) => Ok(file),
        Staged::Added { file, manifest } => {
            write_manifest(ctx, discovery_id, manifest)?;
            Ok(file)
        }
    }
}

/// Drop one staged entry and its bytes. Idempotent.
pub fn unstage(
    ctx: &AppContext,
    discovery_id: &DiscoveryId,
    attachment_id: &str,
) -> Result<(), String> {
    let discovery = super::load(ctx, discovery_id)?;
    match unstage_from_owner(
        ctx.attachments.as_ref(),
        discovery_id.as_str(),
        &discovery.attachments,
        attachment_id,
    ) {
        Some(manifest) => write_manifest(ctx, discovery_id, manifest),
        None => Ok(()),
    }
}

/// Stage the batch the New-discovery modal collected before there was a
/// Discovery to hang it on.
///
/// The whole batch lands before the row is handed back, so the first turn a
/// user can take already sees every file — the ordering
/// `FeatureLaunch::staged_attachments` exists to guarantee, one aggregate over.
pub fn stage_batch(
    ctx: &AppContext,
    discovery_id: &DiscoveryId,
    staged: Vec<StagedAttachmentInput>,
) -> Result<Vec<AttachedFile>, String> {
    staged
        .into_iter()
        .map(|file| stage(ctx, discovery_id, file))
        .collect()
}

/// Put the bytes inside the interview's checkout and say where they landed.
///
/// The same copy `spawn.rs` makes before a step's agent turn, for the same
/// reason: a harness fenced to its working directory refuses to `Read` the
/// host-local store, so a prompt naming that store names a file the agent
/// cannot open. The destination is under `artifacts/`, which no fence touches
/// here — the artifact scope this worktree carries is applied over the entries
/// the repository already had, and this directory is not one of them.
///
/// `None` when nothing landed, which is the honest answer for a prompt to
/// carry: the store path at least exists, where a worktree path that was never
/// written does not.
pub(super) async fn materialize(
    ctx: &AppContext,
    discovery: &Discovery,
    worktree_path: &str,
    machine_str: &str,
) -> Option<String> {
    if discovery.attachments.is_empty() {
        return None;
    }
    let copied =
        crate::adapters::step_executor::artifacts::materialize_user_attachments_to_worktree(
            discovery.id.as_str(),
            &discovery.attachments,
            ctx.attachments.as_ref(),
            worktree_path,
            ctx.exec.as_ref(),
            machine_str,
        )
        .await;
    if copied.len() < discovery.attachments.len() {
        return None;
    }
    Some(crate::paths::join_on(
        worktree_path,
        ["artifacts", "_context"],
        crate::paths::targets_windows_host(machine_str),
    ))
}

fn write_manifest(
    ctx: &AppContext,
    discovery_id: &DiscoveryId,
    attachments: Vec<AttachedFile>,
) -> Result<(), String> {
    ctx.discoveries.update(
        discovery_id,
        &DiscoveryPatch {
            attachments: Some(attachments),
            ..Default::default()
        },
        crate::paths::now_ms(),
    )
}

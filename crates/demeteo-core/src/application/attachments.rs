//! Attachment-commit logic shared by the Tauri `commands::attachments`
//! wrappers (`src-tauri/src/commands/attachments.rs`) and the step executor's
//! pre-execution staged-attachment path
//! (`adapters/step_executor/impl_traits/bootstrap.rs`). Split out so both call
//! sites — one behind a Tauri command, one inside the engine — share
//! identical validation, dedup, and storage rules.

use crate::domain::attachment::{
    compute_sha256_hex, ext_for_mime, mime_for_ext, resolved_ext, sanitize_attachment_filename,
    AttachedFile,
};
use crate::domain::ids::FeatureId;
use crate::error::AppError;
use crate::ports::attachment_store::{AttachmentJsonPort, AttachmentStore};
use crate::ports::db::FeatureRepository;
use std::path::Path;
use std::sync::Arc;
use tracing::info;

pub const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_ATTACHMENTS_PER_FEATURE: usize = 10;

/// Staged attachment supplied at feature-start time.
///
/// Mirrors the wire shape of `feature_add_attachment` but bundled into one
/// batch so the IPC `start_feature` command can persist all of them BEFORE
/// the executor spawns the agent driver. Without this batching the agent's
/// first turn races the post-launch `feature_add_attachment` calls and the
/// user sees "no image attached" responses from a freshly-attached
/// screenshot.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct StagedAttachmentInput {
    /// Absolute path on disk (drag-and-drop). Empty when bytes were
    /// ferried through IPC instead.
    pub source_path: String,
    pub mime: Option<String>,
    pub source_filename: Option<String>,
    /// In-memory bytes for a browser `File` selection that did not
    /// yield an absolute path on disk. Mutually exclusive with the
    /// path branch — when `Some`, `source_path` is ignored.
    pub bytes: Option<Vec<u8>>,
}

/// Commit a single attachment to the manifest. Shared by
/// `feature_add_attachment` (post-launch path) and
/// [`commit_staged_attachments`] (pre-execution path) so both flows
/// apply identical validation, dedup, and storage rules.
///
/// `feature_id` is assumed to exist (caller verifies — the post-launch
/// IPC reads via `ctx.features.get`, the pre-execution path inserts
/// the row before calling).
#[allow(clippy::too_many_arguments)]
pub fn commit_attachment_inner(
    features: &Arc<dyn FeatureRepository>,
    attachment_json: &Arc<dyn AttachmentJsonPort>,
    attachments: &Arc<dyn AttachmentStore>,
    feature_id: &str,
    source_path: &str,
    mime: Option<&str>,
    source_filename: Option<&str>,
    bytes: Option<Vec<u8>>,
) -> Result<AttachedFile, AppError> {
    let fid = FeatureId::from(feature_id.to_string());
    let _ = features
        .get(&fid)?
        .ok_or_else(|| AppError::not_found(format!("feature not found: {}", feature_id)))?;

    let bytes = if let Some(b) = bytes {
        if b.is_empty() {
            return Err(AppError::validation("attachment bytes are empty"));
        }
        if b.len() as u64 > MAX_ATTACHMENT_BYTES {
            return Err(AppError::validation(format!(
                "attachment too large: {} bytes (max {})",
                b.len(),
                MAX_ATTACHMENT_BYTES
            )));
        }
        b
    } else {
        let src = std::path::PathBuf::from(source_path);
        let meta = std::fs::metadata(&src).map_err(|e| {
            AppError::validation(format!("could not stat source file {}: {}", source_path, e))
        })?;
        if !meta.is_file() {
            return Err(AppError::validation(format!(
                "source path is not a regular file: {}",
                source_path
            )));
        }
        if meta.len() > MAX_ATTACHMENT_BYTES {
            return Err(AppError::validation(format!(
                "attachment too large: {} bytes (max {})",
                meta.len(),
                MAX_ATTACHMENT_BYTES
            )));
        }
        std::fs::read(&src).map_err(|e| {
            AppError::validation(format!("could not read source file {}: {}", source_path, e))
        })?
    };

    let src_path = std::path::PathBuf::from(source_path);
    let sha256 = compute_sha256_hex(&bytes);
    let resolved_mime = resolve_mime(mime, source_filename, &src_path);
    let ext = match ext_for_mime(&resolved_mime) {
        Some(e) => e.to_string(),
        None => Path::new(source_filename.unwrap_or(source_path))
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_else(|| "bin".to_string()),
    };

    if !is_supported_attachment(&resolved_mime, &ext) {
        return Err(AppError::validation(format!(
            "unsupported attachment type: mime={} ext={} (allowed: png, jpg, gif, webp, tiff, pdf, txt, md, json)",
            resolved_mime, ext
        )));
    }

    let current = attachment_json.get_attachments(&fid)?;
    if let Some(existing) = current.iter().find(|a| a.sha256 == sha256).cloned() {
        return Ok(existing);
    }
    if current.len() >= MAX_ATTACHMENTS_PER_FEATURE {
        return Err(AppError::validation(format!(
            "feature already has {} attachments (max {})",
            current.len(),
            MAX_ATTACHMENTS_PER_FEATURE
        )));
    }

    attachments.write(feature_id, &sha256, &ext, &bytes)?;

    let display_name = sanitize_attachment_filename(source_filename.unwrap_or(&sha256));
    let id = format!("at-{}", crate::paths::new_id());
    let file = AttachedFile {
        id: id.clone(),
        name: display_name,
        mime: resolved_mime,
        sha256: sha256.clone(),
        size: bytes.len() as u64,
        source_filename: source_filename.unwrap_or(&id).to_string(),
    };

    let mut next = current;
    next.push(file.clone());
    attachment_json.set_attachments(&fid, &next)?;

    info!(
        feature_id = %feature_id,
        attachment_id = %file.id,
        sha256 = %sha256,
        bytes = file.size,
        mime = %file.mime,
        "feature attachment committed"
    );

    Ok(file)
}

/// Persist every staged attachment to `feature_id` before the agent
/// driver is spawned. Returns the full list of `AttachedFile`s in
/// insertion order on success; on the first validation failure the
/// call short-circuits and surfaces the error to the caller — the
/// feature row still exists but no agent has been started yet, so the
/// frontend can prompt the user to retry.
pub fn commit_staged_attachments(
    features: &Arc<dyn FeatureRepository>,
    attachment_json: &Arc<dyn AttachmentJsonPort>,
    attachments: &Arc<dyn AttachmentStore>,
    feature_id: &str,
    staged: Vec<StagedAttachmentInput>,
) -> Result<Vec<AttachedFile>, AppError> {
    let mut out = Vec::with_capacity(staged.len());
    for s in staged {
        let attached = commit_attachment_inner(
            features,
            attachment_json,
            attachments,
            feature_id,
            &s.source_path,
            s.mime.as_deref(),
            s.source_filename.as_deref(),
            s.bytes,
        )?;
        out.push(attached);
    }
    Ok(out)
}

/// What staging one file on an owner did.
///
/// The two arms exist because idempotence-on-content has to be visible to the
/// caller: an owner whose manifest did not change must not be written back,
/// or every re-drop of the same screenshot bumps a row's `updated_at` and
/// reorders the list the user is looking at.
pub enum Staged {
    /// These bytes were already staged. Nothing was written.
    Unchanged(AttachedFile),
    /// The bytes are on disk. The manifest is the caller's to persist on
    /// whichever row owns it.
    Added {
        file: AttachedFile,
        manifest: Vec<AttachedFile>,
    },
}

/// Stage one file against something that is **not** a Feature.
///
/// [`AttachmentStore`] keys on a plain string with no feature dimension in it,
/// so a Ticket (§9.3) and a Discovery (§4.6) each own their bytes under their
/// own id. What this cannot call is [`commit_attachment_inner`], which asserts
/// a `features` row exists — the state both of them are precisely not in.
///
/// `owner_label` names the owner in the one error a user can act on; nothing
/// else branches on it.
pub fn stage_on_owner(
    store: &dyn AttachmentStore,
    owner_id: &str,
    owner_label: &str,
    current: Vec<AttachedFile>,
    file: StagedAttachmentInput,
) -> Result<Staged, String> {
    let source_path = file.source_path.as_str();
    let source_filename = file.source_filename.as_deref();
    let bytes = read_staged_bytes(source_path, file.bytes)?;

    let sha256 = compute_sha256_hex(&bytes);
    if let Some(existing) = current.iter().find(|a| a.sha256 == sha256) {
        return Ok(Staged::Unchanged(existing.clone()));
    }
    if current.len() >= MAX_ATTACHMENTS_PER_FEATURE {
        return Err(format!(
            "{owner_label} already has {} attachments (max {})",
            current.len(),
            MAX_ATTACHMENTS_PER_FEATURE
        ));
    }

    let src = std::path::PathBuf::from(source_path);
    let resolved_mime = resolve_mime(file.mime.as_deref(), source_filename, &src);
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

    store.write(owner_id, &sha256, &ext, &bytes)?;

    let id = format!("at-{}", crate::paths::new_id());
    let file = AttachedFile {
        id: id.clone(),
        name: sanitize_attachment_filename(source_filename.unwrap_or(&sha256)),
        mime: resolved_mime,
        sha256,
        size: bytes.len() as u64,
        source_filename: source_filename.unwrap_or(&id).to_string(),
    };

    let mut manifest = current;
    manifest.push(file.clone());
    Ok(Staged::Added { file, manifest })
}

/// Drop one staged entry and its bytes. `None` when the owner never held it,
/// so the caller writes nothing — which is what makes a double remove a no-op
/// rather than a second row write.
pub fn unstage_from_owner(
    store: &dyn AttachmentStore,
    owner_id: &str,
    current: &[AttachedFile],
    attachment_id: &str,
) -> Option<Vec<AttachedFile>> {
    let target = current.iter().find(|a| a.id == attachment_id)?;
    let stored = store.lookup_path(owner_id, &target.sha256, &resolved_ext(target));
    let _ = store.delete(&stored.to_string_lossy());
    Some(
        current
            .iter()
            .filter(|a| a.id != attachment_id)
            .cloned()
            .collect(),
    )
}

/// Read an owner's staged bytes back out as a launch batch.
///
/// The bytes travel rather than the path because the store is keyed by owner:
/// the Feature the launch creates has its own key, and a
/// [`StagedAttachmentInput::source_path`] pointing into another owner's
/// directory would make the copy depend on a layout only the store may know.
pub fn staged_batch_for(
    store: &dyn AttachmentStore,
    owner_id: &str,
    attachments: &[AttachedFile],
) -> Result<Vec<StagedAttachmentInput>, String> {
    attachments
        .iter()
        .map(|a| {
            let stored = store.lookup_path(owner_id, &a.sha256, &resolved_ext(a));
            let bytes = store.read(&stored.to_string_lossy())?;
            Ok(StagedAttachmentInput {
                source_path: String::new(),
                mime: Some(a.mime.clone()),
                source_filename: Some(a.source_filename.clone()),
                bytes: Some(bytes),
            })
        })
        .collect()
}

fn read_staged_bytes(source_path: &str, bytes: Option<Vec<u8>>) -> Result<Vec<u8>, String> {
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

pub fn resolve_mime(
    supplied: Option<&str>,
    source_filename: Option<&str>,
    source_path: &Path,
) -> String {
    if let Some(m) = supplied {
        if !m.trim().is_empty() {
            return m.to_string();
        }
    }
    if let Some(name) = source_filename {
        if let Some(ext) = Path::new(name).extension().and_then(|s| s.to_str()) {
            if let Some(m) = mime_for_ext(ext) {
                return m.to_string();
            }
        }
    }
    if let Some(ext) = source_path.extension().and_then(|s| s.to_str()) {
        if let Some(m) = mime_for_ext(ext) {
            return m.to_string();
        }
    }
    "application/octet-stream".to_string()
}

/// Returns true when the resolved mime + extension pair corresponds to
/// a supported attachment type. The mime is the authoritative signal;
/// the extension is a fallback for callers that supply a non-IANA
/// mime (e.g. `text/x-patch`) but a clean extension.
pub fn is_supported_attachment(mime: &str, ext: &str) -> bool {
    let lower_mime = mime.to_ascii_lowercase();
    if lower_mime.starts_with("image/") {
        return matches!(
            lower_mime.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/tiff"
        );
    }
    matches!(
        lower_mime.as_str(),
        "text/plain" | "text/markdown" | "application/json" | "application/pdf"
    ) || matches!(
        ext.to_ascii_lowercase().as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "tiff"
            | "tif"
            | "pdf"
            | "txt"
            | "md"
            | "markdown"
            | "json"
    )
}

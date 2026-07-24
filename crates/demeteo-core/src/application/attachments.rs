//! Attachment-commit logic shared by the Tauri `commands::attachments`
//! wrappers (`src-tauri/src/commands/attachments.rs`) and the step executor's
//! pre-execution staged-attachment path
//! (`adapters/step_executor/impl_traits/mod.rs`). Split out so both call
//! sites — one behind a Tauri command, one inside the engine — share
//! identical validation, dedup, and storage rules.

use crate::domain::attachment::{
    compute_sha256_hex, ext_for_mime, mime_for_ext, sanitize_attachment_filename, AttachedFile,
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

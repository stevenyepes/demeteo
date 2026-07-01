//! Tauri commands for the per-feature attachment subsystem.
//!
//! Three additive commands wired through `commands::mod`:
//!
//! * `feature_add_attachment` — validate (size, ext, sha256), copy
//!   bytes from a user-supplied absolute path into the
//!   `AttachmentStore`, dedup by content hash, append an
//!   [`AttachedFile`] row to the feature's manifest via
//!   [`AttachmentJsonPort`], and return the new manifest entry. A
//!   re-upload of the same content under a different filename is an
//!   idempotent no-op beyond updating the row metadata to reflect
//!   the latest name — the on-disk file is shared.
//! * `feature_list_attachments` — return the feature's manifest, or
//!   `[]` if the feature has no attachments column populated.
//! * `feature_remove_attachment` — drop the manifest entry and the
//!   on-disk bytes. Idempotent.
//!
//! Validation rules (mirrored in the Start-Feature modal and Gate
//! view):
//!
//! * Reject if the file does not exist or is not a regular file.
//! * Reject if `size > 100 MiB` (v1 hard cap from the implementation
//!   spec).
//! * Reject if the per-feature attachment count would exceed 10.
//! * Refuse a re-upload whose bytes don't match an existing
//!   attachment's sha256 (the on-disk path would be `<sha256>.<ext>`,
//!   so two different contents can never share a path). The
//!   `FsAttachmentStore::write` defensive collision check enforces
//!   this and surfaces the error here.
//!
//! No new feature keys or capabilities — file reads happen inside
//! Rust, outside the webview's `fs:` scope.

use crate::domain::attachment::{
    compute_sha256_hex, ext_for_mime, mime_for_ext, sanitize_attachment_filename, AttachedFile,
};
use crate::domain::ids::FeatureId;
use crate::error::AppError;
use crate::state::AppContext;
use std::path::Path;
use tauri::State;
use tracing::{info, warn};

const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;
const MAX_ATTACHMENTS_PER_FEATURE: usize = 10;

#[tauri::command]
pub async fn feature_add_attachment(
    ctx: State<'_, AppContext>,
    feature_id: String,
    source_path: String,
    mime: Option<String>,
    source_filename: Option<String>,
) -> Result<AttachedFile, AppError> {
    let fid = FeatureId::from(feature_id.clone());
    let _ = ctx
        .features
        .get(&fid)?
        .ok_or_else(|| AppError::not_found(format!("feature not found: {}", feature_id)))?;

    let src = std::path::PathBuf::from(&source_path);
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

    let bytes = std::fs::read(&src).map_err(|e| {
        AppError::validation(format!("could not read source file {}: {}", source_path, e))
    })?;

    let sha256 = compute_sha256_hex(&bytes);

    let resolved_mime = resolve_mime(mime.as_deref(), source_filename.as_deref(), &src);
    let ext = match ext_for_mime(&resolved_mime) {
        Some(e) => e.to_string(),
        None => Path::new(source_filename.as_deref().unwrap_or(&source_path))
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_else(|| "bin".to_string()),
    };

    // Re-upload idempotency: if a manifest entry with the same sha256
    // already exists, return it as-is — the on-disk file is shared.
    let current = ctx.attachment_json.get_attachments(&fid)?;
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

    ctx.attachments.write(&feature_id, &sha256, &ext, &bytes)?;

    let display_name = sanitize_attachment_filename(source_filename.as_deref().unwrap_or(&sha256));
    let id = format!("at-{}", crate::paths::new_id());
    let file = AttachedFile {
        id: id.clone(),
        name: display_name,
        mime: resolved_mime,
        sha256: sha256.clone(),
        size: bytes.len() as u64,
        source_filename: source_filename.unwrap_or_else(|| id.clone()),
    };

    let mut next = current;
    next.push(file.clone());
    ctx.attachment_json.set_attachments(&fid, &next)?;

    info!(
        feature_id = %feature_id,
        attachment_id = %file.id,
        sha256 = %sha256,
        bytes = file.size,
        mime = %file.mime,
        "feature attachment added"
    );

    Ok(file)
}

#[tauri::command]
pub async fn feature_list_attachments(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<Vec<AttachedFile>, AppError> {
    let fid = FeatureId::from(feature_id);
    let _ = ctx
        .features
        .get(&fid)?
        .ok_or_else(|| AppError::not_found(format!("feature not found: {}", fid.as_str())))?;
    ctx.attachment_json
        .get_attachments(&fid)
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn feature_remove_attachment(
    ctx: State<'_, AppContext>,
    feature_id: String,
    attachment_id: String,
) -> Result<(), AppError> {
    let fid = FeatureId::from(feature_id.clone());
    let _ = ctx
        .features
        .get(&fid)?
        .ok_or_else(|| AppError::not_found(format!("feature not found: {}", feature_id)))?;

    let current = ctx.attachment_json.get_attachments(&fid)?;
    let mut remaining: Vec<AttachedFile> = Vec::with_capacity(current.len());
    let mut removed: Option<AttachedFile> = None;
    for a in current.into_iter() {
        if a.id == attachment_id {
            removed = Some(a);
        } else {
            remaining.push(a);
        }
    }

    let removed = match removed {
        Some(r) => r,
        None => return Ok(()), // idempotent: nothing to remove
    };

    ctx.attachment_json
        .set_attachments(&fid, &remaining)
        .map_err(AppError::from)?;

    // Best-effort on-disk cleanup; the bytes may already be shared
    // by another manifest row with the same sha256. If no other row
    // references this sha256, drop the file.
    let still_used = remaining.iter().any(|a| a.sha256 == removed.sha256);
    if !still_used {
        let ext = ext_for_mime(&removed.mime)
            .map(str::to_string)
            .unwrap_or_else(|| {
                Path::new(&removed.source_filename)
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_ascii_lowercase())
                    .unwrap_or_else(|| "bin".to_string())
            });
        let path = ctx
            .attachments
            .lookup_path(&feature_id, &removed.sha256, &ext);
        if path.exists() {
            if let Err(e) = ctx.attachments.delete(&path.to_string_lossy()) {
                warn!(
                    error = %e,
                    path = %path.display(),
                    "could not delete attachment file (already absent?)"
                );
            }
        }
    }

    info!(
        feature_id = %feature_id,
        attachment_id = %attachment_id,
        sha256 = %removed.sha256,
        "feature attachment removed"
    );
    Ok(())
}

fn resolve_mime(
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

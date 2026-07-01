//! Tauri commands for the per-feature attachment subsystem.
//!
//! Four additive commands wired through `commands::mod`:
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
//! * `attachment_read` — return the bytes of a previously-uploaded
//!   attachment for the React preview Modal. Resolves the row by
//!   `(feature_id, attachment_id)` and goes through the same
//!   path-within-root check as every other on-disk read. Never used
//!   on the prompt-injection path.
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

    // Reject unsupported file types. The frontend's `accept` attribute
    // is a hint only — users can switch the picker to "All Files" and
    // select anything (and Tauri drag-and-drop hands us arbitrary paths
    // with no mime hint at all). Keep this list in sync with
    // `domain::attachment::mime_for_ext` and the `ACCEPTED_EXTS` set in
    // `src/components/AttachmentDropzone.tsx`.
    if !is_supported_attachment(&resolved_mime, &ext) {
        return Err(AppError::validation(format!(
            "unsupported attachment type: mime={} ext={} (allowed: png, jpg, gif, webp, pdf, txt, md, json)",
            resolved_mime, ext
        )));
    }

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

/// Read the bytes of a previously-uploaded attachment.
///
/// Resolves the manifest row by `attachment_id` (scoped to the feature
/// so an attacker can't probe other features' attachments by guessing
/// ids), derives the on-disk extension the same way
/// [`feature_remove_attachment`] does, and returns the bytes via the
/// existing [`AttachmentStore::read`] port — which enforces the
/// "path within attachments root" safety check before touching the
/// filesystem.
///
/// Use case: the React preview Modal for out-of-session files (files
/// that arrived through Tauri drag-and-drop with no browser `File`
/// handle). Never used on the prompt-injection path — the orchestrator
/// already mirrors bytes into the per-step worktree via
/// `resolve_and_materialize_attachments`.
#[tauri::command]
pub async fn attachment_read(
    ctx: State<'_, AppContext>,
    feature_id: String,
    attachment_id: String,
) -> Result<Vec<u8>, AppError> {
    let fid = FeatureId::from(feature_id.clone());
    let _ = ctx
        .features
        .get(&fid)?
        .ok_or_else(|| AppError::not_found(format!("feature not found: {}", feature_id)))?;

    let current = ctx
        .attachment_json
        .get_attachments(&fid)
        .map_err(AppError::from)?;

    let attached = current
        .iter()
        .find(|a| a.id == attachment_id)
        .cloned()
        .ok_or_else(|| {
            AppError::not_found(format!(
                "attachment {} not found on feature {}",
                attachment_id, feature_id
            ))
        })?;

    let ext = derive_ext(&attached.mime, &attached.source_filename);
    let path = ctx
        .attachments
        .lookup_path(&feature_id, &attached.sha256, &ext);
    let path_str = path.to_string_lossy().to_string();

    let bytes = ctx.attachments.read(&path_str).map_err(AppError::from)?;
    Ok(bytes)
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
        let ext = derive_ext(&removed.mime, &removed.source_filename);
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

/// Returns true when the resolved mime + extension pair corresponds to
/// a supported attachment type. The mime is the authoritative signal;
/// the extension is a fallback for callers that supply a non-IANA
/// mime (e.g. `text/x-patch`) but a clean extension.
fn is_supported_attachment(mime: &str, ext: &str) -> bool {
    let lower_mime = mime.to_ascii_lowercase();
    if lower_mime.starts_with("image/") {
        return matches!(
            lower_mime.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        );
    }
    matches!(
        lower_mime.as_str(),
        "text/plain" | "text/markdown" | "application/json" | "application/pdf"
    ) || matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "pdf" | "txt" | "md" | "markdown" | "json"
    )
}

/// Lowercase extension for a stored attachment: prefer the mime
/// reverse-lookup, fall back to `source_filename`'s tail, then
/// `bin`. Mirrors the `feature_add_attachment` extension choice so
/// read/lookup/remove all hit the same `<sha256>.<ext>` path.
fn derive_ext(mime: &str, source_filename: &str) -> String {
    ext_for_mime(mime).map(str::to_string).unwrap_or_else(|| {
        Path::new(source_filename)
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_else(|| "bin".to_string())
    })
}

#[cfg(test)]
#[path = "../../tests/infrastructure/attachments_command.rs"]
mod tests;

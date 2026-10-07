//! Attachment-commit logic shared by the Tauri `commands::attachments`
//! wrappers (`src-tauri/src/commands/attachments.rs`) and the step executor's
//! pre-execution staged-attachment path
//! (`adapters/step_executor/impl_traits/bootstrap.rs`). Split out so both call
//! sites — one behind a Tauri command, one inside the engine — share
//! identical validation, dedup, and storage rules.

use crate::domain::attachment::{
    compute_sha256_hex, content_matches_mime, ext_for_mime, mime_for_ext, normalize_mime,
    path_form_stored_name, resolved_ext, sanitize_attachment_filename, AttachedFile,
};
use crate::domain::ids::FeatureId;
use crate::error::AppError;
use crate::ports::attachment_store::{AttachmentJsonPort, AttachmentStore};
use crate::ports::db::FeatureRepository;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;

pub const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_ATTACHMENTS_PER_FEATURE: usize = 10;
pub const MAX_AGENT_ATTACHMENT_BATCH_BYTES: u64 = 256 * 1024 * 1024;

/// Ceiling on the raw item count of one `start_feature` attachment list,
/// distinct or not. Duplicates collapse to one, so the distinct count
/// ([`MAX_ATTACHMENTS_PER_FEATURE`]) cannot bound the list: tens of thousands
/// of items naming one file would each be read and hashed before dedup saw
/// them. A little headroom over the distinct cap keeps a caller that repeats a
/// file from being refused for it.
const MAX_AGENT_ATTACHMENT_ITEMS: usize = MAX_ATTACHMENTS_PER_FEATURE * 2;

const AGENT_ATTACHMENT_MIMES: [&str; 9] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/tiff",
    "application/pdf",
    "text/plain",
    "text/markdown",
    "application/json",
];

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

/// One file an external caller attaches to a launch. Which of `path` and
/// `content_base64` is set is decided by `resolve_agent_attachment`, so this
/// stays a flat bag of optionals. Re-exported from `agent_surface`, which is
/// where callers outside this module import it.
#[derive(Debug, Clone, Default)]
pub struct AgentAttachment {
    pub path: Option<String>,
    pub content_base64: Option<String>,
    pub mime: Option<String>,
    pub filename: Option<String>,
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

    let sha256 = compute_sha256_hex(&bytes);
    let ResolvedAttachmentType {
        mime: resolved_mime,
        ext,
    } = resolve_attachment_type(mime, source_filename, Path::new(source_path));

    if !is_supported_attachment(&resolved_mime, &ext) {
        return Err(AppError::validation(unsupported_type_message(
            &resolved_mime,
            &ext,
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

fn unsupported_type_message(mime: &str, ext: &str) -> String {
    format!(
        "unsupported attachment type: mime={mime} ext={ext} (allowed: png, jpg, gif, webp, tiff, pdf, txt, md, json)"
    )
}

fn too_large_message(len: u64) -> String {
    format!("attachment too large: {len} bytes (max {MAX_ATTACHMENT_BYTES})")
}

/// Whether a standard-base64 string of `encoded_len` characters can only decode
/// to more than [`MAX_ATTACHMENT_BYTES`]. Lets a caller refuse before the
/// decoder allocates the buffer it would then reject.
fn inline_encoded_len_exceeds_limit(encoded_len: u64) -> bool {
    encoded_len > 4 * MAX_ATTACHMENT_BYTES.div_ceil(3)
}

/// Validate one external-caller attachment into the staged form a launch
/// carries: bytes in memory, never a path, so the bytes that were checked are
/// the bytes committed.
///
/// The accepted set is the strict `AGENT_ATTACHMENT_MIMES` list with no
/// extension fallback, unlike [`is_supported_attachment`]: this surface can name
/// any host file, so a caller-chosen filename must not widen what is accepted.
/// Content is sniffed for the same reason. `forbidden` is the set of roots a
/// path item may not resolve into; see [`agent_attachment_forbidden_roots`].
#[cfg(test)]
fn resolve_agent_attachment(
    index: usize,
    item: &AgentAttachment,
    forbidden: &[PathBuf],
) -> Result<StagedAttachmentInput, String> {
    let read = |path: &Path, mime: &str, limit: u64| read_agent_path(path, mime, forbidden, limit);
    read_agent_item(item, MAX_ATTACHMENT_BYTES, &read)
        .and_then(finish_agent_item)
        .map_err(|e| format!("attachments[{index}]: {e}"))
}

/// Reads the file a path item names, given its spelled mime and the most it may
/// return; injected so a test can stand in for a file whose stat lies.
type PathReader<'a> = dyn Fn(&Path, &str, u64) -> Result<Vec<u8>, String> + 'a;

/// An item whose bytes are read but not yet checked, so a caller can charge a
/// batch budget for them before anything is sniffed.
struct ReadAgentItem {
    bytes: Vec<u8>,
    mime: String,
    name: String,
}

/// Reads one item. A path read may hand back up to `limit` + 1 bytes: the
/// reader stops one byte past what it was allowed so the caller can tell
/// "exactly at the limit" from "over it" without holding the rest.
fn read_agent_item(
    item: &AgentAttachment,
    limit: u64,
    read: &PathReader,
) -> Result<ReadAgentItem, String> {
    let path = trimmed_path(item);
    let inline = item
        .content_base64
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    let filename = item
        .filename
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty());
    let supplied = item
        .mime
        .as_deref()
        .map(normalize_mime)
        .filter(|m| !m.is_empty());

    let (bytes, mime, name) = match (path, inline) {
        (Some(path), None) => {
            let (mime, basename) = agent_path_type(path, supplied.as_deref())?;
            let bytes = read(Path::new(path), &mime, limit.min(MAX_ATTACHMENT_BYTES))?;
            if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
                return Err(format!(
                    "attachment too large: more than {MAX_ATTACHMENT_BYTES} bytes (max {MAX_ATTACHMENT_BYTES})"
                ));
            }
            let name = path_form_stored_name(&mime, &basename, filename);
            (bytes, mime, name)
        }
        (None, Some(encoded)) => {
            let name = filename.ok_or("filename is required with content_base64")?;
            let mime =
                agent_attachment_mime(AgentSource::Inline { filename: name }, supplied.as_deref())?;
            (decode_inline(encoded)?, mime, name.to_string())
        }
        _ => return Err("give exactly one of path or content_base64".to_string()),
    };
    Ok(ReadAgentItem { bytes, mime, name })
}

fn finish_agent_item(read: ReadAgentItem) -> Result<StagedAttachmentInput, String> {
    let ReadAgentItem { bytes, mime, name } = read;
    if bytes.is_empty() {
        return Err("attachment bytes are empty".to_string());
    }
    check_agent_content(&mime, &bytes)?;

    Ok(StagedAttachmentInput {
        source_path: String::new(),
        mime: Some(mime),
        source_filename: Some(name),
        bytes: Some(bytes),
    })
}

/// Where an agent attachment's type signal comes from. On the path form it is
/// the file's own basename and nothing the caller typed can move it; inline there
/// is no file, so the caller's `filename` and `mime` are the only signal.
enum AgentSource<'a> {
    Path { basename: &'a str },
    Inline { filename: &'a str },
}

/// The mime an agent attachment is accepted as, decided from names alone so it
/// needs no I/O and an unsupported name is refused identically whether or not
/// the file exists. A `supplied` mime on the path form may only confirm what the
/// basename already says; it cannot rename a `credentials` file into text.
fn agent_attachment_mime(source: AgentSource, supplied: Option<&str>) -> Result<String, String> {
    let (name, declared) = match source {
        AgentSource::Path { basename } => (basename, None),
        AgentSource::Inline { filename } => (filename, supplied),
    };
    let ResolvedAttachmentType { mime, ext } =
        resolve_attachment_type(declared, Some(name), Path::new(""));
    if !AGENT_ATTACHMENT_MIMES.contains(&mime.as_str()) {
        return Err(unsupported_type_message(&mime, &ext));
    }
    if let (AgentSource::Path { .. }, Some(supplied)) = (&source, supplied) {
        if supplied != mime {
            return Err(format!(
                "mime {supplied} does not match the file's type {mime}"
            ));
        }
    }
    Ok(mime)
}

/// Whether the file a path resolves to is the type its spelled name claimed. A
/// symlink `notes.txt` -> `~/.ssh/config` spells text and targets a file the
/// name policy would refuse; judging the spelled name alone would let it through.
fn canonical_name_matches_mime(canonical: &Path, spelled_mime: &str) -> bool {
    let Some(basename) = canonical.file_name().map(|n| n.to_string_lossy()) else {
        return false;
    };
    agent_attachment_mime(
        AgentSource::Path {
            basename: &basename,
        },
        None,
    )
    .is_ok_and(|mime| mime == spelled_mime)
}

fn check_agent_content(mime: &str, bytes: &[u8]) -> Result<(), String> {
    if content_matches_mime(mime, bytes) {
        Ok(())
    } else {
        Err(format!("content does not look like {mime}"))
    }
}

/// The type a path item resolves to and the basename it was read from, before
/// the filesystem is touched.
fn agent_path_type(path: &str, supplied: Option<&str>) -> Result<(String, String), String> {
    let src = Path::new(path);
    if !src.is_absolute() {
        return Err(format!("path must be absolute: {path}"));
    }
    let basename = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| format!("could not take a filename from {path}"))?;
    let mime = agent_attachment_mime(
        AgentSource::Path {
            basename: &basename,
        },
        supplied,
    )?;
    Ok((mime, basename))
}

/// Roots a path item may not resolve into: Demeteo's own state, which holds
/// every Feature's staged bytes and the database.
///
/// `workspace_dir` defaults to `app_data_dir`, so project repos clone under the
/// very directory being protected and a mockup in one is legitimate. While the
/// workspace sits inside the data directory only Demeteo's own entries are
/// forbidden; the rest of that directory is not, which is a known gap. Once the
/// workspace lives elsewhere nothing legitimate remains in the data directory
/// and all of it is forbidden.
pub fn agent_attachment_forbidden_roots(app_data_dir: &Path, workspace_dir: &Path) -> Vec<PathBuf> {
    let workspace_inside =
        canonical_or_given(workspace_dir).starts_with(canonical_or_given(app_data_dir));
    if !workspace_inside {
        return vec![app_data_dir.to_path_buf()];
    }
    [
        "attachments",
        "artifacts",
        "demeteo.db",
        "demeteo.db-wal",
        "demeteo.db-shm",
        "demeteo.db-journal",
    ]
    .iter()
    .map(|entry| app_data_dir.join(entry))
    .collect()
}

fn canonical_or_given(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Compared on canonical paths, with `Path::starts_with` rather than a string
/// prefix, so neither a symlink nor `data-dir-sibling` walks around it.
fn inside_any(canonical: &Path, roots: &[PathBuf]) -> bool {
    roots
        .iter()
        .any(|root| canonical.starts_with(canonical_or_given(root)))
}

/// Validate a whole `start_feature` attachment list into the batch a launch
/// carries, or refuse all of it: the first item error wins, so a launch never
/// starts with part of what the caller asked for.
///
/// Items with identical bytes collapse to the first, which is what lets eleven
/// items that name ten distinct files through.
pub fn resolve_agent_attachments(
    items: Vec<AgentAttachment>,
    forbidden: &[PathBuf],
) -> Result<Vec<StagedAttachmentInput>, String> {
    resolve_agent_attachments_capped(items, forbidden, MAX_AGENT_ATTACHMENT_BATCH_BYTES)
}

fn resolve_agent_attachments_capped(
    items: Vec<AgentAttachment>,
    forbidden: &[PathBuf],
    batch_cap: u64,
) -> Result<Vec<StagedAttachmentInput>, String> {
    let read = |path: &Path, mime: &str, limit: u64| read_agent_path(path, mime, forbidden, limit);
    resolve_agent_attachments_with(items, batch_cap, &read)
}

/// The batch is charged for the bytes each item *yielded*, never for what a
/// stat claimed: a virtual file reports length 0, and a file can grow after its
/// stat. Each read is also handed only what is left of the budget, so one call
/// never holds more than `batch_cap` plus one byte, whatever any file reports.
/// Duplicates are charged too: each is read and hashed before dedup can see it.
fn resolve_agent_attachments_with(
    items: Vec<AgentAttachment>,
    batch_cap: u64,
    read: &PathReader,
) -> Result<Vec<StagedAttachmentInput>, String> {
    if items.len() > MAX_AGENT_ATTACHMENT_ITEMS {
        return Err(format!(
            "too many attachment items: {} (max {MAX_AGENT_ATTACHMENT_ITEMS})",
            items.len()
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    let mut bytes_read: u64 = 0;
    for (index, item) in items.iter().enumerate() {
        let remaining = batch_cap.saturating_sub(bytes_read);
        let item_err = |e: String| format!("attachments[{index}]: {e}");
        let read_item = read_agent_item(item, remaining, read).map_err(item_err)?;
        bytes_read = bytes_read.saturating_add(read_item.bytes.len() as u64);
        refuse_over_batch_cap(bytes_read, batch_cap)?;
        let staged = finish_agent_item(read_item).map_err(item_err)?;
        let Some(bytes) = staged.bytes.as_deref() else {
            continue;
        };
        if !seen.insert(compute_sha256_hex(bytes)) {
            continue;
        }
        if out.len() >= MAX_ATTACHMENTS_PER_FEATURE {
            return Err(format!(
                "feature would have {} attachments (max {})",
                out.len() + 1,
                MAX_ATTACHMENTS_PER_FEATURE
            ));
        }
        out.push(staged);
    }
    Ok(out)
}

fn refuse_over_batch_cap(total: u64, cap: u64) -> Result<(), String> {
    if total > cap {
        return Err(format!(
            "attachments are too large together: {total} bytes (max {cap})"
        ));
    }
    Ok(())
}

/// The one spelling of a path item's path, so what is counted against the
/// batch and what is read are the same file.
fn trimmed_path(item: &AgentAttachment) -> Option<&str> {
    item.path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
}

fn decode_inline(encoded: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    if inline_encoded_len_exceeds_limit(encoded.len() as u64) {
        return Err(format!(
            "content_base64 is too large: {} characters",
            encoded.len()
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "content_base64 is not valid base64".to_string())?;
    if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
        return Err(too_large_message(bytes.len() as u64));
    }
    Ok(bytes)
}

const UNREADABLE_PATH: &str = "could not read the file at path";

/// One refusal for a path that is missing, not a regular file, or unreadable, so
/// the message is no oracle on what the host holds; the later `content does not
/// look like` refusal stays distinguishable because a legitimate caller needs it.
///
/// Returns at most `limit` + 1 bytes; see [`read_up_to_one_past`].
fn read_agent_path(
    path: &Path,
    spelled_mime: &str,
    forbidden: &[PathBuf],
    limit: u64,
) -> Result<Vec<u8>, String> {
    let unreadable = |_: std::io::Error| UNREADABLE_PATH.to_string();
    let canonical = std::fs::canonicalize(path).map_err(unreadable)?;
    if inside_any(&canonical, forbidden) {
        return Err("path is inside Demeteo's own data directory".to_string());
    }
    if !canonical_name_matches_mime(&canonical, spelled_mime) {
        return Err(UNREADABLE_PATH.to_string());
    }
    // Stat before open: opening a FIFO blocks until a writer appears.
    if !std::fs::metadata(&canonical).map_err(unreadable)?.is_file() {
        return Err(UNREADABLE_PATH.to_string());
    }
    let file = std::fs::File::open(&canonical).map_err(unreadable)?;
    let len = file.metadata().map_err(unreadable)?.len();
    if len > MAX_ATTACHMENT_BYTES {
        return Err(too_large_message(len));
    }
    read_up_to_one_past(file, limit)
}

/// `len()` bounds nothing for `/proc`-style files, which report 0, or for a file
/// that grows after the stat, so the bound is enforced on what is actually read.
/// Stops one byte past `limit` rather than failing, so the caller decides which
/// limit that was (one file's, or what is left of a batch) and says so.
fn read_up_to_one_past(source: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    source
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| UNREADABLE_PATH.to_string())?;
    Ok(bytes)
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

    let ResolvedAttachmentType {
        mime: resolved_mime,
        ext,
    } = resolve_attachment_type(
        file.mime.as_deref(),
        source_filename,
        Path::new(source_path),
    );
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

/// The mime and stored extension an attachment resolves to. Support is not
/// decided here: each caller words its own refusal, so this carries no verdict.
pub struct ResolvedAttachmentType {
    pub mime: String,
    pub ext: String,
}

/// The one mime → extension ladder. The mime wins; only a mime with no
/// canonical extension falls back to the filename's (then the path's), and
/// `bin` when there is none.
pub fn resolve_attachment_type(
    mime: Option<&str>,
    source_filename: Option<&str>,
    source_path: &Path,
) -> ResolvedAttachmentType {
    let mime = resolve_mime(mime, source_filename, source_path);
    let ext = match ext_for_mime(&mime) {
        Some(e) => e.to_string(),
        None => source_filename
            .map(Path::new)
            .unwrap_or(source_path)
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_else(|| "bin".to_string()),
    };
    ResolvedAttachmentType { mime, ext }
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

#[cfg(test)]
#[path = "../../tests/application/attachments.rs"]
mod tests;

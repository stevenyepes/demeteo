//! `AttachmentStore` port — durable, per-feature binary attachment storage.
//!
//! Mirrors the structure of [`crate::ports::artifact_store::ArtifactStore`]
//! (both are per-feature binary stores behind a `Send + Sync` trait), but
//! differs in two important ways:
//!
//! * **Content-addressable on-disk layout.** The file is written at
//!   `<root>/<feature_id>/<sha256>.<ext>` so a re-upload of the same
//!   bytes is a no-op and the orchestrator can emit a stable path
//!   manifest in the rendered prompt even if the user re-attaches
//!   the same image.
//! * **No step dimension.** Artifacts are keyed by step because a
//!   workflow run is a sequence of steps with their own outputs;
//!   attachments aren't owned by steps — they're owned by the
//!   feature run itself.
//!
//! The JSON-column side of the contract lives on a sibling trait,
//! [`AttachmentJsonPort`], in this same module. It is split off so the
//! on-disk store (FS-only, easily swapped for S3 / SFTP-on-remote in
//! the future) doesn't need to know about SQLite, and the SQLite side
//! doesn't need to know about file layout. The two traits are
//! implemented side-by-side by the same `SqliteAdapter` + filesystem
//! pair, but each can be swapped independently.
//!
//! Implementations must be `Send + Sync` because the executor may
//! hold an `Arc<dyn AttachmentStore>` across `await` points (the
//! per-step copy into `{worktree}/artifacts/_context/`).

use std::path::Path;

use crate::domain::attachment::AttachedFile;
use crate::domain::ids::FeatureId;

pub trait AttachmentStore: Send + Sync {
    /// Drop the entire on-disk directory for `feature_id`. No-op when
    /// the directory is already absent (idempotent — `feature_cleanup`
    /// may run multiple times in different lifecycle branches).
    ///
    /// Wired into the orchestrator's "cancel/cleanup" lifecycle so the
    /// cleanup pass doesn't need any other state to release disk. The
    /// DB-side `attachments_json` column is the caller's responsibility
    /// (this port only owns bytes on disk).
    fn clear_feature(&self, feature_id: &str) -> Result<(), String>;

    /// Drop a single attachment file from disk by its stored path.
    /// The stored path is whatever `lookup_path` returned for the
    /// attachment, so the caller doesn't need to re-derive the
    /// content-addressable layout.
    fn delete(&self, stored_path: &str) -> Result<(), String>;

    /// Resolve the on-disk path for `(feature_id, sha256, ext)` without
    /// touching the filesystem. Pure path math.
    fn lookup_path(&self, feature_id: &str, sha256: &str, ext: &str) -> std::path::PathBuf;

    /// Walk `<root>/<feature_id>` and return every stored file's
    /// absolute path. Used by `feature_cleanup` audits and
    /// by tests.
    fn list_for_feature(&self, feature_id: &str) -> Result<Vec<String>, String>;

    /// Read the bytes at `stored_path`. Returns an error if the file
    /// has been deleted or the path is outside the store's root.
    fn read(&self, stored_path: &str) -> Result<Vec<u8>, String>;

    /// Return the root directory the store writes under (for tests
    /// and diagnostics).
    fn root(&self) -> &Path;

    /// Persist `bytes` for `(feature_id, sha256, ext)`. Creates
    /// `<root>/<feature_id>/` if missing. Returns the absolute path
    /// the bytes were written to. Idempotent on content match (writes
    /// the same `<sha256>.<ext>` twice → one file on disk).
    fn write(
        &self,
        feature_id: &str,
        sha256: &str,
        ext: &str,
        bytes: &[u8],
    ) -> Result<String, String>;
}

/// JSON-manifest persistence for the per-feature attachment list.
///
/// The blob lives on the `features` row itself (`attachments_json`,
/// migration V19), so this trait lives separately from
/// [`AttachmentStore`] — the FS adapter doesn't know about SQLite,
/// and the SQLite adapter doesn't need to know about the on-disk
/// file layout.
///
/// `set_attachments` is destructive: the new list replaces the
/// stored blob. It is the caller's responsibility to first read,
/// mutate, then write. Callers should treat this as the database
/// "tail" of a `get-add-OR-remove` cycle.
pub trait AttachmentJsonPort: Send + Sync {
    /// Read the current manifest for `feature_id`. Returns an empty
    /// vec when the feature has no attachments column populated
    /// (NULL or empty JSON array). Returns `Err` only on DB failure.
    fn get_attachments(&self, feature_id: &FeatureId) -> Result<Vec<AttachedFile>, String>;

    /// Replace the manifest with `attachments`. An empty slice sets
    /// the column to `NULL` (matches the legacy V12-era JSON column
    /// convention of `NULL ↔ empty list`).
    fn set_attachments(
        &self,
        feature_id: &FeatureId,
        attachments: &[AttachedFile],
    ) -> Result<(), String>;
}

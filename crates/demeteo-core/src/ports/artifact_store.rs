//! ArtifactStore port — the durable per-step record layer.
//!
//! The executor and the tool bridge both call this port. The default
//! implementation is the filesystem adapter under
//! `src-tauri/src/adapters/artifact_store/fs.rs`; future
//! implementations could be S3, SFTP-remote, or in-memory for tests.
//!
//! See `docs/ARCHITECTURE.md` §2 (`ArtifactStore` is in the locked port
//! catalogue) and `AGENT_INTEGRATION.md` §3.4 for the AgentEvent side
//! of this contract.

use crate::domain::artifact::Artifact;

/// The artifact persistence port. All implementations must be
/// `Send + Sync` because the executor holds an `Arc<dyn ArtifactStore>`
/// across `await` points and may resolve artifacts on a background
/// task (e.g. computing a diff in `GitOpsHelper`).
pub trait ArtifactStore: Send + Sync {
    /// Persist `artifact` for the given `(feature_id, step_id)`. The
    /// returned string is a stable *reference* the orchestrator stores
    /// in `step_executions.artifact_paths` and that
    /// `resolve_attached_artifacts` later reads back via `get`.
    ///
    /// The reference is implementation-defined. The FS adapter returns
    /// the absolute path; an S3 adapter would return an S3 URI.
    fn put(&self, feature_id: &str, step_id: &str, artifact: &Artifact) -> Result<String, String>;

    /// Read the artifact content by reference. Returns the raw
    /// content string; for `WorktreeRef` artifacts this is the JSON
    /// envelope the frontend dispatches on.
    fn get(&self, reference: &str) -> Result<String, String>;

    /// List all stored references for a step, in insertion order. Used
    /// by `resolve_attached_artifacts` to materialize a step's
    /// contribution when a downstream step's template references it
    /// by step-id only.
    fn list_for_step(&self, feature_id: &str, step_id: &str) -> Result<Vec<String>, String>;

    /// Drop all artifacts for a step. Used when the executor rolls
    /// back a step (e.g. `step_retry` resets the row, so the old
    /// artifact bundle on disk is stale and confusing).
    fn clear_step(&self, feature_id: &str, step_id: &str) -> Result<(), String>;
}

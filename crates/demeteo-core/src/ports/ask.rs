//! Persistence for the standalone Ask chat (V51).
//!
//! One trait over one aggregate, unlike [`DiscoveryPort`](crate::ports::discovery::DiscoveryPort)
//! `/` `TicketPort` split: Ask has no decomposition surface for a second reader
//! to want without the transcript.

use crate::domain::ids::{AskThreadId, ProjectId};
use crate::domain::models::{AskMessage, AskStatus, AskThread, EffortLevel};

/// The fields one transition may change on an [`AskThread`].
///
/// Nullable columns take the `Option<Option<T>>` of
/// [`FeaturePatch`](crate::ports::db::FeaturePatch), on the same terms.
///
/// Agent kind and machine choice are absent, same as
/// [`DiscoveryPatch`](crate::ports::discovery::DiscoveryPatch) omits the
/// interviewer choice: a thread's harness is fixed at creation, and a patch
/// field nothing writes reads as a supported operation.
#[derive(Debug, Default, Clone)]
pub struct AskThreadPatch {
    pub title: Option<String>,
    pub status: Option<AskStatus>,
    pub model: Option<Option<String>>,
    pub effort: Option<Option<EffortLevel>>,
    /// Not nullable — a thread always has a network posture.
    pub network: Option<bool>,
    pub worktree_path: Option<Option<String>>,
    pub session_id: Option<Option<String>>,
    /// Folded into the stored totals rather than replacing them, so two turns
    /// that finish out of order still sum to what was spent — the same
    /// reasoning as `DiscoveryPatch::add_cost`.
    pub add_turns: i64,
    pub add_cost_usd: f64,
    pub add_tokens: i64,
}

pub trait AskPort: Send + Sync {
    fn create(&self, thread: &AskThread) -> Result<(), String>;
    fn get(&self, id: &AskThreadId) -> Result<Option<AskThread>, String>;
    /// A project's Ask threads, most recently touched first.
    fn list_for_project(&self, project_id: &ProjectId) -> Result<Vec<AskThread>, String>;
    fn update(&self, id: &AskThreadId, patch: &AskThreadPatch, now: i64) -> Result<(), String>;
    /// Take the thread and its transcript, via the declared foreign key.
    fn delete(&self, id: &AskThreadId) -> Result<(), String>;
    fn append_message(&self, message: &AskMessage) -> Result<(), String>;
    /// The whole transcript in the order it was said.
    fn list_messages(&self, id: &AskThreadId) -> Result<Vec<AskMessage>, String>;
}

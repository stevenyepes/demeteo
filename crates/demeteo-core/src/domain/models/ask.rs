//! A lightweight, project-scoped chat with an agent (V51,
//! `ask_thread` / `ask_message`).
//!
//! Deliberately not a Discovery variant: an Ask thread never decomposes into
//! tickets, so it carries none of Discovery's decomposition surface. It
//! reuses Discovery's shared vocabulary — [`MessageRole`], [`TurnActivity`],
//! [`EffortLevel`], [`validate_title`](crate::domain::models::discovery::validate_title),
//! [`TITLE_MAX_CHARS`](crate::domain::models::discovery::TITLE_MAX_CHARS) — rather than
//! forking semantically identical types.

use serde::{Deserialize, Serialize};

use crate::domain::ids::{AskThreadId, MachineId, ProjectId};
use crate::domain::models::{EffortLevel, MessageRole, TurnActivity};

/// One Ask chat thread, as the row holds it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskThread {
    pub id: AskThreadId,
    pub project_id: ProjectId,
    pub title: String,
    pub status: AskStatus,
    pub agent_kind: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<EffortLevel>,
    pub machine_id: MachineId,
    /// The tree the thread reads the repo in, reserved for the turn loop
    /// (`ask-turn-loop`). This ticket never populates it.
    #[serde(default)]
    pub worktree_path: Option<String>,
    /// The harness session the next turn can `--resume`, reserved for the
    /// turn loop. This ticket never populates it.
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub turn_count: i64,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub tokens: i64,
    /// Whether the thread's agent may reach the network. Matches the
    /// hard-coded `Access::Allow` posture that predates this column.
    #[serde(default = "network_default")]
    pub network: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

pub(crate) fn network_default() -> bool {
    true
}

/// One turn of an Ask conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskMessage {
    pub id: String,
    pub thread_id: AskThreadId,
    pub role: MessageRole,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub tokens: Option<i64>,
    #[serde(default)]
    pub turn_activity: Option<TurnActivity>,
    #[serde(default)]
    pub canvas_paths: Option<Vec<CanvasPathVerdict>>,
    #[serde(default)]
    pub checked_commit_sha: Option<String>,
    pub created_at: i64,
}

/// Whether a path a canvas node cited resolves against the tree checked at
/// `AskMessage::checked_commit_sha`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasPathVerdict {
    pub node_id: String,
    pub path: String,
    pub resolved: bool,
}

/// Whether an Ask thread is still open.
///
/// Mirrors `DiscoveryStatus`'s two-state, non-destructive vocabulary: closing
/// a thread does not touch its transcript. This ticket adds no close/reopen
/// operation — `Closed` exists so a later turn-loop/UI ticket has somewhere
/// to land without a second status migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskStatus {
    Open,
    Closed,
}

impl AskStatus {
    /// The stable lowercase identifier used on the wire and in the DB.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }

    /// Parse a stored status. `None` for anything unknown; callers reading a
    /// row degrade a `None` to [`AskStatus::Closed`] at the call site, the
    /// same convention `DiscoveryStatus::parse` readers follow.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/models/ask.rs"]
mod tests;

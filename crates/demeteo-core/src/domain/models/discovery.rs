//! The planning conversation a Feature can come out of (V47,
//! `docs/PRD_DISCOVERY.md` §8.1).
//!
//! [`DiscoveryMessage`] is the record of what was said, and it is the
//! authority: [`Discovery::resume_session_id`] only names the harness's own
//! copy, which the harness is free to prune. Anything that reconstructs a turn
//! must be able to do it from the messages alone.

use serde::{Deserialize, Serialize};

use crate::domain::attachment::AttachedFile;
use crate::domain::ids::{DiscoveryId, MachineId, ProjectId};
use crate::domain::models::EffortLevel;

/// One planning conversation, as the row holds it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discovery {
    pub id: DiscoveryId,
    pub project_id: ProjectId,
    pub title: String,
    pub status: DiscoveryStatus,
    /// Where the interview's turns run. [`MachineId::is_local`] for the
    /// desktop host.
    pub machine_id: MachineId,
    /// Chosen per Discovery rather than inherited from the project default,
    /// as are [`Discovery::model`] and [`Discovery::effort`]: interviewing and
    /// implementing want different things from a model (§4.5).
    pub agent_kind: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<EffortLevel>,
    /// The harness session the next turn can `--resume`. `None` means the turn
    /// re-seeds from [`DiscoveryMessage`] instead — the state a Discovery
    /// reaches whenever the harness has forgotten the session, which is the
    /// resumed-a-week-later case this feature exists for (§4.4).
    #[serde(default)]
    pub resume_session_id: Option<String>,
    /// The tree the interview reads the repo in. `None` both before the first
    /// turn that needs one and after an idle reclaim (§4.6); nothing
    /// distinguishes them because the interview writes nothing, so either is
    /// answered by provisioning again.
    #[serde(default)]
    pub worktree_path: Option<String>,
    /// What the user handed the interviewer (§4.6). Owned by the Discovery
    /// rather than by a turn: the composer's chip row survives the turn that
    /// added it, and every later turn is prompted with the same set.
    #[serde(default)]
    pub attachments: Vec<AttachedFile>,
    #[serde(default)]
    pub total_cost: f64,
    #[serde(default)]
    pub tokens: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Whether the interview is still being conducted.
///
/// Two states, and neither destroys anything: decomposition is not terminal,
/// and a closed Discovery keeps its transcript and its tickets (§8.3, §8.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryStatus {
    Open,
    Closed,
}

impl DiscoveryStatus {
    /// The stable lowercase identifier used on the wire and in the DB.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }

    /// Parse a stored status. `None` for anything unknown, so a row written by
    /// a newer build degrades rather than panicking — mirrors
    /// [`EffortLevel::parse`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }
}

/// One turn of the interview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryMessage {
    pub id: String,
    pub discovery_id: DiscoveryId,
    pub role: MessageRole,
    #[serde(default)]
    pub content: String,
    /// What the turn cost. `None` on a user message, and on an assistant turn
    /// whose harness reported no spend — distinct from `0.0`, which is a
    /// measurement.
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub tokens: Option<i64>,
    pub created_at: i64,
}

/// Who said it.
///
/// There is no system role. What the interviewer is told is assembled per turn
/// from the project's live state (§4.6), so a stored copy would be describing
/// a world that has since moved on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}

impl MessageRole {
    /// The stable lowercase identifier used on the wire and in the DB.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }

    /// Parse a stored role. `None` for anything unknown, so a row written by a
    /// newer build degrades rather than panicking — mirrors
    /// [`DiscoveryStatus::parse`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            _ => None,
        }
    }
}

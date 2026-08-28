//! The planning conversation a Feature can come out of (V47,
//! `docs/PRD_DISCOVERY.md` §8.1).
//!
//! [`DiscoveryMessage`] is the record of what was said, and it is the
//! authority: [`Discovery::resume_session_id`] only names the harness's own
//! copy, which the harness is free to prune. Anything that reconstructs a turn
//! must be able to do it from the messages alone.

use serde::{Deserialize, Serialize};

use crate::domain::action::ActionKind;
use crate::domain::agent_event::AgentEvent;
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

/// The longest a [`Discovery::title`] runs.
///
/// The title is a **name**, not the opening move: it labels the row in Project
/// Home's list and the workspace header, and no prompt reads it. Nothing about
/// a free textarea said so, and a user with an idea in hand types the idea into
/// it — then opens an interview that has never been told any of it, because the
/// interviewer is prompted from the transcript and the transcript starts empty.
///
/// A cap is what makes the field state its own purpose: a box that refuses a
/// paragraph is a box asking for a name, before anyone reads the label. Eighty
/// is a long name and a short sentence, which is the line it has to sit on.
pub const TITLE_MAX_CHARS: usize = 80;

/// A title as it may be stored, or why it may not be.
///
/// Trims first and measures after, so trailing whitespace never costs a user
/// the last word of a name that fits. Measured in `chars` rather than bytes —
/// the cap is about what the list can show, and a byte count would refuse a
/// shorter name for being written in a language with wider code points.
pub fn validate_title(raw: &str) -> Result<String, String> {
    let title = raw.trim();
    if title.is_empty() {
        return Err("A discovery needs a name.".into());
    }
    let length = title.chars().count();
    if length > TITLE_MAX_CHARS {
        return Err(format!(
            "A discovery's name is a label for the list, and this one is {length} characters — \
             keep it under {TITLE_MAX_CHARS}. Say the idea itself in the interview: the first \
             thing you send is what the interviewer is asked about."
        ));
    }
    Ok(title.to_string())
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
    /// What the turn *did*, for the meta line under a settled bubble
    /// (`docs/DISCOVERY_UI_SPEC.md` §3.4.3). `None` on a user message and on
    /// any assistant turn stored before the column existed — which is why the
    /// renderer must be able to say nothing rather than say zero.
    #[serde(default)]
    pub activity: Option<TurnActivity>,
    pub created_at: i64,
}

/// What one turn did, small enough to live on the message row.
///
/// Counts and a bounded sample of commands, never a ledger: the live surface
/// keeps the full sequence for as long as the turn is on screen, and the point
/// of persisting anything is that a bubble the user scrolls back to reads the
/// same as it did while it streamed. A per-tool-call table would answer
/// questions nothing asks and would grow without bound on a turn that greps in
/// a loop.
///
/// The commands are stored as the agent issued them, first line only. Turning
/// one into the name a human reads (`git log`) is a rendering decision and is
/// made once, on the surface, so the live turn and the settled one cannot
/// disagree about it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnActivity {
    #[serde(default)]
    pub reads: u32,
    #[serde(default)]
    pub edits: u32,
    #[serde(default)]
    pub writes: u32,
    /// Every command the turn ran, including the ones past [`TurnActivity::MAX_COMMANDS`]
    /// that `commands` does not name — so a surface can say how much it is not showing.
    #[serde(default)]
    pub ran: u32,
    #[serde(default)]
    pub commands: Vec<String>,
}

impl TurnActivity {
    /// How many distinct commands are kept. Six fits the meta line; past that
    /// the count carries the rest.
    pub const MAX_COMMANDS: usize = 6;
    /// Enough for a command plus its flags. A `run_bash` target can be a whole
    /// script — the parser hands over the literal `command` input — and the
    /// row is not where that belongs.
    pub const MAX_COMMAND_CHARS: usize = 120;

    /// Fold one streamed event in. Everything that is not a tool call is
    /// ignored, including a call's later status: a command that failed was
    /// still run, and a turn that reports six reads of which one errored is
    /// describing the same work.
    pub fn observe(&mut self, event: &AgentEvent) {
        let AgentEvent::ToolCall { action, target, .. } = event else {
            return;
        };
        match action {
            ActionKind::Read => self.reads += 1,
            ActionKind::Edit => self.edits += 1,
            ActionKind::Write => self.writes += 1,
            ActionKind::RunBash => {
                self.ran += 1;
                self.remember_command(target);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.reads == 0 && self.edits == 0 && self.writes == 0 && self.ran == 0
    }

    fn remember_command(&mut self, target: &str) {
        if self.commands.len() >= Self::MAX_COMMANDS {
            return;
        }
        let sample: String = target
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .chars()
            .take(Self::MAX_COMMAND_CHARS)
            .collect();
        if sample.is_empty() || self.commands.iter().any(|c| c == &sample) {
            return;
        }
        self.commands.push(sample);
    }
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

#[cfg(test)]
#[path = "../../../tests/domain/models/discovery.rs"]
mod tests;

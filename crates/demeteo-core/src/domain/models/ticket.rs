//! A pending Feature spec, owned by a [`Discovery`](crate::domain::models::Discovery)
//! (V47, `docs/PRD_DISCOVERY.md` §6).
//!
//! Everything about a Ticket that a screen shows — startable, blocked, in
//! flight, landed — is derived on read from [`Ticket::blocked_by`] and the
//! forge state of each dependency's Feature (§6.3). The three states below are
//! the whole stored vocabulary, and nothing may add a fourth that caches a
//! derived answer.

use serde::{Deserialize, Serialize};

use crate::domain::attachment::AttachedFile;
use crate::domain::ids::{DiscoveryId, FeatureId, TicketId, WorkflowId};
use crate::domain::models::EffortLevel;

/// One unit of planned work, as the row holds it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    pub id: TicketId,
    pub discovery_id: DiscoveryId,
    /// The number a user says out loud. Assigned once and never reissued:
    /// re-running decomposition adds and removes tickets around this one
    /// (§5.3), so a position in a list would rename every ticket after a
    /// deletion.
    pub seq: i64,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub acceptance: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
    /// Prerequisites, always inside this same Discovery (§6.2). Work depending
    /// on something outside it is described in the text and sequenced by hand.
    #[serde(default)]
    pub blocked_by: Vec<TicketId>,
    #[serde(default)]
    pub test_command: Option<String>,
    #[serde(default)]
    pub workflow_id: Option<WorkflowId>,
    #[serde(default)]
    pub agent_kind: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<EffortLevel>,
    /// Staged here and committed to the Feature when the Ticket starts (§9.3):
    /// there is no `feature_id` to attach them to until then, so a Ticket that
    /// is never started never writes an attachment row.
    #[serde(default)]
    pub attachments: Vec<AttachedFile>,
    pub state: TicketState,
    /// Why the plan gave this Ticket up (§6.6). The record is the point —
    /// deleting the Ticket would free its dependents just as well and destroy
    /// the evidence that the option was considered.
    #[serde(default)]
    pub drop_reason: Option<String>,
    /// Why this Ticket was started regardless of its edges, and when (§6.5).
    /// The reason reaches the agent in its own prerequisite briefing (§7.2),
    /// which is what stops a bypass from becoming an unexplained one.
    #[serde(default)]
    pub force_start_reason: Option<String>,
    #[serde(default)]
    pub force_started_at: Option<i64>,
    /// The **current** attempt. Superseded ones are kept in
    /// [`TicketFeatureAttempt`] rather than here, because §6.4 needs one
    /// unambiguous Feature to read `mr_state` from.
    #[serde(default)]
    pub feature_id: Option<FeatureId>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What has been done about a Ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketState {
    Unstarted,
    /// Has a Feature, and is therefore immutable to a re-decomposition (§5.3).
    Started,
    /// Given up on, which satisfies dependents exactly as a closed PR does
    /// (§6.4) — one rule rather than two.
    Dropped,
}

impl TicketState {
    /// The stable lowercase identifier used on the wire and in the DB.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unstarted => "unstarted",
            Self::Started => "started",
            Self::Dropped => "dropped",
        }
    }

    /// Parse a stored state. `None` for anything unknown, so a row written by
    /// a newer build degrades rather than panicking — mirrors
    /// [`EffortLevel::parse`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "unstarted" => Some(Self::Unstarted),
            "started" => Some(Self::Started),
            "dropped" => Some(Self::Dropped),
            _ => None,
        }
    }
}

/// One Feature a Ticket has been run as.
///
/// Retries happen inside a Feature (`step_retry`, `replay_from_step`), so a
/// second row here is the cancel-and-restart case, not the normal path (§7.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketFeatureAttempt {
    pub ticket_id: TicketId,
    pub feature_id: FeatureId,
    pub started_at: i64,
    /// `None` while this is the attempt [`Ticket::feature_id`] names.
    #[serde(default)]
    pub superseded_at: Option<i64>,
}

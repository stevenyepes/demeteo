//! Persistence for the planning conversation (V47) and the work it emits.
//!
//! Two traits over one aggregate. [`TicketPort`] is separated from
//! [`DiscoveryPort`] because its readers are not the interview's: the
//! `mr_monitor` poll arrives holding a feature id and nothing else, and the
//! graph and board read tickets without ever touching a transcript. A single
//! trait would hand every one of them the message log as well.
//!
//! Neither trait enforces §8.4's refusal to delete a Discovery whose tickets
//! have Features — [`DiscoveryPort::delete`] does what it says. The check is
//! the caller's, over what [`TicketPort::list_for_discovery`] returns, because
//! it is a policy decision and belongs in `domain/` where a test can reach it
//! without a database (AGENTS.md §3).

use crate::domain::attachment::AttachedFile;
use crate::domain::ids::{DiscoveryId, FeatureId, ProjectId, TicketId, WorkflowId};
use crate::domain::models::{
    Discovery, DiscoveryMessage, DiscoveryStatus, EffortLevel, Ticket, TicketFeatureAttempt,
    TicketState,
};

/// The fields one transition may change on a [`Discovery`].
///
/// Nullable columns take the `Option<Option<T>>` of
/// [`FeaturePatch`](crate::ports::db::FeaturePatch), on the same terms.
///
/// The interviewer choice — machine, agent kind — is absent. Switching either
/// mid-Discovery costs nothing given that the transcript is authoritative
/// (§4.4), but it is not offered in the first cut (§11), and a patch field
/// nothing writes reads as a supported operation.
#[derive(Debug, Default, Clone)]
pub struct DiscoveryPatch {
    pub title: Option<String>,
    pub status: Option<DiscoveryStatus>,
    pub model: Option<Option<String>>,
    pub effort: Option<Option<EffortLevel>>,
    pub resume_session_id: Option<Option<String>>,
    pub worktree_path: Option<Option<String>>,
    /// Folded into the stored totals rather than replacing them, so two turns
    /// that finish out of order still sum to what was spent (§8.5).
    pub add_cost: f64,
    pub add_tokens: i64,
}

pub trait DiscoveryPort: Send + Sync {
    /// A project's Discoveries, most recently touched first. Closed ones are
    /// included: closing is soft and keeps everything (§8.4).
    fn list_for_project(&self, project_id: &ProjectId) -> Result<Vec<Discovery>, String>;
    fn get(&self, id: &DiscoveryId) -> Result<Option<Discovery>, String>;
    fn create(&self, discovery: &Discovery) -> Result<(), String>;
    fn update(&self, id: &DiscoveryId, patch: &DiscoveryPatch, now: i64) -> Result<(), String>;
    /// Take the Discovery, its transcript and its tickets. See the module docs
    /// for what the caller owes §8.4 first.
    fn delete(&self, id: &DiscoveryId) -> Result<(), String>;
    fn append_message(&self, message: &DiscoveryMessage) -> Result<(), String>;
    /// The whole transcript in the order it was said — the authority a turn
    /// re-seeds from when the harness no longer knows the session.
    fn list_messages(&self, id: &DiscoveryId) -> Result<Vec<DiscoveryMessage>, String>;
}

/// The fields one transition may change on a [`Ticket`].
///
/// Nullable columns use `Option<Option<T>>`; the JSON-backed lists use a plain
/// `Option<Vec<_>>`, where `Some(vec![])` is the clear.
#[derive(Debug, Default, Clone)]
pub struct TicketPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub acceptance: Option<Vec<String>>,
    pub files: Option<Vec<String>>,
    pub blocked_by: Option<Vec<TicketId>>,
    pub test_command: Option<Option<String>>,
    pub workflow_id: Option<Option<WorkflowId>>,
    pub agent_kind: Option<Option<String>>,
    pub model: Option<Option<String>>,
    pub effort: Option<Option<EffortLevel>>,
    pub attachments: Option<Vec<AttachedFile>>,
    pub state: Option<TicketState>,
    pub drop_reason: Option<Option<String>>,
    pub force_start_reason: Option<Option<String>>,
    pub force_started_at: Option<Option<i64>>,
    pub feature_id: Option<Option<FeatureId>>,
}

pub trait TicketPort: Send + Sync {
    /// One Discovery's tickets in [`Ticket::seq`] order. This is the whole
    /// graph: §6.2 closes edges over the aggregate, so nothing outside this
    /// list can be pointed at.
    fn list_for_discovery(&self, discovery_id: &DiscoveryId) -> Result<Vec<Ticket>, String>;
    fn get(&self, id: &TicketId) -> Result<Option<Ticket>, String>;
    /// Write a decomposition's set, replacing any row of the same id.
    ///
    /// Additive by omission: a ticket absent from `tickets` is left alone, not
    /// removed. Which of the existing rows a re-decomposition may revise or
    /// delete is §5.3's rule and the caller's diff to apply — a batch that
    /// deleted what it did not mention would take started tickets with it.
    fn upsert_batch(&self, tickets: &[Ticket]) -> Result<(), String>;
    fn update(&self, id: &TicketId, patch: &TicketPatch, now: i64) -> Result<(), String>;
    fn delete(&self, id: &TicketId) -> Result<(), String>;
    /// The number the next Ticket gets: one past the highest this Discovery
    /// currently holds. Not one past the count, which would reissue the number
    /// of any ticket removed from the middle — §5.3 forbids renumbering, so
    /// two tickets sharing a number is two tickets a user cannot tell apart.
    fn next_seq(&self, discovery_id: &DiscoveryId) -> Result<i64, String>;
    /// Which Tickets name this Feature as their current attempt — how the
    /// `mr_monitor` poll gets from a PR transition back to the graph it
    /// unblocks (§6.3).
    ///
    /// A `Vec` because nothing in the schema makes it at most one, and a
    /// caller that assumed otherwise would silently skip a graph.
    fn for_feature(&self, feature_id: &FeatureId) -> Result<Vec<Ticket>, String>;
    /// Record a Feature as this Ticket's attempt. Idempotent on
    /// `(ticket, feature)`.
    fn record_attempt(
        &self,
        ticket_id: &TicketId,
        feature_id: &FeatureId,
        now: i64,
    ) -> Result<(), String>;
    /// Close every attempt still open on this Ticket, which is what makes room
    /// for a new current one.
    fn supersede_attempts(&self, ticket_id: &TicketId, now: i64) -> Result<(), String>;
    /// Every Feature this Ticket has been run as, oldest first. The audit §7.1
    /// asks for is only an audit if something can read it back.
    fn list_attempts(&self, ticket_id: &TicketId) -> Result<Vec<TicketFeatureAttempt>, String>;
}

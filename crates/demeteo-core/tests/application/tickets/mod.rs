// Tests extracted from `crates/demeteo-core/src/application/tickets/mod.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::ids::{DiscoveryId, FeatureId, TicketId};

fn ticket(id: &str, seq: i64) -> Ticket {
    Ticket {
        id: TicketId::from(id.to_string()),
        discovery_id: DiscoveryId::from("d-1".to_string()),
        seq,
        title: format!("ticket {seq}"),
        description: String::new(),
        acceptance: Vec::new(),
        files: Vec::new(),
        blocked_by: Vec::new(),
        test_command: None,
        workflow_id: None,
        agent_kind: None,
        model: None,
        effort: None,
        attachments: Vec::new(),
        state: TicketState::Unstarted,
        drop_reason: None,
        force_start_reason: None,
        force_started_at: None,
        feature_id: None,
        created_at: 0,
        updated_at: 0,
    }
}

/// §6.5 makes the recorded reason the whole content of a force start, so a row
/// carrying only whitespace must not read as one — the bypass would then reach
/// the agent's briefing with nothing to say for itself.
#[test]
fn a_blank_force_reason_is_not_a_force_start() {
    let mut t = ticket("t-1", 1);
    t.force_start_reason = Some("   ".to_string());
    assert!(!node_of(&t, None).force_started);

    t.force_start_reason = Some("no forge remote".to_string());
    assert!(node_of(&t, None).force_started);
}

/// The projection carries `mr_state` through verbatim; the derived layer owns
/// what the word means.
#[test]
fn the_projection_carries_the_forge_state_verbatim() {
    let mut t = ticket("t-1", 1);
    t.state = TicketState::Started;
    t.feature_id = Some(FeatureId::from("f-1".to_string()));
    let node = node_of(&t, Some("closed"));
    assert_eq!(node.state, TicketNodeState::Started);
    assert_eq!(node.mr_state.as_deref(), Some("closed"));
}

/// §8.4: a Ticket with a Feature owns a branch, a worktree and a PR that
/// outlive the plan, so the whole Discovery is held.
#[test]
fn a_discovery_with_a_started_ticket_refuses_deletion() {
    let mut started = ticket("t-2", 2);
    started.feature_id = Some(FeatureId::from("f-1".to_string()));
    let refusal = deletion_refusal(&[ticket("t-1", 1), started])
        .expect("a started ticket should hold the discovery");
    assert!(refusal.contains("#2"), "{refusal}");
    assert!(!refusal.contains("#1"), "{refusal}");
}

#[test]
fn a_discovery_of_unstarted_tickets_may_go() {
    assert!(deletion_refusal(&[ticket("t-1", 1), ticket("t-2", 2)]).is_none());
}

/// The two spellings of "has a Feature" are one answer, and either locks. A
/// row whose `state` this build could not name reads as `started` on purpose;
/// consulting `feature_id` alone would hand it back as editable.
#[test]
fn a_started_row_with_no_feature_still_locks() {
    let mut t = ticket("t-1", 1);
    t.state = TicketState::Started;
    assert!(is_locked(&t));

    let mut t = ticket("t-2", 2);
    t.state = TicketState::Dropped;
    assert!(!is_locked(&t));
}

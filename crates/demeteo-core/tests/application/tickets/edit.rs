// Tests extracted from `crates/demeteo-core/src/application/tickets/edit.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::ids::{DiscoveryId, FeatureId};
use crate::domain::models::TicketState;

fn row(seq: i64, blocked_by: &[&str]) -> Ticket {
    Ticket {
        id: TicketId::from(format!("t-{seq}")),
        discovery_id: DiscoveryId::from("d-1"),
        seq,
        title: format!("ticket {seq}"),
        description: "why".to_string(),
        acceptance: vec!["it works".to_string()],
        files: Vec::new(),
        blocked_by: blocked_by.iter().map(|id| TicketId::from(*id)).collect(),
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

fn edit(blocked_by: &[&str]) -> TicketEdit {
    TicketEdit {
        title: "a title".to_string(),
        description: "a description".to_string(),
        acceptance: vec!["it works".to_string()],
        files: Vec::new(),
        blocked_by: blocked_by.iter().map(|id| (*id).to_string()).collect(),
        test_command: None,
        workflow_id: None,
        agent_kind: None,
        model: None,
        effort: None,
    }
    .normalized()
}

/// §5.4: the lock is the Feature, and nothing else about the row.
#[test]
fn a_started_ticket_refuses_the_edit_and_names_its_number() {
    let mut started = row(2, &[]);
    started.state = TicketState::Started;
    started.feature_id = Some(FeatureId::from("f-1"));
    let refusal = refusal(&started, &[row(1, &[]), started.clone()], &edit(&[]))
        .expect("a started ticket is locked");
    assert!(refusal.contains("#2"), "{refusal}");
}

/// §5.3 draws immutability at *has a Feature*, and a dropped ticket has none —
/// `diff_proposal` lets a re-decomposition revise one on the same reading.
#[test]
fn a_dropped_ticket_is_still_editable() {
    let mut dropped = row(1, &[]);
    dropped.state = TicketState::Dropped;
    dropped.drop_reason = Some("the plan moved on".to_string());
    assert_eq!(refusal(&dropped, &[dropped.clone()], &edit(&[])), None);
}

/// The hazard a per-edge check cannot see: neither ticket is on a cycle until
/// this edge exists, so the whole resulting set is what gets validated.
#[test]
fn an_edge_that_closes_a_cycle_is_refused_and_names_both_by_number() {
    let one = row(1, &[]);
    let two = row(2, &["t-1"]);
    let refusal = refusal(&one, &[one.clone(), two], &edit(&["t-2"]))
        .expect("1 waiting on 2 waiting on 1 is a cycle");
    assert!(refusal.contains("cycle"), "{refusal}");
    assert!(
        refusal.contains("#1") && refusal.contains("#2"),
        "{refusal}"
    );
}

/// §6.2 closes the graph over one Discovery. The stranger keeps its stored id
/// in the message, having no number in this plan to be named by.
#[test]
fn an_edge_out_of_the_discovery_is_refused() {
    let one = row(1, &[]);
    let refusal = refusal(&one, std::slice::from_ref(&one), &edit(&["t-99"]))
        .expect("an edge outside the discovery is not an edge");
    assert!(refusal.contains("t-99"), "{refusal}");
}

#[test]
fn a_ticket_may_not_wait_on_itself() {
    let one = row(1, &[]);
    assert!(refusal(&one, std::slice::from_ref(&one), &edit(&["t-1"])).is_some());
}

/// The title is the run's name and the plan's only handle on the ticket.
#[test]
fn an_emptied_title_is_refused() {
    let one = row(1, &[]);
    let mut blanked = edit(&[]);
    blanked.title = "   ".to_string();
    assert!(refusal(&one, std::slice::from_ref(&one), &blanked).is_some());
}

/// What a form leaves behind: half-typed rows, and selects the user set back
/// to the inherit option. An empty string in a nullable column would read as a
/// choice everywhere downstream.
#[test]
fn blank_rows_and_blank_selects_do_not_reach_the_row() {
    let normalized = TicketEdit {
        title: "  a title  ".to_string(),
        description: "a description".to_string(),
        acceptance: vec!["it works".to_string(), "   ".to_string()],
        files: vec!["  src/a.rs".to_string(), String::new()],
        blocked_by: vec!["t-2".to_string(), " t-2 ".to_string(), String::new()],
        test_command: Some("  ".to_string()),
        workflow_id: Some(String::new()),
        agent_kind: Some(" claude-code ".to_string()),
        model: None,
        effort: None,
    }
    .normalized();

    assert_eq!(normalized.title, "a title");
    assert_eq!(normalized.acceptance, vec!["it works".to_string()]);
    assert_eq!(normalized.files, vec!["src/a.rs".to_string()]);
    assert_eq!(normalized.blocked_by, vec!["t-2".to_string()]);
    assert_eq!(normalized.test_command, None);
    assert_eq!(normalized.workflow_id, None);
    assert_eq!(normalized.agent_kind.as_deref(), Some("claude-code"));
}

/// Clearing a column is `Some(None)`, never the absence [`TicketPatch`]
/// reserves for *leave alone* — the whole reason the wire carries the whole
/// form.
#[test]
fn clearing_a_field_writes_a_null_rather_than_leaving_it_alone() {
    let patch = edit(&[]).patch();
    assert!(matches!(patch.model, Some(None)));
    assert!(matches!(patch.workflow_id, Some(None)));
    assert!(matches!(patch.effort, Some(None)));
    assert!(patch.state.is_none());
    assert!(patch.drop_reason.is_none());
    assert!(patch.feature_id.is_none());
}

// Tests extracted from `crates/demeteo-core/src/application/tickets/launch.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;
use crate::application::tickets::node_of;
use crate::domain::ids::{DiscoveryId, TicketId};
use crate::domain::ticket_graph::derive_board;

fn ticket(id: &str, seq: i64, title: &str) -> Ticket {
    Ticket {
        id: TicketId::from(id.to_string()),
        discovery_id: DiscoveryId::from("d-1".to_string()),
        seq,
        title: title.to_string(),
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

fn refusal_for(subject_index: usize, tickets: &[Ticket]) -> Option<String> {
    let nodes: Vec<_> = tickets
        .iter()
        .map(|t| {
            let state = match t.state {
                TicketState::Started => Some("open"),
                _ => None,
            };
            node_of(t, state)
        })
        .collect();
    let board = derive_board(&nodes);
    start_refusal(
        &tickets[subject_index],
        &board.standings[subject_index],
        tickets,
    )
}

/// §7.1/§11: Demeteo shows what is startable and starts nothing itself, so the
/// refusal is the only thing standing between an unmet edge and a run cut from
/// a base branch that lacks the prerequisite's code.
#[test]
fn an_unmet_edge_refuses_the_start_and_names_the_blocker() {
    let mut dependent = ticket("t-2", 2, "the multiplexer");
    dependent.blocked_by = vec![TicketId::from("t-1".to_string())];
    let tickets = vec![ticket("t-1", 1, "the registry"), dependent];

    let refusal = refusal_for(1, &tickets).expect("an outstanding edge must refuse");
    assert!(refusal.contains("#2"), "{refusal}");
    assert!(refusal.contains("#1 \"the registry\""), "{refusal}");
    assert!(refusal.contains("force start"), "{refusal}");
}

/// §6.5's hatch is per ticket, so it clears every edge at once — including in
/// the case it exists for, a project with no forge where no edge will ever be
/// satisfied on its own.
#[test]
fn a_recorded_reason_clears_every_edge_at_once() {
    let mut dependent = ticket("t-3", 3, "conformance");
    dependent.blocked_by = vec![
        TicketId::from("t-1".to_string()),
        TicketId::from("t-2".to_string()),
    ];
    dependent.force_start_reason = Some("this project has no forge remote".to_string());
    let tickets = vec![
        ticket("t-1", 1, "the registry"),
        ticket("t-2", 2, "the keypair"),
        dependent,
    ];

    assert_eq!(refusal_for(2, &tickets), None);
}

#[test]
fn a_dropped_ticket_and_a_started_one_are_refused_for_their_own_reasons() {
    let mut dropped = ticket("t-1", 1, "the guide");
    dropped.state = TicketState::Dropped;
    let mut started = ticket("t-2", 2, "the registry");
    started.state = TicketState::Started;
    let tickets = vec![dropped, started];

    let d = refusal_for(0, &tickets).expect("a dropped ticket has nothing to start");
    assert!(d.contains("dropped"), "{d}");
    let s = refusal_for(1, &tickets).expect("a started ticket is already running");
    assert!(s.contains("already been started"), "{s}");
}

/// An unknown edge target is drift, not a plan (`BlockerReason::Unknown`), and
/// the refusal must not present it as a ticket the user could go and finish.
#[test]
fn an_edge_naming_nothing_refuses_and_says_so() {
    let mut dependent = ticket("t-2", 2, "the multiplexer");
    dependent.blocked_by = vec![TicketId::from("t-gone".to_string())];
    let tickets = vec![dependent];

    let refusal = refusal_for(0, &tickets).expect("a dangling edge blocks");
    assert!(
        refusal.contains("'t-gone' (not a ticket in this plan)"),
        "{refusal}"
    );
}

/// §7.2: the briefing rides in the launched prompt itself, not beside it.
#[test]
fn the_launch_description_carries_the_briefing_and_the_ticket_fields() {
    let mut t = ticket("t-1", 1, "the registry");
    t.description = "Key sessions by client id.".to_string();
    t.acceptance = vec!["two clients share one runner".to_string()];
    t.files = vec!["crates/demeteo-runner/src/session.rs".to_string()];
    t.test_command = Some("npm run checks:code".to_string());

    let body = launch_description(&t, "#0 \"the port\" landed.");
    assert!(body.starts_with("Key sessions by client id."), "{body}");
    assert!(
        body.contains("## Acceptance criteria\n- two clients share one runner"),
        "{body}"
    );
    assert!(
        body.contains("`crates/demeteo-runner/src/session.rs`"),
        "{body}"
    );
    assert!(body.contains("`npm run checks:code`"), "{body}");
    assert!(
        body.trim_end().ends_with("#0 \"the port\" landed."),
        "{body}"
    );
}

/// A ticket whose description was never filled in still needs a prompt body —
/// `FeatureLaunch` refuses an empty one.
#[test]
fn an_empty_description_falls_back_to_the_title() {
    let t = ticket("t-1", 1, "the registry");
    assert!(launch_description(&t, "none").starts_with("the registry"));
}

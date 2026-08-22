// The derived ticket layer. `super` = `domain::ticket_graph`.
//
// No port doubles anywhere below, and that is the point of the projection the
// module takes: every rule the PRD settles is reachable from a plain value.

use super::*;

fn node(id: &str, state: TicketNodeState, blocked_by: &[&str]) -> TicketNode {
    TicketNode {
        id: id.to_string(),
        state,
        blocked_by: blocked_by.iter().map(|s| s.to_string()).collect(),
        mr_state: None,
        force_started: false,
    }
}

fn started(id: &str, mr_state: &str, blocked_by: &[&str]) -> TicketNode {
    TicketNode {
        mr_state: Some(mr_state.to_string()),
        ..node(id, TicketNodeState::Started, blocked_by)
    }
}

fn lane_of(board: &TicketBoard, id: &str) -> TicketLane {
    board
        .standings
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("no standing for '{id}'"))
        .lane
}

fn standing<'a>(board: &'a TicketBoard, id: &str) -> &'a TicketStanding {
    board
        .standings
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("no standing for '{id}'"))
}

fn proposed(id: &str, body: &str, blocked_by: &[&str]) -> ProposedTicket<String> {
    ProposedTicket {
        id: id.to_string(),
        blocked_by: blocked_by.iter().map(|s| s.to_string()).collect(),
        body: body.to_string(),
    }
}

fn stored(
    id: &str,
    body: &str,
    blocked_by: &[&str],
    state: TicketNodeState,
) -> CurrentTicket<String> {
    CurrentTicket {
        id: id.to_string(),
        state,
        blocked_by: blocked_by.iter().map(|s| s.to_string()).collect(),
        body: body.to_string(),
    }
}

#[test]
fn only_a_terminal_forge_state_or_a_drop_releases_dependents() {
    // Forge state is the authority: a run that reported success without
    // merging leaves nothing in the base branch for a dependent to build on.
    assert!(releases_dependents(&started("a", "merged", &[])));
    assert!(releases_dependents(&started("a", "closed", &[])));
    assert!(releases_dependents(&node(
        "a",
        TicketNodeState::Dropped,
        &[]
    )));

    assert!(!releases_dependents(&started("a", "open", &[])));
    assert!(!releases_dependents(&started("a", "draft", &[])));
    assert!(!releases_dependents(&started("a", "none", &[])));
    assert!(!releases_dependents(&node(
        "a",
        TicketNodeState::Started,
        &[]
    )));
    assert!(!releases_dependents(&node(
        "a",
        TicketNodeState::Unstarted,
        &[]
    )));
}

#[test]
fn a_dependency_with_no_pr_keeps_its_dependent_blocked() {
    let nodes = vec![
        node("a", TicketNodeState::Started, &[]),
        node("b", TicketNodeState::Unstarted, &["a"]),
    ];
    let board = derive_board(&nodes);

    assert_eq!(lane_of(&board, "b"), TicketLane::Blocked);
    assert!(!standing(&board, "b").startable);
    assert_eq!(
        standing(&board, "b").blockers,
        vec![Blocker {
            id: "a".to_string(),
            reason: BlockerReason::Outstanding,
        }]
    );
}

#[test]
fn a_dangling_edge_blocks_and_says_so() {
    let nodes = vec![node("b", TicketNodeState::Unstarted, &["ghost"])];
    let board = derive_board(&nodes);

    assert_eq!(lane_of(&board, "b"), TicketLane::Blocked);
    assert_eq!(
        standing(&board, "b").blockers,
        vec![Blocker {
            id: "ghost".to_string(),
            reason: BlockerReason::Unknown,
        }]
    );
}

#[test]
fn a_force_start_starts_a_ticket_its_edges_still_hold() {
    let nodes = vec![
        node("a", TicketNodeState::Unstarted, &[]),
        TicketNode {
            force_started: true,
            ..node("b", TicketNodeState::Unstarted, &["a", "ghost"])
        },
    ];
    let board = derive_board(&nodes);

    assert!(standing(&board, "b").startable);
    assert_eq!(lane_of(&board, "b"), TicketLane::Ready);
    // The waived edges stay on the record rather than disappearing with the
    // override — §6.5, and the input §7.2 renders.
    assert_eq!(standing(&board, "b").blockers.len(), 2);
}

#[test]
fn a_diamond_releases_its_join_only_once_both_sides_are_terminal() {
    let join = |left: &str, right: &str| {
        vec![
            node("root", TicketNodeState::Dropped, &[]),
            started("left", left, &["root"]),
            started("right", right, &["root"]),
            node("join", TicketNodeState::Unstarted, &["left", "right"]),
        ]
    };

    let board = derive_board(&join("merged", "open"));
    assert_eq!(lane_of(&board, "join"), TicketLane::Blocked);
    assert_eq!(standing(&board, "join").blockers.len(), 1);

    let board = derive_board(&join("merged", "closed"));
    assert_eq!(lane_of(&board, "join"), TicketLane::Ready);
    assert!(standing(&board, "join").blockers.is_empty());
}

#[test]
fn a_closed_pr_is_not_landed_and_is_not_live() {
    let nodes = vec![
        started("merged", "merged", &[]),
        started("closed", "closed", &[]),
        started("open", "open", &[]),
        node("dropped", TicketNodeState::Dropped, &[]),
        node("waiting", TicketNodeState::Unstarted, &["open"]),
        node("free", TicketNodeState::Unstarted, &[]),
    ];
    let board = derive_board(&nodes);

    assert_eq!(lane_of(&board, "closed"), TicketLane::Dropped);
    assert_eq!(
        board.progress,
        TicketProgress {
            blocked: 1,
            ready: 1,
            in_flight: 1,
            landed: 1,
            dropped: 2,
            live: 4,
        }
    );
}

#[test]
fn the_briefing_separates_the_three_ways_a_prerequisite_can_be_satisfied() {
    let ticket = node(
        "ticket",
        TicketNodeState::Started,
        &["merged", "closed", "dropped", "open", "ghost"],
    );
    let nodes = vec![
        started("merged", "merged", &[]),
        started("closed", "closed", &[]),
        node("dropped", TicketNodeState::Dropped, &[]),
        started("open", "open", &[]),
        ticket.clone(),
    ];

    let outcomes: Vec<PrerequisiteOutcome> = prerequisite_briefing(&ticket, &nodes)
        .into_iter()
        .map(|note| note.outcome)
        .collect();

    // Closed and dropped both satisfy, and neither put code in the base
    // branch. An agent told only "satisfied" would build on nothing.
    assert_eq!(
        outcomes,
        vec![
            PrerequisiteOutcome::Merged,
            PrerequisiteOutcome::ClosedUnmerged,
            PrerequisiteOutcome::Dropped,
            PrerequisiteOutcome::Outstanding,
            PrerequisiteOutcome::Unknown,
        ]
    );
}

#[test]
fn a_cycle_is_rejected_naming_every_ticket_in_it() {
    let plan = vec![
        proposed("a", "", &["c"]),
        proposed("b", "", &["a"]),
        proposed("c", "", &["b"]),
        proposed("free", "", &[]),
    ];
    let reason = validate_ticket_graph(&plan).expect("a three-ticket loop must be rejected");

    assert!(reason.contains("cycle"), "{reason}");
    for id in ["a", "b", "c"] {
        assert!(reason.contains(id), "cycle message omits '{id}': {reason}");
    }
    assert!(!reason.contains("free"), "{reason}");
}

#[test]
fn a_self_edge_is_named_as_itself_not_as_a_cycle() {
    let reason = validate_ticket_graph(&[proposed("a", "", &["a"])])
        .expect("a ticket may not wait on itself");
    assert!(reason.contains("itself"), "{reason}");
}

#[test]
fn an_edge_out_of_the_set_is_rejected_at_authoring_time() {
    let plan = vec![proposed("a", "", &[]), proposed("b", "", &["elsewhere"])];
    let reason = validate_ticket_graph(&plan).expect("edges are closed over one discovery");
    assert!(reason.contains("elsewhere"), "{reason}");
}

#[test]
fn duplicate_and_empty_ids_are_rejected() {
    assert!(validate_ticket_graph(&[proposed("a", "", &[]), proposed("a", "x", &[])]).is_some());
    assert!(validate_ticket_graph(&[proposed("  ", "", &[])]).is_some());
}

#[test]
fn a_valid_diamond_passes() {
    let plan = vec![
        proposed("root", "", &[]),
        proposed("left", "", &["root"]),
        proposed("right", "", &["root"]),
        proposed("join", "", &["left", "right"]),
    ];
    assert_eq!(validate_ticket_graph(&plan), None);
}

#[test]
fn the_diff_is_additive_over_an_unchanged_started_ticket() {
    let current = vec![
        stored("a", "first", &[], TicketNodeState::Started),
        stored("b", "second", &["a"], TicketNodeState::Unstarted),
        stored("c", "third", &[], TicketNodeState::Unstarted),
    ];
    let proposal = vec![
        // Reissued verbatim, with its edges reordered: not a revision.
        proposed("a", "first", &[]),
        proposed("b", "second rewritten", &["a"]),
        proposed("d", "new", &["b"]),
    ];

    let diff = diff_proposal(&current, &proposal).expect("no started ticket is touched");

    assert_eq!(diff.unchanged, vec!["a".to_string()]);
    assert_eq!(diff.revised, vec!["b".to_string()]);
    assert_eq!(diff.added, vec!["d".to_string()]);
    assert_eq!(diff.removed, vec!["c".to_string()]);
}

#[test]
fn reordering_edges_is_not_a_revision() {
    let current = vec![stored(
        "join",
        "body",
        &["left", "right"],
        TicketNodeState::Unstarted,
    )];
    let proposal = vec![proposed("join", "body", &["right", "left"])];

    let diff = diff_proposal(&current, &proposal).expect("edges are a set");
    assert_eq!(diff.unchanged, vec!["join".to_string()]);
    assert!(diff.revised.is_empty());
}

#[test]
fn revising_or_removing_a_started_ticket_is_a_rejection_with_a_reason() {
    let current = vec![
        stored("a", "first", &[], TicketNodeState::Started),
        stored("b", "second", &[], TicketNodeState::Started),
    ];
    let proposal = vec![proposed("a", "rewritten", &[])];

    let violations = diff_proposal(&current, &proposal).expect_err("started tickets are immutable");

    assert_eq!(
        violations
            .iter()
            .map(|v| (v.id.as_str(), v.change))
            .collect::<Vec<_>>(),
        vec![
            ("a", ImmutableChange::Revised),
            ("b", ImmutableChange::Removed)
        ]
    );
    // A rejection carries the reason the proposed-changes view renders; a
    // silent skip would apply half a plan the user never agreed to.
    assert!(violations.iter().all(|v| !v.reason.is_empty()));
}

#[test]
fn a_dropped_ticket_is_not_immutable() {
    let current = vec![stored("a", "first", &[], TicketNodeState::Dropped)];
    let proposal: Vec<ProposedTicket<String>> = Vec::new();
    let diff = diff_proposal(&current, &proposal).expect("only a Feature locks a ticket");
    assert_eq!(diff.removed, vec!["a".to_string()]);
}

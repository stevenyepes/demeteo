// Tests extracted from `crates/demeteo-core/src/application/tickets/briefing.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;
use crate::application::tickets::node_of;
use crate::domain::attachment::AttachedFile;
use crate::domain::ids::{DiscoveryId, TicketId};

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

fn image(name: &str) -> AttachedFile {
    AttachedFile {
        id: format!("at-{name}"),
        name: name.to_string(),
        mime: "image/png".to_string(),
        sha256: "a".repeat(64),
        size: 12,
        source_filename: name.to_string(),
    }
}

/// Every prerequisite state, one subject, so the four sentences are compared
/// against each other rather than each against itself.
fn plan() -> (Vec<Ticket>, Vec<TicketNode>) {
    let mut merged = ticket("t-1", 1, "the registry");
    merged.state = TicketState::Started;
    let mut closed = ticket("t-2", 2, "the keypair");
    closed.state = TicketState::Started;
    let mut dropped = ticket("t-3", 3, "the operator guide");
    dropped.state = TicketState::Dropped;
    dropped.drop_reason = Some("folded into RUNNER_DEV.md".to_string());
    let unstarted = ticket("t-4", 4, "fair-share scheduling");
    let mut running = ticket("t-5", 5, "the multiplexer");
    running.state = TicketState::Started;

    let mut subject = ticket("t-9", 9, "topology conformance");
    subject.blocked_by = vec![
        TicketId::from("t-1".to_string()),
        TicketId::from("t-2".to_string()),
        TicketId::from("t-3".to_string()),
        TicketId::from("t-4".to_string()),
        TicketId::from("t-5".to_string()),
        TicketId::from("t-gone".to_string()),
    ];

    let tickets = vec![merged, closed, dropped, unstarted, running, subject];
    let nodes = vec![
        node_of(&tickets[0], Some("merged")),
        node_of(&tickets[1], Some("closed")),
        node_of(&tickets[2], None),
        node_of(&tickets[3], None),
        node_of(&tickets[4], Some("open")),
        node_of(&tickets[5], None),
    ];
    (tickets, nodes)
}

/// §7.2's whole point: an agent told nothing assumes the prerequisite's code
/// is in its base branch. Merged is the only outcome allowed to say it is.
#[test]
fn only_a_merged_prerequisite_claims_the_base_branch() {
    let (tickets, nodes) = plan();
    let text = compose(&tickets[5], &tickets, &nodes);
    let lines: Vec<&str> = text.lines().collect();

    assert_eq!(lines.len(), 6, "{text}");
    assert_eq!(
        lines[0],
        "#1 \"the registry\" landed. Its PR merged, so its code is in your base branch."
    );
    assert!(lines[1].contains("closed without merging"), "{}", lines[1]);
    assert!(lines[1].contains("none of its work"), "{}", lines[1]);
    assert!(lines[2].contains("dropped from the plan"), "{}", lines[2]);
    assert!(
        lines[2].contains("folded into RUNNER_DEV.md"),
        "{}",
        lines[2]
    );
    assert!(lines[3].contains("has not started"), "{}", lines[3]);
    assert!(lines[4].contains("still running"), "{}", lines[4]);
    assert!(lines[5].contains("not in this plan"), "{}", lines[5]);

    // The negatives are worded so that no substring of them reads as the
    // positive: "not in your base branch" one careless skim from "in your base
    // branch" is the failure §7.2 exists to prevent, not a form of it.
    let claims: Vec<&&str> = lines
        .iter()
        .filter(|l| l.contains("so its code is in your base branch"))
        .collect();
    assert_eq!(claims.len(), 1, "{text}");
}

/// A dropped prerequisite and one that never started are both "not there", but
/// an open PR is readable and an absent ticket is not — the agent needs the
/// difference, and `PrerequisiteOutcome::Outstanding` does not carry it.
#[test]
fn an_open_pr_reads_differently_from_work_that_never_started() {
    let (tickets, nodes) = plan();
    let text = compose(&tickets[5], &tickets, &nodes);
    assert!(text.contains("#4 \"fair-share scheduling\" has not landed. It has not started"));
    assert!(text.contains("#5 \"the multiplexer\" has not landed. It is still running"));
}

#[test]
fn a_ticket_with_no_edges_says_so() {
    let subject = ticket("t-1", 1, "the registry");
    let nodes = vec![node_of(&subject, None)];
    assert_eq!(
        compose(&subject, std::slice::from_ref(&subject), &nodes),
        "No prerequisites in this discovery."
    );
}

/// §9.3 routes a Ticket's attachments through the placeholder the agent
/// already resolves, rather than a second spelling of the same idea.
#[test]
fn attachments_are_named_in_the_spelling_the_prompt_resolves() {
    let mut subject = ticket("t-1", 1, "the registry");
    subject.attachments = vec![image("runner-topology.png")];
    subject.agent_kind = Some("claude-code".to_string());
    subject.model = Some("opus".to_string());
    let nodes = vec![node_of(&subject, None)];

    let text = compose(&subject, std::slice::from_ref(&subject), &nodes);
    assert!(
        text.contains("Attached: [attachment -- runner-topology.png]"),
        "{text}"
    );
    assert!(!text.contains("does not read images"), "{text}");
}

#[test]
fn a_blind_model_is_told_the_image_is_only_a_path() {
    let mut subject = ticket("t-1", 1, "the registry");
    subject.attachments = vec![image("runner-topology.png")];
    subject.agent_kind = Some("opencode".to_string());
    subject.model = Some("text-embedding-ada-002".to_string());
    let nodes = vec![node_of(&subject, None)];

    assert!(
        compose(&subject, std::slice::from_ref(&subject), &nodes).contains("does not read images")
    );
}

/// §6.5: the reason is the thing that stops a bypass being unexplained,
/// "including for the agent, which reads its own prerequisite list".
#[test]
fn a_force_started_ticket_carries_its_reason_and_names_what_it_bypassed() {
    let (tickets, nodes) = plan();
    let mut subject = tickets[5].clone();
    subject.force_start_reason = Some("no forge remote; t-4 merged out of band".to_string());
    let nodes: Vec<TicketNode> = nodes
        .iter()
        .cloned()
        .map(|n| {
            if n.id == subject.id.0 {
                node_of(&subject, None)
            } else {
                n
            }
        })
        .collect();

    let text = compose(&subject, &tickets, &nodes);
    assert!(
        text.contains("\"no forge remote; t-4 merged out of band\""),
        "{text}"
    );
    assert!(text.contains("started before"), "{text}");
    assert!(text.contains("#4 \"fair-share scheduling\""), "{text}");
    assert!(
        !text.contains("#1 \"the registry\" and"),
        "a landed prerequisite was not bypassed: {text}"
    );
}

#[test]
fn a_dropped_ticket_says_it_was_dropped_rather_than_listing_edges() {
    let (tickets, nodes) = plan();
    let mut subject = tickets[5].clone();
    subject.state = TicketState::Dropped;
    subject.drop_reason = Some("the plan moved on".to_string());
    let text = compose(&subject, &tickets, &nodes);
    assert!(
        text.starts_with("Not started. This ticket was dropped."),
        "{text}"
    );
    assert!(text.contains("the plan moved on"), "{text}");
    assert!(!text.contains("base branch"), "{text}");
}

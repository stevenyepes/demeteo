// Tests extracted from `crates/demeteo-core/src/domain/ticket_plan.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::ticket_graph::{diff_proposal, TicketNodeState};

fn planned(id: &str) -> PlannedTicket {
    PlannedTicket {
        id: id.to_string(),
        title: format!("do {id}"),
        description: "because the conversation settled it".to_string(),
        acceptance: vec!["`npm run checks` exits 0".to_string()],
        files: Vec::new(),
        test_command: None,
        blocked_by: Vec::new(),
        workflow: None,
        agent: None,
        model: None,
        effort: None,
        why: None,
    }
}

fn plan(tickets: Vec<PlannedTicket>) -> TicketPlan {
    TicketPlan { tickets }
}

fn body(title: &str) -> TicketBody {
    TicketBody {
        title: title.to_string(),
        description: String::new(),
        acceptance: Vec::new(),
        files: Vec::new(),
        test_command: None,
        workflow_id: None,
        agent_kind: None,
        model: None,
        effort: None,
    }
}

fn current(id: &str, state: TicketNodeState) -> CurrentTicket<TicketBody> {
    CurrentTicket {
        id: id.to_string(),
        state,
        blocked_by: Vec::new(),
        body: body(id),
    }
}

fn proposal(id: &str, blocked_by: &[&str]) -> ProposedTicket<TicketBody> {
    ProposedTicket {
        id: id.to_string(),
        blocked_by: blocked_by.iter().map(|s| s.to_string()).collect(),
        body: body(id),
    }
}

/// The block is a declared artifact, and a harness asked for one puts it
/// wherever it likes. All three shapes have to reach the same plan or the
/// refusal the user sees is about the fence, not about the graph.
#[test]
fn the_plan_is_read_out_of_prose_a_fence_or_the_bare_object() {
    let bare = r#"{"tickets": [{"id": "a", "title": "A"}]}"#;
    let fenced = format!("Here it is.\n\n```json\n{bare}\n```\n");
    let loose = format!("Here it is: {bare} — that is the plan.");
    for text in [bare.to_string(), fenced, loose] {
        let read = extract_ticket_plan(&text).expect("the plan should be readable");
        assert_eq!(read.tickets.len(), 1, "{text}");
        assert_eq!(read.tickets[0].id, "a");
    }
}

/// `tickets` is required rather than defaulted precisely so the tolerant
/// search can tell this object from the interview question block that shares
/// the transcript with it.
#[test]
fn an_object_that_is_not_a_plan_is_not_read_as_an_empty_one() {
    assert!(extract_ticket_plan(r#"{"question": {"header": "Identity"}}"#).is_none());
}

/// §5.2: nothing invalid reaches a ticket row, and the message is what the
/// agent is re-asked with — so it has to name the ticket at fault.
#[test]
fn a_cycle_is_refused_and_names_the_tickets_on_it() {
    let mut a = planned("a");
    a.blocked_by = vec!["b".to_string()];
    let mut b = planned("b");
    b.blocked_by = vec!["a".to_string()];
    let reason = validate_ticket_plan(&plan(vec![a, b])).expect("a cycle must be refused");
    assert!(reason.contains("cycle"), "{reason}");
    assert!(reason.contains('a') && reason.contains('b'), "{reason}");
}

/// The acceptance criteria are what the ticket's own agent is later held to,
/// so a ticket without them is a run that cannot know it is finished.
#[test]
fn a_ticket_with_no_acceptance_criteria_is_refused() {
    let mut bare = planned("a");
    bare.acceptance = vec!["   ".to_string()];
    let reason = validate_ticket_plan(&plan(vec![bare])).expect("must be refused");
    assert!(reason.contains("acceptance"), "{reason}");

    assert!(validate_ticket_plan(&plan(vec![planned("a")])).is_none());
}

#[test]
fn a_ticket_with_no_description_is_refused() {
    let mut bare = planned("a");
    bare.description = String::new();
    let reason = validate_ticket_plan(&plan(vec![bare])).expect("must be refused");
    assert!(reason.contains("description"), "{reason}");
}

#[test]
fn an_empty_plan_is_refused() {
    assert!(validate_ticket_plan(&plan(Vec::new())).is_some());
}

/// The prompt and the "could not read a ticket list" refusal both quote the
/// same example, so an added field can never be asked for in one and omitted
/// from the other.
#[test]
fn the_shape_example_parses_as_a_plan() {
    let example = ticket_plan_json_shape_example();
    let read = extract_ticket_plan(&example).expect("the example must be a plan");
    assert_eq!(read.tickets.len(), 1);
    assert!(!read.tickets[0].id.trim().is_empty());
}

/// `blocked_by` is a set — reordering it changes nothing about the graph, so
/// it must not read as a revision in a view whose whole job is to show what
/// changed.
#[test]
fn reordered_edges_are_not_a_field_change() {
    let one = body("t");
    let changes = field_changes(
        &one,
        &one,
        &["a".to_string(), "b".to_string()],
        &["b".to_string(), " a ".to_string()],
    );
    assert!(changes.is_empty(), "{changes:?}");
}

#[test]
fn a_changed_field_reports_both_sides() {
    let was = body("old");
    let now = body("new");
    let changes = field_changes(&was, &now, &[], &[]);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].field, "title");
    assert_eq!(changes[0].was, "old");
    assert_eq!(changes[0].now, "new");
}

/// The heart of the partial apply: a change nobody checked leaves its stored
/// row exactly as it was, including one the proposal wanted gone.
#[test]
fn an_unaccepted_change_leaves_the_stored_row_alone() {
    let stored = vec![
        current("keep", TicketNodeState::Unstarted),
        current("go", TicketNodeState::Unstarted),
    ];
    let mut revised = proposal("keep", &[]);
    revised.body.title = "keep, reworded".to_string();
    let proposed = vec![revised, proposal("new", &[])];
    let diff = diff_proposal(&stored, &proposed).expect("no started ticket is touched");

    let none = plan_application(&stored, &proposed, &diff, &[]);
    assert!(none.added.is_empty() && none.revised.is_empty() && none.removed.is_empty());
    let ids: Vec<&str> = none.resulting.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["keep", "go"]);
    assert_eq!(none.resulting[0].body.title, "keep");
}

/// A subset of a valid proposal is not itself valid: declining the new ticket
/// another new one waits on leaves an edge pointing at nothing. Only the
/// resulting set can show it, which is why `plan_application` computes one.
#[test]
fn declining_a_prerequisite_addition_leaves_a_graph_that_refuses() {
    let stored = vec![current("keep", TicketNodeState::Unstarted)];
    let proposed = vec![
        proposal("keep", &[]),
        proposal("first", &[]),
        proposal("second", &["first"]),
    ];
    let diff = diff_proposal(&stored, &proposed).expect("nothing started");

    let both = plan_application(
        &stored,
        &proposed,
        &diff,
        &["first".to_string(), "second".to_string()],
    );
    assert!(validate_ticket_graph(&both.resulting).is_none());

    let partial = plan_application(&stored, &proposed, &diff, &["second".to_string()]);
    let reason = validate_ticket_graph(&partial.resulting)
        .expect("an edge to a declined addition must refuse");
    assert!(reason.contains("first"), "{reason}");
}

/// The mirror case from the other side: keeping a ticket while accepting the
/// removal of what it waits on.
#[test]
fn accepting_only_a_removal_that_something_still_waits_on_refuses() {
    let mut dependent = current("dependent", TicketNodeState::Unstarted);
    dependent.blocked_by = vec!["prerequisite".to_string()];
    let stored = vec![
        current("prerequisite", TicketNodeState::Unstarted),
        dependent,
    ];
    let proposed = vec![proposal("dependent", &[])];
    let diff = diff_proposal(&stored, &proposed).expect("nothing started");
    assert_eq!(diff.removed, ["prerequisite"]);
    assert_eq!(diff.revised, ["dependent"]);

    let removal_only = plan_application(&stored, &proposed, &diff, &["prerequisite".to_string()]);
    let reason =
        validate_ticket_graph(&removal_only.resulting).expect("the surviving edge must refuse");
    assert!(reason.contains("prerequisite"), "{reason}");

    let both = plan_application(
        &stored,
        &proposed,
        &diff,
        &["prerequisite".to_string(), "dependent".to_string()],
    );
    assert!(validate_ticket_graph(&both.resulting).is_none());
    assert_eq!(both.removed, ["prerequisite"]);
    assert_eq!(both.revised.len(), 1);
}

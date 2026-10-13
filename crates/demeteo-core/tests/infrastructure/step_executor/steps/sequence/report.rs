// Tests for the sequence step's report persistence. `super` = the `report`
// module. A real filesystem store over a temp dir: the round trip through
// `put` / `list_for_step` / `get` is what is under test.

use super::*;
use crate::adapters::artifact_store::fs::FsArtifactStore;
use crate::domain::sequence::report::{ObservedCommand, ObservedStatus};

fn temp_store() -> FsArtifactStore {
    let dir = crate::support::test_dir::scratch("demeteo_impl_report");
    FsArtifactStore::new(dir)
}

fn fragment(id: &str, at: i64, cmd: &str) -> TicketReport {
    TicketReport::new(
        id,
        &format!("Ticket {id}"),
        "AC1: MET",
        vec![ObservedCommand {
            command: cmd.into(),
            status: ObservedStatus::Completed,
        }],
        at,
    )
}

#[test]
fn no_fragment_means_no_report() {
    let store = temp_store();
    assert_eq!(
        store_implementation_report(&store, "f", "s-implement"),
        None
    );
}

#[test]
fn every_fragment_the_step_holds_lands_in_one_report() {
    let store = temp_store();
    record_ticket_report(&store, "f", "s-implement", &fragment("b", 2, "npm test"));
    record_ticket_report(&store, "f", "s-implement", &fragment("a", 1, "cargo test"));
    record_ticket_report(&store, "f", "s-other", &fragment("c", 3, "make"));

    let reference = store_implementation_report(&store, "f", "s-implement").unwrap();
    assert!(
        reference.ends_with("implementation-report.md"),
        "{reference}"
    );
    let body = store.get(&reference).unwrap();
    assert!(
        body.find("`a`").unwrap() < body.find("`b`").unwrap(),
        "{body}"
    );
    assert!(body.contains("`cargo test` — completed"), "{body}");
    assert!(
        !body.contains("`make`"),
        "another step's fragment leaked in:\n{body}"
    );
}

/// A ticket re-run under a later attempt replaces its own fragment, so the
/// report never shows both the rolled-back claim and the landed one.
#[test]
fn a_rerun_ticket_replaces_its_fragment() {
    let store = temp_store();
    record_ticket_report(&store, "f", "s-implement", &fragment("a", 1, "old cmd"));
    record_ticket_report(&store, "f", "s-implement", &fragment("a", 5, "new cmd"));
    let body = store
        .get(&store_implementation_report(&store, "f", "s-implement").unwrap())
        .unwrap();
    assert!(
        body.contains("new cmd") && !body.contains("old cmd"),
        "{body}"
    );
}

/// Fragments are known by name, not by whether a body happens to parse: a
/// sequence step may declare JSON deliverables of its own in the same store
/// directory, and those are the step's output, not ticket evidence.
#[test]
fn a_json_artifact_not_named_as_a_fragment_is_not_read_as_one() {
    let store = temp_store();
    let lookalike = crate::domain::artifact::Artifact {
        name: "summary".into(),
        mime: "application/json".into(),
        content: serde_json::to_string(&fragment("a", 1, "cargo test")).unwrap(),
        source: crate::domain::artifact::ArtifactSource::AgentText,
    };
    store.put("f", "s-implement", &lookalike).unwrap();
    assert_eq!(
        store_implementation_report(&store, "f", "s-implement"),
        None
    );
}

/// Re-rendering must not feed the previous report back in as a fragment.
#[test]
fn rendering_twice_is_stable() {
    let store = temp_store();
    record_ticket_report(&store, "f", "s-implement", &fragment("a", 1, "cargo test"));
    let first = store_implementation_report(&store, "f", "s-implement").unwrap();
    let body1 = store.get(&first).unwrap();
    let second = store_implementation_report(&store, "f", "s-implement").unwrap();
    assert_eq!(first, second);
    assert_eq!(body1, store.get(&second).unwrap());
}

/// The note a ticket left survives only in its fragment; a resumed list reads
/// it back from there, keyed by ticket id, and a fragment without one is
/// simply absent from the map.
#[test]
fn landed_tickets_handoffs_are_read_back_from_their_fragments() {
    let store = temp_store();
    record_ticket_report(
        &store,
        "f",
        "s-implement",
        &TicketReport::new(
            "a",
            "Ticket a",
            "AC1: MET\n\n## Handoff\nthe port is a stub",
            vec![],
            1,
        ),
    );
    record_ticket_report(&store, "f", "s-implement", &fragment("b", 2, "npm test"));
    record_ticket_report(
        &store,
        "f",
        "s-other",
        &TicketReport::new("c", "Ticket c", "## Handoff\nother step", vec![], 3),
    );

    let notes = ticket_handoffs(&store, "f", "s-implement");
    assert_eq!(
        notes.get("a").map(String::as_str),
        Some("the port is a stub")
    );
    assert!(
        !notes.contains_key("b"),
        "a fragment without a handoff: {notes:?}"
    );
    assert!(
        !notes.contains_key("c"),
        "another step's fragment leaked in: {notes:?}"
    );
    assert!(
        ticket_handoffs(&store, "f", "s-nothing").is_empty(),
        "no fragments, no notes"
    );
}

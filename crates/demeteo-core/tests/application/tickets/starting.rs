// Tests extracted from `src/application/tickets/starting.rs` (mirrored-tests
// convention). `super` = that module.

use super::*;

fn ticket(id: &str) -> TicketId {
    TicketId::from(id.to_string())
}

fn discovery(id: &str) -> DiscoveryId {
    DiscoveryId::from(id.to_string())
}

#[test]
fn a_join_names_the_other_starts_in_its_discovery_and_no_others() {
    let starting = StartingTickets::default();
    let first = starting.try_claim(&ticket("t-1")).expect("t-1 is free");
    let elsewhere = starting.try_claim(&ticket("t-9")).expect("t-9 is free");
    let unjoined = starting.try_claim(&ticket("t-8")).expect("t-8 is free");
    let second = starting.try_claim(&ticket("t-2")).expect("t-2 is free");

    assert_eq!(first.join(&discovery("d-1")), Vec::<String>::new());
    assert_eq!(elsewhere.join(&discovery("d-2")), Vec::<String>::new());
    assert_eq!(second.join(&discovery("d-1")), ["t-1"]);

    drop(unjoined);
}

#[test]
fn a_dropped_start_is_no_longer_counted() {
    let starting = StartingTickets::default();
    let first = starting.try_claim(&ticket("t-1")).expect("t-1 is free");
    first.join(&discovery("d-1"));
    drop(first);

    let second = starting.try_claim(&ticket("t-2")).expect("t-2 is free");

    assert_eq!(second.join(&discovery("d-1")), Vec::<String>::new());
}

#[test]
fn a_joined_ticket_is_still_claimed() {
    let starting = StartingTickets::default();
    let first = starting.try_claim(&ticket("t-1")).expect("t-1 is free");
    first.join(&discovery("d-1"));

    assert!(starting.try_claim(&ticket("t-1")).is_none());
}

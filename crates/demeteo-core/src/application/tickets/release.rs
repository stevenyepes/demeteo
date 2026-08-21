//! What the MR monitor does to a Discovery when a pull request reaches a
//! terminal state (§6.3).
//!
//! §6.3 chose to derive readiness rather than store it, and then leaned on
//! this hook for the one thing derivation cannot do: tell a user, who is not
//! looking at the board, that something they can start has just been released.
//! The two minutes between a merge and that notice are the poll interval, and
//! §6.4 accepts them.
//!
//! Free functions over the ports they read, not methods on the monitor, so
//! the whole path is reachable from a test without constructing one.

use crate::domain::ids::DiscoveryId;
use crate::domain::models::{Feature, Notification, NotificationKind, Ticket};
use crate::domain::ticket_graph::{derive_board, TicketBoard};
use crate::ports::db::{FeatureRepository, NotificationRepository};
use crate::ports::discovery::{DiscoveryPort, TicketPort};
use crate::ports::notification::{DomainEvent, NotificationPort};

use super::nodes_for;

/// Recompute every Discovery this Feature gates, and say so when something
/// became startable.
///
/// **Idempotency, and the hazard chosen against.** `record_merged`'s own guard
/// keys on the notification row, which leaves two seams inside it: after the
/// guard the work re-runs on every tick until the notification lands, and
/// after the notification it is skipped on any retry. This hook sits in
/// neither — it is called from the poll loop after the transition is
/// persisted, and it guards only its *write*. Recomputing is free and reads
/// nothing but current rows, so a repeat costs a query; a skipped recompute
/// costs the unblock entirely, silently, with no later tick to catch it
/// (`list_with_open_mr` will not return this Feature again). Repeat work over
/// lost work, deliberately.
///
/// **Draft pull requests never arrive here.** `list_with_open_mr` filters
/// `mr_state = 'open'`, so a Feature whose PR went to `draft` stops being
/// polled and can no longer reach `merged` through this path. That is a
/// pre-existing property of the monitor rather than something this hook
/// introduces, and it does not make a board wrong: readiness is derived on
/// read (§6.3), so the moment anything else refreshes that column the
/// dependents are released. What is lost is only the notice.
pub fn release_dependents(
    feature: &Feature,
    tickets: &dyn TicketPort,
    discoveries: &dyn DiscoveryPort,
    features: &dyn FeatureRepository,
    notifications: &dyn NotificationRepository,
    notif: &dyn NotificationPort,
) -> Result<(), String> {
    let owners = tickets.for_feature(&feature.id)?;
    let mut seen: Vec<DiscoveryId> = Vec::new();
    for owner in &owners {
        if seen.contains(&owner.discovery_id) {
            continue;
        }
        seen.push(owner.discovery_id.clone());

        let all = tickets.list_for_discovery(&owner.discovery_id)?;
        let (nodes, _) = nodes_for(&all, features)?;
        let board = derive_board(&nodes);
        let released = released_by(&owner.id.0, &all, &board);
        if released.is_empty() {
            continue;
        }
        announce(
            feature,
            &owner.discovery_id,
            &released,
            discoveries,
            notifications,
            notif,
        )?;
    }
    Ok(())
}

/// The Tickets that are startable *and* were waiting on `prerequisite_id`.
///
/// Not "everything startable": a Discovery is usually holding several ready
/// tickets a user has simply not started yet, and naming them again on every
/// unrelated merge would train the user to ignore the notice. §6.4 asks for
/// the moment a ticket *becomes* startable, and the edge is what makes this
/// merge the cause of it.
pub fn released_by<'a>(
    prerequisite_id: &str,
    tickets: &'a [Ticket],
    board: &TicketBoard,
) -> Vec<&'a Ticket> {
    tickets
        .iter()
        .zip(&board.standings)
        .filter(|(ticket, standing)| {
            standing.startable
                && ticket
                    .blocked_by
                    .iter()
                    .any(|dep| dep.0.trim() == prerequisite_id)
        })
        .map(|(ticket, _)| ticket)
        .collect()
}

fn announce(
    feature: &Feature,
    discovery_id: &DiscoveryId,
    released: &[&Ticket],
    discoveries: &dyn DiscoveryPort,
    notifications: &dyn NotificationRepository,
    notif: &dyn NotificationPort,
) -> Result<(), String> {
    if already_announced(feature, notifications)? {
        return Ok(());
    }
    let title = discoveries
        .get(discovery_id)?
        .map(|d| d.title)
        .unwrap_or_else(|| "a discovery".to_string());
    let names = released
        .iter()
        .map(|t| format!("#{} {}", t.seq, t.title))
        .collect::<Vec<_>>()
        .join(", ");
    let message = format!("{names} — ready to start in '{title}'");

    notifications.add(Notification {
        id: format!("notif-{}", crate::paths::now_ms()),
        project_id: feature.project_id.0.clone(),
        feature_id: feature.id.0.clone(),
        kind: NotificationKind::TicketsStartable,
        message: message.clone(),
        feature_url: Some(format!(
            "/projects/{}/discoveries/{}",
            feature.project_id.0, discovery_id.0
        )),
        read: false,
        created_at: crate::paths::now_ms(),
    })?;

    let _ = notif.emit(&DomainEvent::TicketsStartable {
        project_id: feature.project_id.0.clone(),
        discovery_id: discovery_id.0.clone(),
        discovery_title: title,
        ticket_ids: released.iter().map(|t| t.id.0.clone()).collect(),
        message,
    });
    Ok(())
}

/// The guard, keyed on the transition rather than on any one Ticket: one
/// pull request reaching a terminal state is one event, whatever number of
/// Tickets it happened to release.
fn already_announced(
    feature: &Feature,
    notifications: &dyn NotificationRepository,
) -> Result<bool, String> {
    Ok(notifications
        .list(Some(&feature.project_id), u32::MAX)?
        .iter()
        .any(|n| n.feature_id == feature.id.0 && n.kind == NotificationKind::TicketsStartable))
}

#[cfg(test)]
#[path = "../../../tests/application/tickets/release.rs"]
mod tests;

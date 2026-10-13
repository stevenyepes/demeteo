//! Reading a Discovery's pull requests from the forge on demand, instead of
//! waiting for the MR monitor's next tick.
//!
//! The monitor is the authority and stays it: this module fetches, then hands
//! each answer to [`apply_polled_state`], so a pull request settled here is
//! recorded, notified and cleaned up exactly as one the poll settled. What it
//! adds is reach — a caller that has just merged a pull request does not wait
//! two minutes for its dependents to be released, and a Feature whose pull
//! request sits in `draft`, which the monitor's `open`-only query never
//! returns, is read at all.
//!
//! An answer of `open` is never written, here or by the monitor.
//! `fetch_mr_state` also answers `open` when it has no provider or no access
//! to ask with, so writing it would turn a `draft` into an `open` nobody
//! observed. A draft marked ready therefore stays `draft` on the row until it
//! settles, which costs nothing: both are in flight, and this reads it either
//! way.

use serde::Serialize;

use crate::adapters::mr_monitor::{apply_polled_state, MrMonitorPorts};
use crate::domain::ids::{DiscoveryId, FeatureId, TicketId};
use crate::domain::models::Feature;
use crate::state::AppContext;

use super::{board, nodes_for, DiscoveryBoard};

/// A Discovery's board as of a forge read made just now, with the pull
/// requests that read could not answer for.
#[derive(Debug, Clone, Serialize)]
pub struct RefreshedBoard {
    #[serde(flatten)]
    pub board: DiscoveryBoard,
    /// Empty when every pull request was read. A ticket named here is on the
    /// board as of its last successful read, not as of this call.
    pub unrefreshed: Vec<UnrefreshedPr>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnrefreshedPr {
    pub ticket_id: TicketId,
    pub feature_id: FeatureId,
    pub error_message: String,
}

/// Whether the forge is still worth asking about this attempt: it has a pull
/// request, and that pull request has not settled. Both settled states are
/// final for this purpose — nothing follows `merged`, and a reopened `closed`
/// would answer `open`, which is not written (module docs).
pub fn worth_refreshing(feature: &Feature) -> bool {
    let has_pr = feature.mr_url.as_deref().is_some_and(|url| !url.is_empty());
    has_pr && !matches!(feature.mr_state.as_deref(), Some("merged" | "closed"))
}

/// Read the forge state of every unsettled pull request among
/// `discovery_id`'s tickets, apply each, and return the board that results.
///
/// One unreadable pull request does not fail the refresh: the rest are still
/// applied and the failure is carried out in
/// [`RefreshedBoard::unrefreshed`], since a caller told only "error" could
/// not tell which tickets its board is stale for.
pub async fn refresh_pr_states(
    ctx: &AppContext,
    discovery_id: &DiscoveryId,
) -> Result<RefreshedBoard, String> {
    let tickets = ctx.tickets.list_for_discovery(discovery_id)?;
    let (_, attempts) = nodes_for(&tickets, &*ctx.features)?;
    let ports = MrMonitorPorts {
        features: ctx.features.clone(),
        mr_publisher: ctx.mr_publisher.clone(),
        notifications: ctx.notifications.clone(),
        notif: ctx.notif.clone(),
        tickets: ctx.tickets.clone(),
        discoveries: ctx.discoveries.clone(),
        cache: ctx.feature_cache.clone(),
    };

    let mut unrefreshed = Vec::new();
    for (ticket, feature) in tickets.iter().zip(&attempts) {
        let Some(feature) = feature.as_ref().filter(|f| worth_refreshing(f)) else {
            continue;
        };
        let url = feature.mr_url.as_deref().unwrap_or_default();
        let outcome = match ctx
            .mr_publisher
            .fetch_mr_state(&feature.project_id.0, url)
            .await
        {
            Ok(state) => apply_polled_state(&ports, feature, &state).await,
            Err(error) => Err(error),
        };
        if let Err(error_message) = outcome {
            unrefreshed.push(UnrefreshedPr {
                ticket_id: ticket.id.clone(),
                feature_id: feature.id.clone(),
                error_message,
            });
        }
    }

    Ok(RefreshedBoard {
        board: board(ctx, discovery_id)?,
        unrefreshed,
    })
}

#[cfg(test)]
#[path = "../../../tests/application/tickets/refresh.rs"]
mod tests;

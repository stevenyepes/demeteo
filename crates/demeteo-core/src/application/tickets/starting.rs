//! Which Tickets have a start in flight *in this process*.
//!
//! [`start_refusal`](super::launch::start_refusal) reads the stored state, and
//! a start writes `Started` only once its run exists — seconds later for a
//! detached one (probe, spool, `submit_run`). Two starts inside that window
//! both read `Unstarted`, both launch, and the second `record_start`
//! orphans the first run, which is already being paid for. The Discovery view
//! disables its own buttons while a call is pending; that does not see an MCP
//! `start_ticket`, so the exclusion has to sit where every surface meets.
//!
//! Nothing durable holds it, for
//! [`SyncTurns`](crate::application::sync_turns::SyncTurns)'s reason: a row
//! claiming a start in flight would outlive the process making it and leave
//! the ticket unstartable.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::domain::ids::{DiscoveryId, TicketId};

/// Claimed ticket ids, each with its Discovery once [`TicketStart::join`] has
/// named it. The claim is taken before the ticket row is read, so the
/// Discovery is not known yet when a claim is made.
#[derive(Default)]
pub struct StartingTickets {
    ids: Mutex<HashMap<String, Option<String>>>,
}

impl StartingTickets {
    /// Take the ticket's start, or `None` when one is already under way.
    ///
    /// Check and take are one insert under the lock, so of two racing callers
    /// exactly one gets the claim. A refusal takes nothing, so it cannot
    /// release the claim it was refused for.
    pub fn try_claim(&self, ticket_id: &TicketId) -> Option<TicketStart<'_>> {
        let mut ids = self.lock();
        if ids.contains_key(&ticket_id.0) {
            return None;
        }
        ids.insert(ticket_id.0.clone(), None);
        Some(TicketStart {
            starting: self,
            ticket_id: ticket_id.0.clone(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Option<String>>> {
        self.ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// One ticket's start, held until this drops — through every `?` of a launch
/// that gave up and the drop of a future nobody polls any more. Released by
/// hand it would leak on the first early return, and nothing sweeps this set:
/// the ticket would refuse every start until the app restarted.
pub struct TicketStart<'a> {
    starting: &'a StartingTickets,
    ticket_id: String,
}

impl TicketStart<'_> {
    /// Name this start's Discovery, and return the other tickets already
    /// being started in it.
    ///
    /// A start that bounds itself by how many tickets are in flight reads the
    /// stored board, which cannot see a start that has claimed its ticket and
    /// not yet recorded a run. Naming and reading are one step under the
    /// lock, so of two starts racing in one Discovery the later always sees
    /// the earlier: both can be refused for one free slot, and never both
    /// admitted to it.
    pub fn join(&self, discovery_id: &DiscoveryId) -> Vec<String> {
        let mut ids = self.starting.lock();
        ids.insert(self.ticket_id.clone(), Some(discovery_id.0.clone()));
        ids.iter()
            .filter(|(ticket_id, joined)| {
                **ticket_id != self.ticket_id && joined.as_deref() == Some(discovery_id.0.as_str())
            })
            .map(|(ticket_id, _)| ticket_id.clone())
            .collect()
    }
}

impl Drop for TicketStart<'_> {
    fn drop(&mut self) {
        self.starting.lock().remove(&self.ticket_id);
    }
}

#[cfg(test)]
#[path = "../../../tests/application/tickets/starting.rs"]
mod tests;

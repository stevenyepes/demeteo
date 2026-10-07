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

use std::collections::HashSet;
use std::sync::Mutex;

use crate::domain::ids::TicketId;

#[derive(Default)]
pub struct StartingTickets {
    ids: Mutex<HashSet<String>>,
}

impl StartingTickets {
    /// Take the ticket's start, or `None` when one is already under way.
    ///
    /// Check and take are one insert under the lock, so of two racing callers
    /// exactly one gets the claim. A refusal takes nothing, so it cannot
    /// release the claim it was refused for.
    pub fn try_claim(&self, ticket_id: &TicketId) -> Option<TicketStart<'_>> {
        if !self.lock().insert(ticket_id.0.clone()) {
            return None;
        }
        Some(TicketStart {
            starting: self,
            ticket_id: ticket_id.0.clone(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
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

impl Drop for TicketStart<'_> {
    fn drop(&mut self) {
        self.starting.lock().remove(&self.ticket_id);
    }
}

//! The session-authenticated JSON API: what a signed-in browser reads of its
//! fleet and what it may ask an instance to do.
//!
//! Each endpoint is a thin handler over a use-case function that takes an
//! [`ApiContext`], the tenant and `now`, and no session. The use-cases run
//! over [`ApiStore`] so they are reachable from a test without Postgres, which
//! a handler holding a session is not.
//!
//! # The tenant rule
//!
//! The tenant is the session's `user` and never a value from a path or a
//! body. An instance, run or request of another tenant answers exactly as one
//! that does not exist — `404 not_found` — so a response cannot tell "not
//! yours" from "not there".
//!
//! # The read-scope rule
//!
//! What the Hub itself holds about an instance — presence, its labels, the
//! scopes it last reported — and the status of a request queued for it are
//! shown without `read`. What the instance reported, its run mirror and its
//! snapshot, is shown only while it reports `read` on; without it those fields
//! are withheld, not emptied. Queueing a request needs the scope of its kind
//! alone, not `read` beside it: the set is flat (`docs/HUB.md` §7). None of
//! this is enforcement, which is the instance's (§7, §8): a scope the Hub
//! reads as off spares a round trip.
//!
//! # No local paths
//!
//! A view is built field by field and never serializes a store row or a
//! [`Snapshot`](demeteo_hub_protocol::Snapshot) whole, so nothing an instance
//! keeps to itself (`docs/HUB.md` §3) can reach a response by being added to
//! a type upstream.
//!
//! This module is a leaf: `store` and `socket` never import it.

pub mod dispatch;
pub mod fleet;
pub mod gates;
pub mod port;
pub mod postgres;
pub mod request_status;
pub mod run_state;
pub mod runs;
pub mod scope;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use port::{ApiStore, MirrorEvent, SnapshotRead};
pub use postgres::PostgresApi;

use crate::auth::ceremonies::Ceremonies;
use crate::auth::RelyingParty;
use crate::pairing::signing_key::SigningKey;
use crate::ports::KeyService;
use crate::routes::AppState;
use crate::socket::connections::Connections;

/// Everything a use-case reaches besides the tenant and the clock. Borrowed,
/// so a handler builds one from its `AppState` and a test from parts it owns.
#[derive(Clone, Copy)]
pub struct ApiContext<'a> {
    pub store: &'a dyn ApiStore,
    pub keys: &'a dyn KeyService,
    pub signing_key: &'a SigningKey,
    pub connections: &'a Connections,
    pub ceremonies: &'a Ceremonies,
    pub rp: &'a RelyingParty,
}

impl<'a> ApiContext<'a> {
    /// What a handler runs its use-case over: the process's own registries
    /// and keys, and `store` for the rows.
    pub(crate) fn of(state: &'a AppState, store: &'a dyn ApiStore) -> Self {
        Self {
            store,
            keys: state.keys(),
            signing_key: state.signing_key(),
            connections: state.connections(),
            ceremonies: state.ceremonies(),
            rp: state.rp(),
        }
    }
}

//! Where the supervisor and the session leave their [`ConnectionFacts`] for
//! [`crate::adapters::hub::HubPairing::connection`] to read.

use std::sync::{Arc, Mutex, PoisonError};

use crate::domain::hub_socket::connection::ConnectionFacts;

/// Shared by a pairing and every socket it starts. The lock is a std one,
/// taken only to copy the facts out or to run a synchronous edit on them:
/// [`Self::update`] takes no future, so it can never be held across an
/// `.await` on the socket's own task.
#[derive(Clone, Default)]
pub struct ConnectionCell(Arc<Mutex<ConnectionFacts>>);

impl ConnectionCell {
    pub fn get(&self) -> ConnectionFacts {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn update(&self, edit: impl FnOnce(&mut ConnectionFacts)) {
        edit(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner));
    }
}

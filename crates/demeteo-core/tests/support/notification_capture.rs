//! A `NotificationPort` double that records every event verbatim instead of
//! answering every call with a blanket `Ok(())` and dropping it — the
//! `NotificationPort` analogue of the e2e `FakeExec` trap AGENTS.md §7 warns
//! about, where a double that succeeds unconditionally makes "nothing
//! arrived" true whether or not the code under test actually behaved.
//! `authorize_pkce.rs` uses this to prove a rejected `/authorize` request
//! emits zero `DomainEvent`s — a real, falsifiable assertion, not a
//! tautology of the double's own design.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::ports::notification::{DomainEvent, NotificationPort};

#[derive(Default)]
pub struct CapturingNotificationPort {
    events: Mutex<Vec<DomainEvent>>,
    window_raised: AtomicBool,
}

impl CapturingNotificationPort {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every event recorded so far, in emission order.
    pub fn events(&self) -> Vec<DomainEvent> {
        self.events.lock().unwrap().clone()
    }

    /// Whether `raise_main_window` has been called at least once.
    pub fn window_raised(&self) -> bool {
        self.window_raised.load(Ordering::SeqCst)
    }
}

impl NotificationPort for CapturingNotificationPort {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }

    fn raise_main_window(&self) {
        self.window_raised.store(true, Ordering::SeqCst);
    }
}

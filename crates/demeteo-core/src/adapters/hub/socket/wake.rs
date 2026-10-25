use std::sync::Arc;

use tokio::sync::Notify;

use crate::ports::run_events::{RunEvent, RunEventsPort};

/// A [`RunEventsPort`] that wakes the Hub socket after every successful
/// `append`, so a row written while connected is sent without the socket
/// polling a database that every repository shares one lock on.
///
/// It wakes with `notify_one`, not `notify_waiters`: `notify_waiters` wakes
/// only a task already parked in `notified()`, so an append landing while
/// the session is busy sending the previous page would be lost.
/// `notify_one` with nobody waiting stores one permit, and the session's
/// next `notified()` completes at once — appends that arrive together
/// collapse into that one permit, which is enough because the session pages
/// everything after its cursor rather than the row that woke it.
pub struct WakingRunEvents {
    inner: Arc<dyn RunEventsPort>,
    wake: Arc<Notify>,
}

impl WakingRunEvents {
    pub fn new(inner: Arc<dyn RunEventsPort>) -> Self {
        Self {
            inner,
            wake: Arc::new(Notify::new()),
        }
    }

    pub fn wake(&self) -> Arc<Notify> {
        Arc::clone(&self.wake)
    }
}

impl RunEventsPort for WakingRunEvents {
    fn append(
        &self,
        run_id: &str,
        kind: &str,
        payload_json: Option<&str>,
        now: i64,
    ) -> Result<i64, String> {
        let offset = self.inner.append(run_id, kind, payload_json, now)?;
        self.wake.notify_one();
        Ok(offset)
    }

    fn list_since(&self, run_id: &str, from_offset: i64) -> Result<Vec<RunEvent>, String> {
        self.inner.list_since(run_id, from_offset)
    }

    fn list_after(&self, from_offset: i64, limit: usize) -> Result<Vec<RunEvent>, String> {
        self.inner.list_after(from_offset, limit)
    }
}

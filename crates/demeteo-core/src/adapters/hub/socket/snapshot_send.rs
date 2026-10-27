//! What one connection sends of [`HubSnapshotSource`] (docs/HUB.md §5
//! *Snapshots*).
//!
//! Nothing here ends a session. The Hub keeps no cursor for snapshots, so
//! one that is not built or not sent is made good by the next; that is why a
//! source that fails is logged and passed over, where an unreadable run log
//! ends the session `1011`.

use std::future::{pending, Future};
use std::sync::Arc;
use std::time::Duration;

use demeteo_hub_protocol::Snapshot;
use tokio::time::{sleep_until, Instant};

use crate::domain::hub_socket::MAX_SNAPSHOT_BYTES;
use crate::ports::hub::HubSnapshotSource;

use super::session::encoded;

/// The snapshots of one connection. A new connection starts from
/// [`Default`], so its first snapshot is always sent.
#[derive(Default)]
pub(super) struct SnapshotSend {
    /// The last snapshot sent on this connection.
    last: Option<Snapshot>,
    /// When the rebuild a wake asked for is due.
    due: Option<Instant>,
}

impl SnapshotSend {
    /// A fixed window from the first wake, not a trailing one: a trailing
    /// window never closes under continuous edits, and this bounds the
    /// latency at `debounce` plus the build.
    pub(super) fn woken(&mut self, debounce: Duration) {
        self.due.get_or_insert_with(|| Instant::now() + debounce);
    }

    /// Resolves when the rebuild a wake asked for is due, and never while
    /// none is.
    pub(super) fn due(&self) -> impl Future<Output = ()> + 'static {
        let due = self.due;
        async move {
            match due {
                Some(due) => sleep_until(due).await,
                None => pending().await,
            }
        }
    }

    /// The frame to send now, if any, taken as sent: a send that fails ends
    /// the connection, and this with it. Whatever woke before the build is
    /// in it, so the build disarms [`Self::due`].
    pub(super) async fn frame(&mut self, source: &Arc<dyn HubSnapshotSource>) -> Option<String> {
        self.due = None;
        let snapshot = built(source).await?;
        if self.last.as_ref() == Some(&snapshot) {
            return None;
        }
        let text = encoded(&snapshot)?;
        if text.len() > MAX_SNAPSHOT_BYTES {
            tracing::warn!(bytes = text.len(), "a snapshot too large for the Hub");
            return None;
        }
        self.last = Some(snapshot);
        Some(text)
    }
}

/// Built off the runtime, for the reason [`super::session`]'s run-log read
/// is.
async fn built(source: &Arc<dyn HubSnapshotSource>) -> Option<Snapshot> {
    let source = Arc::clone(source);
    tokio::task::spawn_blocking(move || source.snapshot())
        .await
        .map_err(|e| e.to_string())
        .and_then(|snapshot| snapshot)
        .map_err(|error| tracing::warn!(%error, "a snapshot for the Hub could not be built"))
        .ok()
}

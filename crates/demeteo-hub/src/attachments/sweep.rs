//! The sweep that keeps [the TTL rule](super): whatever has passed its
//! `expires_at`, staged or bound, fetched or not, is deleted.

use super::port::AttachmentStore;
use crate::auth::handlers::unix_now;
use crate::store::StoreError;
use std::time::Duration;
use tokio::time::MissedTickBehavior;

/// Deletes every row whose `expires_at` is at or before `now` (Unix seconds),
/// of every tenant, and answers how many.
pub async fn sweep_once(store: &dyn AttachmentStore, now: u64) -> Result<u64, StoreError> {
    store.expired_deleted(now).await
}

/// Sweeps every `interval_secs`, for as long as the Hub runs. A failed sweep
/// is logged and the next tick tries again: rows it left are still expired.
/// A tick missed behind a slow sweep is delayed rather than burst, so a
/// database that is struggling is not asked again at once.
pub async fn run(store: &dyn AttachmentStore, interval_secs: u64) {
    // `interval` panics on a zero period. Config refuses 0, but the settings
    // are a plain struct anyone can build.
    let mut ticks = tokio::time::interval(Duration::from_secs(interval_secs.max(1)));
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        match sweep_once(store, unix_now()).await {
            Ok(0) => {}
            Ok(deleted) => tracing::info!(deleted, "swept expired attachments"),
            Err(error) => tracing::warn!(%error, "the attachment sweep failed"),
        }
    }
}

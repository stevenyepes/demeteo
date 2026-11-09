//! The production [`HarnessCatalogSource`]: a cache of the last probe, and
//! the probe itself on a task of its own. When a probe is due, and how one
//! folds into the cache, is [`crate::domain::hub_socket`]'s to decide.
//!
//! The cache is a `std` mutex held only to copy in or out, never across an
//! `await`, so a snapshot build in `spawn_blocking` reads it without waiting
//! on a probe. At most one probe runs at a time; a request made while one
//! runs is dropped rather than queued, since the running one answers it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::Notify;

use crate::domain::hub_socket::{
    merged, probe_due, CatalogEntry, HarnessProbe, HarnessProbeOutcome, ProbeTrigger,
    HARNESS_PROBE_TIMEOUT_SECS,
};
use crate::domain::models::{Availability, EffortLevel};
use crate::ports::hub::{HarnessCatalogSource, HarnessModelProbe};
use crate::shared::time::now_ms;

pub struct CachedHarnessCatalog {
    shared: Arc<Shared>,
    runtime: Handle,
    now_ms: fn() -> i64,
    timeout: Duration,
}

struct Shared {
    probe: Arc<dyn HarnessModelProbe>,
    kinds: Vec<String>,
    effort_levels: HashMap<String, Vec<EffortLevel>>,
    cache: Mutex<Cache>,
    probing: AtomicBool,
    /// The snapshot wake: a finished probe changes what a snapshot reports,
    /// as a write to the repositories it reads does.
    wake: Arc<Notify>,
}

#[derive(Default)]
struct Cache {
    entries: Option<Vec<CatalogEntry>>,
    last_finished_ms: Option<i64>,
}

impl CachedHarnessCatalog {
    /// `kinds` are the harnesses probed, each with the effort levels it
    /// accepts, fixed for the life of the process as the registry is.
    pub fn new(
        probe: Arc<dyn HarnessModelProbe>,
        kinds: Vec<(String, Vec<EffortLevel>)>,
        wake: Arc<Notify>,
        runtime: Handle,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                probe,
                kinds: kinds.iter().map(|(kind, _)| kind.clone()).collect(),
                effort_levels: kinds.into_iter().collect(),
                cache: Mutex::default(),
                probing: AtomicBool::new(false),
                wake,
            }),
            runtime,
            now_ms: wall_ms,
            timeout: Duration::from_secs(HARNESS_PROBE_TIMEOUT_SECS),
        }
    }

    pub fn with_clock(mut self, now_ms: fn() -> i64) -> Self {
        self.now_ms = now_ms;
        self
    }

    /// How long one harness's model listing may take; see
    /// [`HARNESS_PROBE_TIMEOUT_SECS`].
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl HarnessCatalogSource for CachedHarnessCatalog {
    fn current(&self) -> Option<Vec<CatalogEntry>> {
        locked(&self.shared.cache).entries.clone()
    }

    fn request_probe(&self, trigger: ProbeTrigger) {
        let last = locked(&self.shared.cache).last_finished_ms;
        if !probe_due(last, (self.now_ms)(), trigger) {
            return;
        }
        if self
            .shared
            .probing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let guard = Probing(Arc::clone(&self.shared));
        let (now_ms, timeout) = (self.now_ms, self.timeout);
        self.runtime.spawn(async move {
            let shared = &guard.0;
            let fresh = probed(shared, now_ms, timeout).await;
            let mut cache = locked(&shared.cache);
            let previous = cache.entries.take().unwrap_or_default();
            cache.entries = Some(merged(&previous, fresh, |kind| {
                shared.effort_levels.get(kind).cloned().unwrap_or_default()
            }));
            cache.last_finished_ms = Some(now_ms());
        });
    }
}

/// Releases the single-flight flag however the probe task ends, a panic
/// included, and only then wakes the snapshot, so the rebuild it wakes reads
/// the stored result.
struct Probing(Arc<Shared>);

impl Drop for Probing {
    fn drop(&mut self) {
        self.0.probing.store(false, Ordering::Release);
        self.0.wake.notify_one();
    }
}

async fn probed(shared: &Shared, now_ms: fn() -> i64, timeout: Duration) -> Vec<HarnessProbe> {
    let mut out = Vec::with_capacity(shared.kinds.len());
    for (kind, availability) in shared.probe.availability(&shared.kinds).await {
        let outcome = match availability {
            Availability::Missing => HarnessProbeOutcome::NotInstalled,
            Availability::Unknown => HarnessProbeOutcome::Unreachable,
            Availability::Installed => {
                match tokio::time::timeout(timeout, shared.probe.models(&kind)).await {
                    Ok(Some((models, source))) => HarnessProbeOutcome::Listed { models, source },
                    Ok(None) | Err(_) => HarnessProbeOutcome::Unreachable,
                }
            }
        };
        out.push(HarnessProbe {
            kind,
            outcome,
            finished_ms: now_ms(),
        });
    }
    out
}

fn wall_ms() -> i64 {
    i64::try_from(now_ms()).unwrap_or(i64::MAX)
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/hub_harness_catalog.rs"]
mod tests;

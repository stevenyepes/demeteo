// Tests extracted from `crates/demeteo-core/src/adapters/hub/harness_catalog.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::hub::snapshot_source::RepoSnapshotSource;
use crate::adapters::hub::testing::ScriptedModelProbe;
use crate::domain::hub_socket::{HARNESS_PROBE_CONNECT_FLOOR_SECS, HARNESS_PROBE_INTERVAL_SECS};
use crate::domain::models::{ConfigOptionValue, ModelSource};
use crate::ports::hub::HubSnapshotSource;
use std::sync::atomic::AtomicI64;

const BOUND: Duration = Duration::from_secs(5);
const SETTLE: Duration = Duration::from_millis(50);
const T0: i64 = 1_700_000_000_000;

fn model(value: &str) -> ConfigOptionValue {
    ConfigOptionValue {
        value: value.to_owned(),
        name: value.to_owned(),
        description: None,
        supports_images: false,
    }
}

fn opencode(gate: &Arc<Notify>) -> Arc<ScriptedModelProbe> {
    Arc::new(
        ScriptedModelProbe::default()
            .listing(
                "opencode",
                vec![model("openai/gpt-6-sol")],
                ModelSource::Probed,
            )
            .gated(gate.clone()),
    )
}

fn catalog(
    probe: Arc<ScriptedModelProbe>,
    wake: &Arc<Notify>,
    clock: fn() -> i64,
) -> CachedHarnessCatalog {
    CachedHarnessCatalog::new(
        probe,
        vec![("opencode".to_owned(), vec![EffortLevel::High])],
        wake.clone(),
        Handle::current(),
    )
    .with_clock(clock)
}

fn probed_at(catalog: &CachedHarnessCatalog) -> Option<Vec<i64>> {
    catalog
        .current()
        .map(|entries| entries.iter().map(|e| e.probed_at_ms).collect())
}

async fn woken(wake: &Notify) {
    timeout_or_panic(wake.notified()).await;
}

async fn timeout_or_panic(work: impl std::future::Future<Output = ()>) {
    tokio::time::timeout(BOUND, work)
        .await
        .expect("the probe finished");
}

#[tokio::test]
async fn a_request_returns_while_its_probe_is_held_and_the_old_entry_stands_meanwhile() {
    static NOW: AtomicI64 = AtomicI64::new(T0);
    fn clock() -> i64 {
        NOW.load(Ordering::SeqCst)
    }
    let (gate, wake) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let probe = opencode(&gate);
    let catalog = catalog(probe.clone(), &wake, clock);

    catalog.request_probe(ProbeTrigger::Connected);
    assert_eq!(
        catalog.current().map(|e| e.len()),
        None,
        "nothing until a probe ends"
    );
    gate.notify_one();
    woken(&wake).await;
    let first = catalog.current().expect("a catalog after the first probe");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].kind, "opencode");
    assert_eq!(first[0].models[0].value, "openai/gpt-6-sol");
    assert_eq!(first[0].effort_levels, [EffortLevel::High]);
    assert_eq!(first[0].source, ModelSource::Probed);

    NOW.store(
        T0 + 1_000 * HARNESS_PROBE_INTERVAL_SECS as i64,
        Ordering::SeqCst,
    );
    catalog.request_probe(ProbeTrigger::Timer);
    tokio::time::sleep(SETTLE).await;
    assert_eq!(probe.listings_of("opencode"), 2, "the second probe is held");
    assert_eq!(probed_at(&catalog), Some(vec![T0]));

    gate.notify_one();
    woken(&wake).await;
    assert_eq!(probed_at(&catalog), Some(vec![clock()]));
    assert_eq!(probe.unscripted(), 0);
}

#[tokio::test]
async fn a_request_made_while_a_probe_runs_starts_no_second_one() {
    static NOW: AtomicI64 = AtomicI64::new(T0);
    fn clock() -> i64 {
        NOW.load(Ordering::SeqCst)
    }
    let (gate, wake) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let probe = opencode(&gate);
    let catalog = catalog(probe.clone(), &wake, clock);

    catalog.request_probe(ProbeTrigger::Connected);
    tokio::time::sleep(SETTLE).await;
    catalog.request_probe(ProbeTrigger::Connected);
    catalog.request_probe(ProbeTrigger::Timer);
    gate.notify_one();
    woken(&wake).await;
    tokio::time::sleep(SETTLE).await;

    assert_eq!(probe.listings_of("opencode"), 1);
}

#[tokio::test]
async fn a_finished_probe_wakes_the_snapshot() {
    static NOW: AtomicI64 = AtomicI64::new(T0);
    fn clock() -> i64 {
        NOW.load(Ordering::SeqCst)
    }
    let (gate, wake) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let catalog = catalog(opencode(&gate), &wake, clock);

    catalog.request_probe(ProbeTrigger::Connected);
    tokio::time::sleep(SETTLE).await;
    assert!(
        tokio::time::timeout(SETTLE, wake.notified()).await.is_err(),
        "no wake while the probe is held"
    );
    gate.notify_one();

    woken(&wake).await;
}

#[tokio::test]
async fn a_listing_that_times_out_keeps_the_entry_it_last_listed() {
    static NOW: AtomicI64 = AtomicI64::new(T0);
    fn clock() -> i64 {
        NOW.load(Ordering::SeqCst)
    }
    let (gate, wake) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let catalog = catalog(opencode(&gate), &wake, clock).with_timeout(SETTLE);
    catalog.request_probe(ProbeTrigger::Connected);
    gate.notify_one();
    woken(&wake).await;

    NOW.store(
        T0 + 1_000 * HARNESS_PROBE_INTERVAL_SECS as i64,
        Ordering::SeqCst,
    );
    catalog.request_probe(ProbeTrigger::Timer);
    woken(&wake).await;

    assert_eq!(probed_at(&catalog), Some(vec![T0]));
}

#[tokio::test]
async fn a_reconnect_inside_the_floor_starts_no_probe() {
    static NOW: AtomicI64 = AtomicI64::new(T0);
    fn clock() -> i64 {
        NOW.load(Ordering::SeqCst)
    }
    let (gate, wake) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let probe = opencode(&gate);
    let catalog = catalog(probe.clone(), &wake, clock);
    catalog.request_probe(ProbeTrigger::Connected);
    gate.notify_one();
    woken(&wake).await;
    let floor = 1_000 * HARNESS_PROBE_CONNECT_FLOOR_SECS as i64;

    NOW.store(T0 + floor - 1, Ordering::SeqCst);
    catalog.request_probe(ProbeTrigger::Connected);
    tokio::time::sleep(SETTLE).await;
    assert_eq!(probe.listings_of("opencode"), 1);

    NOW.store(T0 + floor, Ordering::SeqCst);
    catalog.request_probe(ProbeTrigger::Connected);
    gate.notify_one();
    woken(&wake).await;
    assert_eq!(probe.listings_of("opencode"), 2);
}

#[tokio::test]
async fn a_snapshot_is_built_while_a_probe_is_held() {
    static NOW: AtomicI64 = AtomicI64::new(T0);
    fn clock() -> i64 {
        NOW.load(Ordering::SeqCst)
    }
    let (gate, wake) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let probe = opencode(&gate);
    let catalog = Arc::new(catalog(probe.clone(), &wake, clock));
    let db = Arc::new(
        SqliteAdapter::new(rusqlite::Connection::open_in_memory().expect("an in-memory database"))
            .expect("a migrated database"),
    );
    let source = Arc::new(
        RepoSnapshotSource::new(
            db.clone(),
            db.clone(),
            db.clone(),
            db,
            vec!["opencode".to_owned()],
        )
        .with_harness_catalog(catalog.clone()),
    );
    let built = || {
        let source = source.clone();
        async move {
            tokio::time::timeout(
                BOUND,
                tokio::task::spawn_blocking(move || source.snapshot()),
            )
            .await
            .expect("the build did not wait for the probe")
            .expect("the build did not panic")
            .expect("a snapshot")
        }
    };

    catalog.request_probe(ProbeTrigger::Connected);
    tokio::time::sleep(SETTLE).await;
    assert_eq!(probe.listings_of("opencode"), 1, "the probe is held");
    let snapshot = built().await;
    assert_eq!(snapshot.harness_catalog, None);
    assert_eq!(snapshot.harnesses, Some(vec!["opencode".to_owned()]));

    gate.notify_one();
    woken(&wake).await;
    let catalog = built().await.harness_catalog.expect("the probed catalog");
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].models[0].value, "openai/gpt-6-sol");
}

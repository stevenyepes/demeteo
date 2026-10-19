//! The registry is exercised through the real type; the interleavings are
//! forced with channels rather than sleeps.

use super::*;
use tokio::sync::oneshot;

fn driver(drivers: &Arc<RunDrivers>, run_id: &str) -> DriverGuard {
    match drivers.try_acquire(run_id) {
        Acquire::Driver(g) => g,
        Acquire::Attached => panic!("expected to become the driver for {run_id}"),
    }
}

fn is_attached(drivers: &Arc<RunDrivers>, run_id: &str) -> bool {
    matches!(drivers.try_acquire(run_id), Acquire::Attached)
}

fn rerun_requested(drivers: &RunDrivers, run_id: &str) -> Option<bool> {
    drivers.lock().get(run_id).map(|s| s.rerun_requested)
}

#[test]
fn acquire_then_drop_frees_the_run() {
    let drivers = Arc::new(RunDrivers::default());
    let guard = driver(&drivers, "r1");
    assert_eq!(rerun_requested(&drivers, "r1"), Some(false));
    drop(guard);
    assert_eq!(rerun_requested(&drivers, "r1"), None);
    drop(driver(&drivers, "r1"));
}

#[test]
fn distinct_runs_each_get_a_driver() {
    let drivers = Arc::new(RunDrivers::default());
    let _a = driver(&drivers, "a");
    let _b = driver(&drivers, "b");
}

#[test]
fn second_acquire_attaches_and_sets_the_flag() {
    let drivers = Arc::new(RunDrivers::default());
    let _guard = driver(&drivers, "r1");
    assert!(is_attached(&drivers, "r1"));
    assert_eq!(rerun_requested(&drivers, "r1"), Some(true));
    assert_eq!(rerun_requested(&drivers, "other"), None);
}

#[test]
fn attaching_does_not_release_the_drivers_slot() {
    let drivers = Arc::new(RunDrivers::default());
    let _guard = driver(&drivers, "r1");
    assert!(is_attached(&drivers, "r1"));
    assert!(is_attached(&drivers, "r1"));
}

#[test]
fn exit_check_consumes_the_flag_then_releases() {
    let drivers = Arc::new(RunDrivers::default());
    let mut guard = driver(&drivers, "r1");
    assert!(is_attached(&drivers, "r1"));
    assert!(is_attached(&drivers, "r1"));

    assert!(guard.finish_or_rearm());
    assert_eq!(rerun_requested(&drivers, "r1"), Some(false));
    assert!(!guard.finish_or_rearm());
    assert_eq!(rerun_requested(&drivers, "r1"), None);
}

#[test]
fn exit_check_without_a_request_releases_at_once() {
    let drivers = Arc::new(RunDrivers::default());
    let mut guard = driver(&drivers, "r1");
    assert!(!guard.finish_or_rearm());
    drop(driver(&drivers, "r1"));
}

#[test]
fn dropping_a_released_guard_leaves_the_next_drivers_slot() {
    let drivers = Arc::new(RunDrivers::default());
    let mut first = driver(&drivers, "r1");
    assert!(!first.finish_or_rearm());
    let _second = driver(&drivers, "r1");
    drop(first);
    assert!(is_attached(&drivers, "r1"));
}

#[tokio::test]
async fn driver_loops_once_more_for_a_request_made_while_driving() {
    let drivers = Arc::new(RunDrivers::default());
    let mut guard = driver(&drivers, "r1");
    let mut passes = 0;
    loop {
        passes += 1;
        if passes == 1 {
            assert!(is_attached(&drivers, "r1"));
        }
        assert!(passes < 5, "flag was never consumed");
        if !guard.finish_or_rearm() {
            break;
        }
    }
    assert_eq!(passes, 2);
}

#[tokio::test]
async fn request_arriving_at_the_exit_check_is_not_lost() {
    let drivers = Arc::new(RunDrivers::default());
    let (at_exit_tx, at_exit_rx) = oneshot::channel::<()>();
    let (go_tx, go_rx) = oneshot::channel::<()>();

    let driving = Arc::clone(&drivers);
    let task = tokio::spawn(async move {
        let mut guard = driver(&driving, "r1");
        let mut at_exit = Some(at_exit_tx);
        let mut go = Some(go_rx);
        let mut passes = 0;
        loop {
            passes += 1;
            if let (Some(tx), Some(rx)) = (at_exit.take(), go.take()) {
                let _ = tx.send(());
                let _ = rx.await;
            }
            assert!(passes < 5, "flag was never consumed");
            if !guard.finish_or_rearm() {
                return passes;
            }
        }
    });

    at_exit_rx.await.expect("driver reached its exit check");
    assert!(is_attached(&drivers, "r1"));
    go_tx.send(()).expect("driver still waiting");

    assert_eq!(task.await.expect("driver task"), 2);
    drop(driver(&drivers, "r1"));
}

#[tokio::test]
async fn early_return_releases_the_slot() {
    async fn body(drivers: &Arc<RunDrivers>, bail: bool) -> Result<(), &'static str> {
        let _guard = driver(drivers, "r1");
        if bail {
            return Err("bailed");
        }
        Ok(())
    }

    let drivers = Arc::new(RunDrivers::default());
    assert_eq!(body(&drivers, true).await, Err("bailed"));
    assert_eq!(rerun_requested(&drivers, "r1"), None);
    assert_eq!(body(&drivers, false).await, Ok(()));
}

#[tokio::test]
async fn panic_in_a_guarded_future_releases_the_slot() {
    let drivers = Arc::new(RunDrivers::default());
    let driving = Arc::clone(&drivers);
    let joined = tokio::spawn(async move {
        let _guard = driver(&driving, "r1");
        panic!("driver blew up");
    })
    .await;
    assert!(joined.expect_err("task panicked").is_panic());

    drop(driver(&drivers, "r1"));
}

#[tokio::test]
async fn panic_while_the_lock_is_held_does_not_poison_the_registry() {
    let drivers = Arc::new(RunDrivers::default());
    let poisoner = Arc::clone(&drivers);
    let joined = tokio::spawn(async move {
        let _held = poisoner.slots.lock().expect("fresh lock");
        panic!("poison the registry");
    })
    .await;
    assert!(joined.expect_err("task panicked").is_panic());
    assert!(drivers.slots.is_poisoned());

    let mut guard = driver(&drivers, "r1");
    assert!(is_attached(&drivers, "r1"));
    assert!(guard.finish_or_rearm());
    assert!(!guard.finish_or_rearm());
}

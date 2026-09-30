use super::*;

#[test]
fn a_settled_pr_names_the_reason() {
    assert_eq!(
        CacheReleaseReason::of("awaiting_mr", Some("merged")),
        Some(CacheReleaseReason::Merged)
    );
    assert_eq!(
        CacheReleaseReason::of("completed", Some("closed")),
        Some(CacheReleaseReason::Closed)
    );
}

#[test]
fn a_settled_pr_outranks_a_cleanup() {
    assert_eq!(
        CacheReleaseReason::of("archived", Some("merged")),
        Some(CacheReleaseReason::Merged),
        "the runner records a merge as `completed` + `merged`, which its sweep \
         then agrees with; `archived` would lose the merge"
    );
}

#[test]
fn a_cleanup_with_no_settled_pr_is_a_dismissal() {
    for status in ["archived", "deleted"] {
        assert_eq!(
            CacheReleaseReason::of(status, Some("open")),
            Some(CacheReleaseReason::Dismissed)
        );
        assert_eq!(
            CacheReleaseReason::of(status, None),
            Some(CacheReleaseReason::Dismissed)
        );
    }
}

#[test]
fn an_unfinished_feature_gives_no_reason() {
    assert_eq!(CacheReleaseReason::of("awaiting_mr", Some("open")), None);
    assert_eq!(CacheReleaseReason::of("completed", Some("draft")), None);
    assert_eq!(CacheReleaseReason::of("failed", None), None);
}

#[test]
fn the_wire_spelling_is_snake_case() {
    assert_eq!(
        serde_json::to_value(CacheReleaseReason::Dismissed).unwrap(),
        serde_json::json!("dismissed")
    );
    assert!(serde_json::from_value::<CacheReleaseReason>(serde_json::json!("open")).is_err());
}

#[test]
fn a_live_run_is_refused() {
    for status in ["pending", "running"] {
        let refusal = runner_settle(status, "run-1", CacheReleaseReason::Dismissed)
            .expect_err("a driven run's cache is in use");
        assert!(refusal.contains("run-1"), "{refusal}");
    }
}

#[test]
fn a_finished_run_settles_by_reason() {
    for status in [
        "awaiting_mr",
        "completed",
        "failed",
        "cancelled",
        "interrupted",
    ] {
        assert_eq!(
            runner_settle(status, "run-1", CacheReleaseReason::Merged),
            Ok(RunnerSettle::MrState("merged"))
        );
        assert_eq!(
            runner_settle(status, "run-1", CacheReleaseReason::Closed),
            Ok(RunnerSettle::MrState("closed"))
        );
        assert_eq!(
            runner_settle(status, "run-1", CacheReleaseReason::Dismissed),
            Ok(RunnerSettle::Archive)
        );
    }
}

fn pending(run_id: &str, reason: CacheReleaseReason) -> PendingRunnerRelease {
    PendingRunnerRelease {
        machine_id: "m-1".to_string(),
        run_id: run_id.to_string(),
        reason,
    }
}

#[test]
fn a_repeat_release_for_one_run_replaces_the_earlier_one() {
    let queue = with_pending(Vec::new(), pending("run-1", CacheReleaseReason::Dismissed));
    let queue = with_pending(queue, pending("run-2", CacheReleaseReason::Closed));
    let queue = with_pending(queue, pending("run-1", CacheReleaseReason::Merged));

    assert_eq!(
        queue,
        [
            pending("run-2", CacheReleaseReason::Closed),
            pending("run-1", CacheReleaseReason::Merged),
        ]
    );
}

#[test]
fn the_same_run_id_on_another_machine_is_another_entry() {
    let mut other = pending("run-1", CacheReleaseReason::Merged);
    other.machine_id = "m-2".to_string();
    let queue = with_pending(Vec::new(), pending("run-1", CacheReleaseReason::Merged));
    assert_eq!(with_pending(queue, other).len(), 2);
}

#[test]
fn only_an_unknown_run_is_given_up_on() {
    assert!(!worth_retrying("no such run: run-1"));
    assert!(worth_retrying("unknown method: release_feature_cache"));
    assert!(worth_retrying(
        "run run-1 is still running; its dependency cache is in use"
    ));
    assert!(worth_retrying("Timed out waiting on socket"));
}

#[test]
fn a_runner_too_old_for_the_method_is_reported_once_per_machine() {
    let mut too_old = std::collections::HashSet::new();
    let old = "unknown method: release_feature_cache";

    assert!(worth_logging(&mut too_old, "runner-1", old));
    assert!(!worth_logging(&mut too_old, "runner-1", old));
    assert!(worth_logging(&mut too_old, "runner-2", old));
}

#[test]
fn every_other_failure_is_reported_every_time() {
    let mut too_old = std::collections::HashSet::new();
    let offline = "Timed out waiting on socket";

    assert!(worth_logging(&mut too_old, "runner-1", offline));
    assert!(worth_logging(&mut too_old, "runner-1", offline));
    assert!(too_old.is_empty());
}

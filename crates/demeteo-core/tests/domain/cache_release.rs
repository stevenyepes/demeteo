use super::{
    cache_idle_ttl_days, cache_releasable, cache_releasable_at, idle_past_ttl,
    releasable_after_mr_poll, DEFAULT_CACHE_IDLE_TTL_DAYS,
};

#[test]
fn archived_and_deleted_features_release_their_cache() {
    assert!(cache_releasable("deleted", None));
    assert!(cache_releasable("archived", Some("open")));
}

#[test]
fn a_completed_feature_releases_only_once_its_pr_is_settled() {
    assert!(cache_releasable("completed", Some("merged")));
    assert!(cache_releasable("completed", Some("closed")));
    assert!(!cache_releasable("completed", Some("open")));
    assert!(!cache_releasable("completed", None));
}

#[test]
fn a_replayable_feature_keeps_its_cache_until_its_pr_settles() {
    for status in ["awaiting_mr", "failed", "cancelled", "interrupted"] {
        for mr_state in [None, Some("open"), Some("draft")] {
            assert!(
                !cache_releasable(status, mr_state),
                "{status} with its PR {mr_state:?} can still run"
            );
        }
        assert!(cache_releasable(status, Some("merged")), "{status} merged");
        assert!(cache_releasable(status, Some("closed")), "{status} closed");
    }
}

#[test]
fn a_live_feature_keeps_its_cache_whatever_its_pr_did() {
    for status in ["running", "pending", "awaiting_gate", "some_future_status"] {
        for mr_state in ["merged", "closed"] {
            assert!(
                !cache_releasable(status, Some(mr_state)),
                "{status} is owned by a driver"
            );
        }
    }
}

#[test]
fn a_polled_settle_releases_a_feature_no_driver_owns() {
    for status in ["awaiting_mr", "completed", "failed"] {
        assert!(releasable_after_mr_poll(status, "merged"), "{status}");
        assert!(releasable_after_mr_poll(status, "closed"), "{status}");
    }
    assert!(!releasable_after_mr_poll("completed", "open"));
    assert!(!releasable_after_mr_poll("completed", "draft"));
}

#[test]
fn a_merge_polled_mid_replay_keeps_the_live_run_cache() {
    assert!(!releasable_after_mr_poll("running", "merged"));
    assert!(!releasable_after_mr_poll("awaiting_gate", "merged"));
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const NOW_MS: i64 = 1_000 * DAY_MS;

#[test]
fn an_idle_feature_releases_its_cache_only_past_the_ttl() {
    for status in [
        "failed",
        "cancelled",
        "awaiting_mr",
        "interrupted",
        "completed",
    ] {
        assert!(
            cache_releasable_at(status, Some("open"), NOW_MS - 14 * DAY_MS - 1, NOW_MS, 14),
            "{status} idle past the ttl"
        );
        assert!(
            !cache_releasable_at(status, Some("open"), NOW_MS - 14 * DAY_MS, NOW_MS, 14),
            "{status} idle exactly the ttl"
        );
    }
}

#[test]
fn a_zero_ttl_never_releases_on_idleness() {
    assert!(!cache_releasable_at("failed", None, 0, NOW_MS, 0));
    assert!(cache_releasable_at("archived", None, NOW_MS, NOW_MS, 0));
}

#[test]
fn a_live_or_unknown_status_is_never_released_however_idle() {
    for status in [
        "running",
        "pending",
        "verifying",
        "awaiting_gate",
        "gated",
        "syncing_origin",
        "some_future_status",
    ] {
        assert!(
            !cache_releasable_at(status, Some("merged"), 0, NOW_MS, 14),
            "{status} is live"
        );
    }
}

#[test]
fn a_feature_released_by_status_needs_no_idleness() {
    assert!(cache_releasable_at(
        "completed",
        Some("merged"),
        NOW_MS,
        NOW_MS,
        14
    ));
}

#[test]
fn an_unset_ttl_is_the_default_and_zero_stays_zero() {
    assert_eq!(cache_idle_ttl_days(None), DEFAULT_CACHE_IDLE_TTL_DAYS);
    assert_eq!(cache_idle_ttl_days(Some(0)), 0);
    assert_eq!(cache_idle_ttl_days(Some(3)), 3);
}

#[test]
fn a_clock_behind_the_activity_is_not_idle() {
    assert!(!idle_past_ttl(NOW_MS + 30 * DAY_MS, NOW_MS, 1));
}

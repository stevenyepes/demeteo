//! Which `feature_status` row a paired Hub still needs to see a run end.
//!
//! The Hub knows a Feature only from its `run_events` rows (docs/HUB.md §5)
//! and reads a run whose log holds no terminal `feature_status` as running
//! for good. A status written to `features` without its event therefore
//! never reaches it — a merged PR, a detached run's shadow, an archive or a
//! delete all once did exactly that, and the Hub showed finished runs as
//! running and removed ones as waiting on a gate.

use demeteo_hub_protocol::RunEventKind;

use super::projection::feature_kind;

/// How `status` reads on the Hub once the run is over, or `None` while it is
/// live. `archived` and `deleted` are `None` too: the Hub has no kind for
/// them, which is what [`removal_logs_cancel`] answers for.
fn ending(status: &str) -> Option<RunEventKind> {
    feature_kind(status).filter(|kind| *kind != RunEventKind::FeatureStarted)
}

/// The `feature_status` value each ending is logged under.
fn logged_as(kind: RunEventKind) -> Option<&'static str> {
    match kind {
        RunEventKind::FeatureCompleted => Some("completed"),
        RunEventKind::FeatureFailed => Some("failed"),
        RunEventKind::FeatureCancelled => Some("cancelled"),
        _ => None,
    }
}

/// Whether archiving or deleting a feature whose status was `before` must
/// first log it as `cancelled`.
///
/// A removed run is over, and no gate decision can reach it, but the Hub
/// keeps the last thing it saw — for a run removed mid-flight, often a gate
/// waiting on a human. A run that had already ended keeps how it ended.
pub fn removal_logs_cancel(before: &str) -> bool {
    ending(before).is_none() && !is_removed(before)
}

/// Whether a feature at `status` has a run that can still ask a human
/// anything. An ended run's gate rows can outlive it — a merge or a removal
/// settles the feature without deciding them — and the Hub's fold already
/// drops every gate of an ended run, so the desktop's own list does the same
/// rather than offer a decision nothing would apply.
pub fn run_has_ended(status: &str) -> bool {
    ending(status).is_some() || is_removed(status)
}

fn is_removed(status: &str) -> bool {
    matches!(status, "archived" | "deleted")
}

/// The `feature_status` to append so a feature's log ends the way its row
/// does, or `None` when it already does.
///
/// `logged` is the status of its latest `feature_status` row, and
/// `has_events` whether it has any row at all. A feature with none was never
/// shown to a Hub, so nothing is appended that would make it appear there
/// now. A live status is left to the run that owns it.
pub fn status_backfill(
    current: &str,
    logged: Option<&str>,
    has_events: bool,
) -> Option<&'static str> {
    if !has_events {
        return None;
    }
    let logged_end = logged.and_then(ending);
    if is_removed(current) {
        return logged_end.is_none().then_some("cancelled");
    }
    let end = ending(current)?;
    if logged_end == Some(end) {
        return None;
    }
    logged_as(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_removal_logs_a_cancel_only_for_a_run_that_had_not_ended() {
        for live in [
            "running",
            "awaiting_gate",
            "gated",
            "bootstrapping",
            "interrupted",
        ] {
            assert!(removal_logs_cancel(live), "{live}");
        }
        for ended in [
            "completed",
            "awaiting_mr",
            "failed",
            "cancelled",
            "archived",
            "deleted",
        ] {
            assert!(!removal_logs_cancel(ended), "{ended}");
        }
    }

    #[test]
    fn a_run_has_ended_once_it_settled_or_was_removed() {
        for ended in [
            "completed",
            "awaiting_mr",
            "failed",
            "cancelled",
            "archived",
            "deleted",
        ] {
            assert!(run_has_ended(ended), "{ended}");
        }
        for live in ["running", "awaiting_gate", "gated", "needs-credentials"] {
            assert!(!run_has_ended(live), "{live}");
        }
    }

    #[test]
    fn a_feature_never_logged_is_never_backfilled() {
        assert_eq!(status_backfill("completed", None, false), None);
        assert_eq!(status_backfill("deleted", None, false), None);
    }

    #[test]
    fn an_ended_feature_whose_log_never_said_so_is_backfilled_with_its_end() {
        assert_eq!(status_backfill("completed", None, true), Some("completed"));
        assert_eq!(
            status_backfill("completed", Some("awaiting_gate"), true),
            Some("completed")
        );
        assert_eq!(
            status_backfill("failed", Some("running"), true),
            Some("failed")
        );
        assert_eq!(
            status_backfill("cancelled", Some("running"), true),
            Some("cancelled")
        );
    }

    #[test]
    fn an_ended_feature_whose_log_ends_another_way_is_corrected() {
        assert_eq!(
            status_backfill("completed", Some("failed"), true),
            Some("completed")
        );
    }

    #[test]
    fn a_log_already_ending_the_same_way_is_left_alone() {
        assert_eq!(status_backfill("completed", Some("completed"), true), None);
        assert_eq!(
            status_backfill("completed", Some("awaiting_mr"), true),
            None
        );
        assert_eq!(
            status_backfill("awaiting_mr", Some("completed"), true),
            None
        );
        assert_eq!(status_backfill("failed", Some("failed"), true), None);
    }

    #[test]
    fn a_live_feature_is_left_to_its_run() {
        for live in ["running", "awaiting_gate", "gated", "needs-credentials"] {
            assert_eq!(status_backfill(live, Some("failed"), true), None, "{live}");
            assert_eq!(status_backfill(live, None, true), None, "{live}");
        }
    }

    #[test]
    fn a_removed_feature_is_cancelled_unless_its_log_had_already_ended() {
        assert_eq!(status_backfill("deleted", None, true), Some("cancelled"));
        assert_eq!(
            status_backfill("deleted", Some("awaiting_gate"), true),
            Some("cancelled")
        );
        assert_eq!(
            status_backfill("archived", Some("running"), true),
            Some("cancelled")
        );
        assert_eq!(status_backfill("archived", Some("completed"), true), None);
        assert_eq!(status_backfill("deleted", Some("failed"), true), None);
    }
}

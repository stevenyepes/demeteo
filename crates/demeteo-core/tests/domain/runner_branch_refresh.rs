// Tests extracted from `crates/demeteo-core/src/domain/runner_branch_refresh.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;

const OLD: &str = "1111111111111111111111111111111111111111";
const NEW: &str = "2222222222222222222222222222222222222222";

fn facts<'a>(local: Option<&'a str>, origin: Option<&'a str>) -> BranchFacts<'a> {
    BranchFacts {
        local_tip: local,
        origin_tip: origin,
        local_in_origin: true,
        checkout: None,
    }
}

#[test]
fn a_run_that_may_still_move_the_branch_is_refused_before_anything_else() {
    for status in ["pending", "running", "needs-credentials", "something-new"] {
        assert_eq!(
            plan_refresh(status, &facts(Some(OLD), Some(NEW))),
            RefreshPlan::Refuse(RefreshRefusal::RunActive {
                status: status.to_string()
            }),
            "{status}"
        );
    }
}

#[test]
fn every_settled_status_admits_a_refresh() {
    for status in [
        "completed",
        "awaiting_mr",
        "pr_ready",
        "failed",
        "interrupted",
        "cancelled",
        "over-budget",
    ] {
        assert_eq!(refusal_for_run(status), None, "{status}");
    }
}

#[test]
fn an_unpublished_runner_branch_moves_by_ref_with_the_old_tip_as_the_check() {
    assert_eq!(
        plan_refresh("completed", &facts(Some(OLD), Some(NEW))),
        RefreshPlan::MoveRef {
            from: OLD.to_string(),
            to: NEW.to_string()
        }
    );
}

#[test]
fn a_clean_checkout_fast_forwards_in_its_own_worktree() {
    let mut f = facts(Some(OLD), Some(NEW));
    f.checkout = Some(Checkout {
        worktree: "/w/wt",
        dirty: false,
    });

    assert_eq!(
        plan_refresh("failed", &f),
        RefreshPlan::FastForwardCheckout {
            worktree: "/w/wt".to_string(),
            from: OLD.to_string(),
            to: NEW.to_string()
        }
    );
}

#[test]
fn a_dirty_checkout_is_refused_not_reset() {
    let mut f = facts(Some(OLD), Some(NEW));
    f.checkout = Some(Checkout {
        worktree: "/w/wt",
        dirty: true,
    });

    assert_eq!(
        plan_refresh("failed", &f),
        RefreshPlan::Refuse(RefreshRefusal::DirtyCheckout {
            worktree: "/w/wt".to_string()
        })
    );
}

#[test]
fn runner_commits_origin_lacks_are_refused() {
    let mut f = facts(Some(OLD), Some(NEW));
    f.local_in_origin = false;

    assert_eq!(
        plan_refresh("completed", &f),
        RefreshPlan::Refuse(RefreshRefusal::UnpushedWork {
            local: OLD.to_string(),
            origin: NEW.to_string()
        })
    );
}

/// Equal tips are up to date even when the ancestry read said otherwise — the
/// read is only meaningful for two different commits.
#[test]
fn equal_tips_are_up_to_date() {
    let mut f = facts(Some(NEW), Some(NEW));
    f.local_in_origin = false;

    assert_eq!(
        plan_refresh("completed", &f),
        RefreshPlan::UpToDate {
            tip: NEW.to_string()
        }
    );
}

#[test]
fn a_branch_missing_on_either_side_is_refused() {
    assert_eq!(
        plan_refresh("completed", &facts(Some(OLD), None)),
        RefreshPlan::Refuse(RefreshRefusal::NotOnOrigin)
    );
    assert_eq!(
        plan_refresh("completed", &facts(None, Some(NEW))),
        RefreshPlan::Refuse(RefreshRefusal::NoLocalBranch)
    );
}

#[test]
fn the_worktree_holding_the_branch_is_found_by_its_exact_name() {
    let porcelain = "worktree /w/repo\nHEAD aaaa\nbranch refs/heads/main\n\n\
                     worktree /w/repo_wt\nHEAD bbbb\nbranch refs/heads/feat/a-b\n\n\
                     worktree /w/repo_wt2\nHEAD cccc\nbranch refs/heads/feat/a\n";

    assert_eq!(
        checkout_holding(porcelain, "feat/a").as_deref(),
        Some("/w/repo_wt2")
    );
    assert_eq!(checkout_holding(porcelain, "feat/z"), None);
}

#[test]
fn the_outcome_is_tagged_on_the_wire() {
    let json = serde_json::to_value(BranchRefreshOutcome::Refused {
        reason: "x".to_string(),
    })
    .unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "outcome": "refused", "reason": "x" })
    );
}

fn mark(status: SyncSessionStatus, sha: &str, pushed_at: Option<i64>) -> SyncMark {
    SyncMark {
        status,
        merge_commit_sha: Some(sha.to_string()),
        pushed_at,
    }
}

#[test]
fn a_sync_that_put_something_on_origin_is_news_once() {
    let merged = mark(SyncSessionStatus::Merged, NEW, None);
    let published = mark(SyncSessionStatus::Resolved, NEW, Some(5));
    let unpublished = mark(SyncSessionStatus::Resolved, NEW, None);

    assert!(newly_on_origin(None, Some(&merged)));
    assert!(newly_on_origin(Some(&unpublished), Some(&published)));
    assert!(newly_on_origin(
        Some(&mark(SyncSessionStatus::Merged, OLD, None)),
        Some(&merged)
    ));

    assert!(!newly_on_origin(Some(&merged), Some(&merged)));
    assert!(!newly_on_origin(Some(&published), Some(&published)));
    assert!(!newly_on_origin(None, Some(&unpublished)));
    assert!(!newly_on_origin(
        None,
        Some(&mark(SyncSessionStatus::UpToDate, NEW, None))
    ));
    assert!(!newly_on_origin(Some(&merged), None));
}

#[test]
fn a_refresh_that_landed_is_not_news() {
    let updated = Ok(BranchRefreshOutcome::Updated {
        from: OLD.to_string(),
        to: NEW.to_string(),
    });
    let current = Ok(BranchRefreshOutcome::UpToDate {
        tip: NEW.to_string(),
    });

    assert_eq!(refresh_notice("feat/a", &updated), None);
    assert_eq!(refresh_notice("feat/a", &current), None);
}

#[test]
fn a_refusal_an_old_runner_and_a_failure_each_say_what_follows() {
    let refused = refresh_notice(
        "feat/a",
        &Ok(BranchRefreshOutcome::Refused {
            reason: "the run is still running on the runner".to_string(),
        }),
    )
    .unwrap();
    let old = refresh_notice(
        "feat/a",
        &Err("unknown method: refresh_feature_branch".to_string()),
    )
    .unwrap();
    let failed = refresh_notice("feat/a", &Err("ssh: timed out".to_string())).unwrap();

    assert!(refused.contains("origin/feat/a"), "{refused}");
    assert!(refused.contains("still running"), "{refused}");
    assert!(
        refused.contains("refused rather than overwrite"),
        "{refused}"
    );
    assert!(old.contains("too old"), "{old}");
    assert!(old.contains("will overwrite the sync"), "{old}");
    assert!(failed.contains("ssh: timed out"), "{failed}");
}

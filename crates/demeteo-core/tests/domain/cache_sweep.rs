use std::collections::HashSet;

use super::*;

const NOW: u64 = 100 * DAY_SECS;
const DAY_SECS: u64 = 24 * 60 * 60;
const TTL_DAYS: u32 = 14;
const NOW_MS: i64 = (NOW * 1000) as i64;
const DAY_MS: i64 = (DAY_SECS * 1000) as i64;

fn dir(name: &str, modified_secs: Option<u64>) -> Sibling {
    Sibling {
        name: name.to_string(),
        is_dir: true,
        modified_secs,
    }
}

fn old_dir(name: &str) -> Sibling {
    dir(name, Some(NOW - WORKTREE_GRACE_SECS - 1))
}

fn feature(branch: &str, status: &str, mr_state: Option<&str>) -> KnownFeature {
    KnownFeature {
        branch: branch.to_string(),
        status: status.to_string(),
        mr_state: mr_state.map(str::to_string),
        last_activity_ms: NOW_MS,
    }
}

fn idle_feature(branch: &str, status: &str, idle_ms: i64) -> KnownFeature {
    KnownFeature {
        last_activity_ms: NOW_MS - idle_ms,
        ..feature(branch, status, None)
    }
}

const RECENT_SESSIONS: SessionActivity = SessionActivity {
    holding_worktree: false,
    last_activity_ms: Some(NOW_MS),
};

fn observe<'a>(
    siblings: &'a [Sibling],
    registered: Option<&'a HashSet<String>>,
    features: &'a [KnownFeature],
) -> Observation<'a> {
    Observation {
        clone_name: "demeteo",
        other_repos: &[],
        siblings,
        registered,
        features,
        default_branch: "master",
        sessions: Some(RECENT_SESSIONS),
        cache_idle_ttl_days: TTL_DAYS,
        now_secs: NOW,
    }
}

fn plan_with(
    siblings: &[Sibling],
    registered: Option<&HashSet<String>>,
    features: &[KnownFeature],
) -> Vec<SweepAction> {
    plan(&observe(siblings, registered, features))
}

fn verdict_of(actions: &[SweepAction], name: &str) -> Option<(Verdict, Reason)> {
    actions
        .iter()
        .find(|a| a.name == name)
        .map(|a| (a.verdict, a.reason))
}

#[test]
fn a_released_feature_s_cache_is_deleted() {
    let actions = plan_with(
        &[old_dir("demeteo_cache_feature-f1")],
        None,
        &[feature("feature/f1", "completed", Some("merged"))],
    );
    assert_eq!(
        verdict_of(&actions, "demeteo_cache_feature-f1"),
        Some((Verdict::Delete, Reason::FeatureReleased))
    );
    assert_eq!(actions[0].kind, SiblingKind::Cache);
}

#[test]
fn a_cache_is_kept_while_any_feature_on_its_branch_may_run() {
    let actions = plan_with(
        &[old_dir("demeteo_cache_feature-f1")],
        None,
        &[
            feature("feature/f1", "deleted", None),
            feature("feature/f1", "failed", None),
        ],
    );
    assert_eq!(
        verdict_of(&actions, "demeteo_cache_feature-f1"),
        Some((Verdict::Keep, Reason::FeatureMayRun))
    );
}

#[test]
fn the_default_branch_cache_is_kept_even_when_a_released_feature_names_it() {
    let registered = HashSet::new();
    let actions = plan_with(
        &[old_dir("demeteo_cache_master")],
        Some(&registered),
        &[feature("master", "deleted", None)],
    );
    assert_eq!(
        verdict_of(&actions, "demeteo_cache_master"),
        Some((Verdict::Keep, Reason::SessionsRecent))
    );
}

fn master_verdict(obs: Observation<'_>) -> Option<(Verdict, Reason)> {
    verdict_of(&plan(&obs), "demeteo_cache_master")
}

fn idle_sessions(idle_ms: i64) -> SessionActivity {
    SessionActivity {
        holding_worktree: false,
        last_activity_ms: Some(NOW_MS - idle_ms),
    }
}

#[test]
fn the_default_branch_cache_is_released_once_every_session_is_idle_past_the_ttl() {
    let registered = HashSet::new();
    let siblings = [dir("demeteo_cache_master", Some(0))];
    let ttl_ms = i64::from(TTL_DAYS) * DAY_MS;
    assert_eq!(
        master_verdict(Observation {
            sessions: Some(idle_sessions(ttl_ms + 1)),
            ..observe(&siblings, Some(&registered), &[])
        }),
        Some((Verdict::Delete, Reason::SessionsIdle))
    );
    assert_eq!(
        master_verdict(Observation {
            sessions: Some(idle_sessions(ttl_ms)),
            ..observe(&siblings, Some(&registered), &[])
        }),
        Some((Verdict::Keep, Reason::SessionsRecent))
    );
}

#[test]
fn the_default_branch_cache_is_kept_while_a_session_holds_a_worktree_however_idle() {
    let long_idle = SessionActivity {
        holding_worktree: true,
        ..idle_sessions(365 * DAY_MS)
    };
    let siblings = [dir("demeteo_cache_master", Some(0))];
    let none = HashSet::new();
    assert_eq!(
        master_verdict(Observation {
            sessions: Some(long_idle),
            ..observe(&siblings, Some(&none), &[])
        }),
        Some((Verdict::Keep, Reason::SessionHoldsWorktree))
    );

    for name in ["demeteo_wt_ask-t1", "demeteo_wt_discovery-d1"] {
        let registered: HashSet<String> = [name.to_string()].into();
        assert_eq!(
            master_verdict(Observation {
                sessions: Some(idle_sessions(365 * DAY_MS)),
                ..observe(&siblings, Some(&registered), &[])
            }),
            Some((Verdict::Keep, Reason::SessionHoldsWorktree)),
            "{name}"
        );
    }
}

#[test]
fn a_feature_worktree_does_not_hold_the_default_branch_cache() {
    let registered: HashSet<String> = ["demeteo_wt_s1".to_string(), "demeteo".to_string()].into();
    let siblings = [dir("demeteo_cache_master", Some(0))];
    assert_eq!(
        master_verdict(Observation {
            sessions: Some(idle_sessions(365 * DAY_MS)),
            ..observe(&siblings, Some(&registered), &[])
        }),
        Some((Verdict::Delete, Reason::SessionsIdle))
    );
}

#[test]
fn the_default_branch_cache_is_kept_when_anything_it_is_judged_on_is_unknown() {
    let registered = HashSet::new();
    let aged = [dir("demeteo_cache_master", Some(0))];
    let ageless = [dir("demeteo_cache_master", None)];
    let idle = Some(idle_sessions(365 * DAY_MS));
    assert_eq!(
        master_verdict(Observation {
            sessions: None,
            ..observe(&aged, Some(&registered), &[])
        }),
        Some((Verdict::Keep, Reason::SessionsUnknown))
    );
    assert_eq!(
        master_verdict(Observation {
            sessions: idle,
            ..observe(&aged, None, &[])
        }),
        Some((Verdict::Keep, Reason::RegistrationUnknown))
    );
    assert_eq!(
        master_verdict(Observation {
            sessions: Some(SessionActivity {
                holding_worktree: false,
                last_activity_ms: None,
            }),
            ..observe(&ageless, Some(&registered), &[])
        }),
        Some((Verdict::Keep, Reason::AgeUnknown))
    );
}

#[test]
fn with_no_session_left_the_default_branch_cache_ages_by_its_own_mtime() {
    let registered = HashSet::new();
    let no_sessions = Some(SessionActivity {
        holding_worktree: false,
        last_activity_ms: None,
    });
    let fresh = [dir("demeteo_cache_master", Some(NOW - DAY_SECS))];
    let stale = [dir("demeteo_cache_master", Some(NOW - 30 * DAY_SECS))];
    assert_eq!(
        master_verdict(Observation {
            sessions: no_sessions,
            ..observe(&fresh, Some(&registered), &[])
        }),
        Some((Verdict::Keep, Reason::SessionsRecent))
    );
    assert_eq!(
        master_verdict(Observation {
            sessions: no_sessions,
            ..observe(&stale, Some(&registered), &[])
        }),
        Some((Verdict::Delete, Reason::SessionsIdle))
    );
}

#[test]
fn a_zero_ttl_keeps_the_default_branch_cache_forever() {
    let registered = HashSet::new();
    let siblings = [dir("demeteo_cache_master", Some(0))];
    assert_eq!(
        master_verdict(Observation {
            sessions: Some(idle_sessions(365 * DAY_MS)),
            cache_idle_ttl_days: 0,
            ..observe(&siblings, Some(&registered), &[])
        }),
        Some((Verdict::Keep, Reason::DefaultBranchCache))
    );
}

#[test]
fn a_feature_idle_past_the_ttl_releases_its_cache() {
    let ttl_ms = i64::from(TTL_DAYS) * DAY_MS;
    let actions = plan_with(
        &[
            old_dir("demeteo_cache_feature-idle"),
            old_dir("demeteo_cache_feature-fresh"),
            old_dir("demeteo_cache_feature-mixed"),
        ],
        None,
        &[
            idle_feature("feature/idle", "failed", ttl_ms + 1),
            idle_feature("feature/fresh", "failed", ttl_ms),
            idle_feature("feature/mixed", "cancelled", ttl_ms + 1),
            feature("feature/mixed", "deleted", None),
        ],
    );
    assert_eq!(
        verdict_of(&actions, "demeteo_cache_feature-idle"),
        Some((Verdict::Delete, Reason::FeatureIdle))
    );
    assert_eq!(
        verdict_of(&actions, "demeteo_cache_feature-fresh"),
        Some((Verdict::Keep, Reason::FeatureMayRun))
    );
    assert_eq!(
        verdict_of(&actions, "demeteo_cache_feature-mixed"),
        Some((Verdict::Delete, Reason::FeatureIdle))
    );
}

#[test]
fn a_live_feature_keeps_its_cache_however_long_it_has_been_idle() {
    let actions = plan_with(
        &[old_dir("demeteo_cache_feature-f1")],
        None,
        &[
            idle_feature("feature/f1", "failed", 365 * DAY_MS),
            idle_feature("feature/f1", "awaiting_gate", 365 * DAY_MS),
        ],
    );
    assert_eq!(
        verdict_of(&actions, "demeteo_cache_feature-f1"),
        Some((Verdict::Keep, Reason::FeatureMayRun))
    );
}

#[test]
fn a_cache_no_feature_claims_is_reported_not_deleted() {
    let actions = plan_with(&[old_dir("demeteo_cache_old-prefix-f9")], None, &[]);
    assert_eq!(
        verdict_of(&actions, "demeteo_cache_old-prefix-f9"),
        Some((Verdict::Unknown, Reason::NoKnownFeature))
    );
}

#[test]
fn an_old_unregistered_worktree_is_deleted_sync_worktrees_included() {
    let registered = HashSet::new();
    let actions = plan_with(
        &[
            old_dir("demeteo_wt_s1"),
            old_dir("demeteo_wt_sync_feature-f1"),
        ],
        Some(&registered),
        &[],
    );
    for name in ["demeteo_wt_s1", "demeteo_wt_sync_feature-f1"] {
        assert_eq!(
            verdict_of(&actions, name),
            Some((Verdict::Delete, Reason::Unregistered)),
            "{name}"
        );
    }
}

#[test]
fn a_registered_worktree_is_kept() {
    let registered: HashSet<String> = ["demeteo_wt_s1".to_string()].into();
    let actions = plan_with(&[old_dir("demeteo_wt_s1")], Some(&registered), &[]);
    assert_eq!(
        verdict_of(&actions, "demeteo_wt_s1"),
        Some((Verdict::Keep, Reason::Registered))
    );
}

#[test]
fn no_worktree_is_deleted_when_git_could_not_list_them() {
    let actions = plan_with(&[old_dir("demeteo_wt_s1")], None, &[]);
    assert_eq!(
        verdict_of(&actions, "demeteo_wt_s1"),
        Some((Verdict::Keep, Reason::RegistrationUnknown))
    );
}

#[test]
fn an_unregistered_worktree_inside_the_grace_or_of_unknown_age_is_kept() {
    let registered = HashSet::new();
    let actions = plan_with(
        &[
            dir("demeteo_wt_fresh", Some(NOW - WORKTREE_GRACE_SECS + 1)),
            dir("demeteo_wt_future", Some(NOW + 600)),
            dir("demeteo_wt_ageless", None),
        ],
        Some(&registered),
        &[],
    );
    assert_eq!(
        verdict_of(&actions, "demeteo_wt_fresh"),
        Some((Verdict::Keep, Reason::WithinGrace))
    );
    assert_eq!(
        verdict_of(&actions, "demeteo_wt_future"),
        Some((Verdict::Keep, Reason::WithinGrace))
    );
    assert_eq!(
        verdict_of(&actions, "demeteo_wt_ageless"),
        Some((Verdict::Keep, Reason::AgeUnknown))
    );
}

#[test]
fn only_this_clone_s_prefixed_directories_are_considered() {
    let registered = HashSet::new();
    let file = Sibling {
        is_dir: false,
        ..old_dir("demeteo_wt_stray-file")
    };
    let actions = plan_with(
        &[
            old_dir("demeteo"),
            old_dir("demeteo2_cache_feature-f1"),
            old_dir("demeteo2_wt_s1"),
            old_dir("demeteo_cache_"),
            old_dir("demeteo_wt_"),
            old_dir("demeteo-notes"),
            file,
        ],
        Some(&registered),
        &[feature("feature/f1", "deleted", None)],
    );
    assert!(actions.is_empty(), "{actions:?}");
}

#[test]
fn a_sibling_clone_s_own_directories_are_left_to_it() {
    let registered = HashSet::new();
    let other_repos = ["demeteo_wt_tools".to_string()];
    let siblings = [
        old_dir("demeteo_wt_tools"),
        old_dir("demeteo_wt_tools_wt_s1"),
        old_dir("demeteo_wt_s2"),
    ];
    let actions = plan(&Observation {
        other_repos: &other_repos,
        ..observe(&siblings, Some(&registered), &[])
    });
    assert_eq!(
        verdict_of(&actions, "demeteo_wt_tools"),
        Some((Verdict::Keep, Reason::OtherRepository))
    );
    assert_eq!(
        verdict_of(&actions, "demeteo_wt_tools_wt_s1"),
        Some((Verdict::Keep, Reason::OtherRepository))
    );
    assert_eq!(
        verdict_of(&actions, "demeteo_wt_s2"),
        Some((Verdict::Delete, Reason::Unregistered))
    );
}

#[test]
fn path_basename_reads_any_host_s_separators() {
    assert_eq!(
        path_basename("/home/u/repos/demeteo_wt_s1"),
        "demeteo_wt_s1"
    );
    assert_eq!(
        path_basename("C:/Users/u/repos/demeteo_wt_s1"),
        "demeteo_wt_s1"
    );
    assert_eq!(
        path_basename(r"C:\Users\u\repos\demeteo_wt_s1"),
        "demeteo_wt_s1"
    );
    assert_eq!(
        path_basename("/home/u/repos/demeteo_wt_s1/"),
        "demeteo_wt_s1"
    );
}

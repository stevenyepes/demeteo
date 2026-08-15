//! The four answers a run's starting point has to give, with no repository
//! under them. `super` is `crate::domain::feature_origin`.

use super::*;

fn branch(base: &str) -> FeatureOrigin {
    FeatureOrigin::Branch {
        base: base.to_string(),
    }
}

fn pull_request(number: u32) -> FeatureOrigin {
    FeatureOrigin::Ref {
        fetch_spec: format!("refs/pull/{number}/head"),
        label: format!("PR #{number}"),
    }
}

// ── Where the cut starts ─────────────────────────────────────────────────────

#[test]
fn the_default_branch_arm_cuts_from_the_remote_tracking_ref() {
    assert_eq!(
        FeatureOrigin::DefaultBranch.start_point("main"),
        "origin/main",
        "cutting from the local ref would base the run on whatever this clone last pulled"
    );
}

#[test]
fn a_named_base_cuts_from_that_branch_and_not_the_default() {
    assert_eq!(
        branch("release/2.0").start_point("main"),
        "origin/release/2.0"
    );
}

#[test]
fn a_fetched_ref_cuts_from_where_the_fetch_put_it() {
    assert_eq!(
        pull_request(12).start_point("main"),
        pull_request(12).fetch_plan("main").local_ref,
        "the start point is unresolvable unless it names the ref the plan lands in"
    );
}

#[test]
fn a_fetched_ref_lands_outside_refs_heads() {
    let landed = pull_request(12).fetch_plan("main").local_ref;
    assert!(
        !landed.starts_with("refs/heads/"),
        "a local branch would be offered as a base and pushed by a matching-branch push: {landed}"
    );
}

// ── What git has to fetch first ──────────────────────────────────────────────

#[test]
fn the_branch_arms_fetch_the_branch_they_name() {
    assert_eq!(
        FeatureOrigin::DefaultBranch.fetch_plan("trunk"),
        FetchPlan {
            refspec: "trunk".to_string(),
            local_ref: "origin/trunk".to_string(),
        },
    );
    assert_eq!(
        branch("release/2.0").fetch_plan("main"),
        FetchPlan {
            refspec: "release/2.0".to_string(),
            local_ref: "origin/release/2.0".to_string(),
        },
    );
}

#[test]
fn a_fetched_ref_carries_its_destination_in_the_refspec() {
    assert_eq!(
        pull_request(12).fetch_plan("main"),
        FetchPlan {
            refspec: "refs/pull/12/head:refs/demeteo/origins/pull/12/head".to_string(),
            local_ref: "refs/demeteo/origins/pull/12/head".to_string(),
        },
    );
}

#[test]
fn a_gitlab_merge_request_head_is_the_same_shape() {
    let origin = FeatureOrigin::Ref {
        fetch_spec: "refs/merge-requests/7/head".to_string(),
        label: "!7".to_string(),
    };
    assert_eq!(
        origin.fetch_plan("main"),
        FetchPlan {
            refspec: "refs/merge-requests/7/head:refs/demeteo/origins/merge-requests/7/head"
                .to_string(),
            local_ref: "refs/demeteo/origins/merge-requests/7/head".to_string(),
        },
    );
}

// ── Where a PR opened by the run goes ────────────────────────────────────────

#[test]
fn a_run_from_the_default_branch_targets_it() {
    assert_eq!(FeatureOrigin::DefaultBranch.publish_target("main"), "main");
}

#[test]
fn a_run_from_a_named_base_targets_that_base() {
    assert_eq!(branch("release/2.0").publish_target("main"), "release/2.0");
}

#[test]
fn a_run_from_a_fetched_ref_falls_back_to_the_default_branch() {
    assert_eq!(
        pull_request(12).publish_target("main"),
        "main",
        "no host accepts a pull request head as a merge target"
    );
}

// ── The branch name is the origin's one invariant ────────────────────────────

#[test]
fn every_arm_cuts_the_same_branch_name() {
    let named = |origin: FeatureOrigin| origin.branch_to_cut("demeteo/features/", "f-1");
    assert_eq!(named(FeatureOrigin::DefaultBranch), "demeteo/features/f-1");
    assert_eq!(named(branch("release/2.0")), "demeteo/features/f-1");
    assert_eq!(named(pull_request(12)), "demeteo/features/f-1");
}

// ── Persistence ──────────────────────────────────────────────────────────────

fn round_trip(origin: &FeatureOrigin) -> FeatureOrigin {
    let json = serde_json::to_string(origin).expect("serialize");
    serde_json::from_str(&json).expect("deserialize")
}

#[test]
fn every_arm_survives_a_round_trip() {
    for origin in [
        FeatureOrigin::DefaultBranch,
        branch("release/2.0"),
        pull_request(12),
    ] {
        assert_eq!(round_trip(&origin), origin);
    }
}

#[test]
fn the_arms_are_told_apart_by_a_tag_and_not_by_their_fields() {
    assert_eq!(
        serde_json::to_string(&FeatureOrigin::DefaultBranch).expect("serialize"),
        r#"{"kind":"default_branch"}"#,
        "a fieldless arm still needs a discriminator to come back as itself"
    );
}

#[test]
fn a_column_that_was_never_written_reads_as_the_default_branch() {
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(default)]
        origin: FeatureOrigin,
    }

    let null: Option<FeatureOrigin> = serde_json::from_str("null").expect("deserialize");
    let omitted: Row = serde_json::from_str("{}").expect("deserialize");

    for read in [null.unwrap_or_default(), omitted.origin] {
        assert_eq!(
            read,
            FeatureOrigin::DefaultBranch,
            "every run predating this type started from the default branch"
        );
    }
}

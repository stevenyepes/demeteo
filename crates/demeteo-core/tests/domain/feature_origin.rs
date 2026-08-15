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
        plan(&pull_request(12)).local_ref,
        "the start point is unresolvable unless it names the ref the plan lands in"
    );
}

#[test]
fn a_fetched_ref_lands_outside_refs_heads() {
    let landed = plan(&pull_request(12)).local_ref;
    assert!(
        !landed.starts_with("refs/heads/"),
        "a local branch would be offered as a base and pushed by a matching-branch push: {landed}"
    );
}

// ── What git has to fetch first ──────────────────────────────────────────────

fn plan(origin: &FeatureOrigin) -> FetchPlan {
    origin
        .fetch_plan("main")
        .expect("a plan for a well-formed origin")
}

fn spec(refspec: &str) -> Refspec {
    Refspec::try_from(refspec.to_string()).expect("a well-formed refspec")
}

#[test]
fn the_branch_arms_fetch_the_branch_they_name() {
    assert_eq!(
        FeatureOrigin::DefaultBranch
            .fetch_plan("trunk")
            .expect("plan"),
        FetchPlan {
            refspec: spec("trunk"),
            local_ref: "origin/trunk".to_string(),
        },
    );
    assert_eq!(
        plan(&branch("release/2.0")),
        FetchPlan {
            refspec: spec("release/2.0"),
            local_ref: "origin/release/2.0".to_string(),
        },
    );
}

#[test]
fn a_fetched_ref_carries_its_destination_in_the_refspec() {
    assert_eq!(
        plan(&pull_request(12)),
        FetchPlan {
            refspec: spec("+refs/pull/12/head:refs/demeteo/origins/pull/12/head"),
            local_ref: "refs/demeteo/origins/pull/12/head".to_string(),
        },
    );
}

#[test]
fn a_fetched_ref_forces_its_destination() {
    assert!(
        plan(&pull_request(12)).refspec.as_str().starts_with('+'),
        "a review re-run against a force-pushed head fetches a non-fast-forward, \
         which git rejects without the +, failing the whole run"
    );
}

#[test]
fn a_gitlab_merge_request_head_is_the_same_shape() {
    let origin = FeatureOrigin::Ref {
        fetch_spec: "refs/merge-requests/7/head".to_string(),
        label: "!7".to_string(),
    };
    assert_eq!(
        plan(&origin),
        FetchPlan {
            refspec: spec("+refs/merge-requests/7/head:refs/demeteo/origins/merge-requests/7/head"),
            local_ref: "refs/demeteo/origins/merge-requests/7/head".to_string(),
        },
    );
}

// ── What git must never be handed ────────────────────────────────────────────

#[test]
fn a_refspec_git_would_read_as_an_option_is_refused() {
    for hostile in [
        "--upload-pack=touch /tmp/pwned",
        "-c",
        "+--upload-pack=touch /tmp/pwned",
    ] {
        assert!(
            Refspec::try_from(hostile.to_string()).is_err(),
            "git runs --upload-pack's argument: {hostile}"
        );
    }
    assert!(
        Refspec::try_from("refs/pull/1/head refs/heads/main".to_string()).is_err(),
        "one argv element naming two refs is not one refspec"
    );
    assert!(Refspec::try_from(String::new()).is_err());
    assert!(Refspec::try_from("+".to_string()).is_err());
}

#[test]
fn a_hostile_refspec_cannot_be_deserialised_either() {
    assert!(
        serde_json::from_str::<Refspec>(r#""--upload-pack=id""#).is_err(),
        "a refspec arriving as JSON is exactly the one this repository did not write"
    );
}

#[test]
fn a_branch_named_like_an_option_has_no_fetch_plan() {
    assert!(
        branch("--upload-pack=touch /tmp/pwned")
            .fetch_plan("main")
            .is_err(),
        "the base is a string a person typed, and it lands in argv"
    );
    assert!(FeatureOrigin::DefaultBranch.fetch_plan("-x").is_err());
}

#[test]
fn a_fetched_origin_must_name_a_full_ref() {
    let shorthand = FeatureOrigin::Ref {
        fetch_spec: "main".to_string(),
        label: "main".to_string(),
    };
    assert!(
        shorthand.fetch_plan("main").is_err(),
        "anything else lands under the private namespace looking like a PR head this \
         repository never fetched"
    );
}

// ── The strictness each arm is owed ──────────────────────────────────────────

#[test]
fn only_a_fetched_ref_makes_its_fetch_load_bearing() {
    assert_eq!(
        FeatureOrigin::DefaultBranch
            .branch_cut("main")
            .expect("cut"),
        BranchCut::FromDefaultBranch
    );
    assert_eq!(
        branch("release/2.0").branch_cut("main").expect("cut"),
        BranchCut::FromRemoteBranch {
            refspec: spec("release/2.0"),
            start_point: "origin/release/2.0".to_string(),
        },
        "origin/release/2.0 is already in the clone, so an unreachable origin is stale, not fatal"
    );
    assert_eq!(
        pull_request(12).branch_cut("main").expect("cut"),
        BranchCut::FromFetchedRef {
            refspec: spec("+refs/pull/12/head:refs/demeteo/origins/pull/12/head"),
            start_point: "refs/demeteo/origins/pull/12/head".to_string(),
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

// ── What the published commit parents onto ───────────────────────────────────

#[test]
fn a_run_from_the_default_branch_squashes_onto_it() {
    assert_eq!(FeatureOrigin::DefaultBranch.squash_base("main"), "main");
}

#[test]
fn a_run_from_a_named_base_squashes_onto_that_base() {
    assert_eq!(branch("release/2.0").squash_base("main"), "release/2.0");
}

#[test]
fn a_run_from_a_fetched_ref_squashes_onto_the_ref_it_was_cut_from() {
    assert_eq!(
        pull_request(12).squash_base("main"),
        pull_request(12).start_point("main"),
        "any other parent collapses the pull request's own commits into the run's"
    );
    assert_ne!(
        pull_request(12).squash_base("main"),
        pull_request(12).publish_target("main"),
        "the branch a stacked PR targets is not the commit it is stacked on"
    );
}

// ── What the run measures itself against ─────────────────────────────────────

#[test]
fn a_named_base_is_the_run_s_base() {
    assert_eq!(branch("release/2.0").base_branch(None), Some("release/2.0"));
}

#[test]
fn a_fetched_ref_has_no_base_until_the_launcher_names_one() {
    assert_eq!(
        pull_request(12).base_branch(None),
        None,
        "a pull request head is not a branch anything merges into"
    );
    assert_eq!(
        pull_request(12).base_branch(Some("develop")),
        Some("develop")
    );
}

#[test]
fn a_default_branch_run_ignores_a_review_base() {
    assert_eq!(
        FeatureOrigin::DefaultBranch.base_branch(Some("develop")),
        None,
        "this arm's fetch and cut are the default branch; a review base here would move the cut"
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
fn a_stored_but_unreadable_origin_is_not_the_default_branch() {
    let corrupt = FeatureOrigin::from_column(Some(r#"{"kind":"from_the_moon"}"#));
    assert!(
        corrupt.is_err(),
        "reading it as the default branch resumes the run on the wrong branch, diffs it \
         against the wrong tree and squashes it onto the wrong parent"
    );
    assert!(FeatureOrigin::from_column(Some("{ not json")).is_err());
}

#[test]
fn an_absent_column_still_reads_as_the_default_branch() {
    for absent in [None, Some(""), Some("   ")] {
        assert_eq!(
            FeatureOrigin::from_column(absent).expect("absent is not corrupt"),
            FeatureOrigin::DefaultBranch,
            "every run predating the column started from the default branch"
        );
    }
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

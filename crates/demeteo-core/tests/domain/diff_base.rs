//! What a run is diffed against, with no repository under it. `super` is
//! `crate::domain::diff_base`.

use super::*;

fn from_branch(base: &str) -> FeatureOrigin {
    FeatureOrigin::Branch {
        base: base.to_string(),
    }
}

fn from_pull_request() -> FeatureOrigin {
    FeatureOrigin::Ref {
        fetch_spec: "refs/pull/7/head".to_string(),
        label: "PR #7".to_string(),
    }
}

#[test]
fn the_declared_base_outranks_the_project_default() {
    assert_eq!(
        resolve(Some("release/2.1"), &FeatureOrigin::DefaultBranch, "main"),
        Some("release/2.1"),
        "a run that declared its base is reviewed against it, not against main"
    );
}

#[test]
fn no_declared_base_falls_through_to_the_project_default() {
    assert_eq!(
        resolve(None, &FeatureOrigin::DefaultBranch, "main"),
        Some("main")
    );
}

#[test]
fn an_unset_project_default_names_no_branch_at_all() {
    assert_eq!(
        resolve(None, &FeatureOrigin::DefaultBranch, ""),
        None,
        "an empty default is not a branch name; guessing one is what this returns None to prevent"
    );
    assert_eq!(
        resolve(Some("  "), &FeatureOrigin::DefaultBranch, " "),
        None
    );
}

#[test]
fn a_run_cut_from_a_named_branch_is_measured_against_it() {
    assert_eq!(
        resolve(None, &from_branch("release/2.0"), "main"),
        Some("release/2.0"),
        "measuring it against main would count every commit release/2.0 is missing as this run's work"
    );
}

#[test]
fn the_declared_base_also_outranks_the_branch_the_run_was_cut_from() {
    assert_eq!(
        resolve(Some("release/2.1"), &from_branch("release/2.0"), "main"),
        Some("release/2.1")
    );
}

#[test]
fn a_pull_request_head_supplies_no_base_of_its_own() {
    assert_eq!(
        resolve(None, &from_pull_request(), "main"),
        Some("main"),
        "a fetched PR head is not a branch anything can be diffed against"
    );
    assert_eq!(
        resolve(Some("release/2.1"), &from_pull_request(), "main"),
        Some("release/2.1")
    );
}

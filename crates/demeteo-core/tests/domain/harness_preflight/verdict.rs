//! Which verdicts stop a launch, and what the user is told when one does.

use super::*;

#[test]
fn a_project_with_nothing_configured_still_launches() {
    let verdict = PreflightVerdict::NotConfigured;

    assert!(verdict.detail().is_some());
    assert_eq!(verdict.launch_refusal(), None);
}

#[test]
fn every_binary_resolving_launches() {
    let verdict = PreflightVerdict::Resolved {
        probed: vec!["cargo".to_string()],
    };

    assert_eq!(verdict.launch_refusal(), None);
}

#[test]
fn a_missing_binary_refuses_with_the_sentence_the_stepper_showed() {
    let verdict = PreflightVerdict::MissingBinaries {
        missing: vec!["cargo".to_string()],
    };

    let refusal = verdict.launch_refusal().expect("a missing binary blocks");
    assert_eq!(Some(refusal.clone()), verdict.detail());
    assert!(refusal.contains("cargo"));
}

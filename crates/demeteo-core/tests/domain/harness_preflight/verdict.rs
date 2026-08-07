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

#[test]
fn no_posix_shell_blocks_the_launch_like_a_missing_binary_does() {
    let verdict = PreflightVerdict::MissingPosixShell;

    let refusal = verdict
        .launch_refusal()
        .expect("nothing downstream can supply a shell");
    assert_eq!(Some(refusal), verdict.detail());
    assert_eq!(verdict.phase_status(), "failed");
}

#[test]
fn no_posix_shell_names_the_install_that_is_broken() {
    // The user has a working git, which is why nothing else has complained —
    // so the text has to name what is actually missing and how it got that
    // way, or it reads as nonsense and they go audit PATH.
    let detail = PreflightVerdict::MissingPosixShell
        .detail()
        .expect("a blocked verdict always explains itself");

    assert_eq!(detail, MISSING_POSIX_SHELL_REMEDIATION);
    assert!(detail.contains("Git Bash"));
    assert!(detail.contains("MinGit"));
    assert!(detail.contains("Git for Windows"));
    assert!(detail.contains("DEMETEO_BASH_PATH"));
}

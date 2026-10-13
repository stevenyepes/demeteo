// Tests extracted from `crates/demeteo-core/src/domain/push_lease.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;

const SHA: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[test]
fn a_branch_this_clone_never_saw_on_origin_must_not_exist_there() {
    let lease = lease_from_tracking("demeteo/features/f-1", "");

    assert_eq!(lease.expected, None);
    assert_eq!(
        lease.flag(),
        "--force-with-lease=refs/heads/demeteo/features/f-1:"
    );
}

#[test]
fn a_branch_this_clone_pushed_leases_against_what_it_pushed() {
    let output = format!("refs/remotes/origin/demeteo/features/f-1 {SHA}\n");
    let lease = lease_from_tracking("demeteo/features/f-1", &output);

    assert_eq!(lease.expected.as_deref(), Some(SHA));
    assert_eq!(
        lease.flag(),
        format!("--force-with-lease=refs/heads/demeteo/features/f-1:{SHA}")
    );
}

/// `for-each-ref` matches its pattern as a path prefix, so a branch nested
/// under the one being pushed is listed too and must not be taken for it.
#[test]
fn a_nested_branch_is_not_the_one_being_pushed() {
    let output = format!("refs/remotes/origin/feat/a/b {SHA}\n");

    assert_eq!(lease_from_tracking("feat/a", &output).expected, None);
}

#[test]
fn the_query_names_the_tracking_ref_exactly() {
    assert_eq!(
        tracking_query("feat/a"),
        [
            "for-each-ref".to_string(),
            "--format=%(refname) %(objectname)".to_string(),
            "refs/remotes/origin/feat/a".to_string(),
        ]
    );
}

#[test]
fn a_lease_rejection_is_told_apart_from_every_other_rejection() {
    let stale = "To https://github.com/o/r.git\n ! [rejected]        f -> f (stale info)\n\
                 error: failed to push some refs to 'https://github.com/o/r.git'";
    let non_ff = " ! [rejected]        f -> f (non-fast-forward)\nerror: failed to push some refs";
    let remote = " ! [remote rejected] f -> f (pre-receive hook declined)";

    assert!(is_stale_lease(stale));
    assert!(!is_stale_lease(non_ff));
    assert!(!is_stale_lease(remote));
    assert!(!is_stale_lease(""));
}

#[test]
fn the_refusal_names_the_branch_the_expectation_and_the_likely_cause() {
    let present = PushLease {
        branch: "feat/a".to_string(),
        expected: Some(SHA.to_string()),
    }
    .refusal();
    let absent = PushLease {
        branch: "feat/a".to_string(),
        expected: None,
    }
    .refusal();

    assert!(present.contains("origin/feat/a"), "{present}");
    assert!(present.contains(SHA), "{present}");
    assert!(present.contains("published from the desktop"), "{present}");
    assert!(absent.contains("expected it absent"), "{absent}");
}

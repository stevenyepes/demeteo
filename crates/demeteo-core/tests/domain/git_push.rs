// Tests extracted from `crates/demeteo-core/src/domain/git_push.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;

/// The form Demeteo writes itself, and the one that started this: a token-free
/// userinfo read as the host matches no provider, so the push goes out
/// uncredentialed and dies asking a terminal that is not there for a password.
#[test]
fn a_token_free_userinfo_is_not_the_host() {
    assert_eq!(
        credential_host("https://x-access-token@github.com/stevenyepes/demeteo"),
        Some("github.com")
    );
    assert_eq!(
        credential_host("https://oauth2@gitlab.example.com/acme/widgets.git"),
        Some("gitlab.example.com")
    );
}

#[test]
fn a_plain_https_remote_names_its_host() {
    assert_eq!(
        credential_host("https://github.com/o/r.git"),
        Some("github.com")
    );
}

/// A port is not part of the host a provider is matched by.
#[test]
fn a_port_is_not_part_of_the_host() {
    assert_eq!(
        credential_host("https://git.internal:8443/o/r.git"),
        Some("git.internal")
    );
}

/// Somebody put a secret in this URL on purpose. Installing a helper over it
/// would authenticate as a different identity than the one that was asked for.
#[test]
fn a_url_that_already_carries_a_password_is_left_alone() {
    assert_eq!(
        credential_host("https://x-access-token:ghp_realtoken@github.com/o/r.git"),
        None
    );
}

/// The user's key already authenticates these, and there is no password
/// exchange for a helper to take part in.
#[test]
fn a_remote_that_carries_its_own_credential_needs_none() {
    assert_eq!(credential_host("git@github.com:o/r.git"), None);
    assert_eq!(credential_host("ssh://git@github.com/o/r.git"), None);
    assert_eq!(credential_host("file:///srv/mirrors/r.git"), None);
    assert_eq!(credential_host("/srv/mirrors/r.git"), None);
    assert_eq!(credential_host(""), None);
}

/// Git never reached origin, so "the branch may have moved — fetch and sync
/// again" is advice that cannot work, offered with confidence. Every wording
/// here is one this tree has actually seen.
#[test]
fn the_credential_failures_are_told_apart_from_a_refusal() {
    for stderr in [
        "fatal: could not read Password for 'https://x-access-token@github.com': \
         No such device or address",
        "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
        "remote: Invalid username or password.\nfatal: Authentication failed for 'https://…'",
        "git@github.com: Permission denied (publickey).",
    ] {
        assert!(is_credential_failure(stderr), "missed: {stderr}");
    }
}

/// The refusals a fetch really does fix must keep the advice that fixes them.
#[test]
fn a_branch_that_moved_is_not_a_credential_failure() {
    for stderr in [
        "! [rejected]        main -> main (fetch first)\nerror: failed to push some refs",
        "! [remote rejected] main -> main (pre-receive hook declined)",
        "error: failed to push some refs to 'https://github.com/o/r.git'",
    ] {
        assert!(!is_credential_failure(stderr), "false positive: {stderr}");
    }
}

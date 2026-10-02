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

/// Port, path, query and the user all survive; only the password goes.
#[test]
fn a_password_is_dropped_and_everything_else_kept() {
    assert_eq!(
        token_free_origin("https://x-access-token:tok@host/o/r.git").as_deref(),
        Some("https://x-access-token@host/o/r.git")
    );
    assert_eq!(
        token_free_origin("https://oauth2:tok@host:8443/o/r?x=1").as_deref(),
        Some("https://oauth2@host:8443/o/r?x=1")
    );
    assert_eq!(
        token_free_origin("http://u:p@host").as_deref(),
        Some("http://u@host")
    );
}

/// With no user left to keep, `https://@host/r` would be an empty-user URL;
/// the userinfo goes entirely.
#[test]
fn a_password_with_no_user_drops_the_whole_userinfo() {
    let clean = token_free_origin("https://:tok@host/r").unwrap();
    assert_eq!(clean, "https://host/r");
    assert_eq!(credential_host(&clean), Some("host"));
    assert_eq!(token_free_origin(&clean), None);
}

/// The rewritten URL is what the provider lookup reads its host from.
#[test]
fn the_rewritten_url_names_the_host_of_the_original() {
    let clean = token_free_origin("https://oauth2:tok@gitlab.example.com:8443/o/r").unwrap();
    assert_eq!(credential_host(&clean), Some("gitlab.example.com"));
}

#[test]
fn nothing_to_rewrite_is_none() {
    for url in [
        "git@github.com:o/r.git",
        "ssh://git@github.com/o/r.git",
        "ssh://user:pw@github.com/o/r.git",
        "file:///srv/mirrors/r.git",
        "/srv/mirrors/r.git",
        "https://user@host/o/r",
        "https://host/o/r",
        "",
    ] {
        assert_eq!(token_free_origin(url), None, "rewrote: {url}");
    }
}

#[test]
fn rewriting_is_idempotent() {
    let once = token_free_origin("https://x-access-token:tok@host:8443/o/r.git?a=b").unwrap();
    assert_eq!(token_free_origin(&once), None);
}

#[test]
fn the_embedded_password_is_the_part_after_the_first_colon() {
    assert_eq!(
        embedded_password("https://x-access-token:tok@host/o/r").as_deref(),
        Some("tok")
    );
    assert_eq!(
        embedded_password("http://u:a:b@host:8443/o/r?x=1").as_deref(),
        Some("a:b")
    );
    assert_eq!(
        embedded_password("https://:tok@host/r").as_deref(),
        Some("tok")
    );
}

#[test]
fn no_password_to_read_is_none() {
    for url in [
        "https://user@host/o/r",
        "https://host/o/r",
        "ssh://user:pw@host/o/r.git",
        "git@host:o/r.git",
        "/srv/mirrors/r.git",
        "",
    ] {
        assert_eq!(embedded_password(url), None, "read a password from: {url}");
    }
}

/// `HttpRemote` ends the authority at the first `/`, `?` or `#`, so a password
/// holding one is not userinfo. Pinned because the limit is deliberate — see
/// `docs/KNOWN_ISSUES.md` — and a "fix" that re-parses at the last `@` would
/// mis-read a path that contains one.
#[test]
fn a_password_with_an_unencoded_delimiter_is_not_read() {
    assert_eq!(embedded_password("https://u:p/ss@host/r"), None);
    assert_eq!(token_free_origin("https://u:p/ss@host/r"), None);
    assert_eq!(embedded_password("https://u:p?ss@host/r"), None);
    assert_eq!(embedded_password("https://u:p#ss@host/r"), None);
}

#[test]
fn percent_escapes_decode_and_malformed_ones_stay_literal() {
    assert_eq!(percent_decode("a%2Fb%3Fc%23d").as_deref(), Some("a/b?c#d"));
    assert_eq!(percent_decode("%e2%82%ac").as_deref(), Some("€"));
    assert_eq!(percent_decode("100%").as_deref(), Some("100%"));
    assert_eq!(percent_decode("%zz%4").as_deref(), Some("%zz%4"));
    assert_eq!(percent_decode("%ff"), None, "not UTF-8");
}

#[test]
fn a_url_embeds_a_secret_raw_or_percent_encoded() {
    assert!(embeds_secret("https://u:tok@host/r", "tok"));
    assert!(embeds_secret("https://u:p%40ss%2Fw@host/r", "p@ss/w"));
    assert!(embeds_secret("https://u:p%40ss@host/r", "p%40ss"));
    assert!(!embeds_secret("https://u:tok@host/r", "other"));
    assert!(!embeds_secret("https://u:tok@host/r", ""));
    assert!(!embeds_secret("https://u@host/r", "tok"));
    assert!(!embeds_secret("git@host:o/r.git", "tok"));
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

#[test]
fn a_host_loses_its_port_and_nothing_else() {
    assert_eq!(host_without_port("gitlab.local:8443"), "gitlab.local");
    assert_eq!(host_without_port("github.com"), "github.com");
    assert_eq!(host_without_port(""), "");
}

#[test]
fn the_remote_host_and_the_provider_host_share_one_rule() {
    let remote = "https://oauth2@gitlab.local:8443/o/r.git";
    assert_eq!(
        credential_host(remote),
        Some(host_without_port("gitlab.local:8443"))
    );
}

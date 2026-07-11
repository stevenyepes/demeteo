// Tests extracted from `crates/demeteo-core/src/shared/secret_scrub.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn passes_clean_text_through_borrowed() {
    let s = "run reached terminal state: completed";
    assert!(matches!(scrub_secrets(s), Cow::Borrowed(_)));
    assert_eq!(scrub_secrets(s), s);
}

#[test]
fn masks_github_classic_pat() {
    let pat = "ghp_0123456789abcdefABCDEF0123456789abcdef";
    let msg = format!("fatal: could not read Password for '{pat}'");
    let out = scrub_secrets(&msg);
    assert!(!out.contains(pat), "token leaked: {out}");
    assert!(out.contains(MASK));
}

#[test]
fn masks_github_fine_grained_pat() {
    let pat = "github_pat_11ABCDEF0123456789_abcdefghijklmnopqrstuvwxyz0123456789ABCDEFXYZ";
    let out = scrub_secrets(pat);
    assert_eq!(out, MASK);
}

#[test]
fn masks_gitlab_pat() {
    let pat = "glpat-ABCDEF0123456789xyzA";
    let msg = format!("remote error with {pat} here");
    let out = scrub_secrets(&msg);
    assert!(!out.contains(pat));
    assert!(out.contains("remote error with *** here"));
}

#[test]
fn masks_url_embedded_credential() {
    // The classic leak vector: a token embedded as HTTP basic-auth.
    let url = "https://x-access-token:ghp_secretsecretsecret0123456789abcd@github.com/o/r.git";
    let out = scrub_secrets(url);
    assert!(
        !out.contains("ghp_secretsecretsecret0123456789abcd"),
        "leaked: {out}"
    );
    assert!(
        out.contains("https://x-access-token:***@github.com/o/r.git"),
        "got: {out}"
    );
}

#[test]
fn masks_generic_url_basic_auth() {
    // Even a non-prefixed password in a URL is masked by the userinfo pass.
    let url = "https://alice:sup3rSecretValue@example.com/path";
    let out = scrub_secrets(url);
    assert!(!out.contains("sup3rSecretValue"), "leaked: {out}");
    assert!(
        out.contains("https://alice:***@example.com/path"),
        "got: {out}"
    );
}

#[test]
fn leaves_token_free_userinfo_untouched() {
    // M4.3's token-free clone URL must not be mangled.
    let url = "https://x-access-token@github.com/o/r.git";
    assert_eq!(scrub_secrets(url), url);
}

#[test]
fn does_not_mask_lone_prefix_word() {
    // A bare prefix in prose isn't a token.
    let s = "the ghp_ prefix identifies a classic token";
    assert_eq!(scrub_secrets(s), s);
}

#[test]
fn masks_multiple_secrets_in_one_string() {
    let s = "first ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa then glpat-bbbbbbbbbbbbbbbbbbbb";
    let out = scrub_secrets(s);
    assert!(!out.contains("ghp_aaaa"));
    assert!(!out.contains("glpat-bbbb"));
    assert_eq!(out, "first *** then ***");
}

#[test]
fn scrubs_url_and_bare_token_together() {
    let s = "clone https://oauth2:glpat-tokentokentokentoken1@gitlab.com/x.git failed; \
             ghp_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb rejected";
    let out = scrub_secrets(s);
    assert!(!out.contains("glpat-tokentokentokentoken1"));
    assert!(!out.contains("ghp_bbbb"));
}

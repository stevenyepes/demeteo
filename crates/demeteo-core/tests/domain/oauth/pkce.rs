// domain::oauth::pkce's own vocabulary. `super` = `domain::oauth::pkce`.

use super::*;

// RFC 7636 Appendix B test vector.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

#[test]
fn valid_s256_pair_passes() {
    assert_eq!(verify_pkce(VERIFIER, CHALLENGE, "S256"), Ok(()));
}

#[test]
fn wrong_verifier_is_rejected() {
    assert_eq!(
        verify_pkce("not-the-verifier", CHALLENGE, "S256"),
        Err(OAuthError::InvalidToken)
    );
}

#[test]
fn plain_method_is_rejected_never_falls_back() {
    assert_eq!(
        verify_pkce(VERIFIER, CHALLENGE, "plain"),
        Err(OAuthError::UnsupportedPkceMethod)
    );
}

#[test]
fn missing_challenge_is_rejected() {
    assert_eq!(
        verify_pkce(VERIFIER, "", "S256"),
        Err(OAuthError::MissingPkce)
    );
}

#[test]
fn validate_challenge_request_accepts_a_well_formed_s256_request() {
    assert_eq!(validate_challenge_request(CHALLENGE, "S256"), Ok(()));
}

#[test]
fn validate_challenge_request_rejects_non_s256_method() {
    assert_eq!(
        validate_challenge_request(CHALLENGE, "plain"),
        Err(OAuthError::UnsupportedPkceMethod)
    );
}

#[test]
fn validate_challenge_request_rejects_missing_challenge() {
    assert_eq!(
        validate_challenge_request("", "S256"),
        Err(OAuthError::MissingPkce)
    );
}

#[test]
fn validate_challenge_request_checks_method_before_challenge() {
    assert_eq!(
        validate_challenge_request("", ""),
        Err(OAuthError::UnsupportedPkceMethod)
    );
}

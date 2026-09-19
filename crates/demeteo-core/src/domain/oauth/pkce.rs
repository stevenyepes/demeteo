//! PKCE `S256` verification — the only method this authorization server
//! accepts. Per `docs/MCP_INTEGRATION.md` §5, a missing `code_challenge_method`
//! is never treated as an implicit `plain`, so the caller must always pass
//! the method it received, even when absent from the request.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};

use super::OAuthError;

/// Structural PKCE request validation — a missing `code_challenge`, or a
/// `code_challenge_method` other than `S256`, is rejected the same way
/// whether or not a `code_verifier` exists yet to check it against.
///
/// `GET /authorize` never sees a verifier (only `POST /token` does), so it
/// calls this directly rather than [`verify_pkce`]; that function calls this
/// first too, so the two can never disagree about what "well-formed PKCE
/// request" means.
pub fn validate_challenge_request(challenge: &str, method: &str) -> Result<(), OAuthError> {
    if method != "S256" {
        return Err(OAuthError::UnsupportedPkceMethod);
    }
    if challenge.is_empty() {
        return Err(OAuthError::MissingPkce);
    }
    Ok(())
}

/// Verifies `verifier` against `challenge` for the given PKCE `method`.
///
/// Comparison is not constant-time: both `verifier` and `challenge` cross
/// the same loopback process, so PKCE's threat model here is a passive
/// network observer, not a local timing attacker.
pub fn verify_pkce(verifier: &str, challenge: &str, method: &str) -> Result<(), OAuthError> {
    validate_challenge_request(challenge, method)?;
    let computed = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    if computed != challenge {
        return Err(OAuthError::InvalidToken);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/domain/oauth/pkce.rs"]
mod pkce_tests;

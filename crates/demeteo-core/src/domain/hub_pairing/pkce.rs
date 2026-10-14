//! The client half of PKCE `S256` for the Hub pairing. `domain/oauth/pkce.rs`
//! is the verifying half of a different authorization server, so it is not
//! shared (see [`super`]).

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};

/// `BASE64URL_NOPAD(SHA256(verifier))`, RFC 7636 §4.2.
pub fn s256_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

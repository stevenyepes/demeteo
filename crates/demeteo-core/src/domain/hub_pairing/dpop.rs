//! The text of the one DPoP proof (RFC 9449 §4) a desktop sends, to
//! `/token`. What `crates/demeteo-hub/src/pairing/dpop.rs` verifies is the
//! contract: `typ` and `alg` are exact strings, `htu` is compared whole and
//! unnormalised, and the signature is the 64-byte JWS `r ‖ s`, not DER.
//!
//! `iat` and `jti` are arguments so the text is deterministic; the clock,
//! the randomness and the signature belong to the adapter that holds the key.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::json;

use super::jwk::jwk_value;

/// `BASE64URL(header) . BASE64URL(claims)`, the bytes the key signs.
pub fn dpop_signing_input(jwk_point: &[u8; 65], htu: &str, iat: u64, jti: &str) -> String {
    let header = json!({"typ": "dpop+jwt", "alg": "ES256", "jwk": jwk_value(jwk_point)});
    let claims = json!({"jti": jti, "htm": "POST", "htu": htu, "iat": iat});
    format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    )
}

pub fn dpop_proof(signing_input: &str, raw_sig64: &[u8; 64]) -> String {
    format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(raw_sig64))
}

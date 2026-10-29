//! The device key's public half as the Hub names it: the RFC 7638
//! thumbprint a token is bound to, and the fingerprint a user compares
//! between the approval page and this desktop. Both mirror
//! `crates/demeteo-hub/src/pairing/jwk.rs` (see [`super`]); a byte of
//! disagreement makes `/token` refuse every proof as bound to another key.
//!
//! Every function takes the 65-byte SEC1 uncompressed point `04 ‖ x ‖ y`,
//! the form a signing library exports. The leading tag is not read.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const FINGERPRINT_PREFIX: &str = "SHA256:";
const FINGERPRINT_GROUPS: usize = 4;
const FINGERPRINT_GROUP_LEN: usize = 4;

fn coordinates(point: &[u8; 65]) -> (String, String) {
    (
        URL_SAFE_NO_PAD.encode(&point[1..33]),
        URL_SAFE_NO_PAD.encode(&point[33..65]),
    )
}

/// The required members of an EC key in lexicographic order with no
/// whitespace (RFC 7638 §3.2). Written out rather than serialised: the
/// thumbprint is a digest of this text, and a serialiser's member order is
/// a feature flag away from changing it.
pub fn canonical_jwk(point: &[u8; 65]) -> String {
    let (x, y) = coordinates(point);
    format!(r#"{{"crv":"P-256","kty":"EC","x":"{x}","y":"{y}"}}"#)
}

pub fn thumbprint(point: &[u8; 65]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(canonical_jwk(point).as_bytes()))
}

/// The first 96 bits of the thumbprint in groups of four. The Hub renders
/// the same string on its approval page from the same thumbprint, so the
/// format is not this crate's alone to change.
pub fn fingerprint(jkt: &str) -> String {
    let mut out = String::from(FINGERPRINT_PREFIX);
    let shown = jkt.chars().take(FINGERPRINT_GROUPS * FINGERPRINT_GROUP_LEN);
    for (position, character) in shown.enumerate() {
        if position > 0 && position % FINGERPRINT_GROUP_LEN == 0 {
            out.push(' ');
        }
        out.push(character);
    }
    out
}

/// The public JWK carried in a DPoP header. The Hub recomputes the
/// thumbprint from the decoded coordinates, so member order here is free.
pub fn jwk_value(point: &[u8; 65]) -> Value {
    let (x, y) = coordinates(point);
    json!({"crv": "P-256", "kty": "EC", "x": x, "y": y})
}

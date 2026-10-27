//! The desktop's check of a Hub request against the `request_signing_key` it
//! pinned at pairing (docs/HUB.md §6.1).

use aws_lc_rs::signature::{UnparsedPublicKey, ED25519};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use demeteo_hub_protocol::SignedRequest;

use super::SignatureCheck;

/// Verifies over [`SignedRequest::signing_bytes`] and never over the JSON the
/// request arrived as: the Hub signs the canonical encoding, and JSON text has
/// no single spelling for a desktop to reproduce.
///
/// Both the pinned key and the signature are unpadded base64url, the spelling
/// the pin is stored in. Anything that does not decode, a key or signature of
/// the wrong length, and a request too long to encode are all `Invalid`.
pub fn verify_signature(request: &SignedRequest, pinned_key_b64url: &str) -> SignatureCheck {
    let (Ok(key), Ok(signature), Ok(message)) = (
        URL_SAFE_NO_PAD.decode(pinned_key_b64url),
        URL_SAFE_NO_PAD.decode(&request.signature),
        request.signing_bytes(),
    ) else {
        return SignatureCheck::Invalid;
    };
    match UnparsedPublicKey::new(&ED25519, key).verify(&message, &signature) {
        Ok(()) => SignatureCheck::Valid,
        Err(_) => SignatureCheck::Invalid,
    }
}

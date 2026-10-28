//! A trusted passkey's public key, from the `COSE_Key` bytes its base64url
//! `public_key` holds (docs/HUB.md §8).
//!
//! Each algorithm has exactly one accepted byte string, compared byte for byte
//! with no CBOR decoder, for the reason the header of
//! `crates/demeteo-hub/src/auth/cose.rs` gives: the endorsement challenge
//! commits to the exact bytes. The Hub registers ES256 only, so the EdDSA form
//! reaches this parser from tests alone.

const ES256_HEAD: [u8; 10] = [0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20];
const ES256_Y_HEAD: [u8; 3] = [0x22, 0x58, 0x20];
const EDDSA_HEAD: [u8; 10] = [0xa4, 0x01, 0x01, 0x03, 0x27, 0x20, 0x06, 0x21, 0x58, 0x20];

const SEC1_UNCOMPRESSED: u8 = 0x04;

/// Neither form is checked to be a valid point here: that refusal belongs to
/// the signature verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasskeyKey {
    /// SEC1 uncompressed, `04 ‖ x ‖ y`, the form `aws_lc_rs::signature` reads.
    Es256([u8; 65]),
    Ed25519([u8; 32]),
}

/// Accepts exactly two encodings, and `None` for every other byte string:
///
/// - ES256, 77 bytes: `a5 01 02 03 26 20 01 21 58 20 <x:32> 22 58 20 <y:32>`
/// - EdDSA over Ed25519, 42 bytes: `a4 01 01 03 27 20 06 21 58 20 <x:32>`
pub fn parse_cose_key(bytes: &[u8]) -> Option<PasskeyKey> {
    if let Some(x) = bytes.strip_prefix(&EDDSA_HEAD) {
        return <[u8; 32]>::try_from(x).ok().map(PasskeyKey::Ed25519);
    }
    let (x, rest) = bytes.strip_prefix(&ES256_HEAD)?.split_first_chunk::<32>()?;
    let y = <[u8; 32]>::try_from(rest.strip_prefix(&ES256_Y_HEAD)?).ok()?;
    let mut sec1 = [0u8; 65];
    sec1[0] = SEC1_UNCOMPRESSED;
    sec1[1..33].copy_from_slice(x);
    sec1[33..].copy_from_slice(&y);
    Some(PasskeyKey::Es256(sec1))
}

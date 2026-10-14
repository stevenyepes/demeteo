//! Verifies a DPoP proof (RFC 9449 §4): a compact JWS a client signs for one
//! request, with the key its tokens are bound to and carrying that key.
//!
//! [`verify`] answers one question: did the holder of the key in the proof
//! sign for this method and this URL, about now. Three things RFC 9449 §4.3
//! also asks for are the caller's, because none of them can be decided from
//! the proof alone:
//!
//! - **Which key.** A proof is valid under whatever key it carries, so a
//!   verified proof says nothing until [`VerifiedProof::jkt`] is compared
//!   with the thumbprint a code or a token is bound to.
//! - **Replay.** Within [`DPOP_MAX_AGE_SECS`] of `iat` the same proof
//!   verifies every time. Remembering [`VerifiedProof::jti`] is what makes it
//!   single-use.
//! - **`ath`.** The claim is read and handed over as [`VerifiedProof::ath`],
//!   and not judged: whether a proof has to name a token, and which, is a
//!   question about the request it came with. `/token` presents no access
//!   token and ignores it; the socket requires it and compares it with the
//!   token in `Authorization`
//!   ([`socket::handler`](crate::socket::handler)).
//! - **`nonce`.** Not read here.
//!
//! # What is refused that a JOSE library would take
//!
//! `alg` is an allowlist of one. A proof names its own algorithm and carries
//! its own key, so any algorithm accepted here is one an attacker may choose:
//! `none` asks for no signature at all, and `HS256` asks for the public key
//! in the header to be used as a shared secret.
//!
//! `htu` is compared as a whole string with what the caller expects. RFC 9449
//! §4.3 recommends normalising both sides first; nothing is normalised here.
//! A normaliser is a URL parser that can disagree with the client's, and the
//! caller's string comes from configuration, so there is one spelling to
//! match and a client that spells the URL another way is refused.
//!
//! Every check that needs no curve arithmetic runs before the one that does:
//! a proof that is wrong in its structure costs no ECDSA verification.

use aws_lc_rs::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_FIXED};
use serde_json::{Map, Value};

use super::jwk::{text, DeviceJwk};
use super::DPOP_MAX_AGE_SECS;
use crate::auth::challenge::base64url_decode;

const PROOF_TYPE: &str = "dpop+jwt";
const ALGORITHM: &str = "ES256";
/// The proof published in RFC 9449 §4.1 is 438 bytes. The bound is on the
/// text as it arrived, so it is checked before anything is decoded or parsed.
const MAX_PROOF_LEN: usize = 4096;
const MAX_JTI_LEN: usize = 128;

/// The request a proof must have been made for. `htu` is the caller's to
/// build, from configuration: nothing here reads a request header.
#[derive(Debug, Clone, Copy)]
pub struct Expected<'a> {
    pub htm: &'a str,
    pub htu: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedProof {
    pub jwk: DeviceJwk,
    /// The thumbprint of `jwk`.
    pub jkt: String,
    pub jti: String,
    pub iat: u64,
    /// The `ath` claim as the proof spelled it, `None` when it carries none
    /// or one that is not a string. Signed, and otherwise unexamined.
    pub ath: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProofError {
    #[error("not a compact JWS of a JSON header and JSON claims with `iat` and `jti`")]
    Malformed,
    #[error("`typ` is not `{PROOF_TYPE}`")]
    Type,
    #[error("`alg` is not `{ALGORITHM}`")]
    Algorithm,
    #[error("`jwk` is not an ES256 public key")]
    Key,
    #[error("the proof is for another HTTP method")]
    Method,
    #[error("the proof is for another URL")]
    Uri,
    #[error("`iat` is more than {DPOP_MAX_AGE_SECS} seconds from now")]
    Stale,
    #[error("the signature does not verify under the key in the proof")]
    Signature,
}

/// `now` is Unix seconds, as `iat` is. A proof dated in the future is as
/// stale as one dated in the past: the window is on both sides of `now`, so
/// a client whose clock runs ahead is tolerated by the same margin and no
/// further.
///
/// The signature is the 64-byte `r ‖ s` of JWS (RFC 7518 §3.4), not the DER
/// form WebAuthn uses and [`auth::assertion`](crate::auth::assertion)
/// verifies. A point that is not on the curve is refused here, as
/// [`ProofError::Signature`], because [`DeviceJwk`] does not check it.
pub fn verify(proof: &str, expected: &Expected, now: u64) -> Result<VerifiedProof, ProofError> {
    if proof.len() > MAX_PROOF_LEN {
        return Err(ProofError::Malformed);
    }
    let Ok([header, claims, signature]) =
        <[&str; 3]>::try_from(proof.split('.').collect::<Vec<_>>())
    else {
        return Err(ProofError::Malformed);
    };
    let signed = &proof[..header.len() + 1 + claims.len()];
    let signature = base64url_decode(signature).ok_or(ProofError::Malformed)?;
    let header = object(header)?;
    let claims = object(claims)?;

    if text(&header, "typ") != Some(PROOF_TYPE) {
        return Err(ProofError::Type);
    }
    if text(&header, "alg") != Some(ALGORITHM) {
        return Err(ProofError::Algorithm);
    }
    let jwk = header
        .get("jwk")
        .and_then(|jwk| DeviceJwk::parse(jwk).ok())
        .ok_or(ProofError::Key)?;

    if text(&claims, "htm") != Some(expected.htm) {
        return Err(ProofError::Method);
    }
    if text(&claims, "htu") != Some(expected.htu) {
        return Err(ProofError::Uri);
    }
    let iat = claims
        .get("iat")
        .and_then(Value::as_u64)
        .ok_or(ProofError::Malformed)?;
    if now.abs_diff(iat) > DPOP_MAX_AGE_SECS {
        return Err(ProofError::Stale);
    }
    let jti = text(&claims, "jti")
        .filter(|jti| (1..=MAX_JTI_LEN).contains(&jti.len()))
        .ok_or(ProofError::Malformed)?;

    UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, jwk.point.to_sec1_uncompressed())
        .verify(signed.as_bytes(), &signature)
        .map_err(|_| ProofError::Signature)?;

    Ok(VerifiedProof {
        jwk,
        jkt: jwk.thumbprint(),
        jti: jti.to_owned(),
        iat,
        ath: text(&claims, "ath").map(str::to_owned),
    })
}

fn object(segment: &str) -> Result<Map<String, Value>, ProofError> {
    base64url_decode(segment)
        .and_then(|json| serde_json::from_slice(&json).ok())
        .ok_or(ProofError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::challenge::base64url_encode;
    use crate::pairing::testing::{claims, SoftDeviceKey};
    use serde_json::json;

    const HTM: &str = "POST";
    const HTU: &str = "https://hub.example/token";
    const NOW: u64 = 1_700_000_000;
    const JTI: &str = "jti-0";

    /// Spelled here and not read from the constants under test: a test built
    /// on `DPOP_MAX_AGE_SECS` follows the bound wherever it is moved.
    const WINDOW: u64 = 60;
    const LONGEST_PROOF: usize = 4096;
    const LONGEST_JTI: usize = 128;
    /// 64 bytes in unpadded base64url.
    const SIGNATURE_TEXT_LEN: usize = 86;

    fn verified(proof: &str) -> Result<VerifiedProof, ProofError> {
        let expected = Expected { htm: HTM, htu: HTU };
        verify(proof, &expected, NOW)
    }

    fn refusal(proof: &str) -> Option<ProofError> {
        verified(proof).err()
    }

    /// A key, and the header and claims of a proof by it that verifies.
    struct Control {
        key: SoftDeviceKey,
        header: Value,
        claims: Value,
    }

    /// Every refusal below is of a proof that differs from this one in the
    /// one thing its test names. That this one verifies is asserted on each
    /// call, so no test can pass against a proof that was never valid.
    fn control() -> Control {
        let key = SoftDeviceKey::new();
        let header = key.header();
        let claims = claims(HTM, HTU, NOW, JTI);
        let proof = key.proof_with(&header, &claims);
        assert_eq!(verified(&proof).map(|proof| proof.jti), Ok(JTI.to_owned()));
        Control {
            key,
            header,
            claims,
        }
    }

    fn with(object: &Value, member: &str, value: Value) -> Value {
        let mut object = object.clone();
        object[member] = value;
        object
    }

    fn without(object: &Value, member: &str) -> Value {
        let mut object = object.clone();
        if let Some(members) = object.as_object_mut() {
            members.remove(member);
        }
        object
    }

    fn segments(proof: &str) -> [&str; 3] {
        <[&str; 3]>::try_from(proof.split('.').collect::<Vec<_>>()).unwrap()
    }

    fn reencoded(segment: &str, change: impl FnOnce(&mut Vec<u8>)) -> String {
        let mut bytes = base64url_decode(segment).unwrap();
        change(&mut bytes);
        base64url_encode(&bytes)
    }

    /// `segment` with the lowest bit of the last byte of `marker` flipped.
    /// Both markers used below end in `0`, which that turns into `1`: the
    /// segment is the same JSON with one character of one string changed.
    fn with_a_bit_flipped_in(segment: &str, marker: &str) -> String {
        reencoded(segment, |bytes| {
            let start = bytes
                .windows(marker.len())
                .position(|window| window == marker.as_bytes())
                .unwrap();
            bytes[start + marker.len() - 1] ^= 0x01;
        })
    }

    /// A proof by `key` that verifies and is exactly `len` bytes long.
    ///
    /// Unpadded base64url has no length of 1 modulo 4, so filler in the
    /// claims alone cannot reach every total; up to two bytes in the header
    /// move the header's own length through the other residues.
    fn proof_of_len(control: &Control, len: usize) -> String {
        let shortest = control.key.proof_with(&control.header, &control.claims);
        let most_filler = (len - shortest.len()) * 3 / 4;
        for kid in 0..3 {
            let header = with(&control.header, "kid", json!("k".repeat(kid)));
            for filler in most_filler.saturating_sub(40)..=most_filler {
                let claims = with(&control.claims, "filler", json!("f".repeat(filler)));
                let proof = control.key.proof_with(&header, &claims);
                if proof.len() == len {
                    return proof;
                }
            }
        }
        panic!("no filler makes a proof of {len} bytes");
    }

    #[test]
    fn a_proof_verifies_and_is_named_by_the_thumbprint_its_key_reports() {
        let key = SoftDeviceKey::new();
        let proof = key.proof(HTM, HTU, NOW, JTI);
        assert_eq!(
            verified(&proof),
            Ok(VerifiedProof {
                jwk: DeviceJwk::parse(&key.jwk()).unwrap(),
                jkt: key.thumbprint(),
                jti: JTI.to_owned(),
                iat: NOW,
                ath: None,
            })
        );
    }

    #[test]
    fn ath_is_handed_over_as_the_proof_spelled_it_and_a_proof_is_not_refused_for_it() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let carrying = |ath: Value| key.proof_with(&header, &with(&claims, "ath", ath));

        let named = verified(&carrying(json!("not-a-digest")));
        assert_eq!(
            named.map(|proof| proof.ath),
            Ok(Some("not-a-digest".to_owned()))
        );
        for ath in [json!(7), json!(null), json!(["a"])] {
            let shown = ath.to_string();
            assert_eq!(
                verified(&carrying(ath)).map(|proof| proof.ath),
                Ok(None),
                "{shown}"
            );
        }
    }

    /// The proof of RFC 9449 §4.1, which this crate's signer did not make:
    /// the tests above sign and verify with one library, and would agree
    /// with each other over a signing input both had wrong.
    #[test]
    fn the_proof_published_in_rfc_9449_verifies() {
        let published = "eyJ0eXAiOiJkcG9wK2p3dCIsImFsZyI6IkVTMjU2IiwiandrIjp7Imt0eSI6Ik\
                         VDIiwieCI6Imw4dEZyaHgtMzR0VjNoUklDUkRZOXpDa0RscEJoRjQyVVFVZldWQVdCR\
                         nMiLCJ5IjoiOVZFNGpmX09rX282NHpiVFRsY3VOSmFqSG10NnY5VERWclUwQ2R2R1JE\
                         QSIsImNydiI6IlAtMjU2In19.eyJqdGkiOiItQndDM0VTYzZhY2MybFRjIiwiaHRtIj\
                         oiUE9TVCIsImh0dSI6Imh0dHBzOi8vc2VydmVyLmV4YW1wbGUuY29tL3Rva2VuIiwia\
                         WF0IjoxNTYyMjYyNjE2fQ.2-GxA6T8lP4vfrg8v-FdWP0A0zdrj8igiMLvqRMUvwnQg\
                         4PtFLbdLXiOSsX0x7NVY-FNyJK70nfbV37xRZT3Lg";
        let expected = Expected {
            htm: "POST",
            htu: "https://server.example.com/token",
        };
        let verified = verify(published, &expected, 1_562_262_616).unwrap();
        assert_eq!(verified.jkt, "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I");
        assert_eq!(verified.jti, "-BwC3ESc6acc2lTc");
        assert_eq!(verified.iat, 1_562_262_616);
    }

    #[test]
    fn two_keys_are_named_by_two_thumbprints() {
        let (one, other) = (SoftDeviceKey::new(), SoftDeviceKey::new());
        let jkt = |key: &SoftDeviceKey| verified(&key.proof(HTM, HTU, NOW, JTI)).unwrap().jkt;
        assert_ne!(jkt(&one), jkt(&other));
        assert_eq!(jkt(&other), other.thumbprint());
    }

    #[test]
    fn a_typ_other_than_dpop_jwt_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        for typ in [json!("jwt"), json!("JWT"), json!("at+jwt"), json!(null)] {
            let proof = key.proof_with(&with(&header, "typ", typ.clone()), &claims);
            assert_eq!(refusal(&proof), Some(ProofError::Type), "{typ}");
        }
        let proof = key.proof_with(&without(&header, "typ"), &claims);
        assert_eq!(refusal(&proof), Some(ProofError::Type));
    }

    #[test]
    fn alg_none_with_an_empty_signature_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let signed = key.proof_with(&with(&header, "alg", json!("none")), &claims);
        let [header, claims, _] = segments(&signed);
        let unsigned = format!("{header}.{claims}.");
        assert_eq!(refusal(&unsigned), Some(ProofError::Algorithm));
    }

    /// Each proof carries a valid ES256 signature by its key over its own
    /// header, so the name of the algorithm is all that is wrong with it.
    #[test]
    fn an_alg_other_than_es256_is_refused_whatever_signature_follows() {
        let Control {
            key,
            header,
            claims,
        } = control();
        for alg in ["HS256", "none", "ES384", "RS256", "EdDSA", "es256"] {
            let proof = key.proof_with(&with(&header, "alg", json!(alg)), &claims);
            assert_eq!(refusal(&proof), Some(ProofError::Algorithm), "{alg}");
        }
        let proof = key.proof_with(&without(&header, "alg"), &claims);
        assert_eq!(refusal(&proof), Some(ProofError::Algorithm));
    }

    #[test]
    fn a_proof_for_another_method_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        for htm in [json!("GET"), json!("post"), json!("POST "), json!(null)] {
            let proof = key.proof_with(&header, &with(&claims, "htm", htm.clone()));
            assert_eq!(refusal(&proof), Some(ProofError::Method), "{htm}");
        }
    }

    #[test]
    fn a_proof_for_another_origin_another_path_or_a_trailing_slash_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        for htu in [
            "https://hub.example.evil.example/token",
            "https://evil.example/token",
            "http://hub.example/token",
            "https://hub.example/authorize",
            "https://hub.example/token/",
            "https://hub.example/token?x=1",
            "https://HUB.example/token",
            "/token",
        ] {
            let proof = key.proof_with(&header, &with(&claims, "htu", json!(htu)));
            assert_eq!(refusal(&proof), Some(ProofError::Uri), "{htu}");
        }
        let proof = key.proof_with(&header, &without(&claims, "htu"));
        assert_eq!(refusal(&proof), Some(ProofError::Uri));
    }

    #[test]
    fn the_url_a_proof_must_name_is_the_one_the_caller_expects() {
        let elsewhere = "https://other.example/v2/token";
        let proof = SoftDeviceKey::new().proof(HTM, elsewhere, NOW, JTI);
        let expected = Expected {
            htm: HTM,
            htu: elsewhere,
        };
        assert!(verify(&proof, &expected, NOW).is_ok());
        assert_eq!(refusal(&proof), Some(ProofError::Uri));
    }

    #[test]
    fn a_proof_is_accepted_at_exactly_the_window_on_either_side_and_refused_one_second_beyond() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let dated = |iat: u64| key.proof_with(&header, &with(&claims, "iat", json!(iat)));
        for iat in [NOW - WINDOW, NOW - 1, NOW + 1, NOW + WINDOW] {
            assert_eq!(verified(&dated(iat)).map(|proof| proof.iat), Ok(iat));
        }
        for iat in [NOW - WINDOW - 1, NOW + WINDOW + 1, 0, u64::MAX] {
            assert_eq!(refusal(&dated(iat)), Some(ProofError::Stale), "{iat}");
        }
    }

    #[test]
    fn an_iat_that_is_not_a_whole_number_of_seconds_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let now_as_text = NOW.to_string();
        for iat in [json!(now_as_text), json!(1.7e9), json!(-1), json!(null)] {
            let proof = key.proof_with(&header, &with(&claims, "iat", iat.clone()));
            assert_eq!(refusal(&proof), Some(ProofError::Malformed), "{iat}");
        }
        let proof = key.proof_with(&header, &without(&claims, "iat"));
        assert_eq!(refusal(&proof), Some(ProofError::Malformed));
    }

    #[test]
    fn a_jti_of_1_to_128_bytes_is_accepted_and_one_byte_either_side_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let named = |jti: Value| key.proof_with(&header, &with(&claims, "jti", jti));
        for len in [1, LONGEST_JTI] {
            let jti = "j".repeat(len);
            assert_eq!(verified(&named(json!(jti))).map(|proof| proof.jti), Ok(jti));
        }
        for jti in [json!(""), json!("j".repeat(LONGEST_JTI + 1)), json!(7)] {
            assert_eq!(
                refusal(&named(jti.clone())),
                Some(ProofError::Malformed),
                "{jti}"
            );
        }
        let proof = key.proof_with(&header, &without(&claims, "jti"));
        assert_eq!(refusal(&proof), Some(ProofError::Malformed));
    }

    #[test]
    fn one_flipped_bit_in_the_signature_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let proof = key.proof_with(&header, &claims);
        let [header, claims, signature] = segments(&proof);
        for byte in [0, 31, 32, 63] {
            let flipped = reencoded(signature, |bytes| bytes[byte] ^= 0x01);
            assert_eq!(
                refusal(&format!("{header}.{claims}.{flipped}")),
                Some(ProofError::Signature),
                "byte {byte}"
            );
        }
    }

    #[test]
    fn one_flipped_bit_in_the_payload_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let proof = key.proof_with(&header, &claims);
        let [header, claims, signature] = segments(&proof);
        let flipped = format!(
            "{header}.{}.{signature}",
            with_a_bit_flipped_in(claims, JTI)
        );
        assert_ne!(flipped, proof);
        assert_eq!(refusal(&flipped), Some(ProofError::Signature));
    }

    /// The bit is in a member [`verify`] does not read, so the header is
    /// still one it accepts and only the signature over it is wrong.
    #[test]
    fn one_flipped_bit_in_the_header_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let proof = key.proof_with(&with(&header, "kid", json!("kid-0")), &claims);
        assert!(verified(&proof).is_ok());
        let [header, claims, signature] = segments(&proof);
        let flipped = format!(
            "{}.{claims}.{signature}",
            with_a_bit_flipped_in(header, "kid-0")
        );
        assert_ne!(flipped, proof);
        assert_eq!(refusal(&flipped), Some(ProofError::Signature));
    }

    #[test]
    fn a_signature_of_any_length_but_64_bytes_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let proof = key.proof_with(&header, &claims);
        let [header, claims, signature] = segments(&proof);
        assert_eq!(signature.len(), SIGNATURE_TEXT_LEN);
        for len in [0, 32, 63, 65] {
            let resized = reencoded(signature, |bytes| bytes.resize(len, 0x5a));
            assert_eq!(
                refusal(&format!("{header}.{claims}.{resized}")),
                Some(ProofError::Signature),
                "{len} bytes"
            );
        }
    }

    #[test]
    fn a_proof_whose_jwk_carries_a_d_member_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let private = with(&key.jwk(), "d", json!(base64url_encode(&[0x5a; 32])));
        let proof = key.proof_with(&with(&header, "jwk", private), &claims);
        assert_eq!(refusal(&proof), Some(ProofError::Key));
    }

    #[test]
    fn a_proof_whose_jwk_is_not_a_p256_public_key_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let jwk = key.jwk();
        for jwk in [
            with(&jwk, "kty", json!("RSA")),
            with(&jwk, "crv", json!("P-384")),
            with(&jwk, "x", json!(base64url_encode(&[0x5a; 31]))),
            without(&jwk, "y"),
            json!(key.thumbprint()),
            json!(null),
        ] {
            let proof = key.proof_with(&with(&header, "jwk", jwk.clone()), &claims);
            assert_eq!(refusal(&proof), Some(ProofError::Key), "{jwk}");
        }
        let proof = key.proof_with(&without(&header, "jwk"), &claims);
        assert_eq!(refusal(&proof), Some(ProofError::Key));
    }

    /// For one `x`, two `y` lie on the curve, and they sum to the field's
    /// prime, which is odd: one of them is even and the other is not. `y`
    /// with its second-lowest bit flipped has the parity of `y` and is not
    /// `y`, so it is neither of the two.
    #[test]
    fn a_proof_whose_jwk_is_a_point_off_the_curve_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let jwk = key.jwk();
        let y = reencoded(jwk["y"].as_str().unwrap(), |bytes| bytes[31] ^= 0x02);
        let off_curve = with(&jwk, "y", json!(y));
        assert!(DeviceJwk::parse(&off_curve).is_ok());
        let proof = key.proof_with(&with(&header, "jwk", off_curve), &claims);
        assert_eq!(refusal(&proof), Some(ProofError::Signature));
    }

    #[test]
    fn a_proof_carrying_a_key_that_did_not_sign_it_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let other = SoftDeviceKey::new();
        let proof = key.proof_with(&with(&header, "jwk", other.jwk()), &claims);
        assert_eq!(refusal(&proof), Some(ProofError::Signature));
    }

    #[test]
    fn a_proof_of_two_or_of_four_segments_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let proof = key.proof_with(&header, &claims);
        let [header, claims, signature] = segments(&proof);
        for other in [
            format!("{header}.{claims}"),
            format!("{header}.{signature}"),
            format!("{header}{claims}{signature}"),
            format!("{proof}."),
            format!("{proof}.{signature}"),
            format!("{header}.{claims}..{signature}"),
            String::new(),
        ] {
            assert_eq!(refusal(&other), Some(ProofError::Malformed), "{other}");
        }
    }

    #[test]
    fn a_segment_that_is_not_unpadded_base64url_of_a_json_object_is_refused() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let proof = key.proof_with(&header, &claims);
        let [header, claims, signature] = segments(&proof);
        let array = base64url_encode(b"[]");
        let text = base64url_encode(b"not json");
        for other in [
            format!("{header}=.{claims}.{signature}"),
            format!("{header}.{claims}=.{signature}"),
            format!("{header}.{claims}.{signature}="),
            format!("{header}.{claims}.{signature}*"),
            format!("{array}.{claims}.{signature}"),
            format!("{header}.{array}.{signature}"),
            format!("{text}.{claims}.{signature}"),
            format!("{header}.{text}.{signature}"),
        ] {
            assert_eq!(refusal(&other), Some(ProofError::Malformed), "{other}");
        }
    }

    #[test]
    fn a_proof_of_4096_bytes_is_accepted_and_one_byte_longer_is_refused() {
        let control = control();
        let longest = proof_of_len(&control, LONGEST_PROOF);
        assert!(verified(&longest).is_ok());
        let longer = proof_of_len(&control, LONGEST_PROOF + 1);
        assert_eq!(refusal(&longer), Some(ProofError::Malformed));
    }

    /// The signature is checked last: a proof wrong in its structure is
    /// refused for that, with a signature that would have failed too.
    #[test]
    fn a_proof_wrong_in_structure_and_in_signature_is_refused_for_its_structure() {
        let Control {
            key,
            header,
            claims,
        } = control();
        let cases = [
            (
                with(&header, "typ", json!("jwt")),
                claims.clone(),
                ProofError::Type,
            ),
            (
                with(&header, "alg", json!("HS256")),
                claims.clone(),
                ProofError::Algorithm,
            ),
            (without(&header, "jwk"), claims.clone(), ProofError::Key),
            (
                header.clone(),
                with(&claims, "htm", json!("GET")),
                ProofError::Method,
            ),
            (
                header.clone(),
                with(&claims, "htu", json!("/")),
                ProofError::Uri,
            ),
            (
                header.clone(),
                with(&claims, "iat", json!(0)),
                ProofError::Stale,
            ),
            (
                header.clone(),
                with(&claims, "jti", json!("")),
                ProofError::Malformed,
            ),
            (header.clone(), claims.clone(), ProofError::Signature),
        ];
        for (header, claims, error) in cases {
            let proof = key.proof_with(&header, &claims);
            let [header, claims, signature] = segments(&proof);
            let forged = reencoded(signature, |bytes| bytes[0] ^= 0x01);
            assert_eq!(
                refusal(&format!("{header}.{claims}.{forged}")),
                Some(error),
                "{error}"
            );
        }
    }
}

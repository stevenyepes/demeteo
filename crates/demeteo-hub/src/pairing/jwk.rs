//! The device key as its holder presents it: an ES256 public JWK (RFC 7517),
//! and the two names the Hub derives from it.
//!
//! The thumbprint (RFC 7638) is the key's identity: what a token is bound to
//! and what a proof is compared against. It is a digest over JSON text, so it
//! is computed over text built here, from the decoded coordinates, and never
//! over what the client sent. A client chooses its member order, its
//! whitespace and its extra members, and a digest that followed those would
//! give one key as many identities as it has spellings.
//!
//! P-256 only, as [`auth::cose`](crate::auth::cose) is: a key of any other
//! type or curve has no form here and is refused.

use crate::auth::challenge::{base64url_decode, base64url_encode};
use crate::auth::cose::P256Point;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const FINGERPRINT_PREFIX: &str = "SHA256:";
const FINGERPRINT_GROUPS: usize = 4;
const FINGERPRINT_GROUP_LEN: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JwkError {
    #[error("a JWK is a JSON object")]
    NotAnObject,
    #[error("the JWK carries the private member `d`")]
    PrivateMember,
    #[error("`kty` is not `EC`")]
    WrongKeyType,
    #[error("`crv` is not `P-256`")]
    WrongCurve,
    #[error("`{0}` is not unpadded base64url of 32 bytes")]
    BadCoordinate(&'static str),
}

/// A device's public key. Whether the point lies on the curve is not known
/// here, as [`P256Point`] says of itself: the signature verifier refuses one
/// that does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceJwk {
    pub point: P256Point,
}

impl DeviceJwk {
    /// Members other than the four of the thumbprint are ignored, with one
    /// exception. A JWK with a `d` member is a private key, whatever `d`
    /// holds: it is refused outright, because accepting the public half of it
    /// would bind a token to a key its holder has just sent over the wire.
    pub fn parse(jwk: &Value) -> Result<Self, JwkError> {
        let members = jwk.as_object().ok_or(JwkError::NotAnObject)?;
        if members.contains_key("d") {
            return Err(JwkError::PrivateMember);
        }
        if text(members, "kty") != Some("EC") {
            return Err(JwkError::WrongKeyType);
        }
        if text(members, "crv") != Some("P-256") {
            return Err(JwkError::WrongCurve);
        }
        Ok(Self {
            point: P256Point {
                x: coordinate(members, "x")?,
                y: coordinate(members, "y")?,
            },
        })
    }

    /// The required members of an EC key in lexicographic order with no
    /// whitespace (RFC 7638 §3.2): the text the thumbprint is a digest of,
    /// and the only form of the key that is stored.
    pub fn canonical_json(&self) -> String {
        format!(
            r#"{{"crv":"P-256","kty":"EC","x":"{}","y":"{}"}}"#,
            base64url_encode(&self.point.x),
            base64url_encode(&self.point.y),
        )
    }

    pub fn thumbprint(&self) -> String {
        base64url_encode(&Sha256::digest(self.canonical_json().as_bytes()))
    }
}

/// What a user compares between the approval page and their desktop: the
/// first 96 bits of the thumbprint, in groups short enough to read aloud.
/// The desktop renders the same string from the same thumbprint, so this
/// format is not the Hub's alone to change.
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

pub(super) fn text<'a>(members: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    members.get(name).and_then(Value::as_str)
}

fn coordinate(members: &Map<String, Value>, name: &'static str) -> Result<[u8; 32], JwkError> {
    text(members, name)
        .and_then(base64url_decode)
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or(JwkError::BadCoordinate(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The key of the proof in RFC 9449 §4.1. Its thumbprint was computed
    /// outside this crate, with Python's `hashlib` and `base64` over the
    /// canonical text typed by hand.
    const X: &str = "l8tFrhx-34tV3hRICRDY9zCkDlpBhF42UQUfWVAWBFs";
    const Y: &str = "9VE4jf_Ok_o64zbTTlcuNJajHmt6v9TDVrU0CdvGRDA";
    const THUMBPRINT: &str = "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I";

    fn published() -> Value {
        json!({"kty": "EC", "x": X, "y": Y, "crv": "P-256"})
    }

    /// The published key with one member replaced or added. Every refusal
    /// below is of a key that parses once that member is put back, which
    /// [`the_published_key_parses_to_its_two_coordinates`] holds.
    fn with(member: &str, value: Value) -> Value {
        let mut jwk = published();
        jwk[member] = value;
        jwk
    }

    fn without(member: &str) -> Value {
        let mut jwk = published();
        if let Some(members) = jwk.as_object_mut() {
            members.remove(member);
        }
        jwk
    }

    fn of_length(len: usize) -> Value {
        Value::String(base64url_encode(&vec![0x5a; len]))
    }

    #[test]
    fn the_published_key_parses_to_its_two_coordinates() {
        let parsed = DeviceJwk::parse(&published()).unwrap();
        assert_eq!(base64url_encode(&parsed.point.x), X);
        assert_eq!(base64url_encode(&parsed.point.y), Y);
        assert_ne!(parsed.point.x, parsed.point.y);
    }

    #[test]
    fn a_kty_other_than_ec_is_refused() {
        for kty in [json!("RSA"), json!("OKP"), json!("ec"), json!(""), json!(2)] {
            assert_eq!(
                DeviceJwk::parse(&with("kty", kty.clone())),
                Err(JwkError::WrongKeyType),
                "{kty}"
            );
        }
        assert_eq!(
            DeviceJwk::parse(&without("kty")),
            Err(JwkError::WrongKeyType)
        );
    }

    #[test]
    fn a_crv_other_than_p256_is_refused() {
        for crv in [
            json!("P-384"),
            json!("P-521"),
            json!("secp256k1"),
            json!("p-256"),
            json!("P256"),
            json!(1),
        ] {
            assert_eq!(
                DeviceJwk::parse(&with("crv", crv.clone())),
                Err(JwkError::WrongCurve),
                "{crv}"
            );
        }
        assert_eq!(DeviceJwk::parse(&without("crv")), Err(JwkError::WrongCurve));
    }

    #[test]
    fn a_coordinate_of_32_bytes_is_accepted_and_one_byte_either_side_is_refused() {
        for name in ["x", "y"] {
            assert!(DeviceJwk::parse(&with(name, of_length(32))).is_ok());
            for len in [0, 1, 31, 33, 48, 64] {
                assert_eq!(
                    DeviceJwk::parse(&with(name, of_length(len))),
                    Err(JwkError::BadCoordinate(name)),
                    "{name} of {len} bytes"
                );
            }
        }
    }

    #[test]
    fn a_coordinate_that_is_missing_or_not_a_string_is_refused() {
        for name in ["x", "y"] {
            assert_eq!(
                DeviceJwk::parse(&without(name)),
                Err(JwkError::BadCoordinate(name))
            );
            for value in [json!(null), json!(7), json!([X])] {
                assert_eq!(
                    DeviceJwk::parse(&with(name, value.clone())),
                    Err(JwkError::BadCoordinate(name)),
                    "{name} = {value}"
                );
            }
        }
    }

    /// Each of these is the right 32 bytes to a lenient decoder.
    #[test]
    fn a_coordinate_in_any_other_text_form_of_its_bytes_is_refused() {
        let other_forms = [
            ("x", format!("{X}="), "padded"),
            ("y", format!("{Y}="), "padded"),
            ("x", X.replace('-', "+"), "standard alphabet"),
            ("y", Y.replace('_', "/"), "standard alphabet"),
            ("x", format!("{}t", &X[..X.len() - 1]), "trailing bits"),
        ];
        for (name, form, why) in other_forms {
            assert_ne!(form, published()[name]);
            assert_eq!(
                DeviceJwk::parse(&with(name, Value::String(form))),
                Err(JwkError::BadCoordinate(name)),
                "{name} {why}"
            );
        }
    }

    #[test]
    fn a_key_carrying_a_d_member_is_refused_whatever_d_holds() {
        for d in [of_length(32), json!(""), json!(null), json!(false)] {
            assert_eq!(
                DeviceJwk::parse(&with("d", d.clone())),
                Err(JwkError::PrivateMember),
                "d = {d}"
            );
        }
    }

    #[test]
    fn anything_but_a_json_object_is_refused() {
        for jwk in [json!(null), json!("EC"), json!([published()])] {
            assert_eq!(DeviceJwk::parse(&jwk), Err(JwkError::NotAnObject), "{jwk}");
        }
    }

    #[test]
    fn the_canonical_json_is_the_four_required_members_in_order_with_no_whitespace() {
        let parsed = DeviceJwk::parse(&published()).unwrap();
        assert_eq!(
            parsed.canonical_json(),
            "{\"crv\":\"P-256\",\"kty\":\"EC\",\
             \"x\":\"l8tFrhx-34tV3hRICRDY9zCkDlpBhF42UQUfWVAWBFs\",\
             \"y\":\"9VE4jf_Ok_o64zbTTlcuNJajHmt6v9TDVrU0CdvGRDA\"}"
        );
    }

    #[test]
    fn the_thumbprint_of_the_published_key_is_the_known_answer() {
        let parsed = DeviceJwk::parse(&published()).unwrap();
        assert_eq!(parsed.thumbprint(), THUMBPRINT);
    }

    /// Re-serialising what the client sent would carry every one of these
    /// members into the digest.
    #[test]
    fn members_outside_the_required_four_change_neither_the_thumbprint_nor_the_canonical_json() {
        let mut dressed = published();
        dressed["alg"] = json!("ES256");
        dressed["use"] = json!("sig");
        dressed["kid"] = json!("a");
        dressed["a"] = json!({"nested": [1, 2]});
        let parsed = DeviceJwk::parse(&dressed).unwrap();
        assert_eq!(parsed, DeviceJwk::parse(&published()).unwrap());
        assert_eq!(parsed.thumbprint(), THUMBPRINT);
        assert!(!parsed.canonical_json().contains("alg"));
    }

    #[test]
    fn swapping_the_coordinates_changes_the_thumbprint() {
        let swapped = DeviceJwk::parse(&json!({"kty": "EC", "crv": "P-256", "x": Y, "y": X}));
        assert_ne!(swapped.unwrap().thumbprint(), THUMBPRINT);
    }

    #[test]
    fn the_fingerprint_is_the_first_16_characters_in_four_groups_of_four() {
        assert_eq!(fingerprint(THUMBPRINT), "SHA256:0ZcO CORZ NYy- DWpq");
        assert_eq!(
            fingerprint("abcdefghijklmnopqrstuvwxyz"),
            "SHA256:abcd efgh ijkl mnop"
        );
    }

    #[test]
    fn two_thumbprints_that_differ_only_after_the_16th_character_share_a_fingerprint() {
        let other = format!("{}{}", &THUMBPRINT[..16], "A".repeat(27));
        assert_ne!(other, THUMBPRINT);
        assert_eq!(fingerprint(&other), fingerprint(THUMBPRINT));
        let earlier = format!("{}A{}", &THUMBPRINT[..15], &THUMBPRINT[16..]);
        assert_ne!(fingerprint(&earlier), fingerprint(THUMBPRINT));
    }

    #[test]
    fn a_string_shorter_than_16_characters_is_grouped_as_far_as_it_goes() {
        assert_eq!(fingerprint(""), "SHA256:");
        assert_eq!(fingerprint("abcd"), "SHA256:abcd");
        assert_eq!(fingerprint("abcde"), "SHA256:abcd e");
    }
}

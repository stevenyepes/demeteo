//! The one canonical byte encoder, and the encoding contract it implements.
//!
//! Every signed or challenged byte string in this crate is built here, by hand.
//! No `serde` or `serde_json` call may participate in producing these bytes:
//! JSON key order, whitespace and number spelling are not stable enough to sign,
//! and a renamed field must not silently change what a human approved.
//!
//! Primitives, all big-endian, no padding, no separators:
//!
//! - `str` / bytes: `u32` byte length, then the UTF-8 bytes. The length counts
//!   bytes, not chars. Longer than `u32::MAX` is [`EncodeError::TooLong`].
//! - `u8`, `u32`, `u64`, `i64`: fixed width.
//! - `Option<T>`: `0x00`, or `0x01` followed by `T`.
//! - `Vec<T>`: `u32` item count, then each item.
//! - Enums: an explicit tag byte, never the serde spelling — renaming a variant
//!   must not change signed bytes. `Decision`: approve=1, cancel=2. `Effort`:
//!   low=1, medium=2, high=3, xhigh=4, max=5.
//!
//! Each builder begins with `str(label)` then `u32(version)`, so two purposes
//! cannot produce equal bytes. Builders return the **pre-image**, not a digest:
//! SHA-256, base64url and signature verification belong to the caller.

use std::fmt;

use crate::{Decision, Effort, PROTOCOL_VERSION};

/// Why a value could not be encoded. A refusal to produce bytes; never
/// truncated or approximate output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    /// A string, byte run or list longer than `u32::MAX`.
    TooLong,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncodeError::TooLong => {
                f.write_str("value is longer than u32::MAX and cannot be length-prefixed")
            }
        }
    }
}

impl std::error::Error for EncodeError {}

/// Over `usize` rather than a slice so the overflow case is testable without
/// allocating 4 GiB.
fn len_prefix(len: usize) -> Result<u32, EncodeError> {
    u32::try_from(len).map_err(|_| EncodeError::TooLong)
}

pub(crate) fn decision_tag(decision: Decision) -> u8 {
    match decision {
        Decision::Approve => 1,
        Decision::Cancel => 2,
    }
}

pub(crate) fn effort_tag(effort: Effort) -> u8 {
    match effort {
        Effort::Low => 1,
        Effort::Medium => 2,
        Effort::High => 3,
        Effort::Xhigh => 4,
        Effort::Max => 5,
    }
}

pub(crate) struct Encoder {
    buf: Vec<u8>,
}

impl Encoder {
    /// Starts a message with its domain label and version.
    pub(crate) fn begin(label: &str, version: u32) -> Result<Self, EncodeError> {
        let mut enc = Encoder { buf: Vec::new() };
        enc.put_str(label)?;
        enc.put_u32(version);
        Ok(enc)
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.buf
    }

    pub(crate) fn put_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub(crate) fn put_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub(crate) fn put_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub(crate) fn put_str(&mut self, s: &str) -> Result<(), EncodeError> {
        let len = len_prefix(s.len())?;
        self.put_u32(len);
        self.buf.extend_from_slice(s.as_bytes());
        Ok(())
    }

    pub(crate) fn put_opt<T>(
        &mut self,
        v: Option<T>,
        put: impl FnOnce(&mut Self, T) -> Result<(), EncodeError>,
    ) -> Result<(), EncodeError> {
        match v {
            None => {
                self.put_u8(0);
                Ok(())
            }
            Some(inner) => {
                self.put_u8(1);
                put(self, inner)
            }
        }
    }

    pub(crate) fn put_vec<T>(
        &mut self,
        items: &[T],
        mut put: impl FnMut(&mut Self, &T) -> Result<(), EncodeError>,
    ) -> Result<(), EncodeError> {
        let count = len_prefix(items.len())?;
        self.put_u32(count);
        for item in items {
            put(self, item)?;
        }
        Ok(())
    }
}

/// The pre-image of the WebAuthn challenge a human's passkey signs to decide a
/// gate. It is the bytes only: hashing them with SHA-256 and base64url-encoding
/// the digest is the caller's job, as is verifying any signature over it.
///
/// Layout: `str("demeteo-hub/v1/gate")`, `u32(PROTOCOL_VERSION)`, `str
/// instance_id`, `str feature_id`, `str step_execution_id`, `u8 decision`,
/// `str request_id`, `u64 expires_at`.
///
/// `feedback` is deliberately **not** bound. A relaying Hub can rewrite it, so
/// no consumer may give it authority.
pub fn gate_challenge(
    instance_id: &str,
    feature_id: &str,
    step_execution_id: &str,
    decision: Decision,
    request_id: &str,
    expires_at: u64,
) -> Result<Vec<u8>, EncodeError> {
    let mut enc = Encoder::begin("demeteo-hub/v1/gate", PROTOCOL_VERSION)?;
    enc.put_str(instance_id)?;
    enc.put_str(feature_id)?;
    enc.put_str(step_execution_id)?;
    enc.put_u8(decision_tag(decision));
    enc.put_str(request_id)?;
    enc.put_u64(expires_at);
    Ok(enc.finish())
}

/// The pre-image of the challenge an already-trusted passkey signs to endorse a
/// new one into the pinned set. It is the bytes only: SHA-256, base64url and
/// verifying the assertion are the caller's job.
///
/// Layout: `str("demeteo-hub/v1/endorse")`, `u32(PROTOCOL_VERSION)`, `str
/// instance_id`, `str request_id`, `u64 expires_at`, `str new_credential_id`,
/// `str new_public_key`.
pub fn endorsement_challenge(
    instance_id: &str,
    request_id: &str,
    expires_at: u64,
    new_credential_id: &str,
    new_public_key: &str,
) -> Result<Vec<u8>, EncodeError> {
    let mut enc = Encoder::begin("demeteo-hub/v1/endorse", PROTOCOL_VERSION)?;
    enc.put_str(instance_id)?;
    enc.put_str(request_id)?;
    enc.put_u64(expires_at);
    enc.put_str(new_credential_id)?;
    enc.put_str(new_public_key)?;
    Ok(enc.finish())
}

/// The pre-image of the challenge a trusted passkey signs to remove another from
/// the pinned set. It is the bytes only: SHA-256, base64url and verifying the
/// assertion are the caller's job.
///
/// Revocation is its own purpose, with its own label, because
/// [`docs/HUB.md`](../../../docs/HUB.md) §8 requires an assertion over the
/// credential to remove (prefix `demeteo-hub/v1/revoke`); an endorsement
/// signature must never be replayable as a removal.
///
/// Layout: `str("demeteo-hub/v1/revoke")`, `u32(PROTOCOL_VERSION)`, `str
/// instance_id`, `str request_id`, `u64 expires_at`, `str credential_id`.
pub fn revocation_challenge(
    instance_id: &str,
    request_id: &str,
    expires_at: u64,
    credential_id: &str,
) -> Result<Vec<u8>, EncodeError> {
    let mut enc = Encoder::begin("demeteo-hub/v1/revoke", PROTOCOL_VERSION)?;
    enc.put_str(instance_id)?;
    enc.put_str(request_id)?;
    enc.put_u64(expires_at);
    enc.put_str(credential_id)?;
    Ok(enc.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    type Challenge = Result<Vec<u8>, EncodeError>;

    fn baseline() -> Challenge {
        gate_challenge("inst", "feat", "exec", Decision::Approve, "req", 1000)
    }

    #[test]
    fn equal_inputs_give_identical_bytes() {
        assert_eq!(baseline(), baseline());
        assert!(baseline().is_ok());
    }

    #[test]
    fn each_bound_field_changes_the_bytes() {
        let base = baseline();
        let variants: [(&str, Challenge); 6] = [
            (
                "instance_id",
                gate_challenge("inst2", "feat", "exec", Decision::Approve, "req", 1000),
            ),
            (
                "feature_id",
                gate_challenge("inst", "feat2", "exec", Decision::Approve, "req", 1000),
            ),
            (
                "step_execution_id",
                gate_challenge("inst", "feat", "exec2", Decision::Approve, "req", 1000),
            ),
            (
                "decision",
                gate_challenge("inst", "feat", "exec", Decision::Cancel, "req", 1000),
            ),
            (
                "request_id",
                gate_challenge("inst", "feat", "exec", Decision::Approve, "req2", 1000),
            ),
            (
                "expires_at",
                gate_challenge("inst", "feat", "exec", Decision::Approve, "req", 1001),
            ),
        ];
        for (field, bytes) in variants {
            assert!(bytes.is_ok(), "{field} variant must encode");
            assert_ne!(bytes, base, "changing {field} must change the bytes");
        }
    }

    #[test]
    fn adjacent_strings_cannot_be_shifted_across_a_boundary() {
        let a = gate_challenge("ab", "c", "x", Decision::Approve, "r", 1);
        let b = gate_challenge("a", "bc", "x", Decision::Approve, "r", 1);
        assert!(a.is_ok() && b.is_ok());
        assert_ne!(a, b);

        let a = gate_challenge("i", "ab", "c", Decision::Approve, "r", 1);
        let b = gate_challenge("i", "a", "bc", Decision::Approve, "r", 1);
        assert_ne!(a, b);
    }

    #[test]
    fn empty_strings_encode_as_a_zero_length_prefix() {
        let bytes = gate_challenge("", "", "", Decision::Cancel, "", 0);
        let expected: Vec<u8> = [
            &[0, 0, 0, 19][..],
            b"demeteo-hub/v1/gate",
            &[0, 0, 0, 1],
            &[0, 0, 0, 0],
            &[0, 0, 0, 0],
            &[0, 0, 0, 0],
            &[2],
            &[0, 0, 0, 0],
            &[0, 0, 0, 0, 0, 0, 0, 0],
        ]
        .concat();
        assert_eq!(bytes, Ok(expected));
    }

    #[test]
    fn length_prefix_counts_bytes_not_chars() {
        // "é" is 1 char and 2 bytes; "日本" is 2 chars and 6 bytes.
        let bytes = gate_challenge("é", "日本", "", Decision::Approve, "", 0);
        let expected: Vec<u8> = [
            &[0, 0, 0, 19][..],
            b"demeteo-hub/v1/gate",
            &[0, 0, 0, 1],
            &[0, 0, 0, 2, 0xC3, 0xA9],
            &[0, 0, 0, 6, 0xE6, 0x97, 0xA5, 0xE6, 0x9C, 0xAC],
            &[0, 0, 0, 0],
            &[1],
            &[0, 0, 0, 0],
            &[0, 0, 0, 0, 0, 0, 0, 0],
        ]
        .concat();
        assert_eq!(bytes, Ok(expected));
    }

    #[test]
    fn golden_bytes_match_the_documented_layout() {
        let golden: Vec<u8> = vec![
            // str("demeteo-hub/v1/gate")
            0x00, 0x00, 0x00, 0x13, 0x64, 0x65, 0x6D, 0x65, 0x74, 0x65, 0x6F, 0x2D, 0x68, 0x75,
            0x62, 0x2F, 0x76, 0x31, 0x2F, 0x67, 0x61, 0x74, 0x65,
            // u32(PROTOCOL_VERSION = 1)
            0x00, 0x00, 0x00, 0x01, // str "i1"
            0x00, 0x00, 0x00, 0x02, 0x69, 0x31, // str "f1"
            0x00, 0x00, 0x00, 0x02, 0x66, 0x31, // str "se1"
            0x00, 0x00, 0x00, 0x03, 0x73, 0x65, 0x31, // u8 decision: approve
            0x01, // str "r1"
            0x00, 0x00, 0x00, 0x02, 0x72, 0x31, // u64 expires_at = 258
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02,
        ];
        let actual = gate_challenge("i1", "f1", "se1", Decision::Approve, "r1", 258);
        assert_eq!(actual, Ok(golden));
    }

    fn endorse_baseline() -> Challenge {
        endorsement_challenge("inst", "req", 1000, "cred", "key")
    }

    fn revoke_baseline() -> Challenge {
        revocation_challenge("inst", "req", 1000, "cred")
    }

    #[test]
    fn endorsement_and_revocation_are_deterministic() {
        assert!(endorse_baseline().is_ok());
        assert_eq!(endorse_baseline(), endorse_baseline());
        assert!(revoke_baseline().is_ok());
        assert_eq!(revoke_baseline(), revoke_baseline());
    }

    #[test]
    fn each_endorsement_field_changes_the_bytes() {
        let base = endorse_baseline();
        let variants: [(&str, Challenge); 5] = [
            (
                "instance_id",
                endorsement_challenge("inst2", "req", 1000, "cred", "key"),
            ),
            (
                "request_id",
                endorsement_challenge("inst", "req2", 1000, "cred", "key"),
            ),
            (
                "expires_at",
                endorsement_challenge("inst", "req", 1001, "cred", "key"),
            ),
            (
                "new_credential_id",
                endorsement_challenge("inst", "req", 1000, "cred2", "key"),
            ),
            (
                "new_public_key",
                endorsement_challenge("inst", "req", 1000, "cred", "key2"),
            ),
        ];
        for (field, bytes) in variants {
            assert!(bytes.is_ok(), "{field} variant must encode");
            assert_ne!(bytes, base, "changing {field} must change the bytes");
        }
    }

    #[test]
    fn each_revocation_field_changes_the_bytes() {
        let base = revoke_baseline();
        let variants: [(&str, Challenge); 4] = [
            (
                "instance_id",
                revocation_challenge("inst2", "req", 1000, "cred"),
            ),
            (
                "request_id",
                revocation_challenge("inst", "req2", 1000, "cred"),
            ),
            (
                "expires_at",
                revocation_challenge("inst", "req", 1001, "cred"),
            ),
            (
                "credential_id",
                revocation_challenge("inst", "req", 1000, "cred2"),
            ),
        ];
        for (field, bytes) in variants {
            assert!(bytes.is_ok(), "{field} variant must encode");
            assert_ne!(bytes, base, "changing {field} must change the bytes");
        }
    }

    #[test]
    fn endorsement_credential_and_key_cannot_be_shifted_across_a_boundary() {
        let a = endorsement_challenge("i", "r", 1, "ab", "c");
        let b = endorsement_challenge("i", "r", 1, "a", "bc");
        assert!(a.is_ok() && b.is_ok());
        assert_ne!(a, b);
    }

    #[test]
    fn gate_endorse_and_revoke_never_collide_on_overlapping_inputs() {
        // Same instance_id/request_id/expires_at, and the same string reused in
        // every remaining slot.
        let gate = gate_challenge("s", "s", "s", Decision::Approve, "s", 7);
        let endorse = endorsement_challenge("s", "s", 7, "s", "s");
        let revoke = revocation_challenge("s", "s", 7, "s");
        assert!(gate.is_ok() && endorse.is_ok() && revoke.is_ok());
        assert_ne!(gate, endorse);
        assert_ne!(gate, revoke);
        assert_ne!(endorse, revoke);

        let endorse = endorsement_challenge("i", "r", 7, "c", "c");
        let revoke = revocation_challenge("i", "r", 7, "c");
        assert_ne!(endorse, revoke);
    }

    fn label_of(bytes: &[u8]) -> Option<&[u8]> {
        let len = u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
        bytes.get(4..4 + len)
    }

    #[test]
    fn each_purpose_opens_with_its_own_label() {
        // The layouts differ, so overlapping inputs alone cannot show a shared
        // label; this pins the label itself.
        let gate = gate_challenge("s", "s", "s", Decision::Approve, "s", 7).unwrap();
        let endorse = endorsement_challenge("s", "s", 7, "s", "s").unwrap();
        let revoke = revocation_challenge("s", "s", 7, "s").unwrap();
        let labels = [&gate, &endorse, &revoke].map(|b| label_of(b));
        assert_eq!(
            labels,
            [
                Some(&b"demeteo-hub/v1/gate"[..]),
                Some(&b"demeteo-hub/v1/endorse"[..]),
                Some(&b"demeteo-hub/v1/revoke"[..]),
            ]
        );
    }

    #[test]
    fn length_check_rejects_what_does_not_fit_in_u32() {
        assert_eq!(len_prefix(0), Ok(0));
        assert_eq!(len_prefix(u32::MAX as usize), Ok(u32::MAX));
        assert_eq!(len_prefix(u32::MAX as usize + 1), Err(EncodeError::TooLong));
    }

    #[test]
    fn tags_are_the_documented_bytes() {
        assert_eq!(
            [
                decision_tag(Decision::Approve),
                decision_tag(Decision::Cancel)
            ],
            [1, 2]
        );
        let efforts = [
            Effort::Low,
            Effort::Medium,
            Effort::High,
            Effort::Xhigh,
            Effort::Max,
        ];
        assert_eq!(efforts.map(effort_tag), [1, 2, 3, 4, 5]);
    }

    #[test]
    fn option_and_vec_use_presence_byte_and_count_prefix() {
        let mut enc = Encoder { buf: Vec::new() };
        assert_eq!(
            enc.put_opt(None::<u8>, |e, v| {
                e.put_u8(v);
                Ok(())
            }),
            Ok(())
        );
        assert_eq!(
            enc.put_opt(Some(7u8), |e, v| {
                e.put_u8(v);
                Ok(())
            }),
            Ok(())
        );
        assert_eq!(
            enc.put_vec(&[1u32, 2], |e, v| {
                e.put_u32(*v);
                Ok(())
            }),
            Ok(())
        );
        assert_eq!(
            enc.finish(),
            vec![0, 1, 7, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2]
        );
    }

    #[test]
    fn encode_error_displays_and_is_an_error() {
        let err: &dyn std::error::Error = &EncodeError::TooLong;
        assert!(!err.to_string().is_empty());
    }
}

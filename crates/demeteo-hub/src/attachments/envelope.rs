//! Sealing an attachment under its data key, and the size and expiry verdicts.
//! Synchronous and free of I/O: the key arrives already unwrapped, and what is
//! sealed goes back to the caller to store.
//!
//! A sealed value is `nonce ‖ ciphertext ‖ tag`, AES-256-GCM with a random
//! nonce. The AAD binds it to one row ([`aad`]), so a row's ciphertext copied
//! into another row does not open. The digest is sealed under the same key
//! with [`DIGEST_LABEL`] appended to that AAD, so a sealed digest cannot be
//! opened as the content, nor the content as a digest.

use std::collections::HashSet;

use aws_lc_rs::aead::{Aad, Nonce, RandomizedNonceKey, AES_256_GCM, MAX_TAG_LEN, NONCE_LEN};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::MAX_ATTACHMENTS_PER_DISPATCH;
use crate::store::attachments::AttachmentMeta;
use crate::store::{InstanceId, UserId};

pub const AAD_LEN: usize = 48;
pub const TAG_LEN: usize = MAX_TAG_LEN;
pub const DIGEST_LEN: usize = 32;
pub const DIGEST_LABEL: &[u8] = b"digest";

/// Never carries a byte of the key, the content or its digest: these messages
/// reach logs, and a refusal says only which step failed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EnvelopeError {
    #[error("the data key is not an AES-256 key")]
    BadKey,
    #[error("sealing failed")]
    SealFailed,
    #[error("the sealed value is shorter than a nonce and a tag")]
    Truncated,
    /// A wrong key, another row's AAD and a tampered byte are one refusal:
    /// AES-GCM cannot tell them apart, and neither should a caller.
    #[error("the sealed value did not authenticate")]
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{len} bytes exceed the {max}-byte attachment limit")]
pub struct TooLarge {
    pub len: u64,
    pub max: u64,
}

/// `id ‖ user ‖ instance`, sixteen bytes each.
pub fn aad(id: Uuid, user: UserId, instance: InstanceId) -> [u8; AAD_LEN] {
    let mut out = [0u8; AAD_LEN];
    out[..16].copy_from_slice(id.as_bytes());
    out[16..32].copy_from_slice(user.handle().as_bytes());
    out[32..].copy_from_slice(instance.0.as_bytes());
    out
}

pub fn seal(
    data_key: &[u8],
    aad: &[u8; AAD_LEN],
    plaintext: &[u8],
) -> Result<Vec<u8>, EnvelopeError> {
    seal_with(data_key, aad, plaintext)
}

pub fn open(data_key: &[u8], aad: &[u8; AAD_LEN], sealed: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
    open_with(data_key, aad, sealed)
}

/// Seals the SHA-256 of `plaintext`, computed here so a caller cannot seal a
/// digest of anything else.
pub fn seal_digest(
    data_key: &[u8],
    aad: &[u8; AAD_LEN],
    plaintext: &[u8],
) -> Result<Vec<u8>, EnvelopeError> {
    seal_with(data_key, &digest_aad(aad), &Sha256::digest(plaintext))
}

pub fn open_digest(
    data_key: &[u8],
    aad: &[u8; AAD_LEN],
    sealed: &[u8],
) -> Result<[u8; DIGEST_LEN], EnvelopeError> {
    let opened = open_with(data_key, &digest_aad(aad), sealed)?;
    <[u8; DIGEST_LEN]>::try_from(opened.as_slice()).map_err(|_| EnvelopeError::Rejected)
}

/// The plaintext length of a sealed value `ciphertext_len` bytes long.
pub fn content_len(ciphertext_len: u64) -> u64 {
    ciphertext_len.saturating_sub((NONCE_LEN + TAG_LEN) as u64)
}

/// `max` itself is accepted.
pub fn size_verdict(len: u64, max: u64) -> Result<(), TooLarge> {
    if len > max {
        return Err(TooLarge { len, max });
    }
    Ok(())
}

pub fn expiry_for(now: u64, ttl_secs: u64) -> u64 {
    now.saturating_add(ttl_secs)
}

/// Why a dispatch's attachment ids are refused. Another user's attachment,
/// another instance's and an id nobody holds are one [`NotFound`](Self::NotFound):
/// telling them apart would tell a tenant which ids exist elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttachmentRefusal {
    #[error("the attachment ids repeat one, or name more than a dispatch may")]
    Invalid,
    #[error("an attachment id names nothing this instance holds")]
    NotFound,
    #[error("an attachment is already bound to a request, or has expired")]
    Unavailable,
}

/// The ids as the store reads them, once their shape is acceptable. A
/// duplicate or too many ids is refused before an id that is not a UUID, so
/// a malformed list is [`AttachmentRefusal::Invalid`] whatever it holds.
pub fn requested_ids(ids: &[String]) -> Result<Vec<Uuid>, AttachmentRefusal> {
    if ids.len() > MAX_ATTACHMENTS_PER_DISPATCH {
        return Err(AttachmentRefusal::Invalid);
    }
    let parsed: Vec<Result<Uuid, &str>> = ids
        .iter()
        .map(|id| Uuid::parse_str(id).map_err(|_| id.as_str()))
        .collect();
    let mut seen = HashSet::with_capacity(parsed.len());
    if !parsed.iter().all(|id| seen.insert(*id)) {
        return Err(AttachmentRefusal::Invalid);
    }
    parsed
        .into_iter()
        .map(|id| id.map_err(|_| AttachmentRefusal::NotFound))
        .collect()
}

/// The rows `ids` name, in request order, when every one of them may be bound
/// to a request made at `now`. `ids` are as [`requested_ids`] answered them,
/// already judged for their shape. `rows` is whatever the store returned for
/// the tenant and `instance`; a row of another instance among them is not
/// found.
///
/// Every id is resolved before any is judged available, so one foreign id
/// refuses the set as [`AttachmentRefusal::NotFound`] wherever it stands.
pub fn dispatch_verdict<'a>(
    ids: &[Uuid],
    rows: &'a [AttachmentMeta],
    instance: InstanceId,
    now: u64,
) -> Result<Vec<&'a AttachmentMeta>, AttachmentRefusal> {
    let resolved = ids
        .iter()
        .map(|id| {
            rows.iter()
                .find(|row| row.id == *id && row.instance_id == instance)
                .ok_or(AttachmentRefusal::NotFound)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if resolved
        .iter()
        .any(|row| row.request_id.is_some() || row.expires_at <= now)
    {
        return Err(AttachmentRefusal::Unavailable);
    }
    Ok(resolved)
}

fn digest_aad(aad: &[u8; AAD_LEN]) -> [u8; AAD_LEN + DIGEST_LABEL.len()] {
    let mut out = [0u8; AAD_LEN + DIGEST_LABEL.len()];
    out[..AAD_LEN].copy_from_slice(aad);
    out[AAD_LEN..].copy_from_slice(DIGEST_LABEL);
    out
}

fn key(data_key: &[u8]) -> Result<RandomizedNonceKey, EnvelopeError> {
    RandomizedNonceKey::new(&AES_256_GCM, data_key).map_err(|_| EnvelopeError::BadKey)
}

fn seal_with(data_key: &[u8], aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
    let key = key(data_key)?;
    let mut sealed = Vec::with_capacity(NONCE_LEN + plaintext.len() + TAG_LEN);
    sealed.extend_from_slice(&[0u8; NONCE_LEN]);
    sealed.extend_from_slice(plaintext);
    let (nonce, tag) = key
        .seal_in_place_separate_tag(Aad::from(aad), &mut sealed[NONCE_LEN..])
        .map_err(|_| EnvelopeError::SealFailed)?;
    sealed[..NONCE_LEN].copy_from_slice(nonce.as_ref());
    sealed.extend_from_slice(tag.as_ref());
    Ok(sealed)
}

fn open_with(data_key: &[u8], aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
    if sealed.len() < NONCE_LEN + TAG_LEN {
        return Err(EnvelopeError::Truncated);
    }
    let key = key(data_key)?;
    let (nonce, body) = sealed.split_at(NONCE_LEN);
    let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| EnvelopeError::Truncated)?;
    let mut opened = body.to_vec();
    let len = key
        .open_in_place(nonce, Aad::from(aad), &mut opened)
        .map_err(|_| EnvelopeError::Rejected)?
        .len();
    opened.truncate(len);
    Ok(opened)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7u8; 32];

    fn row_aad() -> [u8; AAD_LEN] {
        aad(Uuid::new_v4(), UserId::generate(), InstanceId::generate())
    }

    #[test]
    fn seal_then_open_round_trips() {
        let aad = row_aad();
        for plaintext in [&b""[..], b"x", b"a file the user attached"] {
            let sealed = seal(&KEY, &aad, plaintext).unwrap();
            assert_eq!(open(&KEY, &aad, &sealed).unwrap(), plaintext);
        }
    }

    #[test]
    fn the_ciphertext_is_not_the_plaintext() {
        let plaintext = b"a file the user attached";
        let sealed = seal(&KEY, &row_aad(), plaintext).unwrap();
        assert!(!sealed
            .windows(plaintext.len())
            .any(|window| window == plaintext));
    }

    #[test]
    fn another_rows_aad_does_not_open() {
        let user = UserId::generate();
        let instance = InstanceId::generate();
        let own = aad(Uuid::new_v4(), user, instance);
        let other = aad(Uuid::new_v4(), user, instance);
        let sealed = seal(&KEY, &own, b"content").unwrap();
        assert_eq!(open(&KEY, &other, &sealed), Err(EnvelopeError::Rejected));
    }

    #[test]
    fn another_key_does_not_open() {
        let aad = row_aad();
        let sealed = seal(&KEY, &aad, b"content").unwrap();
        assert_eq!(
            open(&[8u8; 32], &aad, &sealed),
            Err(EnvelopeError::Rejected)
        );
    }

    #[test]
    fn a_flipped_byte_does_not_open() {
        let aad = row_aad();
        let sealed = seal(&KEY, &aad, b"content").unwrap();
        for at in [0, NONCE_LEN, sealed.len() - 1] {
            let mut tampered = sealed.clone();
            tampered[at] ^= 1;
            assert_eq!(
                open(&KEY, &aad, &tampered),
                Err(EnvelopeError::Rejected),
                "byte {at}"
            );
        }
    }

    #[test]
    fn a_truncated_value_and_a_short_key_are_refused() {
        let aad = row_aad();
        let sealed = seal(&KEY, &aad, b"").unwrap();
        assert_eq!(
            open(&KEY, &aad, &sealed[..sealed.len() - 1]),
            Err(EnvelopeError::Truncated)
        );
        assert_eq!(seal(&[0u8; 16], &aad, b"x"), Err(EnvelopeError::BadKey));
    }

    #[test]
    fn content_len_recovers_the_plaintext_length() {
        let aad = row_aad();
        for plaintext in [&b""[..], b"x", &[0u8; 1000]] {
            let sealed = seal(&KEY, &aad, plaintext).unwrap();
            assert_eq!(content_len(sealed.len() as u64), plaintext.len() as u64);
        }
        assert_eq!(content_len(3), 0);
    }

    #[test]
    fn the_limit_itself_is_accepted_and_one_byte_more_is_not() {
        let max = 26_214_400;
        assert_eq!(size_verdict(max, max), Ok(()));
        assert_eq!(
            size_verdict(max + 1, max),
            Err(TooLarge { len: max + 1, max })
        );
    }

    #[test]
    fn expiry_is_ttl_past_now() {
        assert_eq!(expiry_for(1_000, 3600), 4_600);
        assert_eq!(expiry_for(u64::MAX, 3600), u64::MAX);
    }

    #[test]
    fn the_digest_round_trips_and_opens_only_as_a_digest() {
        let aad = row_aad();
        let plaintext = b"a file the user attached";
        let sealed = seal_digest(&KEY, &aad, plaintext).unwrap();
        let expected: [u8; DIGEST_LEN] = Sha256::digest(plaintext).into();
        assert_eq!(open_digest(&KEY, &aad, &sealed).unwrap(), expected);
        assert_eq!(open(&KEY, &aad, &sealed), Err(EnvelopeError::Rejected));

        let content = seal(&KEY, &aad, &expected).unwrap();
        assert_eq!(
            open_digest(&KEY, &aad, &content),
            Err(EnvelopeError::Rejected)
        );
    }

    const NOW: u64 = 1_700_000_000;

    fn staged(instance: InstanceId) -> AttachmentMeta {
        AttachmentMeta {
            id: Uuid::new_v4(),
            instance_id: instance,
            request_id: None,
            wrapped_key: "vault:v1:filler".to_owned(),
            sealed_digest: vec![0xa5; 60],
            size_bytes: 64,
            expires_at: NOW + 60,
        }
    }

    fn named(rows: &[&AttachmentMeta]) -> Vec<String> {
        rows.iter().map(|row| row.id.to_string()).collect()
    }

    /// The ids as a dispatch body holds them, through the parse the handler
    /// makes before the verdict.
    fn verdict<'a>(
        ids: &[String],
        rows: &'a [AttachmentMeta],
        instance: InstanceId,
        now: u64,
    ) -> Result<Vec<&'a AttachmentMeta>, AttachmentRefusal> {
        dispatch_verdict(&requested_ids(ids)?, rows, instance, now)
    }

    #[test]
    fn the_rows_come_back_in_request_order() {
        let instance = InstanceId::generate();
        let (first, second) = (staged(instance), staged(instance));
        let rows = [first.clone(), second.clone()];

        let resolved = verdict(&named(&[&second, &first]), &rows, instance, NOW);

        assert_eq!(resolved, Ok(vec![&rows[1], &rows[0]]));
        assert_eq!(verdict(&[], &rows, instance, NOW), Ok(vec![]));
    }

    #[test]
    fn a_duplicate_id_is_invalid() {
        let instance = InstanceId::generate();
        let row = staged(instance);
        let rows = [row.clone()];
        let upper = row.id.to_string().to_uppercase();
        for ids in [named(&[&row, &row]), vec![row.id.to_string(), upper]] {
            assert_eq!(
                verdict(&ids, &rows, instance, NOW),
                Err(AttachmentRefusal::Invalid),
                "{ids:?}"
            );
        }
        let repeated = vec!["not-a-uuid".to_owned(), "not-a-uuid".to_owned()];
        assert_eq!(
            verdict(&repeated, &rows, instance, NOW),
            Err(AttachmentRefusal::Invalid)
        );
    }

    #[test]
    fn more_ids_than_a_dispatch_may_name_are_invalid() {
        let instance = InstanceId::generate();
        let rows: Vec<AttachmentMeta> = (0..=MAX_ATTACHMENTS_PER_DISPATCH)
            .map(|_| staged(instance))
            .collect();
        let all: Vec<String> = rows.iter().map(|row| row.id.to_string()).collect();

        assert_eq!(
            verdict(&all, &rows, instance, NOW),
            Err(AttachmentRefusal::Invalid)
        );
        let at_limit = &all[..MAX_ATTACHMENTS_PER_DISPATCH];
        assert_eq!(
            verdict(at_limit, &rows, instance, NOW).map(|rows| rows.len()),
            Ok(MAX_ATTACHMENTS_PER_DISPATCH)
        );
    }

    #[test]
    fn an_id_that_is_not_a_uuid_or_matches_no_row_is_not_found() {
        let instance = InstanceId::generate();
        let rows = [staged(instance)];
        for id in [
            "not-a-uuid".to_owned(),
            String::new(),
            Uuid::new_v4().to_string(),
        ] {
            assert_eq!(
                verdict(std::slice::from_ref(&id), &rows, instance, NOW),
                Err(AttachmentRefusal::NotFound),
                "{id:?}"
            );
        }
    }

    #[test]
    fn a_row_of_another_instance_is_not_found() {
        let instance = InstanceId::generate();
        let rows = [staged(InstanceId::generate())];
        assert_eq!(
            verdict(&named(&[&rows[0]]), &rows, instance, NOW),
            Err(AttachmentRefusal::NotFound)
        );
    }

    #[test]
    fn a_bound_or_expired_row_is_unavailable() {
        let instance = InstanceId::generate();
        let bound = AttachmentMeta {
            request_id: Some([3u8; 16]),
            ..staged(instance)
        };
        let expired = AttachmentMeta {
            expires_at: NOW,
            ..staged(instance)
        };
        let live = AttachmentMeta {
            expires_at: NOW + 1,
            ..staged(instance)
        };
        let rows = [bound.clone(), expired.clone(), live.clone()];
        for row in [&bound, &expired] {
            assert_eq!(
                verdict(&named(&[&live, row]), &rows, instance, NOW),
                Err(AttachmentRefusal::Unavailable),
                "{:?}",
                row.id
            );
        }
        assert!(verdict(&named(&[&live]), &rows, instance, NOW).is_ok());
    }

    #[test]
    fn one_foreign_id_refuses_the_whole_set() {
        let instance = InstanceId::generate();
        let own = staged(instance);
        let bound = AttachmentMeta {
            request_id: Some([3u8; 16]),
            ..staged(instance)
        };
        let rows = [own.clone(), bound.clone()];
        let foreign = Uuid::new_v4().to_string();

        for ids in [
            vec![own.id.to_string(), foreign.clone()],
            vec![foreign.clone(), own.id.to_string()],
            vec![bound.id.to_string(), foreign],
        ] {
            assert_eq!(
                verdict(&ids, &rows, instance, NOW),
                Err(AttachmentRefusal::NotFound),
                "{ids:?}"
            );
        }
    }
}

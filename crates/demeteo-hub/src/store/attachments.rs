use super::{Db, InstanceId, StoreError, UserId};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

/// A data key already wrapped by Transit. The `vault:` prefix is the
/// service's own marker; a bare plaintext key does not carry it, so
/// [`WrappedKey::parse`] refuses the one mistake that would put a key in Postgres.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedKey(String);

const TRANSIT_PREFIX: &str = "vault:";

impl WrappedKey {
    pub fn parse(wrapped: impl Into<String>) -> Option<Self> {
        let wrapped = wrapped.into();
        wrapped.starts_with(TRANSIT_PREFIX).then_some(Self(wrapped))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct NewAttachment {
    pub instance_id: InstanceId,
    pub request_id: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub wrapped_key: WrappedKey,
}

#[derive(Debug, Clone, FromRow)]
pub struct Attachment {
    pub id: Uuid,
    pub instance_id: InstanceId,
    pub request_id: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub wrapped_key: String,
    /// SHA-256 of `ciphertext`, computed by [`insert_attachment`] and never
    /// taken from the caller, so it always describes the bytes it sits next to.
    /// Never a digest of the plaintext: a plaintext hash beside the ciphertext
    /// would let anyone with database access confirm a guessed file without
    /// the key.
    pub sha256: Vec<u8>,
    pub size_bytes: i64,
}

fn ciphertext_digest(ciphertext: &[u8]) -> [u8; 32] {
    Sha256::digest(ciphertext).into()
}

fn size_bytes(len: usize) -> Result<i64, StoreError> {
    i64::try_from(len).map_err(|_| StoreError::TooLarge { bytes: len })
}

pub async fn insert_attachment(
    user: UserId,
    db: &Db,
    new: &NewAttachment,
) -> Result<Attachment, StoreError> {
    let size_bytes = size_bytes(new.ciphertext.len())?;
    Ok(sqlx::query_as::<_, Attachment>(
        "INSERT INTO attachments \
         (id, user_id, instance_id, request_id, ciphertext, wrapped_key, sha256, size_bytes) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         RETURNING id, instance_id, request_id, ciphertext, wrapped_key, sha256, size_bytes",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .bind(new.instance_id)
    .bind(&new.request_id)
    .bind(&new.ciphertext)
    .bind(new.wrapped_key.as_str())
    .bind(ciphertext_digest(&new.ciphertext).as_slice())
    .bind(size_bytes)
    .fetch_one(db)
    .await?)
}

pub async fn get_attachment(
    user: UserId,
    db: &Db,
    id: Uuid,
) -> Result<Option<Attachment>, StoreError> {
    Ok(sqlx::query_as::<_, Attachment>(
        "SELECT id, instance_id, request_id, ciphertext, wrapped_key, sha256, size_bytes \
         FROM attachments WHERE user_id = $1 AND id = $2",
    )
    .bind(user)
    .bind(id)
    .fetch_optional(db)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_transit_wrapped_text_is_accepted() {
        assert!(WrappedKey::parse("vault:v1:abc").is_some());
        assert!(WrappedKey::parse("3q2+7w==").is_none());
        assert!(WrappedKey::parse("").is_none());
    }

    #[test]
    fn digest_is_sha256_of_the_ciphertext() {
        let digest = ciphertext_digest(b"abc");
        assert_eq!(digest[..4], [0xba, 0x78, 0x16, 0xbf]);
        assert_eq!(digest[28..], [0xf2, 0x00, 0x15, 0xad]);
    }

    #[test]
    fn a_length_beyond_bigint_is_a_typed_error() {
        assert_eq!(size_bytes(3).unwrap(), 3);
        assert!(matches!(
            size_bytes(usize::MAX),
            Err(StoreError::TooLarge { bytes: usize::MAX })
        ));
    }
}

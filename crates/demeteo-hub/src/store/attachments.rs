use super::requests::{stored_expiry, RequestId, RequestStatus};
use super::{timestamp, Db, InstanceId, StoreError, UserId};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgConnection};
use time::OffsetDateTime;
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

/// An attachment as it is staged by an upload. There is no request yet: it
/// is bound at dispatch by [`bind_to_request`]. `id` is the caller's because
/// the ciphertext is sealed under an AAD that names it, before the row exists.
///
/// None of this, [`AttachmentMeta`] or [`StagedAttachment`] has a field for
/// the content, its digest in the clear or the data key; `docs/HUB.md` §6.3
/// says why.
#[derive(Debug, Clone)]
pub struct NewAttachment {
    pub id: Uuid,
    pub instance_id: InstanceId,
    pub ciphertext: Vec<u8>,
    pub wrapped_key: WrappedKey,
    /// The content's SHA-256, sealed under the same data key as `ciphertext`.
    pub sealed_digest: Vec<u8>,
    /// Unix seconds.
    pub expires_at: u64,
}

/// A row without its ciphertext: what the upload gets back and what a
/// dispatch judges. A dispatch reads up to
/// [`MAX_ATTACHMENTS_PER_DISPATCH`](crate::attachments::MAX_ATTACHMENTS_PER_DISPATCH)
/// rows before its verdict, so one carrying the ciphertext would load each
/// in full, refused or not, on every dispatch a tenant repeats.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentMeta {
    pub id: Uuid,
    pub instance_id: InstanceId,
    /// `None` while staged; set once, to the request that named it.
    pub request_id: Option<RequestId>,
    pub wrapped_key: String,
    pub sealed_digest: Vec<u8>,
    /// The length of the ciphertext, not of the content.
    pub size_bytes: u64,
    /// Unix seconds: the upload's TTL while staged, the request's expiry once
    /// bound.
    pub expires_at: u64,
}

/// The whole row. Only [`fetchable`] reads one: the fetch is the one caller
/// that hands the ciphertext on.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedAttachment {
    pub id: Uuid,
    pub instance_id: InstanceId,
    pub request_id: Option<RequestId>,
    pub ciphertext: Vec<u8>,
    pub wrapped_key: String,
    pub sealed_digest: Vec<u8>,
    pub size_bytes: u64,
    pub expires_at: u64,
}

impl StagedAttachment {
    pub fn meta(&self) -> AttachmentMeta {
        AttachmentMeta {
            id: self.id,
            instance_id: self.instance_id,
            request_id: self.request_id,
            wrapped_key: self.wrapped_key.clone(),
            sealed_digest: self.sealed_digest.clone(),
            size_bytes: self.size_bytes,
            expires_at: self.expires_at,
        }
    }
}

/// The columns of [`AttachmentMeta`], which the insert returns and
/// [`staged_of`] selects. `ciphertext` is not one of them, and a test keeps
/// it out. A macro and not a `const` so that `concat!` can splice it into
/// the `&'static str` sqlx runs without an audit marker.
macro_rules! meta_columns {
    () => {
        "id, instance_id, request_id, wrapped_key, sealed_digest, size_bytes, expires_at"
    };
}

const INSERT_ATTACHMENT: &str = concat!(
    "INSERT INTO attachments \
     (id, user_id, instance_id, ciphertext, wrapped_key, sealed_digest, sha256, \
      size_bytes, expires_at) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
     RETURNING ",
    meta_columns!()
);

const STAGED_OF: &str = concat!(
    "SELECT ",
    meta_columns!(),
    " FROM attachments WHERE user_id = $1 AND instance_id = $2 AND id = ANY($3)"
);

#[derive(FromRow)]
struct MetaRow {
    id: Uuid,
    instance_id: InstanceId,
    request_id: Option<RequestId>,
    wrapped_key: String,
    sealed_digest: Vec<u8>,
    size_bytes: i64,
    expires_at: OffsetDateTime,
}

#[derive(FromRow)]
struct StagedRow {
    #[sqlx(flatten)]
    meta: MetaRow,
    ciphertext: Vec<u8>,
}

impl TryFrom<MetaRow> for AttachmentMeta {
    type Error = StoreError;

    fn try_from(row: MetaRow) -> Result<Self, StoreError> {
        Ok(Self {
            id: row.id,
            instance_id: row.instance_id,
            request_id: row.request_id,
            wrapped_key: row.wrapped_key,
            sealed_digest: row.sealed_digest,
            size_bytes: u64::try_from(row.size_bytes)
                .map_err(|_| StoreError::SizeOutOfRange(row.size_bytes))?,
            expires_at: stored_expiry(row.expires_at)?,
        })
    }
}

impl TryFrom<StagedRow> for StagedAttachment {
    type Error = StoreError;

    fn try_from(row: StagedRow) -> Result<Self, StoreError> {
        let meta = AttachmentMeta::try_from(row.meta)?;
        Ok(Self {
            id: meta.id,
            instance_id: meta.instance_id,
            request_id: meta.request_id,
            ciphertext: row.ciphertext,
            wrapped_key: meta.wrapped_key,
            sealed_digest: meta.sealed_digest,
            size_bytes: meta.size_bytes,
            expires_at: meta.expires_at,
        })
    }
}

fn ciphertext_digest(ciphertext: &[u8]) -> [u8; 32] {
    Sha256::digest(ciphertext).into()
}

fn size_bytes(len: usize) -> Result<i64, StoreError> {
    i64::try_from(len).map_err(|_| StoreError::TooLarge { bytes: len })
}

/// `sha256` is the digest of `ciphertext`, computed here and never taken from
/// the caller, so it always describes the bytes it sits next to. Never a
/// digest of the content: one beside the ciphertext would let anyone with
/// database access confirm a guessed file without the key.
pub async fn insert_attachment(
    user: UserId,
    db: &Db,
    new: &NewAttachment,
) -> Result<AttachmentMeta, StoreError> {
    let size_bytes = size_bytes(new.ciphertext.len())?;
    let expires_at = timestamp(new.expires_at)?;
    sqlx::query_as::<_, MetaRow>(INSERT_ATTACHMENT)
        .bind(new.id)
        .bind(user)
        .bind(new.instance_id)
        .bind(&new.ciphertext)
        .bind(new.wrapped_key.as_str())
        .bind(&new.sealed_digest)
        .bind(ciphertext_digest(&new.ciphertext).as_slice())
        .bind(size_bytes)
        .bind(expires_at)
        .fetch_one(db)
        .await?
        .try_into()
}

/// Every row of `ids` the instance holds, whether staged, bound or past its
/// expiry: which of them a dispatch may name is the caller's verdict, not a
/// filter here. An id the instance does not hold is simply absent.
pub async fn staged_of(
    user: UserId,
    db: &Db,
    instance: InstanceId,
    ids: &[Uuid],
) -> Result<Vec<AttachmentMeta>, StoreError> {
    sqlx::query_as::<_, MetaRow>(STAGED_OF)
        .bind(user)
        .bind(instance)
        .bind(ids)
        .fetch_all(db)
        .await?
        .into_iter()
        .map(AttachmentMeta::try_from)
        .collect()
}

/// The row, only while the request it is bound to is still `queued` and not
/// past its `expires_at`: a request that was answered, judged expired or
/// never made has nothing left to fetch for.
pub async fn fetchable(
    user: UserId,
    db: &Db,
    instance: InstanceId,
    id: Uuid,
    now: u64,
) -> Result<Option<StagedAttachment>, StoreError> {
    let now = timestamp(now)?;
    sqlx::query_as::<_, StagedRow>(
        "SELECT a.id, a.instance_id, a.request_id, a.ciphertext, a.wrapped_key, \
                a.sealed_digest, a.size_bytes, a.expires_at \
         FROM attachments a JOIN instance_requests r \
           ON r.user_id = a.user_id AND r.instance_id = a.instance_id AND r.id = a.request_id \
         WHERE a.user_id = $1 AND a.instance_id = $2 AND a.id = $3 \
           AND r.status = $4 AND r.expires_at > $5",
    )
    .bind(user)
    .bind(instance)
    .bind(id)
    .bind(RequestStatus::Queued.as_str())
    .bind(now)
    .fetch_optional(db)
    .await?
    .map(StagedAttachment::try_from)
    .transpose()
}

/// Deletes the row, only while [`fetchable`] would still read it, and says
/// whether this call was the one that did. Of two fetches that both read it,
/// only the one this answers `true` may hand the content over; and a request
/// answered or expired since the read, while the fetch was opening the row,
/// gets `false` too. An expired request's row is left for the sweep at its
/// expiry; an answered one's went with the outcome, in
/// [`record_outcome`](super::requests::record_outcome).
pub async fn consumed(
    user: UserId,
    db: &Db,
    instance: InstanceId,
    id: Uuid,
    now: u64,
) -> Result<bool, StoreError> {
    let now = timestamp(now)?;
    Ok(sqlx::query_scalar::<_, Uuid>(
        "DELETE FROM attachments a \
         WHERE a.user_id = $1 AND a.instance_id = $2 AND a.id = $3 \
           AND EXISTS (SELECT 1 FROM instance_requests r \
                       WHERE r.user_id = a.user_id AND r.instance_id = a.instance_id \
                         AND r.id = a.request_id AND r.status = $4 AND r.expires_at > $5) \
         RETURNING a.id",
    )
    .bind(user)
    .bind(instance)
    .bind(id)
    .bind(RequestStatus::Queued.as_str())
    .bind(now)
    .fetch_optional(db)
    .await?
    .is_some())
}

/// Binds every staged, unexpired row of `ids` to the request and moves its
/// expiry to the request's. Returns how many rows it bound; a count below
/// `ids.len()` means one was bound already, expired or never the instance's,
/// and the caller must roll back the transaction this ran in and answer
/// [`StoreError::AttachmentUnavailable`]. `ids` must hold no duplicate, or
/// that count cannot be compared with its length.
pub async fn bind_to_request(
    user: UserId,
    tx: &mut PgConnection,
    instance: InstanceId,
    ids: &[Uuid],
    request_id: &RequestId,
    expires_at: u64,
    now: u64,
) -> Result<u64, StoreError> {
    let expires_at = timestamp(expires_at)?;
    let now = timestamp(now)?;
    Ok(sqlx::query(
        "UPDATE attachments SET request_id = $5, expires_at = $6 \
         WHERE user_id = $1 AND instance_id = $2 AND id = ANY($3) \
           AND request_id IS NULL AND expires_at > $4",
    )
    .bind(user)
    .bind(instance)
    .bind(ids)
    .bind(now)
    .bind(request_id.as_slice())
    .bind(expires_at)
    .execute(&mut *tx)
    .await?
    .rows_affected())
}

/// One tenant's share of the sweep: [`pool::tenants`](super::pool::tenants)
/// names them, since a delete naming no tenant is what the
/// [module root](super) rules out.
pub async fn delete_expired(user: UserId, db: &Db, now: u64) -> Result<u64, StoreError> {
    let now = timestamp(now)?;
    Ok(
        sqlx::query("DELETE FROM attachments WHERE user_id = $1 AND expires_at <= $2")
            .bind(user)
            .bind(now)
            .execute(db)
            .await?
            .rows_affected(),
    )
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
    fn neither_the_insert_nor_the_dispatch_read_returns_the_ciphertext() {
        let (written, returned) = INSERT_ATTACHMENT.split_once("RETURNING").unwrap();
        assert!(written.contains("ciphertext"));
        for read in [returned, STAGED_OF] {
            assert!(!read.contains("ciphertext"), "{read}");
        }
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

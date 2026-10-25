//! What the upload, fetch and sweep ask of the store.
//!
//! A port and not the `store` functions themselves for the reason
//! [`ApiStore`](crate::api::port::ApiStore) is one: the tests that prove
//! these use-cases run where no database is reachable.

use crate::store::attachments::{AttachmentMeta, NewAttachment, StagedAttachment};
use crate::store::{InstanceId, StoreError, UserId};
use async_trait::async_trait;
use uuid::Uuid;

/// Every method but [`expired_deleted`](Self::expired_deleted) takes the
/// tenant and the instance first, and a row of another tenant or another
/// instance reads exactly as one that does not exist: absent, `None` or
/// `false`, never an error.
#[async_trait]
pub trait AttachmentStore: Send + Sync {
    async fn staged(&self, user: UserId, new: NewAttachment) -> Result<AttachmentMeta, StoreError>;

    /// Every row of `ids` the instance holds, whatever its state: which of
    /// them a dispatch may name is the caller's verdict.
    async fn staged_of(
        &self,
        user: UserId,
        instance: InstanceId,
        ids: &[Uuid],
    ) -> Result<Vec<AttachmentMeta>, StoreError>;

    /// The row, only while it is bound to a request that is still `queued`
    /// and whose `expires_at` is after `now` (Unix seconds). The only answer
    /// here that carries the ciphertext; [`AttachmentMeta`] says why the
    /// others do not.
    async fn fetchable(
        &self,
        user: UserId,
        instance: InstanceId,
        id: Uuid,
        now: u64,
    ) -> Result<Option<StagedAttachment>, StoreError>;

    /// Deletes the row, only while [`fetchable`](Self::fetchable) at `now`
    /// would still answer it, and says whether this call was the one that
    /// did: of two fetches only one hands the content over, and none does
    /// for a request answered or expired since its read.
    async fn consumed(
        &self,
        user: UserId,
        instance: InstanceId,
        id: Uuid,
        now: u64,
    ) -> Result<bool, StoreError>;

    /// Deletes every row whose `expires_at` is at or before `now`, of every
    /// tenant, one tenant at a time, and answers how many.
    async fn expired_deleted(&self, now: u64) -> Result<u64, StoreError>;
}

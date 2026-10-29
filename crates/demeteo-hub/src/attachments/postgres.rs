//! [`AttachmentStore`] over the `store` functions. Nothing here holds SQL:
//! every tenant filter is the store function's, where the `user_id`-first
//! rule of the [store root](crate::store) is kept.

use super::port::AttachmentStore;
use crate::store::attachments::{
    consumed, delete_expired, fetchable, insert_attachment, staged_of, AttachmentMeta,
    NewAttachment, StagedAttachment,
};
use crate::store::pool::tenants;
use crate::store::{Db, InstanceId, StoreError, UserId};
use async_trait::async_trait;
use uuid::Uuid;

pub struct PostgresAttachments {
    db: Db,
}

impl PostgresAttachments {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

#[async_trait]
impl AttachmentStore for PostgresAttachments {
    async fn staged(&self, user: UserId, new: NewAttachment) -> Result<AttachmentMeta, StoreError> {
        insert_attachment(user, &self.db, &new).await
    }

    async fn staged_of(
        &self,
        user: UserId,
        instance: InstanceId,
        ids: &[Uuid],
    ) -> Result<Vec<AttachmentMeta>, StoreError> {
        staged_of(user, &self.db, instance, ids).await
    }

    async fn fetchable(
        &self,
        user: UserId,
        instance: InstanceId,
        id: Uuid,
        now: u64,
    ) -> Result<Option<StagedAttachment>, StoreError> {
        fetchable(user, &self.db, instance, id, now).await
    }

    async fn consumed(
        &self,
        user: UserId,
        instance: InstanceId,
        id: Uuid,
        now: u64,
    ) -> Result<bool, StoreError> {
        consumed(user, &self.db, instance, id, now).await
    }

    /// A tenant's failure ends the sweep with what it deleted so far
    /// uncounted; the next sweep finds whatever this one left.
    async fn expired_deleted(&self, now: u64) -> Result<u64, StoreError> {
        let mut deleted = 0;
        for user in tenants(&self.db).await? {
            deleted += delete_expired(user, &self.db, now).await?;
        }
        Ok(deleted)
    }
}

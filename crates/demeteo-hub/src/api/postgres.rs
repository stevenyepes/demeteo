//! [`ApiStore`] over the `store` reads. Nothing here holds SQL: every tenant
//! filter is the store function's, where the `user_id`-first rule of the
//! [store root](crate::store) is kept.

use super::port::{mirror_event, snapshot_read, ApiStore, MirrorEvent, SnapshotRead};
use crate::auth::challenge::base64url_encode;
use crate::auth::policy::Status;
use crate::socket::port::SocketStore;
use crate::socket::postgres::PostgresStore;
use crate::store::instances::{find_instance, list_instances, Instance};
use crate::store::passkeys::list_passkeys;
use crate::store::requests::{find_request, RequestId, RequestRecord};
use crate::store::run_mirror::mirror_of;
use crate::store::snapshots::read_snapshot;
use crate::store::{Db, InstanceId, StoreError, UserId};
use async_trait::async_trait;

pub struct PostgresApi {
    db: Db,
}

impl PostgresApi {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

#[async_trait]
impl ApiStore for PostgresApi {
    async fn instances(&self, user: UserId) -> Result<Vec<Instance>, StoreError> {
        list_instances(user, &self.db).await
    }

    async fn instance(&self, user: UserId, id: InstanceId) -> Result<Option<Instance>, StoreError> {
        find_instance(user, &self.db, id).await
    }

    async fn mirror(
        &self,
        user: UserId,
        instance: InstanceId,
        feature: Option<&str>,
    ) -> Result<Vec<MirrorEvent>, StoreError> {
        let records = mirror_of(user, &self.db, instance, feature).await?;
        Ok(records.into_iter().map(mirror_event).collect())
    }

    async fn snapshot(
        &self,
        user: UserId,
        instance: InstanceId,
    ) -> Result<Option<SnapshotRead>, StoreError> {
        let body = read_snapshot(user, &self.db, instance).await?;
        Ok(body.as_deref().map(snapshot_read))
    }

    async fn request(
        &self,
        user: UserId,
        instance: InstanceId,
        id: RequestId,
    ) -> Result<Option<RequestRecord>, StoreError> {
        find_request(user, &self.db, instance, &id).await
    }

    async fn trusted_credential_ids(&self, user: UserId) -> Result<Vec<String>, StoreError> {
        let passkeys = list_passkeys(user, &self.db).await?;
        Ok(passkeys
            .iter()
            .filter(|passkey| passkey.status == Status::Trusted)
            .map(|passkey| base64url_encode(&passkey.credential_id))
            .collect())
    }

    async fn requests_of(
        &self,
        user: UserId,
        instance: InstanceId,
    ) -> Result<Box<dyn SocketStore>, StoreError> {
        Ok(Box::new(PostgresStore::new(
            self.db.clone(),
            user,
            instance,
        )))
    }
}

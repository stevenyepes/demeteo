use super::port::GitTokenStore;
use crate::api::port::{mirror_event, MirrorEvent};
use crate::store::instances::{find_instance, InstanceKind};
use crate::store::requests::{bind_git_token_feature, find_start, RequestId, StartRecord};
use crate::store::run_mirror::mirror_of;
use crate::store::{Db, InstanceId, StoreError, UserId};
use async_trait::async_trait;
use demeteo_hub_protocol::RunEvent;

pub struct PostgresGitTokens {
    db: Db,
}

impl PostgresGitTokens {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

#[async_trait]
impl GitTokenStore for PostgresGitTokens {
    async fn instance_kind(
        &self,
        user: UserId,
        instance: InstanceId,
    ) -> Result<Option<InstanceKind>, StoreError> {
        Ok(find_instance(user, &self.db, instance)
            .await?
            .and_then(|row| InstanceKind::parse(&row.kind)))
    }

    async fn start(
        &self,
        user: UserId,
        instance: InstanceId,
        id: RequestId,
    ) -> Result<Option<StartRecord>, StoreError> {
        find_start(user, &self.db, instance, &id).await
    }

    async fn bind(
        &self,
        user: UserId,
        instance: InstanceId,
        id: RequestId,
        feature_id: &str,
    ) -> Result<Option<String>, StoreError> {
        bind_git_token_feature(user, &self.db, instance, &id, feature_id).await
    }

    async fn events(
        &self,
        user: UserId,
        instance: InstanceId,
        feature_id: &str,
    ) -> Result<Vec<RunEvent>, StoreError> {
        let rows = mirror_of(user, &self.db, instance, Some(feature_id)).await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| match mirror_event(row) {
                MirrorEvent::Readable(event) => Some(event),
                MirrorEvent::Unreadable { .. } => None,
            })
            .collect())
    }
}

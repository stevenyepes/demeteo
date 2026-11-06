//! What a git-token fetch asks of the store. A port so that the use-case
//! runs without Postgres, as [`ApiStore`](crate::api::ApiStore) does for the
//! session API.

use crate::store::instances::InstanceKind;
use crate::store::requests::{RequestId, StartRecord};
use crate::store::{InstanceId, StoreError, UserId};
use async_trait::async_trait;
use demeteo_hub_protocol::RunEvent;

/// Every method takes the tenant first and reads only that tenant's rows:
/// another tenant's instance or request reads as one that does not exist.
#[async_trait]
pub trait GitTokenStore: Send + Sync {
    /// `None` for no such instance, and for a `kind` this build cannot read.
    async fn instance_kind(
        &self,
        user: UserId,
        instance: InstanceId,
    ) -> Result<Option<InstanceKind>, StoreError>;

    async fn start(
        &self,
        user: UserId,
        instance: InstanceId,
        id: RequestId,
    ) -> Result<Option<StartRecord>, StoreError>;

    /// [`bind_git_token_feature`](crate::store::requests::bind_git_token_feature).
    async fn bind(
        &self,
        user: UserId,
        instance: InstanceId,
        id: RequestId,
        feature_id: &str,
    ) -> Result<Option<String>, StoreError>;

    /// The Feature's mirrored events in offset order, the unreadable left
    /// out as the run views leave them.
    async fn events(
        &self,
        user: UserId,
        instance: InstanceId,
        feature_id: &str,
    ) -> Result<Vec<RunEvent>, StoreError>;
}

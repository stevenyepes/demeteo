use crate::store::hub_projects::{HubProjectFields, HubProjectId, HubProjectRow};
use crate::store::{StoreError, UserId};
use async_trait::async_trait;

/// The tenant's Hub Projects. A port so the handlers' use-cases run without
/// Postgres. Every method reads or writes one tenant's rows only; another
/// tenant's project reads as one that does not exist.
#[async_trait]
pub trait HubProjectStore: Send + Sync {
    /// Ordered by name, then by id.
    async fn projects(&self, user: UserId) -> Result<Vec<HubProjectRow>, StoreError>;

    async fn project(
        &self,
        user: UserId,
        id: HubProjectId,
    ) -> Result<Option<HubProjectRow>, StoreError>;

    /// [`StoreError::ProjectExists`] when the tenant already has a project on
    /// the same host and repository.
    async fn insert(
        &self,
        user: UserId,
        id: HubProjectId,
        fields: &HubProjectFields,
    ) -> Result<HubProjectRow, StoreError>;

    /// Replaces every field; `None` when the tenant has no such project.
    async fn update(
        &self,
        user: UserId,
        id: HubProjectId,
        fields: &HubProjectFields,
    ) -> Result<Option<HubProjectRow>, StoreError>;

    async fn remove(&self, user: UserId, id: HubProjectId) -> Result<bool, StoreError>;
}

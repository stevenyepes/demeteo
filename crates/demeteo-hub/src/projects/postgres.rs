use super::port::HubProjectStore;
use crate::store::hub_projects::{
    delete_hub_project, find_hub_project, insert_hub_project, list_hub_projects,
    update_hub_project, HubProjectFields, HubProjectId, HubProjectRow,
};
use crate::store::{Db, StoreError, UserId};
use async_trait::async_trait;

pub struct PostgresProjects {
    db: Db,
}

impl PostgresProjects {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

#[async_trait]
impl HubProjectStore for PostgresProjects {
    async fn projects(&self, user: UserId) -> Result<Vec<HubProjectRow>, StoreError> {
        list_hub_projects(user, &self.db).await
    }

    async fn project(
        &self,
        user: UserId,
        id: HubProjectId,
    ) -> Result<Option<HubProjectRow>, StoreError> {
        find_hub_project(user, &self.db, id).await
    }

    async fn insert(
        &self,
        user: UserId,
        id: HubProjectId,
        fields: &HubProjectFields,
    ) -> Result<HubProjectRow, StoreError> {
        insert_hub_project(user, &self.db, id, fields).await
    }

    async fn update(
        &self,
        user: UserId,
        id: HubProjectId,
        fields: &HubProjectFields,
    ) -> Result<Option<HubProjectRow>, StoreError> {
        update_hub_project(user, &self.db, id, fields).await
    }

    async fn remove(&self, user: UserId, id: HubProjectId) -> Result<bool, StoreError> {
        delete_hub_project(user, &self.db, id).await
    }
}

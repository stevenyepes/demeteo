use super::port::HubProjectStore;
use crate::store::hub_projects::{HubProjectFields, HubProjectId, HubProjectRow};
use crate::store::{StoreError, UserId};
use async_trait::async_trait;
use std::sync::Mutex;
use time::OffsetDateTime;

/// Test double: rows per tenant, listed in the store's order, with its
/// `UNIQUE (user_id, host, full_name)` kept so a duplicate fails as Postgres
/// would.
#[derive(Default)]
pub struct MemoryProjects {
    rows: Mutex<Vec<(UserId, HubProjectRow)>>,
}

impl MemoryProjects {
    fn rows(&self) -> std::sync::MutexGuard<'_, Vec<(UserId, HubProjectRow)>> {
        self.rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn names_repository(
    owner: UserId,
    row: &HubProjectRow,
    user: UserId,
    fields: &HubProjectFields,
) -> bool {
    owner == user && row.fields.host == fields.host && row.fields.full_name == fields.full_name
}

#[async_trait]
impl HubProjectStore for MemoryProjects {
    async fn projects(&self, user: UserId) -> Result<Vec<HubProjectRow>, StoreError> {
        let mut rows: Vec<HubProjectRow> = self
            .rows()
            .iter()
            .filter(|(owner, _)| *owner == user)
            .map(|(_, row)| row.clone())
            .collect();
        rows.sort_by(|a, b| a.fields.name.cmp(&b.fields.name).then(a.id.0.cmp(&b.id.0)));
        Ok(rows)
    }

    async fn project(
        &self,
        user: UserId,
        id: HubProjectId,
    ) -> Result<Option<HubProjectRow>, StoreError> {
        Ok(self
            .rows()
            .iter()
            .find(|(owner, row)| *owner == user && row.id == id)
            .map(|(_, row)| row.clone()))
    }

    async fn insert(
        &self,
        user: UserId,
        id: HubProjectId,
        fields: &HubProjectFields,
    ) -> Result<HubProjectRow, StoreError> {
        let mut rows = self.rows();
        if rows
            .iter()
            .any(|(owner, row)| names_repository(*owner, row, user, fields))
        {
            return Err(StoreError::ProjectExists);
        }
        let row = HubProjectRow {
            id,
            fields: fields.clone(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        };
        rows.push((user, row.clone()));
        Ok(row)
    }

    async fn update(
        &self,
        user: UserId,
        id: HubProjectId,
        fields: &HubProjectFields,
    ) -> Result<Option<HubProjectRow>, StoreError> {
        let mut rows = self.rows();
        if rows
            .iter()
            .any(|(owner, row)| row.id != id && names_repository(*owner, row, user, fields))
        {
            return Err(StoreError::ProjectExists);
        }
        Ok(rows
            .iter_mut()
            .find(|(owner, row)| *owner == user && row.id == id)
            .map(|(_, row)| {
                row.fields = fields.clone();
                row.clone()
            }))
    }

    async fn remove(&self, user: UserId, id: HubProjectId) -> Result<bool, StoreError> {
        let mut rows = self.rows();
        let before = rows.len();
        rows.retain(|(owner, row)| !(*owner == user && row.id == id));
        Ok(rows.len() != before)
    }
}

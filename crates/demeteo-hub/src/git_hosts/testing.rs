use super::port::GitHostStore;
use crate::store::git_hosts::{GitHostId, GitHostRow, NewGitHost};
use crate::store::{StoreError, UserId};
use async_trait::async_trait;
use std::sync::Mutex;
use time::OffsetDateTime;

/// Test double: rows in insertion order, per tenant, with the store's
/// `UNIQUE (user_id, host, app_id)` kept so a test that connects one App
/// twice fails as Postgres would.
#[derive(Default)]
pub struct MemoryGitHosts {
    rows: Mutex<Vec<(UserId, GitHostRow)>>,
}

impl MemoryGitHosts {
    fn rows(&self) -> std::sync::MutexGuard<'_, Vec<(UserId, GitHostRow)>> {
        self.rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[async_trait]
impl GitHostStore for MemoryGitHosts {
    async fn hosts(&self, user: UserId) -> Result<Vec<GitHostRow>, StoreError> {
        Ok(self
            .rows()
            .iter()
            .filter(|(owner, _)| *owner == user)
            .map(|(_, row)| row.clone())
            .collect())
    }

    async fn hosts_on(&self, user: UserId, host: &str) -> Result<Vec<GitHostRow>, StoreError> {
        Ok(self
            .hosts(user)
            .await?
            .into_iter()
            .filter(|row| row.host == host)
            .collect())
    }

    async fn insert(&self, user: UserId, host: &NewGitHost) -> Result<(), StoreError> {
        let mut rows = self.rows();
        if rows.iter().any(|(owner, row)| {
            *owner == user && row.host == host.host && row.app_id == host.app_id
        }) {
            return Err(StoreError::Database(sqlx::Error::Protocol(
                "duplicate key value violates unique constraint".into(),
            )));
        }
        rows.push((
            user,
            GitHostRow {
                id: host.id,
                host: host.host.clone(),
                api_base: host.api_base.clone(),
                web_base: host.web_base.clone(),
                app_id: host.app_id,
                app_slug: host.app_slug.clone(),
                transit_key: host.transit_key.clone(),
                account: host.account.clone(),
                created_at: OffsetDateTime::UNIX_EPOCH,
            },
        ));
        Ok(())
    }

    async fn remove(&self, user: UserId, id: GitHostId) -> Result<bool, StoreError> {
        let mut rows = self.rows();
        let before = rows.len();
        rows.retain(|(owner, row)| !(*owner == user && row.id == id));
        Ok(rows.len() != before)
    }
}

use crate::store::git_hosts::{GitHostId, GitHostRow, NewGitHost};
use crate::store::{StoreError, UserId};
use async_trait::async_trait;

/// The tenant's connected git hosts. A port so the minter and the manifest
/// flow run without Postgres. Every method reads or writes one tenant's rows
/// only; another tenant's host reads as one that does not exist.
#[async_trait]
pub trait GitHostStore: Send + Sync {
    async fn hosts(&self, user: UserId) -> Result<Vec<GitHostRow>, StoreError>;

    /// `host` lowercase, as a start's repository spells it.
    async fn hosts_on(&self, user: UserId, host: &str) -> Result<Vec<GitHostRow>, StoreError>;

    async fn insert(&self, user: UserId, host: &NewGitHost) -> Result<(), StoreError>;

    async fn remove(&self, user: UserId, id: GitHostId) -> Result<bool, StoreError>;
}

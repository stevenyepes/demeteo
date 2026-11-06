use super::port::GitHostStore;
use crate::store::git_hosts::{
    git_hosts, git_hosts_on, insert_git_host, remove_git_host, GitHostId, GitHostRow, NewGitHost,
};
use crate::store::{Db, StoreError, UserId};
use async_trait::async_trait;

pub struct PostgresGitHosts {
    db: Db,
}

impl PostgresGitHosts {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

#[async_trait]
impl GitHostStore for PostgresGitHosts {
    async fn hosts(&self, user: UserId) -> Result<Vec<GitHostRow>, StoreError> {
        git_hosts(user, &self.db).await
    }

    async fn hosts_on(&self, user: UserId, host: &str) -> Result<Vec<GitHostRow>, StoreError> {
        git_hosts_on(user, &self.db, host).await
    }

    async fn insert(&self, user: UserId, host: &NewGitHost) -> Result<(), StoreError> {
        insert_git_host(user, &self.db, host).await
    }

    async fn remove(&self, user: UserId, id: GitHostId) -> Result<bool, StoreError> {
        remove_git_host(user, &self.db, id).await
    }
}

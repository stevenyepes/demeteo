use super::{Db, InstanceId, StoreError, UserId};
use sqlx::FromRow;
use time::OffsetDateTime;

#[derive(Debug, Clone, FromRow)]
pub struct Instance {
    pub id: InstanceId,
    pub user_id: UserId,
    pub name: String,
    pub os: String,
    pub arch: String,
    pub version: String,
    pub last_seen: Option<OffsetDateTime>,
    pub granted_scopes: Vec<String>,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct NewInstance {
    pub id: InstanceId,
    pub name: String,
    pub os: String,
    pub arch: String,
    pub version: String,
    pub granted_scopes: Vec<String>,
}

pub async fn list_instances(user: UserId, db: &Db) -> Result<Vec<Instance>, StoreError> {
    Ok(sqlx::query_as::<_, Instance>(
        "SELECT id, user_id, name, os, arch, version, last_seen, granted_scopes, created_at \
         FROM instances WHERE user_id = $1 ORDER BY created_at, id",
    )
    .bind(user)
    .fetch_all(db)
    .await?)
}

/// `instances.id` is globally unique (it is the install id), so naming an id
/// another user already holds fails on the primary key instead of adopting it.
pub async fn insert_instance(
    user: UserId,
    db: &Db,
    new: &NewInstance,
) -> Result<Instance, StoreError> {
    Ok(sqlx::query_as::<_, Instance>(
        "INSERT INTO instances (id, user_id, name, os, arch, version, granted_scopes) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         RETURNING id, user_id, name, os, arch, version, last_seen, granted_scopes, created_at",
    )
    .bind(new.id)
    .bind(user)
    .bind(&new.name)
    .bind(&new.os)
    .bind(&new.arch)
    .bind(&new.version)
    .bind(&new.granted_scopes)
    .fetch_one(db)
    .await?)
}

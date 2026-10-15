use super::{timestamp, Db, InstanceId, StoreError, UserId};
use crate::socket::SNAPSHOT_KIND;

/// Makes `body` the instance's one snapshot of `kind`, whatever that slot
/// held before, and says whether it did.
///
/// `body` is JSON text cast to `JSONB`, as
/// [`NewEvent::event`](super::run_mirror::NewEvent::event) is. The slot is
/// replaced whole and never merged: what is read back is the last body
/// written and nothing of an earlier one.
///
/// The primary key is `(instance_id, kind)` and holds no `user_id`, so the
/// conflict is found by an instance id alone. The `WHERE` on the update is
/// what keeps that from being a way into another tenant's slot: there the
/// statement changes no row and the answer is `false`. An instance the tenant
/// does not have, with nothing in the slot yet, is refused one step earlier
/// instead, by the foreign key on `(user_id, instance_id)`, and that is
/// [`StoreError::Database`].
pub async fn replace_snapshot(
    user: UserId,
    db: &Db,
    instance: InstanceId,
    kind: &str,
    body: &str,
    now: u64,
) -> Result<bool, StoreError> {
    let now = timestamp(now)?;
    let done = sqlx::query(
        "INSERT INTO snapshots (user_id, instance_id, kind, body, updated_at) \
         VALUES ($1, $2, $3, $4::jsonb, $5) \
         ON CONFLICT (instance_id, kind) DO UPDATE \
         SET body = EXCLUDED.body, updated_at = EXCLUDED.updated_at \
         WHERE snapshots.user_id = $1",
    )
    .bind(user)
    .bind(instance)
    .bind(kind)
    .bind(body)
    .bind(now)
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// The body of the instance's snapshot under [`SNAPSHOT_KIND`], as JSON text
/// and unparsed, for the reason [`MirrorRecord::event`] is. `None` when the
/// tenant has no such instance or the instance has sent no snapshot yet.
///
/// [`MirrorRecord::event`]: super::run_mirror::MirrorRecord::event
pub async fn read_snapshot(
    user: UserId,
    db: &Db,
    instance: InstanceId,
) -> Result<Option<String>, StoreError> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT body::text FROM snapshots WHERE user_id = $1 AND instance_id = $2 AND kind = $3",
    )
    .bind(user)
    .bind(instance)
    .bind(SNAPSHOT_KIND)
    .fetch_optional(db)
    .await?)
}

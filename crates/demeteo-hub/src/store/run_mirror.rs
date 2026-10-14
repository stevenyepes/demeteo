use super::{Db, InstanceId, StoreError, UserId};

/// One entry of an instance's run log, as the mirror keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEvent {
    pub feature_id: String,
    pub event_offset: u64,
    /// The event as JSON text, which the statement casts to `JSONB`. Text and
    /// not a `serde_json::Value`: binding one takes sqlx's `json` feature,
    /// which this crate leaves off. The store does not read it, so what it
    /// may hold is the caller's to guarantee.
    pub event: String,
}

fn stored_offset(offset: u64) -> Result<i64, StoreError> {
    i64::try_from(offset).map_err(|_| StoreError::OffsetTooLarge(offset))
}

/// Stores one event of the instance's run log and writes the instance's
/// cursor, in one transaction, and returns the cursor it wrote. `None` when
/// the tenant has no such instance; nothing is written then.
///
/// An event the mirror already holds under the same feature and offset is
/// dropped and the stored row kept: an instance resends whatever it was not
/// acked for, and the first copy is as good as the second.
///
/// The cursor written is whatever `decide` answers when shown the cursor the
/// row held and this event's offset. It is asked for a dropped event as for a
/// stored one, and its answer is written as given: nothing here compares two
/// offsets, in SQL or out of it, so the rule for what an ack may name has one
/// home and it is the caller's.
///
/// The cursor is a column of `instances` and not `max(event_offset)` over
/// `run_mirror`. The mirror's primary key leads with `feature_id`, so a
/// maximum across an instance's Features is no single index probe and grows
/// with its history, and the cursor is read for every `hello` and written for
/// every event. Writing it in the transaction that stores the row is what
/// keeps the column from naming an event the mirror does not hold.
///
/// The instance's row is locked for the length of the transaction, so that
/// two events of one instance take turns and the second is shown the cursor
/// the first wrote. A connection that was displaced can still be storing an
/// event when its successor starts; unordered, the later commit would put
/// back a cursor computed from the earlier one's starting point. The lock is
/// `FOR NO KEY UPDATE`, the one the `UPDATE` below takes anyway: `FOR UPDATE`
/// would also hold up every insert of a row that references the instance,
/// such as a request being queued for it.
pub async fn mirror_event(
    user: UserId,
    db: &Db,
    instance: InstanceId,
    new: &NewEvent,
    decide: impl FnOnce(u64, u64) -> u64,
) -> Result<Option<u64>, StoreError> {
    let event_offset = stored_offset(new.event_offset)?;
    let mut tx = db.begin().await?;
    let before = sqlx::query_scalar::<_, i64>(
        "SELECT event_cursor FROM instances WHERE user_id = $1 AND id = $2 FOR NO KEY UPDATE",
    )
    .bind(user)
    .bind(instance)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(before) = before else {
        // `passkeys::decide_locked` has why a ROLLBACK that fails is not an
        // error.
        let _ = tx.rollback().await;
        return Ok(None);
    };
    let before = u64::try_from(before).map_err(|_| StoreError::CursorOutOfRange(before))?;

    sqlx::query(
        "INSERT INTO run_mirror (user_id, instance_id, feature_id, event_offset, event) \
         VALUES ($1, $2, $3, $4, $5::jsonb) \
         ON CONFLICT (instance_id, feature_id, event_offset) DO NOTHING",
    )
    .bind(user)
    .bind(instance)
    .bind(&new.feature_id)
    .bind(event_offset)
    .bind(&new.event)
    .execute(&mut *tx)
    .await?;

    let cursor = decide(before, new.event_offset);
    sqlx::query("UPDATE instances SET event_cursor = $3 WHERE user_id = $1 AND id = $2")
        .bind(user)
        .bind(instance)
        .bind(stored_offset(cursor)?)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(cursor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_offset_beyond_bigint_is_refused_and_not_wrapped() {
        assert_eq!(stored_offset(0).unwrap(), 0);
        assert_eq!(stored_offset(41).unwrap(), 41);
        let last = u64::try_from(i64::MAX).unwrap();
        assert_eq!(stored_offset(last).unwrap(), i64::MAX);

        for too_large in [last + 1, u64::MAX] {
            let err = stored_offset(too_large).unwrap_err();
            assert!(
                matches!(err, StoreError::OffsetTooLarge(o) if o == too_large),
                "{err}"
            );
        }
    }
}

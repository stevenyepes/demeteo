//! The [`SocketStore`] a served session writes through: one tenant's
//! instance, in Postgres.
//!
//! Every statement is [`store`](crate::store)'s and every verdict the
//! caller's, so this file imports no `sqlx`, spells no SQL and compares no
//! two offsets, expiries or outcomes. That is what lets a session test over
//! `MemoryHub`, the store double of `testing`, say anything about a served
//! session: both stores apply the closure they are handed, and a rule
//! written here would hold in production and in no test that runs without a
//! database.
//!
//! What is this file's own is what an answer the port has no room for
//! becomes. A store function says `None` or `false` where the tenant has no
//! such row; the port returns a bare cursor or `()`. Here that is an error
//! wherever the session was about to act on a row that is not there, and
//! `Ok` in the two places the port names it as no failure: a departure by a
//! connection that was displaced, and an expiry of a request that is no
//! longer `queued`.
//!
//! **Rejected: making the store functions fail instead.** Their `None` and
//! `false` say the tenant has no such row and nothing more, which is an
//! answer, and the one their own tests hold them to. That it is a fault is
//! true of a session, admitted a moment ago as an instance that had a row,
//! and that is something the store does not know.

use super::connections::ConnectionId;
use super::port::SocketStore;
use super::requests::OutcomeChange;
use super::SNAPSHOT_KIND;
use crate::store::instances::{granted_scopes, record_departure, record_hello};
use crate::store::requests::{
    insert_request, insert_request_binding, list_queued_requests, mark_request_expired,
    mark_request_sent, record_outcome, NewRequest, QueuedRequest, RequestId, RequestState,
};
use crate::store::run_mirror::{mirror_event, NewEvent};
use crate::store::snapshots::replace_snapshot;
use crate::store::{Db, InstanceId, StoreError, UserId};
use async_trait::async_trait;
use demeteo_hub_protocol::{Hello, RequestOutcome, RunEvent, Scope, Snapshot};
use serde::Serialize;
use uuid::Uuid;

/// The rows of `instance` as `user` holds them. Nothing here checks that
/// `user` has such an instance: both are whatever the caller was admitted
/// as, and a pair that names no row finds none to write.
pub struct PostgresStore {
    db: Db,
    user: UserId,
    instance: InstanceId,
}

impl PostgresStore {
    pub fn new(db: Db, user: UserId, instance: InstanceId) -> Self {
        Self { db, user, instance }
    }
}

/// Whether a `hello` names another installation than the one before it did.
/// The first `hello` of a pairing has none before it and is not a change.
fn is_another_install(previous: Option<&str>, reported: &str) -> bool {
    previous.is_some_and(|previous| previous != reported)
}

fn outcome_to_write(change: OutcomeChange) -> Option<RequestOutcome> {
    match change {
        OutcomeChange::Record(outcome) => Some(outcome),
        OutcomeChange::Keep => None,
    }
}

/// The text a `JSONB` column is handed. No value of the protocol's types
/// fails to serialise; this is where the `Err` goes in place of a panic.
fn json_of<T: Serialize>(value: &T, what: &'static str) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|_| StoreError::Unencodable(what))
}

#[async_trait]
impl SocketStore for PostgresStore {
    fn instance(&self) -> InstanceId {
        self.instance
    }

    /// An instance row that is not there is [`StoreError::NoSuchInstance`]
    /// and never an insert, for the reason on [`record_hello`].
    ///
    /// An `install_id` other than the one the row held is written like any
    /// other and **the cursor is kept**. It is what a desktop reports after
    /// its database was lost under a keyring that survived: the same device
    /// key, and a run log that starts again at offset 1. Its events then
    /// arrive beneath a cursor that does not move for them, and one that
    /// meets a row of the earlier installation under the same Feature and
    /// offset is dropped as a copy of it. Resetting the cursor, or setting
    /// the earlier installation's rows aside, would repair that, and either
    /// is a product decision nobody has taken. So the change is logged, by
    /// instance id alone, and nothing else is done about it.
    async fn greeted(
        &self,
        hello: &Hello,
        connection: ConnectionId,
        now: u64,
    ) -> Result<u64, StoreError> {
        let greeted = record_hello(
            self.user,
            &self.db,
            self.instance,
            hello,
            connection.as_uuid(),
            now,
        )
        .await?
        .ok_or(StoreError::NoSuchInstance)?;
        if is_another_install(greeted.previous_install_id.as_deref(), &hello.install_id) {
            tracing::warn!(
                instance = %self.instance.0,
                "an instance reported another install id than its last hello did; its cursor is kept"
            );
        }
        Ok(greeted.event_cursor)
    }

    async fn ingested(
        &self,
        event: &RunEvent,
        decide: fn(u64, u64) -> u64,
    ) -> Result<u64, StoreError> {
        let new = NewEvent {
            feature_id: event.feature_id.clone(),
            event_offset: event.offset,
            event: json_of(event, "run event")?,
        };
        mirror_event(self.user, &self.db, self.instance, &new, decide)
            .await?
            .ok_or(StoreError::NoSuchInstance)
    }

    async fn snapshot_replaced(&self, snapshot: &Snapshot, now: u64) -> Result<(), StoreError> {
        let body = json_of(snapshot, "snapshot")?;
        replace_snapshot(
            self.user,
            &self.db,
            self.instance,
            SNAPSHOT_KIND,
            &body,
            now,
        )
        .await?
        .then_some(())
        .ok_or(StoreError::NoSuchInstance)
    }

    async fn granted_scopes(&self) -> Result<Vec<Scope>, StoreError> {
        granted_scopes(self.user, &self.db, self.instance)
            .await?
            .ok_or(StoreError::NoSuchInstance)
    }

    async fn request_inserted(&self, request: NewRequest) -> Result<(), StoreError> {
        insert_request(self.user, &self.db, self.instance, &request).await
    }

    async fn request_inserted_binding(
        &self,
        request: NewRequest,
        attachments: &[Uuid],
        now: u64,
    ) -> Result<(), StoreError> {
        insert_request_binding(
            self.user,
            &self.db,
            self.instance,
            &request,
            attachments,
            now,
        )
        .await
    }

    async fn queued_requests(&self) -> Result<Vec<QueuedRequest>, StoreError> {
        list_queued_requests(self.user, &self.db, self.instance).await
    }

    /// A request that is not there is [`StoreError::NoSuchRequest`]. The id
    /// is one a session read from this instance's queue and rows are never
    /// deleted, so it is not an answer a working store gives. It is an error
    /// and not `Ok` because a delivery that went uncounted is one
    /// [`outcome_change`](super::requests::outcome_change) takes for never
    /// made.
    async fn request_sent(&self, id: RequestId, now: u64) -> Result<(), StoreError> {
        mark_request_sent(self.user, &self.db, self.instance, &id, now)
            .await?
            .then_some(())
            .ok_or(StoreError::NoSuchRequest)
    }

    async fn request_expired(&self, id: RequestId) -> Result<(), StoreError> {
        mark_request_expired(self.user, &self.db, self.instance, &id).await?;
        Ok(())
    }

    async fn outcome_recorded(
        &self,
        id: RequestId,
        decide: &(dyn Fn(RequestState) -> OutcomeChange + Sync),
        now: u64,
    ) -> Result<bool, StoreError> {
        record_outcome(self.user, &self.db, self.instance, &id, now, |state| {
            outcome_to_write(decide(state))
        })
        .await
    }

    async fn departed(&self, connection: ConnectionId, now: u64) -> Result<(), StoreError> {
        record_departure(
            self.user,
            &self.db,
            self.instance,
            connection.as_uuid(),
            now,
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use demeteo_hub_protocol::RefusalReason;

    #[test]
    fn only_an_install_id_that_replaces_a_different_one_is_another_install() {
        assert!(is_another_install(Some("install-a"), "install-b"));
        assert!(!is_another_install(Some("install-a"), "install-a"));
        assert!(!is_another_install(None, "install-a"));
        assert!(is_another_install(Some(""), "install-a"));
    }

    #[test]
    fn a_change_to_record_is_written_as_its_outcome_and_a_kept_row_is_not_written() {
        let refused = RequestOutcome::Refused {
            reason: RefusalReason::ScopeOff,
        };
        for outcome in [RequestOutcome::Accepted, refused] {
            assert_eq!(
                outcome_to_write(OutcomeChange::Record(outcome)),
                Some(outcome)
            );
        }
        assert_eq!(outcome_to_write(OutcomeChange::Keep), None);
    }
}

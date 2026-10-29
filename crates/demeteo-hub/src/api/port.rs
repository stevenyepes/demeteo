//! What the API's use-cases ask of the store: one tenant's rows, read.
//!
//! A port and not the `store` functions themselves so that a use-case runs
//! without Postgres. The tests that prove the API's behaviour run where no
//! database is reachable, and they stand a double here instead.

use crate::socket::port::SocketStore;
use crate::store::instances::Instance;
use crate::store::requests::{RequestId, RequestRecord};
use crate::store::run_mirror::MirrorRecord;
use crate::store::{InstanceId, StoreError, UserId};
use async_trait::async_trait;
use demeteo_hub_protocol::{RunEvent, Snapshot};

/// Every method takes the tenant first and answers only with that tenant's
/// rows: another tenant's instance, request or mirror reads exactly as one
/// that does not exist, `None` or empty, never an error.
#[async_trait]
pub trait ApiStore: Send + Sync {
    async fn instances(&self, user: UserId) -> Result<Vec<Instance>, StoreError>;

    async fn instance(&self, user: UserId, id: InstanceId) -> Result<Option<Instance>, StoreError>;

    /// Ordered by `(feature_id, offset)`, of one Feature when `feature`
    /// names it.
    async fn mirror(
        &self,
        user: UserId,
        instance: InstanceId,
        feature: Option<&str>,
    ) -> Result<Vec<MirrorEvent>, StoreError>;

    /// `None` when the instance has sent no snapshot yet.
    async fn snapshot(
        &self,
        user: UserId,
        instance: InstanceId,
    ) -> Result<Option<SnapshotRead>, StoreError>;

    async fn request(
        &self,
        user: UserId,
        instance: InstanceId,
        id: RequestId,
    ) -> Result<Option<RequestRecord>, StoreError>;

    /// The credential ids of the tenant's trusted passkeys, as unpadded
    /// base64url, for a ceremony's `allowCredentials`.
    async fn trusted_credential_ids(&self, user: UserId) -> Result<Vec<String>, StoreError>;

    /// The store a request for `instance` is queued through. Like
    /// [`PostgresStore::new`](crate::socket::postgres::PostgresStore::new) it
    /// checks nothing: a caller asks [`instance`](Self::instance) first, and
    /// a pair that names no row queues nothing.
    async fn requests_of(
        &self,
        user: UserId,
        instance: InstanceId,
    ) -> Result<Box<dyn SocketStore>, StoreError>;
}

/// One mirrored event as it was read back. Unreadability is a fact about one
/// row and not a failed read, for the reason
/// [`QueuedEnvelope`](crate::store::requests::QueuedEnvelope) gives: rows are
/// never deleted, so a run whose log held one would be unreadable for good.
#[derive(Debug, Clone, PartialEq)]
pub enum MirrorEvent {
    Readable(RunEvent),
    /// The row holds JSON that is not a [`RunEvent`] of this build. Its key
    /// still says which event it was.
    Unreadable {
        feature_id: String,
        offset: u64,
    },
}

/// The instance's snapshot as it was read back, unreadable for the reason a
/// [`MirrorEvent`] can be.
#[derive(Debug, Clone, PartialEq)]
pub enum SnapshotRead {
    Readable(Box<Snapshot>),
    Unreadable,
}

pub fn mirror_event(record: MirrorRecord) -> MirrorEvent {
    match serde_json::from_str(&record.event) {
        Ok(event) => MirrorEvent::Readable(event),
        Err(_) => MirrorEvent::Unreadable {
            feature_id: record.feature_id,
            offset: record.offset,
        },
    }
}

pub fn snapshot_read(body: &str) -> SnapshotRead {
    match serde_json::from_str(body) {
        Ok(snapshot) => SnapshotRead::Readable(Box::new(snapshot)),
        Err(_) => SnapshotRead::Unreadable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use demeteo_hub_protocol::{RunEventKind, RunEventPayload};

    fn record(event: String) -> MirrorRecord {
        MirrorRecord {
            feature_id: "f-1".to_owned(),
            offset: 7,
            event,
        }
    }

    #[test]
    fn a_row_holding_a_run_event_reads_as_that_event() {
        let event = RunEvent {
            v: 1,
            feature_id: "f-1".to_owned(),
            offset: 7,
            kind: RunEventKind::FeatureStarted,
            occurred_at: 1_700_000_000,
            payload: RunEventPayload {
                step_id: None,
                step_execution_id: None,
                ticket_id: None,
            },
        };
        let text = serde_json::to_string(&event).unwrap();

        assert_eq!(mirror_event(record(text)), MirrorEvent::Readable(event));
    }

    #[test]
    fn a_row_that_is_no_run_event_is_unreadable_under_its_own_key() {
        for not_a_run_event in ["not json", "{}", r#"{"v":1,"feature_id":"f-1"}"#, "null"] {
            assert_eq!(
                mirror_event(record(not_a_run_event.to_owned())),
                MirrorEvent::Unreadable {
                    feature_id: "f-1".to_owned(),
                    offset: 7,
                },
                "{not_a_run_event}"
            );
        }
    }

    #[test]
    fn a_body_that_is_no_snapshot_is_unreadable() {
        let snapshot = Snapshot {
            v: 1,
            projects: Vec::new(),
            workflows: Vec::new(),
            run_shape: Vec::new(),
            features: Vec::new(),
            harnesses: None,
        };
        let text = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(
            snapshot_read(&text),
            SnapshotRead::Readable(Box::new(snapshot))
        );

        for not_a_snapshot in ["not json", "{}", r#"{"v":1,"projects":"x"}"#] {
            assert_eq!(
                snapshot_read(not_a_snapshot),
                SnapshotRead::Unreadable,
                "{not_a_snapshot}"
            );
        }
    }
}

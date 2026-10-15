//! [`ApiStore`] without a database, for several tenants at once.
//!
//! A [`MemoryFleet`] is a set of [`MemoryHub`]s, one per paired instance, so
//! a request queued through [`requests_of`](ApiStore::requests_of) lands in
//! the very rows a session's [`MemoryStore`] reads and the API reads back.
//! Like a `MemoryHub` it panics on a call it has no answer to, for the reason
//! on [`socket::testing`](crate::socket::testing). A pair it does not hold is
//! not such a call: "not yours" has an answer, the one the SQL gives, and
//! answering it is what the tenant rule of [the module](super) is tested by.

use super::port::{mirror_event, snapshot_read, ApiStore, MirrorEvent, SnapshotRead};
use crate::pairing::authorize::scope_name;
use crate::socket::connections::{ConnectionId, Connections};
use crate::socket::cursor;
use crate::socket::port::SocketStore;
use crate::socket::requests::OutcomeChange;
use crate::socket::testing::{MemoryHub, MemoryStore, RequestView};
use crate::store::instances::{Instance, NewInstanceLabels};
use crate::store::requests::{NewRequest, QueuedRequest, RequestId, RequestRecord, RequestState};
use crate::store::run_mirror::MirrorRecord;
use crate::store::{InstanceId, StoreError, UserId};
use async_trait::async_trait;
use demeteo_hub_protocol::{Hello, RunEvent, Scope, Snapshot};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;

/// What [`MemoryFleet::paired`] writes as the consent screen's labels: the
/// columns a pairing fills and a `hello` overwrites. Spelled so that a test
/// reading one can tell it never came from a `hello`.
pub fn paired_labels() -> NewInstanceLabels {
    NewInstanceLabels {
        name: "paired-name".to_owned(),
        os: "paired-os".to_owned(),
        arch: "paired-arch".to_owned(),
        version: "paired-version".to_owned(),
    }
}

struct Held {
    hub: MemoryHub,
    labels: NewInstanceLabels,
    /// Postgres's clock at the insert, which is the wall clock here too.
    created_at: OffsetDateTime,
}

#[derive(Default)]
struct Fleet {
    /// In pairing order.
    held: Vec<Held>,
    trusted: HashMap<UserId, Vec<String>>,
}

#[derive(Default)]
pub struct MemoryFleet {
    fleet: Mutex<Fleet>,
}

impl MemoryFleet {
    pub fn new() -> Self {
        Self::default()
    }

    /// A tenant nobody holds yet. Minted here because a `UserId` cannot be
    /// made outside the crate.
    pub fn tenant(&self) -> UserId {
        UserId::generate()
    }

    /// An instance of `user` as its pairing left it, labelled with
    /// [`paired_labels`].
    pub fn paired(&self, user: UserId, scopes: &[Scope]) -> InstanceId {
        let hub = MemoryHub::paired_to(user, scopes);
        let instance = hub.instance_id();
        self.fleet().held.push(Held {
            hub,
            labels: paired_labels(),
            created_at: OffsetDateTime::now_utc(),
        });
        instance
    }

    /// Writes the labels of a `hello` and makes the instance present or
    /// absent. The scopes stay as they were, and no time column is written.
    pub fn greeted(
        &self,
        instance: InstanceId,
        os: &str,
        arch: &str,
        version: &str,
        present: bool,
    ) {
        let hub = self.hub(instance);
        hub.labelled(version, os, arch);
        hub.present_through(present.then(|| fresh_connection(instance)));
    }

    pub async fn mirrored(&self, instance: InstanceId, event: RunEvent) {
        let stored = self
            .hub(instance)
            .store()
            .ingested(&event, cursor::advanced)
            .await;
        if let Err(error) = stored {
            panic!("memory fleet: a mirror write a test did not script to fail: {error}");
        }
    }

    pub fn mirrored_unreadable(&self, instance: InstanceId, feature_id: &str, offset: u64) {
        self.hub(instance).mirrored_unreadable(feature_id, offset);
    }

    /// Named apart from [`ApiStore::snapshot`], which an inherent method of
    /// the same name would shadow at every call site.
    pub async fn snapshot_sent(&self, instance: InstanceId, snapshot: Snapshot) {
        let stored = self
            .hub(instance)
            .store()
            .snapshot_replaced(&snapshot, wall_clock())
            .await;
        if let Err(error) = stored {
            panic!("memory fleet: a snapshot write a test did not script to fail: {error}");
        }
    }

    /// The credential ids, as unpadded base64url, of `user`'s trusted
    /// passkeys from here on.
    pub fn trusted(&self, user: UserId, credential_ids: Vec<String>) {
        self.fleet().trusted.insert(user, credential_ids);
    }

    pub fn scopes(&self, instance: InstanceId, scopes: &[Scope]) {
        self.hub(instance).scoped(scopes);
    }

    /// Every request ever queued for the instance, oldest first.
    pub fn requests(&self, instance: InstanceId) -> Vec<RequestView> {
        self.hub(instance).requests()
    }

    /// The store a session of the instance writes through. Unlike
    /// [`requests_of`](ApiStore::requests_of) the pair must be one the fleet
    /// holds: a session is only ever admitted as its own instance.
    pub fn store(&self, user: UserId, instance: InstanceId) -> MemoryStore {
        match self.held_by(user, instance) {
            Some(hub) => hub.store(),
            None => panic!("memory fleet: a session of a pair the fleet does not hold"),
        }
    }

    /// The hub of an instance a test paired. Instance ids are minted unique,
    /// so the id alone names it; one that is not here was made up.
    fn hub(&self, instance: InstanceId) -> MemoryHub {
        let fleet = self.fleet();
        match fleet
            .held
            .iter()
            .find(|held| held.hub.instance_id() == instance)
        {
            Some(held) => held.hub.clone(),
            None => panic!("memory fleet: an instance no test paired"),
        }
    }

    fn held_by(&self, user: UserId, instance: InstanceId) -> Option<MemoryHub> {
        let fleet = self.fleet();
        fleet
            .held
            .iter()
            .find(|held| held.hub.user_id() == user && held.hub.instance_id() == instance)
            .map(|held| held.hub.clone())
    }

    fn fleet(&self) -> MutexGuard<'_, Fleet> {
        match self.fleet.lock() {
            Ok(fleet) => fleet,
            Err(_) => panic!("memory fleet: a test panicked while it held the fleet"),
        }
    }
}

#[async_trait]
impl ApiStore for MemoryFleet {
    async fn instances(&self, user: UserId) -> Result<Vec<Instance>, StoreError> {
        let fleet = self.fleet();
        let mut rows: Vec<Instance> = fleet
            .held
            .iter()
            .filter(|held| held.hub.user_id() == user)
            .map(instance_row)
            .collect();
        rows.sort_by_key(|row| (row.created_at, row.id.0));
        Ok(rows)
    }

    async fn instance(&self, user: UserId, id: InstanceId) -> Result<Option<Instance>, StoreError> {
        let fleet = self.fleet();
        Ok(fleet
            .held
            .iter()
            .find(|held| held.hub.user_id() == user && held.hub.instance_id() == id)
            .map(instance_row))
    }

    async fn mirror(
        &self,
        user: UserId,
        instance: InstanceId,
        feature: Option<&str>,
    ) -> Result<Vec<MirrorEvent>, StoreError> {
        let Some(hub) = self.held_by(user, instance) else {
            return Ok(Vec::new());
        };
        Ok(hub
            .mirror()
            .into_iter()
            .filter(|row| feature.is_none_or(|feature| row.feature_id == feature))
            .map(|row| {
                mirror_event(MirrorRecord {
                    feature_id: row.feature_id,
                    offset: row.offset,
                    event: row.event,
                })
            })
            .collect())
    }

    async fn snapshot(
        &self,
        user: UserId,
        instance: InstanceId,
    ) -> Result<Option<SnapshotRead>, StoreError> {
        let snapshot = self.held_by(user, instance).and_then(|hub| hub.snapshot());
        Ok(snapshot.map(|row| snapshot_read(&row.body)))
    }

    async fn request(
        &self,
        user: UserId,
        instance: InstanceId,
        id: RequestId,
    ) -> Result<Option<RequestRecord>, StoreError> {
        let Some(hub) = self.held_by(user, instance) else {
            return Ok(None);
        };
        let Some(request) = hub.requests().into_iter().find(|held| held.id == id) else {
            return Ok(None);
        };
        let Some(created_at) = hub.created_at(id) else {
            panic!("memory fleet: a request held with no time of insert");
        };
        Ok(Some(RequestRecord {
            id: request.id,
            kind: request.kind,
            status: request.status,
            deliveries: request.deliveries,
            outcome: request.outcome,
            expires_at: request.expires_at,
            created_at,
            answered_at: request.answered_at,
        }))
    }

    async fn trusted_credential_ids(&self, user: UserId) -> Result<Vec<String>, StoreError> {
        match self.fleet().trusted.get(&user) {
            Some(ids) => Ok(ids.clone()),
            None => panic!("memory fleet: the passkeys of a tenant no test said it trusted"),
        }
    }

    async fn requests_of(
        &self,
        user: UserId,
        instance: InstanceId,
    ) -> Result<Box<dyn SocketStore>, StoreError> {
        Ok(match self.held_by(user, instance) {
            Some(hub) => Box::new(hub.store()),
            None => Box::new(NotHeld(instance)),
        })
    }
}

fn instance_row(held: &Held) -> Instance {
    let view = held.hub.instance();
    let connection = held.hub.connection();
    Instance {
        id: held.hub.instance_id(),
        user_id: held.hub.user_id(),
        name: held.labels.name.clone(),
        os: view.os.unwrap_or_else(|| held.labels.os.clone()),
        arch: view.arch.unwrap_or_else(|| held.labels.arch.clone()),
        version: view.version.unwrap_or_else(|| held.labels.version.clone()),
        last_seen: view.last_seen.map(at),
        granted_scopes: view
            .scopes
            .iter()
            .map(|scope| scope_name(*scope).to_owned())
            .collect(),
        created_at: held.created_at,
        install_id: view.install_id,
        connection_id: connection.map(|(id, _)| id.as_uuid()),
        connected_at: connection.and_then(|(_, since)| since).map(at),
        event_cursor: match i64::try_from(held.hub.cursor()) {
            Ok(cursor) => cursor,
            Err(_) => panic!("memory fleet: a cursor no column holds"),
        },
    }
}

/// What [`PostgresStore`](crate::socket::postgres::PostgresStore) is for a
/// pair whose instance row is not the tenant's: its scopes read as
/// [`StoreError::NoSuchInstance`], which ends an enqueue before anything else
/// is asked. Anything else is a call this double has no answer to.
struct NotHeld(InstanceId);

#[async_trait]
impl SocketStore for NotHeld {
    fn instance(&self) -> InstanceId {
        self.0
    }

    async fn greeted(&self, _: &Hello, _: ConnectionId, _: u64) -> Result<u64, StoreError> {
        unscripted()
    }

    async fn ingested(&self, _: &RunEvent, _: fn(u64, u64) -> u64) -> Result<u64, StoreError> {
        unscripted()
    }

    async fn snapshot_replaced(&self, _: &Snapshot, _: u64) -> Result<(), StoreError> {
        unscripted()
    }

    async fn granted_scopes(&self) -> Result<Vec<Scope>, StoreError> {
        Err(StoreError::NoSuchInstance)
    }

    async fn request_inserted(&self, _: NewRequest) -> Result<(), StoreError> {
        unscripted()
    }

    async fn queued_requests(&self) -> Result<Vec<QueuedRequest>, StoreError> {
        unscripted()
    }

    async fn request_sent(&self, _: RequestId, _: u64) -> Result<(), StoreError> {
        unscripted()
    }

    async fn request_expired(&self, _: RequestId) -> Result<(), StoreError> {
        unscripted()
    }

    async fn outcome_recorded(
        &self,
        _: RequestId,
        _: &(dyn Fn(RequestState) -> OutcomeChange + Sync),
        _: u64,
    ) -> Result<bool, StoreError> {
        unscripted()
    }

    async fn departed(&self, _: ConnectionId, _: u64) -> Result<(), StoreError> {
        unscripted()
    }
}

fn unscripted() -> ! {
    panic!("memory fleet: a store call for a pair the fleet does not hold, past its scopes")
}

/// A connection id of the kind a registry seats, from a registry of its own:
/// presence here is a column, and no registry is consulted for it.
fn fresh_connection(instance: InstanceId) -> ConnectionId {
    match Connections::default().register(instance) {
        Ok(registered) => registered.connection,
        Err(_) => panic!("memory fleet: an empty registry with no room"),
    }
}

fn at(unix_seconds: u64) -> OffsetDateTime {
    let parsed = i64::try_from(unix_seconds)
        .ok()
        .and_then(|seconds| OffsetDateTime::from_unix_timestamp(seconds).ok());
    match parsed {
        Some(at) => at,
        None => panic!("memory fleet: a time no column holds"),
    }
}

fn wall_clock() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(since) => since.as_secs(),
        Err(_) => panic!("memory fleet: a wall clock before 1970"),
    }
}

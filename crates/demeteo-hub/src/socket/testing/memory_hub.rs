//! The rows that stand where the store does. Why a call without an answer is
//! a panic is on [the module](super).

use async_trait::async_trait;
use demeteo_hub_protocol::{Hello, RequestOutcome, RunEvent, Scope, SignedRequest, Snapshot};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::socket::connections::ConnectionId;
use crate::socket::handler::Admission;
use crate::socket::port::SocketStore;
use crate::socket::requests::OutcomeChange;
use crate::store::requests::{
    NewRequest, QueuedEnvelope, QueuedRequest, RequestId, RequestState, RequestStatus,
};
use crate::store::{InstanceId, StoreError, UserId};

/// The row of the one instance a [`MemoryHub`] holds, as a test reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceView {
    /// `None` until a `hello`, and so are the three labels after it. Pairing
    /// writes labels of its own to the real row; they are not kept here
    /// because nothing a session does reads them.
    pub install_id: Option<String>,
    pub version: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub scopes: Vec<Scope>,
    pub present: bool,
    /// Unix seconds.
    pub last_seen: Option<u64>,
}

/// One row of a [`MemoryHub`]'s mirror, as a test reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorRow {
    pub feature_id: String,
    pub offset: u64,
    /// The text the row holds, where the real row holds `JSONB`.
    pub event: String,
}

/// The snapshot a [`MemoryHub`] holds, as a test reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRow {
    /// The text the row holds, where the real row holds `JSONB`.
    pub body: String,
    /// Unix seconds.
    pub updated_at: u64,
}

/// One request a [`MemoryHub`] holds, as a test reads it. It is the row
/// itself: `id`, `kind` and `expires_at` are the columns as they were handed
/// over, kept beside the envelope and never read out of it.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestView {
    pub id: RequestId,
    pub kind: String,
    pub status: RequestStatus,
    pub deliveries: u32,
    /// Unix seconds. `None` until it was sent.
    pub last_sent_at: Option<u64>,
    pub outcome: Option<RequestOutcome>,
    /// Unix seconds. `None` until an outcome was recorded.
    pub answered_at: Option<u64>,
    pub envelope: SignedRequest,
    /// Unix seconds.
    pub expires_at: u64,
}

struct Rows {
    install_id: Option<String>,
    version: Option<String>,
    os: Option<String>,
    arch: Option<String>,
    scopes: Vec<Scope>,
    connection: Option<ConnectionId>,
    last_seen: Option<u64>,
    cursor: u64,
    /// Keyed as `run_mirror` is, less the instance, of which there is one
    /// here. The key is the whole of what makes an event a duplicate: no
    /// method looks at a stored row to decide whether to keep a new one.
    mirror: BTreeMap<(String, u64), String>,
    /// One slot and not a map by kind: every snapshot is written under
    /// [`SNAPSHOT_KIND`](crate::socket::SNAPSHOT_KIND), so a second place to put one
    /// would be a place the real table does not have.
    snapshot: Option<SnapshotRow>,
    /// In the order they were inserted, which is the order `queue_seq`
    /// gives the real rows.
    requests: Vec<RequestView>,
    /// The requests whose envelope the queue reads as unreadable. Apart from
    /// the rows because a [`RequestView`] keeps the envelope it was queued
    /// with, which is what a test compares a delivery against.
    unreadable: HashSet<RequestId>,
    failure: Option<StoreError>,
    departure_takes: Duration,
}

/// One paired instance's rows, for running a session without a database.
/// Every clone is a handle to the same rows, so a test keeps one to read
/// while a session writes through the [`SocketStore`] of another.
///
/// It stands where the store does and, like
/// [`MemoryFamily`](crate::pairing::testing::MemoryFamily), decides nothing:
/// each method writes what it is handed, under the one condition its
/// statement carries and no other.
///
/// A call it has no answer to is a panic and never a default. An empty list
/// or a `false` handed back for a question nobody scripted reads, to the
/// code under test, exactly like an answer.
#[derive(Clone)]
pub struct MemoryHub {
    user: UserId,
    instance: InstanceId,
    rows: Arc<Mutex<Rows>>,
}

impl MemoryHub {
    /// An instance as its pairing left it: `scopes` are what the user left
    /// checked, it has never connected, and nothing is stored for it.
    pub fn paired(scopes: &[Scope]) -> Self {
        let rows = Rows {
            install_id: None,
            version: None,
            os: None,
            arch: None,
            scopes: scopes.to_vec(),
            connection: None,
            last_seen: None,
            cursor: 0,
            mirror: BTreeMap::new(),
            snapshot: None,
            requests: Vec::new(),
            unreadable: HashSet::new(),
            failure: None,
            departure_takes: Duration::ZERO,
        };
        Self {
            user: UserId::generate(),
            instance: InstanceId::generate(),
            rows: Arc::new(Mutex::new(rows)),
        }
    }

    /// Who a socket of this instance is admitted as, on a token that expires
    /// at `token_expires_at` (Unix seconds). The tenant and the instance were
    /// made up once, in [`paired`](Self::paired), and every admission of one
    /// hub names the same two. It is minted here because a `UserId` cannot be
    /// made outside the crate, which is what keeps a caller from choosing its
    /// tenant.
    pub fn admission(&self, token_expires_at: u64) -> Admission {
        Admission {
            user: self.user,
            instance: self.instance,
            token_expires_at,
        }
    }

    pub fn store(&self) -> MemoryStore {
        MemoryStore(self.clone())
    }

    pub fn instance(&self) -> InstanceView {
        let rows = self.rows();
        InstanceView {
            install_id: rows.install_id.clone(),
            version: rows.version.clone(),
            os: rows.os.clone(),
            arch: rows.arch.clone(),
            scopes: rows.scopes.clone(),
            present: rows.connection.is_some(),
            last_seen: rows.last_seen,
        }
    }

    pub fn cursor(&self) -> u64 {
        self.rows().cursor
    }

    /// Every row of the mirror, in the order of its key.
    pub fn mirror(&self) -> Vec<MirrorRow> {
        self.rows()
            .mirror
            .iter()
            .map(|((feature_id, offset), event)| MirrorRow {
                feature_id: feature_id.clone(),
                offset: *offset,
                event: event.clone(),
            })
            .collect()
    }

    pub fn mirror_len(&self) -> usize {
        self.rows().mirror.len()
    }

    /// `None` until the instance has sent one.
    pub fn snapshot(&self) -> Option<SnapshotRow> {
        self.rows().snapshot.clone()
    }

    /// Every request ever queued for the instance, whatever became of it,
    /// oldest first.
    pub fn requests(&self) -> Vec<RequestView> {
        self.rows().requests.clone()
    }

    /// From here on the queue reads the envelope of request `id` as
    /// [`QueuedEnvelope::Unreadable`], as Postgres reads one whose JSON is no
    /// signed request of this build. Its other columns read as before.
    pub fn spoil_envelope(&self, id: RequestId) {
        let mut rows = self.rows();
        Self::request(&mut rows, id);
        rows.unreadable.insert(id);
    }

    /// The next call through a [`SocketStore`] of this hub writes nothing and
    /// returns `error`; the one after it is answered as usual. Which error
    /// is the test's to say: a double that picked one would be claiming to
    /// know how a database fails.
    pub fn fail_next_call(&self, error: StoreError) {
        self.rows().failure = Some(error);
    }

    /// From here on a departure waits `takes` of real time before it
    /// writes, as one to a database does and one to these rows otherwise
    /// does not. A session that writes nothing slowly ends in the poll that
    /// asked it to, and a test of what waits for it could not tell waiting
    /// from not.
    pub fn departing_slowly(&self, takes: Duration) {
        self.rows().departure_takes = takes;
    }

    /// The row of a request a session names. The ids a session has are the
    /// ones it read from these rows, and no row is ever removed, so one that
    /// is not here was made up.
    fn request(rows: &mut Rows, id: RequestId) -> &mut RequestView {
        match rows.requests.iter_mut().find(|held| held.id == id) {
            Some(request) => request,
            None => panic!("memory hub: a request the instance does not hold"),
        }
    }

    fn rows(&self) -> MutexGuard<'_, Rows> {
        match self.rows.lock() {
            Ok(rows) => rows,
            Err(_) => panic!("memory hub: a test panicked while it held the rows"),
        }
    }

    /// The rows for one store call, or the failure that call was scripted to
    /// meet.
    fn written(&self) -> Result<MutexGuard<'_, Rows>, StoreError> {
        let mut rows = self.rows();
        match rows.failure.take() {
            Some(error) => Err(error),
            None => Ok(rows),
        }
    }
}

/// The [`SocketStore`] of a [`MemoryHub`]'s instance.
pub struct MemoryStore(MemoryHub);

#[async_trait]
impl SocketStore for MemoryStore {
    fn instance(&self) -> InstanceId {
        self.0.instance
    }

    async fn greeted(
        &self,
        hello: &Hello,
        connection: ConnectionId,
        now: u64,
    ) -> Result<u64, StoreError> {
        let mut rows = self.0.written()?;
        rows.install_id = Some(hello.install_id.clone());
        rows.version = Some(hello.app_version.clone());
        rows.os = Some(hello.os.clone());
        rows.arch = Some(hello.arch.clone());
        rows.scopes = hello.scopes.clone();
        rows.connection = Some(connection);
        rows.last_seen = Some(now);
        Ok(rows.cursor)
    }

    /// A row already under the key is kept and the event dropped, which is
    /// `entry().or_insert()` here and `ON CONFLICT DO NOTHING` in Postgres.
    async fn ingested(
        &self,
        event: &RunEvent,
        decide: fn(u64, u64) -> u64,
    ) -> Result<u64, StoreError> {
        let mut rows = self.0.written()?;
        let text = match serde_json::to_string(event) {
            Ok(text) => text,
            Err(error) => panic!("memory hub: a run event that does not serialise: {error}"),
        };
        rows.mirror
            .entry((event.feature_id.clone(), event.offset))
            .or_insert(text);
        rows.cursor = decide(rows.cursor, event.offset);
        Ok(rows.cursor)
    }

    async fn snapshot_replaced(&self, snapshot: &Snapshot, now: u64) -> Result<(), StoreError> {
        let mut rows = self.0.written()?;
        let body = match serde_json::to_string(snapshot) {
            Ok(body) => body,
            Err(error) => panic!("memory hub: a snapshot that does not serialise: {error}"),
        };
        rows.snapshot = Some(SnapshotRow {
            body,
            updated_at: now,
        });
        Ok(())
    }

    async fn granted_scopes(&self) -> Result<Vec<Scope>, StoreError> {
        Ok(self.0.written()?.scopes.clone())
    }

    /// An id the instance already holds is a panic where Postgres answers
    /// with the primary key's refusal: that is a database error, and which
    /// one is not this double's to make up.
    async fn request_inserted(&self, request: NewRequest) -> Result<(), StoreError> {
        let mut rows = self.0.written()?;
        if rows.requests.iter().any(|held| held.id == request.id) {
            panic!("memory hub: a request under an id the instance already holds");
        }
        rows.requests.push(RequestView {
            id: request.id,
            kind: request.kind,
            status: RequestStatus::Queued,
            deliveries: 0,
            last_sent_at: None,
            outcome: None,
            answered_at: None,
            envelope: request.envelope,
            expires_at: request.expires_at,
        });
        Ok(())
    }

    async fn queued_requests(&self) -> Result<Vec<QueuedRequest>, StoreError> {
        let rows = self.0.written()?;
        let queued = rows
            .requests
            .iter()
            .filter(|held| held.status == RequestStatus::Queued)
            .map(|held| QueuedRequest {
                id: held.id,
                envelope: if rows.unreadable.contains(&held.id) {
                    QueuedEnvelope::Unreadable
                } else {
                    QueuedEnvelope::Readable(Box::new(held.envelope.clone()))
                },
                expires_at: held.expires_at,
                deliveries: held.deliveries,
            })
            .collect();
        Ok(queued)
    }

    async fn request_sent(&self, id: RequestId, now: u64) -> Result<(), StoreError> {
        let mut rows = self.0.written()?;
        let request = MemoryHub::request(&mut rows, id);
        request.deliveries += 1;
        request.last_sent_at = Some(now);
        Ok(())
    }

    async fn request_expired(&self, id: RequestId) -> Result<(), StoreError> {
        let mut rows = self.0.written()?;
        let request = MemoryHub::request(&mut rows, id);
        if request.status == RequestStatus::Queued {
            request.status = RequestStatus::Expired;
        }
        Ok(())
    }

    /// An id the instance does not hold is `false` here and not the panic of
    /// `MemoryHub::request`: this id is the peer's to choose, and a session
    /// has an answer to one that is nobody's.
    async fn outcome_recorded(
        &self,
        id: RequestId,
        decide: &(dyn Fn(RequestState) -> OutcomeChange + Sync),
        now: u64,
    ) -> Result<bool, StoreError> {
        let mut rows = self.0.written()?;
        let Some(request) = rows.requests.iter_mut().find(|held| held.id == id) else {
            return Ok(false);
        };
        let state = RequestState {
            status: request.status,
            deliveries: request.deliveries,
            outcome: request.outcome,
        };
        match decide(state) {
            OutcomeChange::Record(outcome) => {
                request.status = RequestStatus::Answered;
                request.outcome = Some(outcome);
                request.answered_at = Some(now);
            }
            OutcomeChange::Keep => {}
        }
        Ok(true)
    }

    async fn departed(&self, connection: ConnectionId, now: u64) -> Result<(), StoreError> {
        let takes = self.0.rows().departure_takes;
        if !takes.is_zero() {
            tokio::time::sleep(takes).await;
        }
        let mut rows = self.0.written()?;
        if rows.connection == Some(connection) {
            rows.connection = None;
            rows.last_seen = Some(now);
        }
        Ok(())
    }
}

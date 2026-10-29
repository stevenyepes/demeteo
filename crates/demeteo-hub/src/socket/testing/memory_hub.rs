//! The rows that stand where the store does. Why a call without an answer is
//! a panic is on [the module](super).

use async_trait::async_trait;
use demeteo_hub_protocol::{Hello, RequestOutcome, RunEvent, Scope, SignedRequest, Snapshot};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use crate::socket::connections::ConnectionId;
use crate::socket::handler::Admission;
use crate::socket::port::SocketStore;
use crate::socket::requests::OutcomeChange;
use crate::store::attachments::{AttachmentMeta, NewAttachment, StagedAttachment};
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
    connected_at: Option<u64>,
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
    /// When each request was inserted, by the wall clock, as Postgres's
    /// `created_at` is: no caller names that time. Apart from the rows
    /// because no session reads it.
    created_at: HashMap<RequestId, u64>,
    /// The requests whose envelope the queue reads as unreadable. Apart from
    /// the rows because a [`RequestView`] keeps the envelope it was queued
    /// with, which is what a test compares a delivery against.
    unreadable: HashSet<RequestId>,
    /// In the order they were staged. Under the same lock as `requests`, so
    /// that a request and the binding of its attachments are one write.
    attachments: Vec<StagedAttachment>,
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
    /// An instance of a tenant nobody else holds, as
    /// [`paired_to`](Self::paired_to) leaves it.
    pub fn paired(scopes: &[Scope]) -> Self {
        Self::paired_to(UserId::generate(), scopes)
    }

    /// An instance of `user` as its pairing left it: `scopes` are what the
    /// user left checked, it has never connected, and nothing is stored for
    /// it.
    pub fn paired_to(user: UserId, scopes: &[Scope]) -> Self {
        let rows = Rows {
            install_id: None,
            version: None,
            os: None,
            arch: None,
            scopes: scopes.to_vec(),
            connection: None,
            connected_at: None,
            last_seen: None,
            cursor: 0,
            mirror: BTreeMap::new(),
            snapshot: None,
            requests: Vec::new(),
            created_at: HashMap::new(),
            unreadable: HashSet::new(),
            attachments: Vec::new(),
            failure: None,
            departure_takes: Duration::ZERO,
        };
        Self {
            user,
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

    pub fn user_id(&self) -> UserId {
        self.user
    }

    pub fn instance_id(&self) -> InstanceId {
        self.instance
    }

    /// The connection the instance is present through, and when it became
    /// so: what `instances.connection_id` and `connected_at` hold, which
    /// [`instance`](Self::instance) folds into `present`.
    pub fn connection(&self) -> Option<(ConnectionId, Option<u64>)> {
        let rows = self.rows();
        rows.connection
            .map(|connection| (connection, rows.connected_at))
    }

    /// Unix seconds, by the wall clock at the insert. `None` for an id the
    /// instance does not hold.
    pub fn created_at(&self, id: RequestId) -> Option<u64> {
        self.rows().created_at.get(&id).copied()
    }

    /// Writes the labels a `hello` writes and nothing else of it: the
    /// scopes, the presence and the cursor stay as they were.
    pub fn labelled(&self, version: &str, os: &str, arch: &str) {
        let mut rows = self.rows();
        rows.version = Some(version.to_owned());
        rows.os = Some(os.to_owned());
        rows.arch = Some(arch.to_owned());
    }

    /// Makes `connection` the one the instance is present through, or with
    /// `None` makes it absent, and changes no time column: a test that needs
    /// one drives [`SocketStore::greeted`] instead.
    pub fn present_through(&self, connection: Option<ConnectionId>) {
        let mut rows = self.rows();
        if connection.is_none() {
            rows.connected_at = None;
        }
        rows.connection = connection;
    }

    /// Makes `scopes` what the instance last reported, as a `hello` would.
    pub fn scoped(&self, scopes: &[Scope]) {
        self.rows().scopes = scopes.to_vec();
    }

    /// Stores a mirror row under `(feature_id, offset)` whose text is no
    /// [`RunEvent`], as Postgres holds one written by another build. A row
    /// already under the key is kept, as [`SocketStore::ingested`] keeps one.
    pub fn mirrored_unreadable(&self, feature_id: &str, offset: u64) {
        self.rows()
            .mirror
            .entry((feature_id.to_owned(), offset))
            .or_insert_with(|| "{}".to_owned());
    }

    /// Stores a snapshot whose text is no [`Snapshot`], as Postgres holds
    /// one written by another build.
    pub fn snapshot_unreadable(&self) {
        self.rows().snapshot = Some(SnapshotRow {
            body: "{}".to_owned(),
            updated_at: 0,
        });
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

    /// Every attachment row the instance holds, in the order staged.
    pub fn attachments(&self) -> Vec<StagedAttachment> {
        self.rows().attachments.clone()
    }

    /// `size_bytes` is the ciphertext's length, as the real insert computes
    /// it. An id the instance already holds is a panic, as a request's is
    /// in [`SocketStore::request_inserted`]: the real refusal is a database
    /// error, and which one is not this double's to make up.
    pub fn attachment_staged(&self, new: NewAttachment) -> AttachmentMeta {
        let mut rows = self.rows();
        if rows.attachments.iter().any(|held| held.id == new.id) {
            panic!("memory hub: an attachment under an id the instance already holds");
        }
        let staged = StagedAttachment {
            id: new.id,
            instance_id: new.instance_id,
            request_id: None,
            size_bytes: new.ciphertext.len() as u64,
            ciphertext: new.ciphertext,
            wrapped_key: new.wrapped_key.as_str().to_owned(),
            sealed_digest: new.sealed_digest,
            expires_at: new.expires_at,
        };
        let meta = staged.meta();
        rows.attachments.push(staged);
        meta
    }

    /// The rows of `ids`, whatever their state, each once however often
    /// `ids` names it, as `id = ANY(..)` reads them.
    pub fn attachments_named(&self, ids: &[Uuid]) -> Vec<AttachmentMeta> {
        self.rows()
            .attachments
            .iter()
            .filter(|held| ids.contains(&held.id))
            .map(StagedAttachment::meta)
            .collect()
    }

    /// The join the real read makes: the row is bound, and its request is
    /// still `queued` and not past its `expires_at`. The row's own
    /// `expires_at` is not consulted, as it is not there.
    pub fn attachment_fetchable(&self, id: Uuid, now: u64) -> Option<StagedAttachment> {
        let rows = self.rows();
        let at = Self::fetchable_at(&rows, id, now)?;
        Some(rows.attachments[at].clone())
    }

    /// Deletes the row only while [`attachment_fetchable`](Self::attachment_fetchable)
    /// would answer it, judged under the same lock as the delete, as the real
    /// delete's `EXISTS` is.
    pub fn attachment_consumed(&self, id: Uuid, now: u64) -> bool {
        let mut rows = self.rows();
        match Self::fetchable_at(&rows, id, now) {
            Some(at) => {
                rows.attachments.remove(at);
                true
            }
            None => false,
        }
    }

    fn fetchable_at(rows: &Rows, id: Uuid, now: u64) -> Option<usize> {
        let at = rows.attachments.iter().position(|held| held.id == id)?;
        let request_id = rows.attachments[at].request_id?;
        rows.requests
            .iter()
            .find(|request| request.id == request_id)
            .filter(|request| request.status == RequestStatus::Queued && request.expires_at > now)
            .map(|_| at)
    }

    /// Deletes every row whose `expires_at` is at or before `now`, bound or
    /// not, and answers how many.
    pub fn attachments_expired(&self, now: u64) -> u64 {
        let mut rows = self.rows();
        let before = rows.attachments.len();
        rows.attachments.retain(|held| held.expires_at > now);
        (before - rows.attachments.len()) as u64
    }

    /// Binds attachment `id` to `request` and moves its expiry to the
    /// request's, as a dispatch does, whatever state the row was in: a test
    /// that needs a bound row should not have to dispatch for it. Both must
    /// be held; the real foreign key refuses a request that is not.
    pub fn attachment_bound(&self, id: Uuid, request: RequestId) {
        let mut rows = self.rows();
        let expires_at = Self::request(&mut rows, request).expires_at;
        let row = Self::attachment(&mut rows, id);
        row.request_id = Some(request);
        row.expires_at = expires_at;
    }

    /// Flips every bit of the last ciphertext byte of attachment `id`, as a
    /// corrupted or tampered row would hold it.
    pub fn attachment_tampered(&self, id: Uuid) {
        let mut rows = self.rows();
        match Self::attachment(&mut rows, id).ciphertext.last_mut() {
            Some(byte) => *byte ^= 0xff,
            None => panic!("memory hub: an attachment with no ciphertext to tamper with"),
        }
    }

    fn attachment(rows: &mut Rows, id: Uuid) -> &mut StagedAttachment {
        match rows.attachments.iter_mut().find(|held| held.id == id) {
            Some(attachment) => attachment,
            None => panic!("memory hub: an attachment the instance does not hold"),
        }
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
        rows.connected_at = Some(now);
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

    async fn request_inserted(&self, request: NewRequest) -> Result<(), StoreError> {
        let mut rows = self.0.written()?;
        queued(&mut rows, request);
        Ok(())
    }

    /// Every row is judged under the one lock before anything is written, so
    /// a refusal leaves the queue and the rows as they were. The rows matched
    /// are counted against `attachments.len()`, as the real `UPDATE`'s are,
    /// so an id named twice is refused here as it is there.
    async fn request_inserted_binding(
        &self,
        request: NewRequest,
        attachments: &[Uuid],
        now: u64,
    ) -> Result<(), StoreError> {
        let mut rows = self.0.written()?;
        let instance = self.0.instance;
        let bindable = |held: &StagedAttachment| {
            attachments.contains(&held.id)
                && held.instance_id == instance
                && held.request_id.is_none()
                && held.expires_at > now
        };
        if rows
            .attachments
            .iter()
            .filter(|held| bindable(held))
            .count()
            != attachments.len()
        {
            return Err(StoreError::AttachmentUnavailable);
        }
        let (id, expires_at) = (request.id, request.expires_at);
        queued(&mut rows, request);
        for held in rows.attachments.iter_mut().filter(|held| bindable(held)) {
            held.request_id = Some(id);
            held.expires_at = expires_at;
        }
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
                rows.attachments.retain(|held| held.request_id != Some(id));
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
            rows.connected_at = None;
            rows.last_seen = Some(now);
        }
        Ok(())
    }
}

/// An id the instance already holds is a panic where Postgres answers with
/// the primary key's refusal: that is a database error, and which one is not
/// this double's to make up.
fn queued(rows: &mut Rows, request: NewRequest) {
    if rows.requests.iter().any(|held| held.id == request.id) {
        panic!("memory hub: a request under an id the instance already holds");
    }
    rows.created_at.insert(request.id, wall_clock());
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
}

fn wall_clock() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(since) => since.as_secs(),
        Err(_) => panic!("memory hub: a wall clock before 1970"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::attachments::WrappedKey;
    use demeteo_hub_protocol::{CancelFeature, RequestPayload};

    const NOW: u64 = 1_700_000_000;

    fn staged(hub: &MemoryHub, expires_at: u64) -> Uuid {
        let id = Uuid::new_v4();
        hub.attachment_staged(NewAttachment {
            id,
            instance_id: hub.instance_id(),
            ciphertext: vec![0x5a; 64],
            wrapped_key: WrappedKey::parse("vault:v1:filler").unwrap(),
            sealed_digest: vec![0xa5; 60],
            expires_at,
        });
        id
    }

    fn request(id: u8, instance: InstanceId) -> NewRequest {
        let envelope = SignedRequest {
            v: 1,
            request_id: String::new(),
            instance_id: instance.0.to_string(),
            issued_at: NOW,
            expires_at: NOW + 600,
            payload: RequestPayload::CancelFeature(CancelFeature {
                feature_id: "f-1".to_owned(),
            }),
            signature: String::new(),
        };
        NewRequest {
            id: [id; 16],
            kind: "cancel_feature".to_owned(),
            expires_at: envelope.expires_at,
            envelope,
        }
    }

    #[tokio::test]
    async fn a_bind_that_cannot_take_every_id_queues_nothing_and_changes_no_row() {
        let hub = MemoryHub::paired(&[]);
        let other = MemoryHub::paired(&[]);
        let store = hub.store();
        let live = staged(&hub, NOW + 60);
        let expired = staged(&hub, NOW);
        let bound = staged(&hub, NOW + 60);
        store
            .request_inserted(request(1, hub.instance_id()))
            .await
            .unwrap();
        hub.attachment_bound(bound, [1; 16]);
        let foreign = staged(&other, NOW + 60);
        let requests = hub.requests();
        let rows = hub.attachments();

        for refused in [
            vec![Uuid::new_v4()],
            vec![foreign],
            vec![bound],
            vec![expired],
            vec![live, foreign],
            vec![live, live],
        ] {
            let err = store
                .request_inserted_binding(request(2, hub.instance_id()), &refused, NOW)
                .await
                .unwrap_err();
            assert!(matches!(err, StoreError::AttachmentUnavailable), "{err}");
            assert_eq!(hub.requests(), requests, "{refused:?}");
            assert_eq!(hub.attachments(), rows, "{refused:?}");
        }
        assert!(other.attachments()[0].request_id.is_none());
    }

    #[tokio::test]
    async fn a_bind_of_live_staged_rows_queues_the_request_and_binds_every_one() {
        let hub = MemoryHub::paired(&[]);
        let store = hub.store();
        let (a, b) = (staged(&hub, NOW + 60), staged(&hub, NOW + 1));
        let untouched = staged(&hub, NOW + 60);

        store
            .request_inserted_binding(request(3, hub.instance_id()), &[b, a], NOW)
            .await
            .unwrap();

        let requests = hub.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].id, [3; 16]);
        assert_eq!(requests[0].status, RequestStatus::Queued);
        for row in hub.attachments() {
            if row.id == untouched {
                assert_eq!((row.request_id, row.expires_at), (None, NOW + 60));
            } else {
                assert_eq!((row.request_id, row.expires_at), (Some([3; 16]), NOW + 600));
            }
        }
    }
}

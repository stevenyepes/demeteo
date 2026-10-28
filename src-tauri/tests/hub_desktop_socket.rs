#![cfg(target_os = "linux")]
//! The desktop's half of the instance socket, judged by the Hub's own code.
//! The header of `src-tauri/tests/hub_pairing.rs` says why that judgement
//! has to live in this package.
//!
//! Every header here is one the desktop's production code built — a
//! [`DeviceKey`] proof and the URLs of `domain::hub_pairing` — so a proof the
//! Hub would refuse cannot pass by having been written to suit it.

use demeteo_core::adapters::database::SqliteAdapter;
use demeteo_core::adapters::hub::device_key::DeviceKey;
use demeteo_core::adapters::hub::snapshot_source::RepoSnapshotSource;
use demeteo_core::adapters::hub::socket::session::{
    run_session, RequestHandlerSlot, SessionContext, SessionEnd,
};
use demeteo_core::adapters::hub::socket::snapshot_wake::WakingWorkflows;
use demeteo_core::adapters::hub::socket::wake::WakingRunEvents;
use demeteo_core::adapters::hub::socket::{supervise, Sleeper, SupervisorDeps};
use demeteo_core::adapters::hub::testing::{
    ConnectionLog, Exchange, MemorySecretStore, ScriptedAttachments, ScriptedConnection,
    ScriptedRefreshEndpoint, ScriptedSnapshotSource, ScriptedSocketDialer, UnreadableRunEvents,
};
use demeteo_core::adapters::hub::HubPairing;
use demeteo_core::adapters::notification_noop::NoopNotificationAdapter;
use demeteo_core::application::hub_requests::{HubRequestPorts, HubRequests};
use demeteo_core::composition::{build_core_context_with_policy_writer, CoreConfig, ExecutionMode};
use demeteo_core::domain::hub_pairing::keyring_names::hub_keyring_account;
use demeteo_core::domain::hub_pairing::{
    hub_origin, socket_dial_url, socket_url, token_url, HubPairingError, HubPins, HubSecret,
    PairingAnswer,
};
use demeteo_core::domain::hub_request::{HubLocalPolicy, HubRequestLogEntry, LogKind, LogOutcome};
use demeteo_core::domain::hub_socket::{
    SocketEnd, StopReason, LIVENESS_DEADLINE_SECS, MAX_SNAPSHOT_BYTES, MAX_UNACKED_EVENTS,
    PAGE_SIZE, PREMATURE_EXPIRY_MARGIN_SECS, SNAPSHOT_DEBOUNCE_SECS, STOP_CLOSE_DEADLINE_SECS,
};
use demeteo_core::domain::models::{Workflow, WorkflowVersion};
use demeteo_core::error::AppError;
use demeteo_core::ports::db::WorkflowRepository;
use demeteo_core::ports::hub::{
    DialFailure, HubPinRepository, HubRequestHandler, HubSecretStore, HubSocketConnection,
    HubSocketDialer, HubTokenEndpoint, SocketGone, SocketReceived, SocketUpgrade,
};
use demeteo_core::ports::run_events::RunEventsPort;
use demeteo_core::shared::time::now_secs;
use demeteo_core::state::AppContext;
use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::auth::challenge::base64url_encode;
use demeteo_hub::auth::testing::{answered, lazy_state};
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::pairing::dpop::{verify, Expected};
use demeteo_hub::pairing::grant::{refresh_change, FamilyChange, FreshPair, TokenSecret};
use demeteo_hub::pairing::signing_key::SigningKey;
use demeteo_hub::pairing::testing::MemoryFamily;
use demeteo_hub::protocol::{
    CursorAck, Hello, ProjectSnapshot, RefusalReason, RequestOutcome, RequestPayload,
    RequestResult, RunEvent, RunEventKind, RunEventPayload, Scope, Snapshot, PROTOCOL_VERSION,
};
use demeteo_hub::routes::AppState;
use demeteo_hub::socket::connections::Connections;
use demeteo_hub::socket::handler::{admitted, Admission};
use demeteo_hub::socket::port::{PeerGone, Received, Transport};
use demeteo_hub::socket::requests;
use demeteo_hub::socket::session::{self, Ended, Limits};
use demeteo_hub::socket::testing::{MemoryHub, MirrorRow, RequestView};
use demeteo_hub::store::requests::RequestStatus;
use demeteo_hub::store::tokens::TokenHash;
use demeteo_hub::store::{InstanceId, StoreError, UserId};
use serde_json::{json, Value};
use std::future::ready;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch, Notify};
use tokio::task::JoinHandle;
use tokio::time::timeout;

/// When every proof below is signed and presented, in Unix seconds.
const NOW: u64 = 1_700_000_000;

/// The Hub's origin, which is also what its socket handler builds the `htu`
/// it compares from.
const HUB_URL: &str = "https://hub.example";

/// A Hub whose pool never connects. Must be made inside a Tokio runtime, as
/// `lazy_state` requires.
fn unconnected_state() -> AppState {
    let rp = RelyingParty {
        rp_id: "hub.example".to_owned(),
        origin: HUB_URL.to_owned(),
    };
    lazy_state("postgres://hub@db.invalid/hub", rp).expect("a parsable database URL")
}

/// `ApiError::INVALID_TOKEN` as a client receives it; the constant itself is
/// private to the Hub.
fn invalid_token() -> (u16, Value) {
    (401, json!({"error": "invalid_token"}))
}

fn dpop(token: &TokenSecret) -> String {
    format!("DPoP {}", token.text())
}

/// A Hub that has paired one desktop: the token rows its grant left, and the
/// desktop's [`DeviceKey`] registered to one instance of the token's user.
struct PairedDesktop {
    state: AppState,
    origin: String,
    key: DeviceKey,
    issued: FreshPair,
    tokens: MemoryFamily,
    user: UserId,
    instance: InstanceId,
}

impl PairedDesktop {
    /// Paired at `NOW`. Must be made inside a Tokio runtime.
    fn new() -> Self {
        let state = unconnected_state();
        let origin = hub_origin(HUB_URL).expect("a valid Hub URL");
        assert_eq!(
            origin,
            state.rp().origin,
            "the desktop spells the Hub's origin"
        );
        let key = DeviceKey::generate().expect("a P-256 key");
        let issued = FreshPair::generate().expect("the system random source");
        let tokens = MemoryFamily::paired(&issued, &key.jkt(), NOW);
        let user = tokens.rows()[0].user_id;
        Self {
            state,
            origin,
            key,
            issued,
            tokens,
            user,
            instance: InstanceId::generate(),
        }
    }

    /// The desktop's proof for `GET /socket`, naming `token`. Every proof
    /// needs a `jti` of its own, or the Hub refuses it as a replay.
    fn socket_proof(&self, token: &TokenSecret, jti: &str) -> String {
        let access = token.text();
        self.key
            .dpop_proof_for("GET", &socket_url(&self.origin), NOW, jti, Some(&access))
            .expect("a signed proof")
    }

    /// `authorization` and `proof` put to the Hub's `admitted` over this
    /// store: the admission, or what the desktop is sent in its place. The
    /// reads answer what the rows say and nothing else.
    async fn admit(&self, authorization: &str, proof: &str) -> Result<Admission, (u16, Value)> {
        let find_token =
            |hash: TokenHash| ready(Ok::<_, StoreError>(self.tokens.find(&hash).cloned()));
        let instance_of_key = |user: UserId, jkt: String| {
            let held = user == self.user && jkt == self.key.jkt();
            ready(Ok::<_, StoreError>(held.then_some(self.instance)))
        };
        let outcome = admitted(
            &self.state,
            Some(authorization),
            Some(proof),
            NOW,
            find_token,
            instance_of_key,
        )
        .await;
        match outcome {
            Ok(admission) => Ok(admission),
            Err(refusal) => Err(answered(refusal).await),
        }
    }

    /// The admission of this desktop's instance under a token issued at
    /// `NOW`, which is good for 900 seconds.
    fn admission(&self) -> Admission {
        Admission {
            user: self.user,
            instance: self.instance,
            token_expires_at: NOW + 900,
        }
    }

    /// A second live access token for the same key, as a refresh leaves
    /// one beside the first until it expires.
    fn rotated(&mut self) -> FreshPair {
        let next = FreshPair::generate().expect("the system random source");
        let rows = next
            .rows(self.tokens.rows()[0].family_id, &self.key.jkt(), NOW)
            .expect("an expiry within the range of `time`");
        let rotation = FamilyChange {
            retire: Vec::new(),
            insert: vec![rows.access, rows.refresh],
        };
        self.tokens.apply(&rotation, NOW);
        next
    }
}

#[tokio::test]
async fn the_hub_admits_the_desktops_socket_upgrade() {
    let desktop = PairedDesktop::new();
    let access = &desktop.issued.access;

    let proof = desktop.socket_proof(access, "desktop-socket-1");

    assert_eq!(
        desktop.admit(&dpop(access), &proof).await,
        Ok(desktop.admission())
    );
}

/// Both tokens are live and bound to the desktop's key, so the only thing
/// wrong with the refused upgrade is the `ath` its proof carries.
#[tokio::test]
async fn the_hub_refuses_a_desktop_socket_proof_naming_another_token() {
    let mut desktop = PairedDesktop::new();
    let next = desktop.rotated();
    let control = desktop.socket_proof(&next.access, "desktop-socket-control");
    assert_eq!(
        desktop.admit(&dpop(&next.access), &control).await,
        Ok(desktop.admission())
    );

    let proof = desktop.socket_proof(&desktop.issued.access, "desktop-socket-other");

    assert_eq!(
        desktop.admit(&dpop(&next.access), &proof).await,
        Err(invalid_token())
    );
}

#[tokio::test]
async fn the_hub_refuses_the_desktops_token_proof_on_the_socket() {
    let desktop = PairedDesktop::new();
    let access = &desktop.issued.access;

    let proof = desktop
        .key
        .dpop_proof(&token_url(&desktop.origin), NOW, "desktop-token-1")
        .expect("a signed proof");

    assert_eq!(
        desktop.admit(&dpop(access), &proof).await,
        Err(invalid_token())
    );
}

#[tokio::test]
async fn the_hub_refuses_a_desktop_socket_proof_for_the_url_it_dials() {
    let desktop = PairedDesktop::new();
    let access = desktop.issued.access.text();
    let dialled = socket_dial_url(&desktop.origin).expect("a dialable origin");

    let proof = desktop
        .key
        .dpop_proof_for("GET", &dialled, NOW, "desktop-wss-1", Some(&access))
        .expect("a signed proof");

    assert_eq!(
        desktop.admit(&dpop(&desktop.issued.access), &proof).await,
        Err(invalid_token())
    );
}

/// The pins of one pairing, under one install id.
struct Pinned(Mutex<Option<HubPins>>);

impl HubPinRepository for Pinned {
    fn install_id(&self) -> Result<String, AppError> {
        Ok("install-1".to_owned())
    }
    fn load(&self) -> Result<Option<HubPins>, AppError> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save(&self, pins: &HubPins) -> Result<(), AppError> {
        *self.0.lock().unwrap() = Some(pins.clone());
        Ok(())
    }
    fn clear(&self) -> Result<(), AppError> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}

/// A refresh must never redeem a code.
struct NoRedemption;

#[async_trait::async_trait]
impl HubTokenEndpoint for NoRedemption {
    async fn exchange(
        &self,
        _: &str,
        _: String,
        _: String,
    ) -> Result<PairingAnswer, HubPairingError> {
        Err(HubPairingError::Transport(
            "a refresh redeemed a code".into(),
        ))
    }
}

fn form_pairs(body: &str) -> Vec<(String, String)> {
    reqwest::Url::parse(&format!("http://x/?{body}"))
        .expect("a form body")
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// The desktop holds the refresh token the Hub issued and the key it is
/// bound to; the Hub then judges the request with its own verifier and
/// its own rotation decision.
#[tokio::test]
async fn the_hub_accepts_the_desktops_refresh_request() {
    let desktop = PairedDesktop::new();
    let pins = Pinned(Mutex::new(None));
    let hub_pins = HubPins {
        hub_origin: desktop.origin.clone(),
        instance_id: "inst-1".to_owned(),
        request_signing_key: "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo".to_owned(),
        rp_id: "hub.example".to_owned(),
        scopes: vec!["read".to_owned()],
        device_jkt: desktop.key.jkt(),
        paired_at: 1_700_000_000,
        passkeys: Vec::new(),
    };
    let refresh = desktop.issued.refresh.text();
    let secrets = MemorySecretStore::paired(&pins, &hub_pins, &desktop.key, &refresh)
        .expect("a seeded store");
    let endpoint = Arc::new(ScriptedRefreshEndpoint::new());
    let next = FreshPair::generate().expect("the system random source");
    endpoint.answer(&next.access.text(), &next.refresh.text(), 900);
    let hub = HubPairing::new(Arc::new(pins), Arc::new(secrets), Arc::new(NoRedemption))
        .with_refresh_endpoint(endpoint.clone());

    let access = hub.refresh_access().await.expect("a refresh");
    let now = now_secs();

    assert_eq!(access.expose(), next.access.text());
    let [call] = &endpoint.calls()[..] else {
        panic!("one /token call, got {:?}", endpoint.calls());
    };
    let htu = format!("{HUB_URL}/token");
    assert_eq!(call.token_url, htu);
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh.as_str()),
    ]
    .map(|(k, v)| (k.to_owned(), v.to_owned()));
    assert_eq!(form_pairs(&call.form_body), form);
    let proof = verify(
        &call.dpop,
        &Expected {
            htm: "POST",
            htu: &htu,
        },
        now,
    )
    .expect("a proof the Hub accepts");
    assert_eq!(proof.jkt, desktop.key.jkt());
    assert_eq!(proof.ath, None);

    let presented = TokenSecret::parse(&form[1].1).expect("a token the Hub can read");
    let family = MemoryFamily::paired(&desktop.issued, &desktop.key.jkt(), now);
    let decision = refresh_change(family.rows(), &presented.hash(), &proof.jkt, now, &next)
        .expect("an expiry within range");
    assert_eq!(decision.verdict, Ok(()));
}

/// What the Hub sends down the bridge.
enum Down {
    Text(String),
    Ping,
    Close(u16),
}

/// The Hub's end of the in-process bridge. It records every text frame the
/// desktop sent, as the Hub's session reads it, and publishes the offset of
/// every `CursorAck` the session sends: the session writes the store before
/// it acks, so a test waits on that and never on a poll interval.
struct HubEnd {
    up: mpsc::UnboundedReceiver<String>,
    down: mpsc::UnboundedSender<Down>,
    heard: Arc<Mutex<Vec<String>>>,
    acks: Arc<watch::Sender<Option<u64>>>,
}

#[async_trait::async_trait]
impl Transport for HubEnd {
    async fn received(&mut self) -> Received {
        match self.up.recv().await {
            Some(text) => {
                self.heard.lock().unwrap().push(text.clone());
                Received::Text(text)
            }
            None => Received::Closed,
        }
    }

    async fn send_text(&mut self, text: String) -> Result<(), PeerGone> {
        let ack = serde_json::from_str::<CursorAck>(&text).ok();
        self.down.send(Down::Text(text)).map_err(|_| PeerGone)?;
        if let Some(ack) = ack {
            self.acks.send_replace(Some(ack.offset));
        }
        Ok(())
    }

    async fn send_ping(&mut self) -> Result<(), PeerGone> {
        self.down.send(Down::Ping).map_err(|_| PeerGone)
    }

    async fn close(self, code: u16) {
        self.down.send(Down::Close(code)).ok();
    }
}

/// The desktop's end of the bridge. Its close sends nothing: dropping the
/// channel is what the Hub's session reads as the peer gone.
struct DesktopEnd {
    up: mpsc::UnboundedSender<String>,
    down: mpsc::UnboundedReceiver<Down>,
}

#[async_trait::async_trait]
impl HubSocketConnection for DesktopEnd {
    async fn received(&mut self) -> SocketReceived {
        match self.down.recv().await {
            Some(Down::Text(text)) => SocketReceived::Text(text),
            Some(Down::Ping) => SocketReceived::Alive,
            Some(Down::Close(code)) => SocketReceived::Closed(Some(code)),
            None => SocketReceived::Closed(None),
        }
    }

    async fn send_text(&mut self, text: String) -> Result<(), SocketGone> {
        self.up.send(text).map_err(|_| SocketGone)
    }

    async fn close(self: Box<Self>, _code: u16) {}
}

/// No ping is sent and no peer given up on within any wait in this file.
const UNHURRIED: Limits = Limits {
    ping_interval: Duration::from_secs(3600),
    pong_deadline: Duration::from_secs(3600),
};

/// How long a test waits for the Hub to have stored what it expects.
const BOUND: Duration = Duration::from_secs(5);

/// A Hub that admits each dial with its own `admitted` and runs the Hub's
/// own session over the bridge, against a [`MemoryHub`].
struct BridgeDialer {
    desktop: PairedDesktop,
    hub: MemoryHub,
    connections: Connections,
    heard: Arc<Mutex<Vec<String>>>,
    acks: Arc<watch::Sender<Option<u64>>>,
    sessions: Mutex<Vec<JoinHandle<Ended>>>,
}

#[async_trait::async_trait]
impl HubSocketDialer for BridgeDialer {
    async fn dial(
        &self,
        upgrade: SocketUpgrade,
    ) -> Result<Box<dyn HubSocketConnection>, DialFailure> {
        let dialled = socket_dial_url(&self.desktop.origin).expect("a dialable origin");
        if upgrade.url != dialled {
            return Err(DialFailure::Unreachable(upgrade.url));
        }
        let authorization = format!("DPoP {}", upgrade.access_token.expose());
        let admission = self
            .desktop
            .admit(&authorization, &upgrade.dpop)
            .await
            .map_err(|(status, body)| DialFailure::Refused {
                status,
                error: body["error"].as_str().map(str::to_owned),
            })?;
        let registered = self
            .connections
            .register(admission.instance)
            .expect("a seat in the registry");
        let (up, hub_up) = mpsc::unbounded_channel();
        let (hub_down, down) = mpsc::unbounded_channel();
        let hub_end = HubEnd {
            up: hub_up,
            down: hub_down,
            heard: Arc::clone(&self.heard),
            acks: Arc::clone(&self.acks),
        };
        let session = session::run(
            hub_end,
            self.hub.store(),
            registered,
            admission,
            || NOW,
            UNHURRIED,
        );
        self.sessions.lock().unwrap().push(tokio::spawn(session));
        Ok(Box::new(DesktopEnd { up, down }))
    }
}

/// A row as the desktop's recorders append it: Feature, kind, payload.
type Row = (&'static str, &'static str, Value);

/// When every row below was written, in milliseconds, as `run_events`
/// stores it.
const WRITTEN_AT_MS: i64 = 1_700_000_123_456;

/// A desktop paired with a [`MemoryHub`] through the Hub's own token rows,
/// its run log a real SQLite table behind the wake decorator, and the
/// upgrade its pairing yields.
struct Bridged {
    dialer: BridgeDialer,
    /// The Hub's key service and the request-signing key in it, whose
    /// public half the desktop's pins hold.
    keys: MemoryKeys,
    signing: SigningKey,
    db: Arc<SqliteAdapter>,
    run_events: Arc<WakingRunEvents>,
    upgrade: SocketUpgrade,
    /// Dials so far. Each needs a proof with a `jti` of its own.
    dials: AtomicUsize,
}

impl Bridged {
    /// The access token is the one `refresh_access` returns, issued as a
    /// rotation the Hub's rows hold, so the dial is admitted only if the
    /// desktop presents what its pairing handed it.
    async fn new(rows: &[Row]) -> Self {
        Self::pinned(
            rows,
            &["read", "gates", "teleport"],
            &[Scope::Read, Scope::Gates],
        )
        .await
    }

    /// Paired with `granted` on the Hub's side and `pinned` in the desktop's
    /// pins.
    async fn pinned(rows: &[Row], pinned: &[&str], granted: &[Scope]) -> Self {
        let mut desktop = PairedDesktop::new();
        let next = desktop.rotated();
        let hub = MemoryHub::paired_to(desktop.user, granted);
        desktop.instance = hub.instance_id();
        let keys = MemoryKeys::with_keys([REQUEST_SIGNING_KEY]);
        let signing = SigningKey::new(REQUEST_SIGNING_KEY);
        let request_signing_key = signing
            .public_key(&keys)
            .await
            .expect("the Hub's public key");

        let db = Arc::new(
            SqliteAdapter::new(rusqlite::Connection::open_in_memory().expect("a scratch DB"))
                .expect("a migrated DB"),
        );
        let hub_pins = HubPins {
            hub_origin: desktop.origin.clone(),
            instance_id: hub.instance_id().0.to_string(),
            request_signing_key: base64url_encode(&request_signing_key),
            rp_id: "hub.example".to_owned(),
            scopes: pinned.iter().map(|&scope| scope.to_owned()).collect(),
            device_jkt: desktop.key.jkt(),
            paired_at: NOW as i64,
            passkeys: Vec::new(),
        };
        let secrets = MemorySecretStore::paired(
            &*db,
            &hub_pins,
            &desktop.key,
            &desktop.issued.refresh.text(),
        )
        .expect("a seeded store");
        let endpoint = Arc::new(ScriptedRefreshEndpoint::new());
        endpoint.answer(&next.access.text(), &next.refresh.text(), 900);
        let pairing = HubPairing::new(db.clone(), Arc::new(secrets), Arc::new(NoRedemption))
            .with_refresh_endpoint(endpoint);
        let access_token = pairing.refresh_access().await.expect("a refresh");
        let upgrade = SocketUpgrade {
            url: socket_dial_url(&desktop.origin).expect("a dialable origin"),
            dpop: desktop.socket_proof(&next.access, "desktop-session-1"),
            access_token,
        };

        let run_events = Arc::new(WakingRunEvents::new(db.clone()));
        let bridged = Self {
            dialer: BridgeDialer {
                desktop,
                hub,
                connections: Connections::default(),
                heard: Arc::default(),
                acks: Arc::new(watch::Sender::new(None)),
                sessions: Mutex::default(),
            },
            keys,
            signing,
            db,
            run_events,
            upgrade,
            dials: AtomicUsize::new(0),
        };
        for row in rows {
            bridged.append(row);
        }
        bridged
    }

    fn append(&self, (feature, kind, payload): &Row) -> u64 {
        let payload = payload.to_string();
        let offset = self
            .run_events
            .append(feature, kind, Some(&payload), WRITTEN_AT_MS)
            .expect("an appended row");
        u64::try_from(offset).expect("a positive id")
    }

    fn context(&self) -> SessionContext {
        let pins = self.db.load().expect("readable pins").expect("pins");
        SessionContext::new(
            self.db.install_id().expect("an install id"),
            APP_VERSION.to_owned(),
            pins.scopes,
            self.run_events.clone(),
            self.run_events.wake(),
            Arc::new(ScriptedSnapshotSource::default()),
            Arc::new(Notify::new()),
        )
    }

    /// Dials once and runs the desktop's session on its own task.
    async fn connect(&self) -> Running {
        self.connect_with(self.context()).await
    }

    async fn connect_with(&self, ctx: SessionContext) -> Running {
        let conn = self.dial().await;
        let (stop, mut stopped) = watch::channel(false);
        let task = tokio::spawn(async move { run_session(conn, &ctx, &mut stopped).await });
        Running { stop, task }
    }

    /// An admitted connection, with no desktop session on it.
    async fn dial(&self) -> Box<dyn HubSocketConnection> {
        let n = self.dials.fetch_add(1, Ordering::SeqCst);
        let desktop = &self.dialer.desktop;
        let access = self.upgrade.access_token.expose().to_owned();
        let dpop = desktop
            .key
            .dpop_proof_for(
                "GET",
                &socket_url(&desktop.origin),
                NOW,
                &format!("desktop-session-{n}"),
                Some(&access),
            )
            .expect("a signed proof");
        let upgrade = SocketUpgrade {
            dpop,
            ..self.upgrade.clone()
        };
        self.dialer
            .dial(upgrade)
            .await
            .map_err(|failure| format!("{failure:?}"))
            .expect("an admitted upgrade")
    }

    /// Returns once every Hub session so far has ended, having read every
    /// frame the desktop sent before its end of the bridge went.
    async fn hub_sessions_ended(&self) -> Vec<Ended> {
        let sessions = std::mem::take(&mut *self.dialer.sessions.lock().unwrap());
        let mut ended = Vec::new();
        for session in sessions {
            let end = timeout(BOUND, session).await.expect("the Hub session ends");
            ended.push(end.expect("the Hub session did not panic"));
        }
        ended
    }

    /// A request of the kind the protocol fixture `name` holds, queued for
    /// this instance at `NOW` as the session API queues one.
    async fn queue_request(&self, name: &str) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("src-tauri has a parent directory")
            .join("crates")
            .join("demeteo-hub-protocol")
            .join("tests")
            .join("fixtures")
            .join(name);
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let fixture: Value = serde_json::from_str(&text).expect("a JSON fixture");
        let payload: RequestPayload =
            serde_json::from_value(fixture["payload"].clone()).expect("a request payload");
        requests::enqueued(
            &self.dialer.hub.store(),
            &self.keys,
            &self.signing,
            &self.dialer.connections,
            payload,
            NOW,
        )
        .await
        .expect("a queued request");
    }

    /// Returns once the Hub has acked `offset` or beyond, which it does only
    /// after storing it.
    async fn acked_through(&self, offset: u64) {
        let mut acks = self.dialer.acks.subscribe();
        timeout(
            BOUND,
            acks.wait_for(|ack| ack.is_some_and(|ack| ack >= offset)),
        )
        .await
        .unwrap_or_else(|_| panic!("the Hub acked offset {offset} within {BOUND:?}"))
        .expect("the bridge is alive");
    }

    fn heard(&self) -> Vec<String> {
        self.dialer.heard.lock().unwrap().clone()
    }

    /// The desktop's own request handler over a scratch context judging by
    /// `policy`, reading the pins this pairing saved, bound as composition
    /// binds it.
    fn handler(&self, tag: &str, policy: &HubLocalPolicy) -> (RequestHandlerSlot, Arc<AppContext>) {
        let (ctx, writer) = build_core_context_with_policy_writer(
            CoreConfig {
                app_data_dir: demeteo_core::test_dir::scratch(&format!("demeteo-hub-socket-{tag}")),
                execution_mode: ExecutionMode::LocalOnly,
            },
            Arc::new(NoopNotificationAdapter),
            tokio::runtime::Handle::current(),
        );
        writer.save(policy).expect("save the policy");
        let ctx = Arc::new(ctx);
        let handler = HubRequests::new(
            ctx.clone(),
            HubRequestPorts {
                requests: ctx.hub_requests.clone(),
                policy: ctx.hub_policy.clone(),
                pins: self.db.clone(),
                passkeys: self.db.clone(),
                attachments: Arc::new(ScriptedAttachments::new()),
                now_ms: Arc::new(|| NOW * 1000),
            },
        );
        let slot = RequestHandlerSlot::default();
        assert!(slot.set(Arc::new(handler)).is_ok());
        (slot, ctx)
    }

    /// The only request queued, once the Hub has recorded an outcome for it.
    async fn answered(&self) -> RequestView {
        timeout(BOUND, async {
            loop {
                if let [request] = &self.dialer.hub.requests()[..] {
                    if request.outcome.is_some() {
                        return request.clone();
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the Hub recorded an outcome within {BOUND:?}"))
    }
}

/// The desktop's activity log, once it holds a row.
async fn logged(ctx: &AppContext) -> HubRequestLogEntry {
    timeout(BOUND, async {
        loop {
            if let Some(entry) = ctx.hub_requests.list_log(1).expect("list the log").pop() {
                return entry;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the desktop logged the request within {BOUND:?}"))
}

/// Both ceilings well above any start a fixture asks for, so a toggle is
/// the only thing a test switches.
fn policy(spend_enabled: bool, gates_enabled: bool) -> HubLocalPolicy {
    HubLocalPolicy {
        spend_enabled,
        gates_enabled,
        run_ceiling_cents: Some(1_000),
        daily_ceiling_cents: Some(10_000),
    }
}

/// The Hub's name for its request-signing key in [`MemoryKeys`].
const REQUEST_SIGNING_KEY: &str = "demeteo-hub-signing";

struct Running {
    stop: watch::Sender<bool>,
    task: JoinHandle<(SessionEnd, bool)>,
}

impl Running {
    async fn stopped(self) -> (SessionEnd, bool) {
        self.stop.send_replace(true);
        timeout(BOUND, self.task)
            .await
            .expect("the session stops")
            .expect("the session did not panic")
    }
}

const APP_VERSION: &str = "9.8.7";

fn mirrored(
    feature_id: &str,
    offset: u64,
    kind: RunEventKind,
    payload: RunEventPayload,
) -> MirrorRow {
    let event = RunEvent {
        v: PROTOCOL_VERSION,
        feature_id: feature_id.to_owned(),
        offset,
        kind,
        occurred_at: (WRITTEN_AT_MS / 1000) as u64,
        payload,
    };
    MirrorRow {
        feature_id: feature_id.to_owned(),
        offset,
        event: serde_json::to_string(&event).expect("a serialisable event"),
    }
}

fn step(step_id: &str) -> RunEventPayload {
    RunEventPayload {
        step_id: Some(step_id.to_owned()),
        ..RunEventPayload::default()
    }
}

fn gate(step_execution_id: &str) -> RunEventPayload {
    RunEventPayload {
        step_execution_id: Some(step_execution_id.to_owned()),
        ..RunEventPayload::default()
    }
}

fn sorted(mut rows: Vec<MirrorRow>) -> Vec<MirrorRow> {
    rows.sort_by(|a, b| (&a.feature_id, a.offset).cmp(&(&b.feature_id, b.offset)));
    rows
}

/// AC3.1–3.3: the `Hello`, the backfill across two Features with the
/// off-allowlist rows left behind, and a row appended while connected.
#[tokio::test]
async fn the_hub_mirrors_the_desktops_run_log_and_a_row_appended_while_connected() {
    let bridged = Bridged::new(&[
        ("f-a", "feature_status", json!({"status": "running"})),
        ("f-a", "agent_spawned", json!({"step_id": "s-1"})),
        (
            "f-a",
            "step_progress",
            json!({"status": "running", "step_id": "s-1"}),
        ),
        ("f-b", "feature_status", json!({"status": "running"})),
        (
            "f-b",
            "step_progress",
            json!({"status": "verifying", "step_id": "s-9"}),
        ),
        ("f-b", "gate_required", json!({"step_execution_id": "e-1"})),
        (
            "f-a",
            "step_progress",
            json!({"status": "completed", "step_id": "s-1"}),
        ),
        ("f-b", "retry_decision", json!({"step_id": "s-9"})),
    ])
    .await;
    let running = bridged.connect().await;

    bridged.acked_through(7).await;

    let heard = bridged.heard();
    let first: Hello = serde_json::from_str(&heard[0]).expect("a hello first");
    let expected = Hello {
        v: PROTOCOL_VERSION,
        install_id: bridged.db.install_id().expect("an install id"),
        app_version: APP_VERSION.to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        scopes: vec![Scope::Read, Scope::Gates],
    };
    assert_eq!(first, expected);
    let hub = &bridged.dialer.hub;
    let backfill = vec![
        mirrored(
            "f-a",
            1,
            RunEventKind::FeatureStarted,
            RunEventPayload::default(),
        ),
        mirrored("f-a", 3, RunEventKind::StepStarted, step("s-1")),
        mirrored("f-a", 7, RunEventKind::StepCompleted, step("s-1")),
        mirrored(
            "f-b",
            4,
            RunEventKind::FeatureStarted,
            RunEventPayload::default(),
        ),
        mirrored("f-b", 6, RunEventKind::GatePending, gate("e-1")),
    ];
    assert_eq!(sorted(hub.mirror()), sorted(backfill.clone()));
    assert_eq!(hub.cursor(), 7);

    let live = bridged.append(&("f-a", "gate_decided", json!({"step_execution_id": "e-2"})));
    bridged.acked_through(live).await;

    let mut expected = backfill;
    expected.push(mirrored(
        "f-a",
        live,
        RunEventKind::GateDecided,
        gate("e-2"),
    ));
    assert_eq!(sorted(hub.mirror()), sorted(expected));
    assert_eq!(hub.cursor(), live);
    assert_eq!(bridged.dialer.sessions.lock().unwrap().len(), 1);
    assert_eq!(running.stopped().await, (SessionEnd::Stopped, true));
}

#[tokio::test]
async fn an_empty_run_log_still_says_hello_and_is_acked() {
    let bridged = Bridged::new(&[]).await;
    let running = bridged.connect().await;

    bridged.acked_through(0).await;

    let heard = bridged.heard();
    let [only] = &heard[..] else {
        panic!("one frame, a hello: {heard:?}");
    };
    serde_json::from_str::<Hello>(only).expect("a hello");
    assert_eq!(bridged.dialer.hub.mirror(), Vec::new());
    assert_eq!(bridged.dialer.hub.cursor(), 0);
    assert_eq!(running.stopped().await, (SessionEnd::Stopped, true));
}

/// More off-allowlist rows than a page holds, then more events than the
/// unacked window: each page starts after the last row read, and the window
/// reopens as the Hub acks.
#[tokio::test]
async fn a_backlog_longer_than_a_page_and_the_unacked_window_reaches_the_hub() {
    let dropped = PAGE_SIZE + 3;
    let events = MAX_UNACKED_EVENTS + PAGE_SIZE;
    let bridged = Bridged::new(&[]).await;
    for _ in 0..dropped {
        bridged.append(&("f-a", "agent_spawned", json!({"step_id": "s-1"})));
    }
    let mut last = 0;
    for n in 0..events {
        let feature = if n % 2 == 0 { "f-a" } else { "f-b" };
        last = bridged.append(&(
            feature,
            "step_progress",
            json!({"status": "running", "step_id": "s-1"}),
        ));
    }
    let running = bridged.connect().await;

    bridged.acked_through(last).await;

    assert_eq!(bridged.dialer.hub.mirror_len(), events);
    assert_eq!(bridged.dialer.hub.cursor(), last);
    assert_eq!(bridged.heard().len(), events + 1);
    assert_eq!(running.stopped().await, (SessionEnd::Stopped, true));
}

/// The offset of every `RunEvent` among `frames`.
fn event_offsets(frames: &[String]) -> Vec<u64> {
    frames
        .iter()
        .filter_map(|text| serde_json::from_str::<RunEvent>(text).ok())
        .map(|event| event.offset)
        .collect()
}

fn progress() -> Row {
    (
        "f-a",
        "step_progress",
        json!({"status": "running", "step_id": "s-1"}),
    )
}

/// AC3.4: the second connection resumes where the Hub's cursor stands.
#[tokio::test]
async fn a_second_connection_sends_nothing_at_or_below_the_hubs_cursor() {
    let bridged = Bridged::new(&[progress(), progress(), progress()]).await;
    let first = bridged.connect().await;
    bridged.acked_through(3).await;
    assert_eq!(first.stopped().await, (SessionEnd::Stopped, true));
    assert_eq!(bridged.hub_sessions_ended().await, [Ended::PeerGone]);
    let more = [progress(), progress()].map(|row| bridged.append(&row));

    let cursor = bridged.dialer.hub.cursor();
    let before = bridged.heard().len();
    let second = bridged.connect().await;
    bridged.acked_through(more[1]).await;

    assert_eq!(cursor, 3);
    let heard = bridged.heard();
    let on_second = &heard[before..];
    serde_json::from_str::<Hello>(&on_second[0]).expect("a hello first");
    assert_eq!(event_offsets(on_second), more);
    assert!(event_offsets(on_second)
        .iter()
        .all(|&offset| offset > cursor));
    assert_eq!(bridged.dialer.hub.mirror_len(), 5);
    assert_eq!(second.stopped().await, (SessionEnd::Stopped, true));
}

/// The Hub's cursor above every id the desktop holds, as after the
/// desktop's database was reset: nothing below it is sent, and the first row
/// above it is.
#[tokio::test]
async fn a_hub_cursor_above_every_local_id_sends_nothing_older_and_waits() {
    let bridged = Bridged::new(&[progress(), progress()]).await;
    let ahead = 5;
    let mut conn = bridged.dial().await;
    let hello = Hello {
        v: PROTOCOL_VERSION,
        install_id: bridged.db.install_id().expect("an install id"),
        app_version: APP_VERSION.to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        scopes: vec![Scope::Read],
    };
    let earlier = RunEvent {
        v: PROTOCOL_VERSION,
        feature_id: "f-z".to_owned(),
        offset: ahead,
        kind: RunEventKind::FeatureStarted,
        occurred_at: NOW,
        payload: RunEventPayload::default(),
    };
    for frame in [json!(hello), json!(earlier)] {
        conn.send_text(frame.to_string())
            .await
            .expect("the Hub end is there");
    }
    bridged.acked_through(ahead).await;
    drop(conn);
    assert_eq!(bridged.hub_sessions_ended().await, [Ended::PeerGone]);

    let before = bridged.heard().len();
    let running = bridged.connect().await;
    bridged.acked_through(ahead).await;
    let mut last = 0;
    while last <= ahead {
        last = bridged.append(&progress());
    }
    bridged.acked_through(last).await;

    assert_eq!(event_offsets(&bridged.heard()[before..]), [last]);
    assert_eq!(running.stopped().await, (SessionEnd::Stopped, true));
}

/// AC3.5: without `read` the desktop says `hello` and sends no event, from
/// the backlog or live, and still reads the Hub until the Hub goes quiet.
#[tokio::test]
async fn pins_without_read_send_a_hello_and_no_event() {
    let bridged = Bridged::pinned(&[progress(), progress()], &["gates"], &[Scope::Gates]).await;
    let liveness = Duration::from_millis(500);
    let running = bridged
        .connect_with(bridged.context().with_liveness(liveness))
        .await;
    bridged.acked_through(0).await;
    bridged.append(&progress());

    let end = timeout(BOUND, running.task)
        .await
        .expect("the session ends")
        .expect("the session did not panic");

    assert_eq!(end, (SessionEnd::Ended(SocketEnd::Silent), true));
    assert_eq!(bridged.hub_sessions_ended().await, [Ended::PeerGone]);
    let heard = bridged.heard();
    let [only] = &heard[..] else {
        panic!("one frame, a hello: {heard:?}");
    };
    let hello: Hello = serde_json::from_str(only).expect("a hello");
    assert_eq!(hello.scopes, [Scope::Gates]);
    assert_eq!(bridged.dialer.hub.mirror(), Vec::new());
}

/// AC3.6: a queued request is delivered and goes unanswered, and the
/// connection carries on.
#[tokio::test]
async fn a_queued_request_goes_unanswered_and_the_connection_carries_on() {
    let bridged = Bridged::new(&[]).await;
    bridged
        .queue_request("signed_request_gate_decision.json")
        .await;
    let running = bridged.connect().await;
    bridged.acked_through(0).await;

    let live = bridged.append(&("f-a", "gate_decided", json!({"step_execution_id": "e-2"})));
    bridged.acked_through(live).await;

    let [request] = &bridged.dialer.hub.requests()[..] else {
        panic!("one request: {:?}", bridged.dialer.hub.requests());
    };
    assert_eq!(request.deliveries, 1, "the request reached the desktop");
    assert_eq!(request.status, RequestStatus::Queued);
    assert_eq!(request.outcome, None);
    let heard = bridged.heard();
    assert!(
        !heard
            .iter()
            .any(|text| serde_json::from_str::<RequestResult>(text).is_ok()),
        "no result: {heard:?}"
    );
    assert_eq!(
        bridged.dialer.hub.mirror(),
        [mirrored(
            "f-a",
            live,
            RunEventKind::GateDecided,
            gate("e-2")
        )]
    );
    assert!(!bridged.dialer.sessions.lock().unwrap()[0].is_finished());
    assert_eq!(running.stopped().await, (SessionEnd::Stopped, true));
}

/// AC5: a start the Hub queued and signed reaches the desktop's handler,
/// and what the Hub stores is the desktop's answer to it.
#[tokio::test]
async fn a_hub_signed_start_with_spend_off_is_answered_scope_off_and_the_hub_records_it() {
    let bridged = Bridged::pinned(
        &[],
        &["read", "gates", "spend"],
        &[Scope::Read, Scope::Gates, Scope::Spend],
    )
    .await;
    let (requests, desktop) = bridged.handler("start-spend-off", &policy(false, true));
    bridged
        .queue_request("signed_request_start_feature.json")
        .await;
    let running = bridged
        .connect_with(bridged.context().with_requests(requests))
        .await;

    let request = bridged.answered().await;

    let refused = RequestOutcome::Refused {
        reason: RefusalReason::ScopeOff,
    };
    assert_eq!(request.kind, "start_feature");
    assert_eq!(request.deliveries, 1);
    assert_eq!(request.status, RequestStatus::Answered);
    assert_eq!(request.outcome, Some(refused));
    let entry = logged(&desktop).await;
    assert_eq!(entry.request_id, request.envelope.request_id);
    assert_eq!(
        (entry.kind, entry.outcome, entry.reason.as_deref()),
        (
            LogKind::StartFeature,
            LogOutcome::Refused,
            Some("scope_off")
        )
    );
    assert!(!bridged.dialer.sessions.lock().unwrap()[0].is_finished());
    assert_eq!(running.stopped().await, (SessionEnd::Stopped, true));
}

/// A handler that is handed a request and never returns, as a launch or an
/// attachment fetch can take minutes to.
#[derive(Default)]
struct StuckHandler {
    handed: Notify,
}

#[async_trait::async_trait]
impl HubRequestHandler for StuckHandler {
    async fn handle(&self, _text: &str) -> Option<RequestResult> {
        self.handed.notify_one();
        std::future::pending().await
    }
}

/// The session never waits on a request: with its handler stuck, a row
/// appended afterwards still reaches the Hub and is acked.
#[tokio::test]
async fn a_request_still_being_handled_does_not_hold_up_the_session() {
    let bridged = Bridged::new(&[]).await;
    let stuck = Arc::new(StuckHandler::default());
    let requests = RequestHandlerSlot::default();
    assert!(requests.set(stuck.clone()).is_ok());
    bridged
        .queue_request("signed_request_gate_decision.json")
        .await;
    let running = bridged
        .connect_with(bridged.context().with_requests(requests))
        .await;
    timeout(BOUND, stuck.handed.notified())
        .await
        .expect("the request reached the handler");

    let live = bridged.append(&progress());
    bridged.acked_through(live).await;

    assert_eq!(event_offsets(&bridged.heard()), [live]);
    assert_eq!(running.stopped().await, (SessionEnd::Stopped, true));
}

/// A gate decision the desktop refuses is answered like any other request:
/// the Hub records the outcome, and the connection carries on. The fixture
/// names a step this scratch desktop does not have.
#[tokio::test]
async fn a_refused_gate_decision_is_answered_and_the_hub_records_it_on_a_live_connection() {
    let bridged = Bridged::new(&[]).await;
    let (requests, desktop) = bridged.handler("gate-refused", &policy(false, true));
    bridged
        .queue_request("signed_request_gate_decision.json")
        .await;
    let running = bridged
        .connect_with(bridged.context().with_requests(requests))
        .await;

    let request = bridged.answered().await;
    let live = bridged.append(&("f-a", "gate_decided", json!({"step_execution_id": "e-2"})));
    bridged.acked_through(live).await;

    let refused = RequestOutcome::Refused {
        reason: RefusalReason::UnknownStep,
    };
    assert_eq!(request.kind, "gate_decision");
    assert_eq!(request.deliveries, 1);
    assert_eq!(request.status, RequestStatus::Answered);
    assert_eq!(request.outcome, Some(refused));
    let entry = logged(&desktop).await;
    assert_eq!(entry.request_id, request.envelope.request_id);
    assert_eq!(
        (entry.kind, entry.outcome, entry.reason.as_deref()),
        (
            LogKind::GateDecision,
            LogOutcome::Refused,
            Some("unknown_step")
        )
    );
    assert!(!bridged.dialer.sessions.lock().unwrap()[0].is_finished());
    assert_eq!(running.stopped().await, (SessionEnd::Stopped, true));
}

/// Short enough to wait out in a test, and long enough for every send a
/// session has room for to finish well inside it.
const SHORT_LIVENESS: Duration = Duration::from_millis(500);

fn cursor_ack(offset: u64) -> SocketReceived {
    let ack = CursorAck {
        v: PROTOCOL_VERSION,
        offset,
    };
    SocketReceived::Text(serde_json::to_string(&ack).expect("a serialisable ack"))
}

/// A session context pinned `read`, reading `run_events`, paired with no
/// Hub.
fn scripted_context(run_events: Arc<dyn RunEventsPort>, wake: Arc<Notify>) -> SessionContext {
    snapshot_context(run_events, wake, Arc::default())
}

/// [`scripted_context`], its snapshots built by `snapshots`.
fn snapshot_context(
    run_events: Arc<dyn RunEventsPort>,
    wake: Arc<Notify>,
    snapshots: Arc<ScriptedSnapshotSource>,
) -> SessionContext {
    SessionContext::new(
        "install-1".to_owned(),
        APP_VERSION.to_owned(),
        vec!["read".to_owned()],
        run_events,
        wake,
        snapshots,
        Arc::new(Notify::new()),
    )
}

/// A context over a run log that fails if the session reads it.
fn unreadable_context() -> (SessionContext, Arc<UnreadableRunEvents>) {
    let run_events = Arc::new(UnreadableRunEvents::default());
    let ctx = scripted_context(run_events.clone(), Arc::new(Notify::new()));
    (ctx, run_events)
}

/// Runs the desktop's session on `conn` until it ends of its own accord.
async fn scripted_session(conn: ScriptedConnection, ctx: &SessionContext) -> (SessionEnd, bool) {
    let (_stop, mut stopped) = watch::channel(false);
    timeout(BOUND, run_session(Box::new(conn), ctx, &mut stopped))
        .await
        .expect("the session ends")
}

/// A send the transport refuses is a connection already gone: nothing is
/// closed, and the Hub is not waited for.
#[tokio::test]
async fn a_hello_the_socket_refuses_ends_closed_without_a_close() {
    let (ctx, run_events) = unreadable_context();
    let conn = ScriptedConnection::new([cursor_ack(0)]).refusing_sends();
    let log = conn.log();

    let end = scripted_session(conn, &ctx).await;

    assert_eq!(end, (SessionEnd::Ended(SocketEnd::Closed(None)), false));
    assert_eq!(log.closed_with(), None);
    assert_eq!(run_events.reads(), 0);
}

#[tokio::test]
async fn a_hello_still_unsent_at_the_liveness_deadline_ends_silent() {
    let (ctx, run_events) = unreadable_context();
    let ctx = ctx.with_liveness(SHORT_LIVENESS);
    let conn = ScriptedConnection::new([cursor_ack(0)]).stalling_sends();
    let log = conn.log();

    let end = scripted_session(conn, &ctx).await;

    assert_eq!(end, (SessionEnd::Ended(SocketEnd::Silent), false));
    assert_eq!(log.closed_with(), None);
    assert_eq!(run_events.reads(), 0);
}

/// The Hub acks the `Hello` and then says nothing: whether the desktop has
/// sent everything or is held at a full unacked window, the deadline ends
/// the session.
#[tokio::test]
async fn a_hub_silent_after_its_first_ack_ends_the_session_silent() {
    for (tag, events, sent) in [
        ("caught_up", 2, 2),
        ("window_full", MAX_UNACKED_EVENTS + 1, MAX_UNACKED_EVENTS),
    ] {
        let (run_events, _) = progress_log(events);
        let ctx =
            scripted_context(run_events.clone(), run_events.wake()).with_liveness(SHORT_LIVENESS);
        let conn = ScriptedConnection::new([cursor_ack(0)]).then_silent();
        let log = conn.log();

        let end = scripted_session(conn, &ctx).await;

        assert_eq!(end, (SessionEnd::Ended(SocketEnd::Silent), true), "{tag}");
        let frames = log.sent();
        serde_json::from_str::<Hello>(&frames[0]).expect("a hello first");
        let offsets: Vec<u64> = (1..=sent as u64).collect();
        assert_eq!(event_offsets(&frames[1..]), offsets, "{tag}");
        assert_eq!(frames.len(), sent + 1, "{tag}");
        assert_eq!(log.closed_with(), None, "{tag}");
        assert_eq!(log.unscripted(), 0, "{tag}");
    }
}

/// A scratch run log holding `count` allowlisted rows, and their offsets.
fn progress_log(count: usize) -> (Arc<WakingRunEvents>, Vec<u64>) {
    let db = Arc::new(
        SqliteAdapter::new(rusqlite::Connection::open_in_memory().expect("a scratch DB"))
            .expect("a migrated DB"),
    );
    let run_events = Arc::new(WakingRunEvents::new(db));
    let offsets = (0..count)
        .map(|_| {
            let (feature, kind, payload) = progress();
            let offset = run_events
                .append(feature, kind, Some(&payload.to_string()), WRITTEN_AT_MS)
                .expect("an appended row");
            u64::try_from(offset).expect("a positive id")
        })
        .collect();
    (run_events, offsets)
}

/// Long enough for a session with no window to send every row it holds.
const SETTLE: Duration = Duration::from_millis(200);

/// Waits for `count` `RunEvent` frames past the `Hello`, then for the
/// session to have had time to send more, and returns what was sent.
async fn sent_once_settled(log: &ConnectionLog, count: usize) -> Vec<u64> {
    timeout(BOUND, log.sent_times(count + 1))
        .await
        .unwrap_or_else(|_| panic!("{count} events sent within {BOUND:?}"));
    tokio::time::sleep(SETTLE).await;
    event_offsets(&log.sent())
}

/// The most `RunEvent` frames ever sent and not yet acked, replayed in the
/// order the session met sends and acks.
fn most_in_flight(exchanges: &[Exchange]) -> usize {
    let mut acked = 0;
    let mut sent = Vec::new();
    let mut most = 0;
    for exchange in exchanges {
        match exchange {
            Exchange::Sent(text) => {
                if let Ok(event) = serde_json::from_str::<RunEvent>(text) {
                    sent.push(event.offset);
                }
            }
            Exchange::Received(SocketReceived::Text(text)) => {
                if let Ok(ack) = serde_json::from_str::<CursorAck>(text) {
                    acked = acked.max(ack.offset);
                }
            }
            Exchange::Received(_) => {}
        }
        most = most.max(sent.iter().filter(|&&offset| offset > acked).count());
    }
    most
}

/// Sending stops at a full window however long the Hub keeps quiet, and an
/// ack for the k-th frame sent frees exactly k more, or what is left.
#[tokio::test]
async fn sending_pauses_at_the_unacked_window_and_an_ack_frees_what_it_acks() {
    let freed = 3;
    let rest = 10;
    let (run_events, offsets) = progress_log(MAX_UNACKED_EVENTS + rest);
    let ctx = scripted_context(run_events.clone(), run_events.wake());
    let (conn, release) = ScriptedConnection::new([cursor_ack(0)]).then_held();
    let log = conn.log();
    let (stop, mut stopped) = watch::channel(false);
    let task = tokio::spawn(async move { run_session(Box::new(conn), &ctx, &mut stopped).await });

    let window = sent_once_settled(&log, MAX_UNACKED_EVENTS).await;
    assert_eq!(window, offsets[..MAX_UNACKED_EVENTS], "the window holds");

    release
        .send(cursor_ack(offsets[freed - 1]))
        .expect("the session reads");
    let reopened = sent_once_settled(&log, MAX_UNACKED_EVENTS + freed).await;
    assert_eq!(reopened, offsets[..MAX_UNACKED_EVENTS + freed], "k freed");

    release
        .send(cursor_ack(offsets[MAX_UNACKED_EVENTS + freed - 1]))
        .expect("the session reads");
    let drained = sent_once_settled(&log, offsets.len()).await;
    assert_eq!(drained, offsets, "the remainder");

    assert_eq!(most_in_flight(&log.exchanges()), MAX_UNACKED_EVENTS);
    stop.send_replace(true);
    let end = timeout(BOUND, task)
        .await
        .expect("the session stops")
        .expect("the session did not panic");
    assert_eq!(end, (SessionEnd::Stopped, true));
    assert_eq!(log.unscripted(), 0);
}

/// A run log the desktop cannot read is its own failure, so it closes
/// `1011` and leaves the Hub to expect it back.
#[tokio::test]
async fn an_unreadable_run_log_closes_the_session_1011() {
    let (ctx, run_events) = unreadable_context();
    let conn = ScriptedConnection::new([cursor_ack(0)]).then_silent();
    let log = conn.log();

    let end = scripted_session(conn, &ctx).await;

    assert_eq!(
        end,
        (SessionEnd::Ended(SocketEnd::Closed(Some(1011))), true)
    );
    assert_eq!(log.closed_with(), Some(1011));
    assert_eq!(run_events.reads(), 1);
    let frames = log.sent();
    let [only] = &frames[..] else {
        panic!("one frame, a hello: {frames:?}");
    };
    serde_json::from_str::<Hello>(only).expect("a hello");
}

#[test]
fn a_context_debounces_snapshots_by_the_default_until_told_otherwise() {
    let ctx = scripted_context(
        Arc::new(UnreadableRunEvents::default()),
        Arc::new(Notify::new()),
    );
    assert_eq!(
        ctx.snapshot_debounce,
        Duration::from_secs(SNAPSHOT_DEBOUNCE_SECS)
    );
    let ctx = ctx.with_snapshot_debounce(SHORT_LIVENESS);
    assert_eq!(ctx.snapshot_debounce, SHORT_LIVENESS);
}

/// A snapshot holding one Project, named `name`.
fn one_project(name: &str) -> Snapshot {
    Snapshot {
        v: PROTOCOL_VERSION,
        projects: vec![ProjectSnapshot {
            id: "p-1".to_owned(),
            name: name.to_owned(),
            remote_url: None,
        }],
        workflows: Vec::new(),
        run_shape: Vec::new(),
    }
}

/// Every `Snapshot` among `frames`.
fn snapshots_in(frames: &[String]) -> Vec<Snapshot> {
    frames
        .iter()
        .filter_map(|text| serde_json::from_str::<Snapshot>(text).ok())
        .collect()
}

/// AC4: nothing is built while the Hub has yet to ack, and one snapshot is
/// sent once it has.
#[tokio::test]
async fn a_snapshot_follows_the_first_cursor_ack() {
    let (run_events, _) = progress_log(0);
    let snapshots = Arc::new(ScriptedSnapshotSource::default());
    snapshots.set(one_project("alpha"));
    let ctx = snapshot_context(run_events.clone(), run_events.wake(), snapshots.clone());
    let snapshot_wake = ctx.snapshot_wake.clone();
    let (conn, release) = ScriptedConnection::new([]).then_held();
    let log = conn.log();
    let (stop, mut stopped) = watch::channel(false);
    let task = tokio::spawn(async move { run_session(Box::new(conn), &ctx, &mut stopped).await });

    timeout(BOUND, log.sent_times(1))
        .await
        .expect("a hello sent");
    snapshot_wake.notify_one();
    tokio::time::sleep(SETTLE).await;
    let frames = log.sent();
    let [hello] = &frames[..] else {
        panic!("one frame before the ack, a hello: {frames:?}");
    };
    serde_json::from_str::<Hello>(hello).expect("a hello");
    assert_eq!(snapshots.calls(), 0, "nothing built before the ack");

    release.send(cursor_ack(0)).expect("the session reads");
    timeout(BOUND, log.sent_times(2))
        .await
        .expect("a snapshot sent");
    tokio::time::sleep(SETTLE).await;

    let frames = log.sent();
    assert_eq!(frames.len(), 2, "{frames:?}");
    assert_eq!(snapshots_in(&frames[1..]), [one_project("alpha")]);
    assert_eq!(snapshots.calls(), 1);
    stop.send_replace(true);
    let end = timeout(BOUND, task)
        .await
        .expect("the session stops")
        .expect("the session did not panic");
    assert_eq!(end, (SessionEnd::Stopped, true));
    assert_eq!(log.unscripted(), 0);
    assert_eq!(snapshots.unscripted(), 0);
}

/// AC4: without `read` nothing is built, acked or woken, even by an edit
/// made while connected.
#[tokio::test]
async fn pins_without_read_send_no_snapshot() {
    let (run_events, _) = progress_log(0);
    let snapshots = Arc::new(ScriptedSnapshotSource::default());
    snapshots.set(one_project("alpha"));
    let (_, workflows) = workflow_db();
    let mut ctx = snapshot_context(run_events.clone(), run_events.wake(), snapshots.clone())
        .with_liveness(SHORT_LIVENESS)
        .with_snapshot_debounce(SHORT_LIVENESS / 5);
    ctx.pinned_scopes = vec!["gates".to_owned()];
    ctx.snapshot_wake = workflows.wake();
    ctx.snapshot_wake.notify_one();
    let conn = ScriptedConnection::new([cursor_ack(0)]).then_silent();
    let log = conn.log();
    let session = tokio::spawn(async move { scripted_session(conn, &ctx).await });
    timeout(BOUND, log.sent_times(1))
        .await
        .expect("a hello sent");

    workflows
        .save_version(workflow_version(2))
        .expect("version saved");
    let end = session.await.expect("the session did not panic");

    assert_eq!(end, (SessionEnd::Ended(SocketEnd::Silent), true));
    let frames = log.sent();
    let [only] = &frames[..] else {
        panic!("one frame, a hello: {frames:?}");
    };
    let hello: Hello = serde_json::from_str(only).expect("a hello");
    assert_eq!(hello.scopes, [Scope::Gates]);
    assert_eq!(snapshots.calls(), 0);
    assert_eq!(log.unscripted(), 0);
}

/// AC4: the Hub's own session takes the frame as a snapshot, after the
/// `Hello`, and stores it without refusing the connection.
#[tokio::test]
async fn a_snapshot_reaches_the_hub_and_is_stored() {
    let bridged = Bridged::new(&[]).await;
    let snapshots = Arc::new(ScriptedSnapshotSource::default());
    snapshots.set(one_project("alpha"));
    let mut ctx = bridged.context();
    ctx.snapshots = snapshots.clone();
    let running = bridged.connect_with(ctx).await;
    bridged.acked_through(0).await;
    // The Hub answers a snapshot with nothing; it reads in order, so the ack
    // of a row sent after it is the proof it was stored.
    let live = bridged.append(&progress());
    bridged.acked_through(live).await;

    let stored = bridged.dialer.hub.snapshot().expect("a stored snapshot");
    let stored: Snapshot = serde_json::from_str(&stored.body).expect("a snapshot body");
    assert_eq!(stored, one_project("alpha"));
    assert!(!bridged.dialer.sessions.lock().unwrap()[0].is_finished());
    assert_eq!(running.stopped().await, (SessionEnd::Stopped, true));
    assert_eq!(bridged.hub_sessions_ended().await, [Ended::PeerGone]);
    assert_eq!(snapshots.unscripted(), 0);
}

/// A snapshot that cannot be built is not the run log: the session carries
/// on, backlog and live rows alike (docs/HUB.md §5 *Snapshots*).
#[tokio::test]
async fn a_failing_snapshot_source_leaves_the_session_relaying_events() {
    let (run_events, offsets) = progress_log(2);
    let snapshots = Arc::new(ScriptedSnapshotSource::default());
    snapshots.fail("the database is locked");
    let ctx = snapshot_context(run_events.clone(), run_events.wake(), snapshots.clone());
    let (conn, _release) = ScriptedConnection::new([cursor_ack(0)]).then_held();
    let log = conn.log();
    let (stop, mut stopped) = watch::channel(false);
    let task = tokio::spawn(async move { run_session(Box::new(conn), &ctx, &mut stopped).await });

    assert_eq!(sent_once_settled(&log, offsets.len()).await, offsets);
    let (feature, kind, payload) = progress();
    let live = run_events
        .append(feature, kind, Some(&payload.to_string()), WRITTEN_AT_MS)
        .expect("an appended row");
    let live = u64::try_from(live).expect("a positive id");
    let mut expected = offsets;
    expected.push(live);
    assert_eq!(sent_once_settled(&log, expected.len()).await, expected);

    assert_eq!(snapshots.calls(), 1);
    assert!(snapshots_in(&log.sent()).is_empty());
    assert_eq!(log.closed_with(), None);
    stop.send_replace(true);
    let end = timeout(BOUND, task)
        .await
        .expect("the session stops")
        .expect("the session did not panic");
    assert_eq!(end, (SessionEnd::Stopped, true));
    assert_eq!(log.unscripted(), 0);
}

/// One the Hub would refuse is not sent, and the connection carries on.
#[tokio::test]
async fn a_snapshot_over_the_hubs_frame_limit_is_not_sent() {
    let (run_events, _) = progress_log(0);
    let snapshots = Arc::new(ScriptedSnapshotSource::default());
    snapshots.set(one_project(&"a".repeat(MAX_SNAPSHOT_BYTES)));
    let ctx = snapshot_context(run_events.clone(), run_events.wake(), snapshots.clone())
        .with_liveness(SHORT_LIVENESS);
    let conn = ScriptedConnection::new([cursor_ack(0)]).then_silent();
    let log = conn.log();

    let end = scripted_session(conn, &ctx).await;

    assert_eq!(end, (SessionEnd::Ended(SocketEnd::Silent), true));
    let frames = log.sent();
    let [only] = &frames[..] else {
        panic!("one frame, a hello: {frames:?}");
    };
    serde_json::from_str::<Hello>(only).expect("a hello");
    assert_eq!(snapshots.calls(), 1);
    assert_eq!(log.closed_with(), None);
}

/// Short enough to wait out, and long enough that a burst of edits spaced
/// [`EDIT_GAP`] apart all land inside one window.
const DEBOUNCE: Duration = Duration::from_millis(300);
const EDIT_GAP: Duration = Duration::from_millis(50);

fn workflow_version(version: u32) -> WorkflowVersion {
    WorkflowVersion {
        id: format!("wf-1-v{version}").as_str().into(),
        workflow_id: "wf-1".into(),
        version,
        steps_json: "[]".to_owned(),
        definition_json: None,
        note: None,
        created_at: 0,
    }
}

/// A scratch DB holding workflow `wf-1` at version 1, and its workflows
/// behind the production decorator that wakes the snapshot.
fn workflow_db() -> (Arc<SqliteAdapter>, Arc<WakingWorkflows>) {
    let db = Arc::new(
        SqliteAdapter::new(rusqlite::Connection::open_in_memory().expect("a scratch DB"))
            .expect("a migrated DB"),
    );
    let workflows = Arc::new(WakingWorkflows::new(db.clone()));
    workflows
        .create(Workflow {
            id: "wf-1".into(),
            name: "Ship".to_owned(),
            description: String::new(),
            is_starter: false,
            created_at: 0,
            updated_at: 0,
            schedule: None,
        })
        .expect("workflow saved");
    workflows
        .save_version(workflow_version(1))
        .expect("version saved");
    (db, workflows)
}

/// The versions of `wf-1` in each snapshot among `frames`.
fn versions_in(frames: &[String]) -> Vec<Vec<u32>> {
    snapshots_in(frames)
        .iter()
        .map(|snapshot| {
            snapshot
                .workflows
                .iter()
                .filter(|workflow| workflow.workflow_id == "wf-1")
                .map(|workflow| workflow.version)
                .collect()
        })
        .collect()
}

/// A session pinned `read` whose snapshots are built by the production
/// source over the DB `workflows` writes to, and woken by its writes.
struct Edited {
    workflows: Arc<WakingWorkflows>,
    log: Arc<ConnectionLog>,
    stop: watch::Sender<bool>,
    task: JoinHandle<(SessionEnd, bool)>,
    _release: mpsc::UnboundedSender<SocketReceived>,
}

impl Edited {
    /// Connected, acked, and past its first snapshot, which holds version 1.
    async fn new() -> Self {
        let (db, workflows) = workflow_db();
        let (run_events, _) = progress_log(0);
        let mut ctx = scripted_context(run_events.clone(), run_events.wake())
            .with_snapshot_debounce(DEBOUNCE);
        ctx.snapshots = Arc::new(RepoSnapshotSource::new(db.clone(), db.clone(), db));
        ctx.snapshot_wake = workflows.wake();
        let (conn, release) = ScriptedConnection::new([cursor_ack(0)]).then_held();
        let log = conn.log();
        let (stop, mut stopped) = watch::channel(false);
        let task =
            tokio::spawn(async move { run_session(Box::new(conn), &ctx, &mut stopped).await });
        timeout(BOUND, log.sent_times(2))
            .await
            .expect("a first snapshot sent");
        assert_eq!(versions_in(&log.sent()), [[1]]);
        Self {
            workflows,
            log,
            stop,
            task,
            _release: release,
        }
    }

    async fn stopped(self) {
        self.stop.send_replace(true);
        let end = timeout(BOUND, self.task)
            .await
            .expect("the session stops")
            .expect("the session did not panic");
        assert_eq!(end, (SessionEnd::Stopped, true));
        assert_eq!(self.log.closed_with(), Some(1001));
        assert_eq!(self.log.unscripted(), 0);
    }
}

/// AC3: the edit reaches the Hub once the window from it has passed, and
/// not before.
#[tokio::test]
async fn editing_a_workflow_sends_a_new_snapshot_within_the_debounce_window() {
    let edited = Edited::new().await;
    let edited_at = Instant::now();

    edited
        .workflows
        .save_version(workflow_version(2))
        .expect("version saved");
    timeout(BOUND, edited.log.sent_times(3))
        .await
        .expect("a second snapshot sent");

    let waited = edited_at.elapsed();
    assert!(waited >= DEBOUNCE, "sent after {waited:?}");
    assert!(waited <= DEBOUNCE + SETTLE, "sent after {waited:?}");
    assert_eq!(versions_in(&edited.log.sent()), [[1], [2]]);
    edited.stopped().await;
}

/// AC4: the window is fixed from the first edit, so edits inside it are
/// sent together, as the latest of them.
#[tokio::test]
async fn a_burst_of_workflow_edits_sends_one_snapshot() {
    let edited = Edited::new().await;

    for version in 2..=4 {
        edited
            .workflows
            .save_version(workflow_version(version))
            .expect("version saved");
        tokio::time::sleep(EDIT_GAP).await;
    }
    timeout(BOUND, edited.log.sent_times(3))
        .await
        .expect("a second snapshot sent");
    tokio::time::sleep(DEBOUNCE + SETTLE).await;

    assert_eq!(versions_in(&edited.log.sent()), [[1], [4]]);
    edited.stopped().await;
}

/// A wake whose rebuild equals what this connection last sent is not sent.
#[tokio::test]
async fn a_wake_that_changes_nothing_sends_no_second_snapshot() {
    let (run_events, _) = progress_log(0);
    let snapshots = Arc::new(ScriptedSnapshotSource::default());
    snapshots.set(one_project("alpha"));
    let ctx = snapshot_context(run_events.clone(), run_events.wake(), snapshots.clone())
        .with_snapshot_debounce(DEBOUNCE);
    let snapshot_wake = ctx.snapshot_wake.clone();
    let (conn, _release) = ScriptedConnection::new([cursor_ack(0)]).then_held();
    let log = conn.log();
    let (stop, mut stopped) = watch::channel(false);
    let task = tokio::spawn(async move { run_session(Box::new(conn), &ctx, &mut stopped).await });
    timeout(BOUND, log.sent_times(2))
        .await
        .expect("a first snapshot sent");

    snapshot_wake.notify_one();
    tokio::time::sleep(DEBOUNCE + SETTLE).await;

    assert_eq!(snapshots.calls(), 2, "rebuilt once for the wake");
    assert_eq!(snapshots_in(&log.sent()), [one_project("alpha")]);
    assert_eq!(log.sent().len(), 2);
    stop.send_replace(true);
    let end = timeout(BOUND, task)
        .await
        .expect("the session stops")
        .expect("the session did not panic");
    assert_eq!(end, (SessionEnd::Stopped, true));
    assert_eq!(snapshots.unscripted(), 0);
}

/// `pair` awaits a stopped session under the refresh lock, so a Hub that
/// never takes the `1001` holds a re-pair only for the stop-close deadline,
/// not the liveness one this context keeps at its default.
#[tokio::test]
async fn a_stopped_session_whose_close_never_completes_ends_within_the_stop_close_deadline() {
    let (run_events, _) = progress_log(0);
    let ctx = scripted_context(run_events.clone(), run_events.wake());
    assert_eq!(ctx.liveness, Duration::from_secs(LIVENESS_DEADLINE_SECS));
    assert_eq!(
        ctx.stop_close,
        Duration::from_secs(STOP_CLOSE_DEADLINE_SECS)
    );
    assert!(ctx.stop_close <= BOUND && ctx.stop_close < ctx.liveness);
    let ctx = ctx.with_stop_close(SHORT_LIVENESS);
    let conn = ScriptedConnection::new([cursor_ack(0)])
        .then_silent()
        .stalling_close();
    let log = conn.log();
    let (stop, mut stopped) = watch::channel(false);
    let task = tokio::spawn(async move { run_session(Box::new(conn), &ctx, &mut stopped).await });
    timeout(BOUND, log.sent_times(1))
        .await
        .expect("a hello sent");

    stop.send_replace(true);
    let (end, _) = timeout(BOUND, task)
        .await
        .expect("the session stops without waiting out its liveness")
        .expect("the session did not panic");

    assert_eq!(end, SessionEnd::Stopped);
    assert_eq!(log.closed_with(), Some(1001));
}

/// Records every wait it is asked for and waits only for its turn on the
/// runtime, so a supervisor that never stops still lets a test's timeout
/// fire.
#[derive(Default)]
struct NoSleep(Mutex<Vec<Duration>>);

#[async_trait::async_trait]
impl Sleeper for NoSleep {
    async fn sleep(&self, duration: Duration) {
        self.0.lock().unwrap().push(duration);
        tokio::task::yield_now().await;
    }
}

/// A desktop paired with no Hub but a scripted `/token` and `/socket`: its
/// pins, device key and refresh token stored, and no access token held, so
/// the supervisor's first act is a refresh.
struct Supervised {
    db: Arc<SqliteAdapter>,
    origin: String,
    key: DeviceKey,
    secrets: Arc<MemorySecretStore>,
    pairing: Arc<HubPairing>,
    endpoint: Arc<ScriptedRefreshEndpoint>,
    dialer: Arc<ScriptedSocketDialer>,
    sleeper: Arc<NoSleep>,
}

impl Supervised {
    fn new() -> Self {
        let db = Arc::new(
            SqliteAdapter::new(rusqlite::Connection::open_in_memory().expect("a scratch DB"))
                .expect("a migrated DB"),
        );
        let origin = hub_origin(HUB_URL).expect("a valid Hub URL");
        let key = DeviceKey::generate().expect("a P-256 key");
        let hub_pins = HubPins {
            hub_origin: origin.clone(),
            instance_id: "inst-1".to_owned(),
            request_signing_key: "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo".to_owned(),
            rp_id: "hub.example".to_owned(),
            scopes: vec!["read".to_owned()],
            device_jkt: key.jkt(),
            paired_at: NOW as i64,
            passkeys: Vec::new(),
        };
        let paired = FreshPair::generate().expect("the system random source");
        let secrets = Arc::new(
            MemorySecretStore::paired(&*db, &hub_pins, &key, &paired.refresh.text())
                .expect("a seeded store"),
        );
        let endpoint = Arc::new(ScriptedRefreshEndpoint::new());
        let pairing = HubPairing::new(db.clone(), secrets.clone(), Arc::new(NoRedemption))
            .with_refresh_endpoint(endpoint.clone());
        Self {
            db,
            origin,
            key,
            secrets,
            pairing: Arc::new(pairing),
            endpoint,
            dialer: Arc::new(ScriptedSocketDialer::new()),
            sleeper: Arc::default(),
        }
    }

    /// Scripts one rotation and returns it.
    fn rotation(&self) -> FreshPair {
        self.rotation_expiring_in(900)
    }

    /// Scripts one rotation whose access token lives `expires_in` seconds.
    fn rotation_expiring_in(&self, expires_in: u64) -> FreshPair {
        let next = FreshPair::generate().expect("the system random source");
        self.endpoint
            .answer(&next.access.text(), &next.refresh.text(), expires_in);
        next
    }

    fn closing_with(&self, code: u16) {
        self.dialer
            .connect(ScriptedConnection::new([SocketReceived::Closed(Some(
                code,
            ))]));
    }

    fn stored_refresh(&self) -> Option<String> {
        let install_id = self.db.install_id().expect("an install id");
        self.secrets
            .get(&hub_keyring_account(&install_id, HubSecret::RefreshToken))
            .expect("a readable store")
    }

    /// Supervises on its own task, as the app does, until it stops of its
    /// own accord.
    async fn run(&self) -> StopReason {
        let run_events = Arc::new(WakingRunEvents::new(self.db.clone()));
        let deps = SupervisorDeps {
            tokens: self.pairing.clone(),
            origin: self.origin.clone(),
            key: DeviceKey::from_pkcs8(&self.key.to_pkcs8().expect("a PKCS#8 key"))
                .expect("the same key"),
            dialer: self.dialer.clone(),
            session: SessionContext::new(
                self.db.install_id().expect("an install id"),
                APP_VERSION.to_owned(),
                vec!["read".to_owned()],
                run_events.clone(),
                run_events.wake(),
                Arc::new(ScriptedSnapshotSource::default()),
                Arc::new(Notify::new()),
            ),
            sleeper: self.sleeper.clone(),
            jitter: Box::new(|| 1.0),
        };
        let (_stop, stopped) = watch::channel(false);
        let reason = timeout(BOUND, tokio::spawn(supervise(deps, stopped)))
            .await
            .expect("the supervisor stops")
            .expect("the supervisor did not panic");
        assert_eq!(self.dialer.unscripted(), 0, "an unscripted dial");
        assert_eq!(self.endpoint.unscripted(), 0, "an unscripted refresh");
        reason
    }

    fn waits(&self) -> Vec<Duration> {
        self.sleeper.0.lock().unwrap().clone()
    }
}

/// AC4: a displaced desktop never redials, or it would displace its
/// successor in turn.
#[tokio::test]
async fn a_socket_closed_4002_stops_displaced_and_never_redials() {
    let supervised = Supervised::new();
    let issued = supervised.rotation();
    supervised.closing_with(4002);

    assert_eq!(supervised.run().await, StopReason::Displaced);

    let upgrades = supervised.dialer.upgrades();
    let [only] = &upgrades[..] else {
        panic!("one dial, got {upgrades:?}");
    };
    assert_eq!(only.access_token.expose(), issued.access.text());
    assert_eq!(supervised.endpoint.calls().len(), 1);
    assert_eq!(supervised.waits(), Vec::<Duration>::new());
}

/// AC4: an expired access token is refreshed once and the redial, with no
/// backoff, presents the token that refresh issued. The first token's local
/// expiry is within the margin when the `4001` comes, as at a real expiry.
#[tokio::test]
async fn a_socket_closed_4001_refreshes_once_and_redials_with_the_new_token() {
    let supervised = Supervised::new();
    let first = supervised.rotation_expiring_in(PREMATURE_EXPIRY_MARGIN_SECS / 2);
    let second = supervised.rotation();
    supervised.closing_with(4001);
    supervised.closing_with(4002);

    assert_eq!(supervised.run().await, StopReason::Displaced);

    let calls = supervised.endpoint.calls();
    assert_eq!(calls.len(), 2, "one refresh to dial, one after the 4001");
    assert_eq!(
        form_pairs(&calls[1].form_body)[1],
        ("refresh_token".to_owned(), first.refresh.text())
    );
    let presented: Vec<String> = supervised
        .dialer
        .upgrades()
        .iter()
        .map(|upgrade| upgrade.access_token.expose().to_owned())
        .collect();
    assert_eq!(presented, [first.access.text(), second.access.text()]);
    assert_eq!(supervised.waits(), Vec::<Duration>::new());
}

/// AC5.2: every dial happens with the rotation that preceded it already in
/// the keyring.
#[tokio::test]
async fn the_rotated_refresh_token_is_stored_before_the_dial() {
    let supervised = Arc::new(Supervised::new());
    let first = supervised.rotation_expiring_in(PREMATURE_EXPIRY_MARGIN_SECS / 2);
    let second = supervised.rotation();
    supervised.closing_with(4001);
    supervised.closing_with(4002);
    let at_dial = Arc::new(Mutex::new(Vec::new()));
    let (seen, store) = (at_dial.clone(), supervised.clone());
    supervised
        .dialer
        .on_dial(move |_| seen.lock().unwrap().push(store.stored_refresh()));

    assert_eq!(supervised.run().await, StopReason::Displaced);

    assert_eq!(
        *at_dial.lock().unwrap(),
        [Some(first.refresh.text()), Some(second.refresh.text())]
    );
}

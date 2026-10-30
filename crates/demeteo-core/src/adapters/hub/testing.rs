//! Doubles of the Hub ports for tests, each refusing whatever it was not set
//! up to accept.
//!
//! [`MemorySecretStore`] never touches the real OS keyring (CI has no Secret
//! Service). It refuses every account but the two [`hub_keyring_account`]
//! derives for its install id, so a secret written under any other name fails
//! the test instead of landing quietly in the map.
//!
//! [`ScriptedRefreshEndpoint`], [`ScriptedDeviceEndpoint`] and
//! [`ScriptedSocketDialer`] answer only from
//! their scripts, and a [`ScriptedConnection`] closes when its script runs out
//! rather than waiting for a frame nobody will send, unless the test scripted
//! that silence or holds the frames back to release them itself. Each counts
//! what it answered past its script, for a test to assert is `0`.
//!
//! [`ScriptedAttachments`] hands each staged attachment over once, as the
//! Hub does, and refuses every other fetch.
//!
//! [`UnreadableRunEvents`] fails every call, so nothing it is asked can pass
//! as a run log that read back empty.
//!
//! [`ScriptedSnapshotSource`] answers from its script and then from the
//! snapshot a test set, and with an `Err` once it has neither, so no session
//! sends a snapshot the test did not choose.
//!
//! [`ScriptedGatePresenter`] accepts the one gate decision it was scripted
//! with, once, and refuses everything else with an `Err` — other arguments, a
//! repeat, and every `gate_pending_for_run` — counting each for a test to
//! assert is `0`.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use demeteo_hub_protocol::{AttachmentRef, Snapshot};
use tokio::sync::{mpsc, watch, Notify};

use crate::domain::hub_pairing::device::{DeviceAuthorization, DeviceGrantEnd};
use crate::domain::hub_pairing::keyring_names::hub_keyring_account;
use crate::domain::hub_pairing::{
    HubPairingError, HubPins, HubSecret, KeyringStatus, PairingAnswer, RefreshAnswer,
};
use crate::domain::hub_socket::RefreshEnd;
use crate::domain::models::GateDecision;
use crate::error::AppError;
use crate::ports::hub::{
    AttachmentFetchError, DialFailure, HubAttachmentFetcher, HubDeviceEndpoint, HubPinRepository,
    HubRefreshEndpoint, HubSecretStore, HubSnapshotSource, HubSocketConnection, HubSocketDialer,
    SocketGone, SocketReceived, SocketUpgrade,
};
use crate::ports::run_events::{RunEvent, RunEventsPort};
use crate::ports::step_executor::GatePresenter;

use super::device_key::DeviceKey;

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct MemorySecretStore {
    accounts: [String; 2],
    available: AtomicBool,
    entries: Mutex<BTreeMap<String, String>>,
    written: Mutex<Vec<String>>,
}

impl MemorySecretStore {
    pub fn new(install_id: &str) -> Self {
        Self {
            accounts: [
                hub_keyring_account(install_id, HubSecret::DeviceKey),
                hub_keyring_account(install_id, HubSecret::RefreshToken),
            ],
            available: AtomicBool::new(true),
            entries: Mutex::new(BTreeMap::new()),
            written: Mutex::new(Vec::new()),
        }
    }

    /// An unavailable store also refuses every read and write, as a keyring
    /// with no backend does.
    pub fn set_available(&self, available: bool) {
        self.available.store(available, Ordering::SeqCst);
    }

    pub fn entries(&self) -> BTreeMap<String, String> {
        self.lock().clone()
    }

    /// `key`, `refresh` and the store of `pins` as a completed pairing left
    /// them, without a Hub to pair with.
    pub fn paired(
        pins: &dyn HubPinRepository,
        hub_pins: &HubPins,
        key: &DeviceKey,
        refresh: &str,
    ) -> Result<Self, AppError> {
        let install_id = pins.install_id()?;
        let store = Self::new(&install_id);
        let key = super::stored_form(key).map_err(|e| AppError::internal(e.to_string()))?;
        store.set(
            &hub_keyring_account(&install_id, HubSecret::DeviceKey),
            &key,
        )?;
        store.set(
            &hub_keyring_account(&install_id, HubSecret::RefreshToken),
            refresh,
        )?;
        pins.save(hub_pins)?;
        Ok(store)
    }

    /// Every value `set` accepted, in order, overwritten ones included: a
    /// secret that was stored and then replaced is still a secret that was
    /// stored.
    pub fn written(&self) -> Vec<String> {
        locked(&self.written).clone()
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, String>> {
        locked(&self.entries)
    }

    fn check(&self, account: &str) -> Result<(), AppError> {
        if !self.available.load(Ordering::SeqCst) {
            return Err(AppError::internal("memory secret store is unavailable"));
        }
        if !self.accounts.iter().any(|a| a == account) {
            return Err(AppError::validation(format!(
                "{account} is not a Hub keyring account of this install"
            )));
        }
        Ok(())
    }
}

impl HubSecretStore for MemorySecretStore {
    fn status(&self) -> KeyringStatus {
        KeyringStatus {
            backend: "in-memory test store",
            available: self.available.load(Ordering::SeqCst),
            reason: None,
        }
    }

    fn get(&self, account: &str) -> Result<Option<String>, AppError> {
        self.check(account)?;
        Ok(self.lock().get(account).cloned())
    }

    fn set(&self, account: &str, value: &str) -> Result<(), AppError> {
        self.check(account)?;
        self.lock().insert(account.to_string(), value.to_string());
        locked(&self.written).push(value.to_string());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), AppError> {
        self.check(account)?;
        self.lock().remove(account);
        Ok(())
    }
}

/// One request a [`ScriptedRefreshEndpoint`] received, as it was handed over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshCall {
    pub token_url: String,
    pub form_body: String,
    pub dpop: String,
}

/// A `/token` refresh grant that answers each call with the next scripted
/// outcome. A call with nothing left to answer it gets
/// [`RefreshEnd::Unanswered`] — the outcome that stops a supervisor, so an
/// unscripted refresh cannot pass as a success — and is counted by
/// [`Self::unscripted`], which a test asserts is `0`. It does not panic: the
/// caller is usually a spawned task, whose panic the test would never see.
///
/// [`Self::hold`] parks every call, after it is recorded, until
/// [`Self::release`], so a test can act while a refresh is in flight.
pub struct ScriptedRefreshEndpoint {
    script: Mutex<VecDeque<Result<RefreshAnswer, RefreshEnd>>>,
    calls: Mutex<Vec<RefreshCall>>,
    called: Notify,
    unscripted: AtomicUsize,
    held: watch::Sender<bool>,
}

impl Default for ScriptedRefreshEndpoint {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptedRefreshEndpoint {
    pub fn new() -> Self {
        Self {
            script: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
            called: Notify::new(),
            unscripted: AtomicUsize::new(0),
            held: watch::Sender::new(false),
        }
    }

    /// Scripts the Hub's rotation answer, in its own JSON shape.
    pub fn answer(&self, access: &str, refresh: &str, expires_in: u64) {
        let body = serde_json::json!({
            "access_token": access,
            "token_type": "DPoP",
            "expires_in": expires_in,
            "refresh_token": refresh,
        });
        let answer = serde_json::from_value(body)
            .unwrap_or_else(|e| panic!("the scripted refresh answer does not parse: {e}"));
        locked(&self.script).push_back(Ok(answer));
    }

    pub fn fail(&self, end: RefreshEnd) {
        locked(&self.script).push_back(Err(end));
    }

    pub fn calls(&self) -> Vec<RefreshCall> {
        locked(&self.calls).clone()
    }

    pub fn unscripted(&self) -> usize {
        self.unscripted.load(Ordering::SeqCst)
    }

    /// Resolves once at least `count` calls have been recorded.
    pub async fn called_times(&self, count: usize) {
        loop {
            let notified = self.called.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if locked(&self.calls).len() >= count {
                return;
            }
            notified.await;
        }
    }

    pub fn hold(&self) {
        self.held.send_replace(true);
    }

    pub fn release(&self) {
        self.held.send_replace(false);
    }
}

#[async_trait]
impl HubRefreshEndpoint for ScriptedRefreshEndpoint {
    async fn refresh(
        &self,
        token_url: &str,
        form_body: String,
        dpop: String,
    ) -> Result<RefreshAnswer, RefreshEnd> {
        locked(&self.calls).push(RefreshCall {
            token_url: token_url.to_owned(),
            form_body,
            dpop,
        });
        self.called.notify_waiters();
        let mut held = self.held.subscribe();
        // The sender lives as long as `self`, so this cannot fail.
        let _ = held.wait_for(|held| !*held).await;
        let next = locked(&self.script).pop_front();
        next.unwrap_or_else(|| {
            self.unscripted.fetch_add(1, Ordering::SeqCst);
            Err(RefreshEnd::Unanswered)
        })
    }
}

/// One request a [`ScriptedDeviceEndpoint`] received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCall {
    pub url: String,
    pub form_body: String,
    pub dpop: String,
}

/// The Hub's `/device_authorization` and device-code `/token`, answering
/// each from its own script. An unscripted authorization is a `Transport`
/// error and an unscripted poll a `Refused` one — never `Pending`, which
/// would keep a pairing polling that the test did not script — and each is
/// counted by [`Self::unscripted`].
#[derive(Default)]
pub struct ScriptedDeviceEndpoint {
    authorizations: Mutex<VecDeque<Result<DeviceAuthorization, HubPairingError>>>,
    polls: Mutex<VecDeque<Result<PairingAnswer, DeviceGrantEnd>>>,
    authorize_calls: Mutex<Vec<DeviceCall>>,
    poll_calls: Mutex<Vec<DeviceCall>>,
    unscripted: AtomicUsize,
}

impl ScriptedDeviceEndpoint {
    pub fn new() -> Self {
        Self::default()
    }

    /// Scripts the Hub's `200`, in its own JSON shape, for `origin`.
    pub fn authorize(&self, origin: &str, device_code: &str, user_code: &str, interval: u64) {
        let verification_uri = format!("{origin}/#/device");
        let body = serde_json::json!({
            "device_code": device_code,
            "user_code": user_code,
            "verification_uri_complete": format!("{verification_uri}/{user_code}"),
            "verification_uri": verification_uri,
            "expires_in": 600,
            "interval": interval,
        });
        let answer = serde_json::from_value(body)
            .unwrap_or_else(|e| panic!("the scripted device authorization does not parse: {e}"));
        locked(&self.authorizations).push_back(Ok(answer));
    }

    pub fn refuse_authorization(&self, error: HubPairingError) {
        locked(&self.authorizations).push_back(Err(error));
    }

    pub fn poll_answers(&self, answer: PairingAnswer) {
        locked(&self.polls).push_back(Ok(answer));
    }

    pub fn poll_ends(&self, end: DeviceGrantEnd) {
        locked(&self.polls).push_back(Err(end));
    }

    pub fn authorize_calls(&self) -> Vec<DeviceCall> {
        locked(&self.authorize_calls).clone()
    }

    pub fn poll_calls(&self) -> Vec<DeviceCall> {
        locked(&self.poll_calls).clone()
    }

    pub fn unscripted(&self) -> usize {
        self.unscripted.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl HubDeviceEndpoint for ScriptedDeviceEndpoint {
    async fn authorize_device(
        &self,
        url: &str,
        form_body: String,
        dpop: String,
    ) -> Result<DeviceAuthorization, HubPairingError> {
        locked(&self.authorize_calls).push(DeviceCall {
            url: url.to_owned(),
            form_body,
            dpop,
        });
        let next = locked(&self.authorizations).pop_front();
        next.unwrap_or_else(|| {
            self.unscripted.fetch_add(1, Ordering::SeqCst);
            Err(HubPairingError::Transport(
                "unscripted authorization".into(),
            ))
        })
    }

    async fn poll_device(
        &self,
        token_url: &str,
        form_body: String,
        dpop: String,
    ) -> Result<PairingAnswer, DeviceGrantEnd> {
        locked(&self.poll_calls).push(DeviceCall {
            url: token_url.to_owned(),
            form_body,
            dpop,
        });
        let next = locked(&self.polls).pop_front();
        next.unwrap_or_else(|| {
            self.unscripted.fetch_add(1, Ordering::SeqCst);
            Err(DeviceGrantEnd::Refused(HubPairingError::Transport(
                "unscripted poll".into(),
            )))
        })
    }
}

/// The Hub's `GET /attachments/{id}`: the bytes staged for an id, the first
/// time it is fetched, checked against the [`AttachmentRef`] as
/// [`super::attachments::HubAttachments`] checks them, so bytes staged under a
/// wrong size or SHA-256 are [`AttachmentFetchError::Mismatch`]. A second
/// fetch of an id, or one never staged, is refused `404`, as the Hub refuses
/// it, and counted by [`Self::unscripted`].
#[derive(Default)]
pub struct ScriptedAttachments {
    staged: Mutex<BTreeMap<String, Vec<u8>>>,
    fetched: Mutex<Vec<String>>,
    unscripted: AtomicUsize,
}

impl ScriptedAttachments {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stage(&self, id: &str, bytes: &[u8]) {
        locked(&self.staged).insert(id.to_owned(), bytes.to_vec());
    }

    /// Every id asked for, in order, refused ones included.
    pub fn fetched(&self) -> Vec<String> {
        locked(&self.fetched).clone()
    }

    pub fn unscripted(&self) -> usize {
        self.unscripted.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl HubAttachmentFetcher for ScriptedAttachments {
    async fn fetch(&self, attachment: &AttachmentRef) -> Result<Vec<u8>, AttachmentFetchError> {
        locked(&self.fetched).push(attachment.id.clone());
        let Some(bytes) = locked(&self.staged).remove(&attachment.id) else {
            self.unscripted.fetch_add(1, Ordering::SeqCst);
            return Err(AttachmentFetchError::Refused { status: 404 });
        };
        super::attachments::checked(attachment, bytes)
    }
}

type DialHook = Box<dyn Fn(&SocketUpgrade) + Send + Sync>;

/// A `/socket` that answers each dial with the next scripted outcome. A dial
/// with nothing left to answer it is refused with [`Self::UNSCRIPTED_STATUS`]
/// — a status the supervisor stops on at once, so a test fails on its
/// [`Self::unscripted`] assertion rather than timing out on a redial loop —
/// and is counted. It does not panic, for the reason
/// [`ScriptedRefreshEndpoint`] gives.
///
/// [`Self::on_dial`] runs a hook inside each dial, before it is answered, so
/// a test can look at what was true at the moment the desktop dialed.
pub struct ScriptedSocketDialer {
    script: Mutex<VecDeque<Result<Box<dyn HubSocketConnection>, DialFailure>>>,
    upgrades: Mutex<Vec<SocketUpgrade>>,
    dialed: Notify,
    unscripted: AtomicUsize,
    hook: Mutex<Option<DialHook>>,
}

impl Default for ScriptedSocketDialer {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptedSocketDialer {
    pub const UNSCRIPTED: &'static str = "unscripted dial";
    /// Not `401`, `429` or `5xx`, which are what the supervisor retries.
    pub const UNSCRIPTED_STATUS: u16 = 418;

    pub fn new() -> Self {
        Self {
            script: Mutex::new(VecDeque::new()),
            upgrades: Mutex::new(Vec::new()),
            dialed: Notify::new(),
            unscripted: AtomicUsize::new(0),
            hook: Mutex::new(None),
        }
    }

    pub fn connect(&self, connection: impl HubSocketConnection + 'static) {
        locked(&self.script).push_back(Ok(Box::new(connection)));
    }

    pub fn refuse(&self, failure: DialFailure) {
        locked(&self.script).push_back(Err(failure));
    }

    pub fn on_dial(&self, hook: impl Fn(&SocketUpgrade) + Send + Sync + 'static) {
        *locked(&self.hook) = Some(Box::new(hook));
    }

    pub fn upgrades(&self) -> Vec<SocketUpgrade> {
        locked(&self.upgrades).clone()
    }

    pub fn unscripted(&self) -> usize {
        self.unscripted.load(Ordering::SeqCst)
    }

    /// Resolves once at least `count` dials have been recorded.
    pub async fn dialed_times(&self, count: usize) {
        loop {
            let notified = self.dialed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if locked(&self.upgrades).len() >= count {
                return;
            }
            notified.await;
        }
    }
}

#[async_trait]
impl HubSocketDialer for ScriptedSocketDialer {
    async fn dial(
        &self,
        upgrade: SocketUpgrade,
    ) -> Result<Box<dyn HubSocketConnection>, DialFailure> {
        if let Some(hook) = locked(&self.hook).as_ref() {
            hook(&upgrade);
        }
        locked(&self.upgrades).push(upgrade);
        self.dialed.notify_waiters();
        let next = locked(&self.script).pop_front();
        next.unwrap_or_else(|| {
            self.unscripted.fetch_add(1, Ordering::SeqCst);
            Err(DialFailure::Refused {
                status: Self::UNSCRIPTED_STATUS,
                error: Some(Self::UNSCRIPTED.to_owned()),
            })
        })
    }
}

/// One frame across a [`ScriptedConnection`], recorded when the session sent
/// it or was handed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exchange {
    Sent(String),
    Received(SocketReceived),
}

/// What a [`ScriptedConnection`] was told, readable after the connection
/// itself has been handed to a dialer.
#[derive(Default)]
pub struct ConnectionLog {
    exchanges: Mutex<Vec<Exchange>>,
    sends: Notify,
    closed_with: Mutex<Option<u16>>,
    unscripted: AtomicUsize,
}

impl ConnectionLog {
    pub fn sent(&self) -> Vec<String> {
        locked(&self.exchanges)
            .iter()
            .filter_map(|exchange| match exchange {
                Exchange::Sent(text) => Some(text.clone()),
                Exchange::Received(_) => None,
            })
            .collect()
    }

    /// Every frame either way, in the order the session met them: a frame
    /// received before a send was acted on before that send.
    pub fn exchanges(&self) -> Vec<Exchange> {
        locked(&self.exchanges).clone()
    }

    /// Resolves once at least `count` frames have been sent.
    pub async fn sent_times(&self, count: usize) {
        loop {
            let notified = self.sends.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.sent().len() >= count {
                return;
            }
            notified.await;
        }
    }

    /// The code the desktop passed to `close`, if it closed.
    pub fn closed_with(&self) -> Option<u16> {
        *locked(&self.closed_with)
    }

    /// Reads answered after the script ran out.
    pub fn unscripted(&self) -> usize {
        self.unscripted.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Copy)]
enum Sends {
    Accepted,
    Gone,
    Stalled,
}

/// Replays its script, then answers `Closed(None)` forever, so a test that
/// scripts too little ends the session instead of hanging it; each of those
/// reads is counted by [`ConnectionLog::unscripted`]. Once it has answered
/// `Closed`, every send is [`SocketGone`].
pub struct ScriptedConnection {
    script: VecDeque<SocketReceived>,
    silent_after: bool,
    held: Option<mpsc::UnboundedReceiver<SocketReceived>>,
    sends: Sends,
    stalling_close: bool,
    gone: bool,
    log: Arc<ConnectionLog>,
}

impl ScriptedConnection {
    pub fn new(script: impl IntoIterator<Item = SocketReceived>) -> Self {
        Self {
            script: script.into_iter().collect(),
            silent_after: false,
            held: None,
            sends: Sends::Accepted,
            stalling_close: false,
            gone: false,
            log: Arc::default(),
        }
    }

    /// Once the script runs out, every read pends forever, as from a Hub
    /// that is still connected and says nothing.
    pub fn then_silent(mut self) -> Self {
        self.silent_after = true;
        self
    }

    /// Once the script runs out, every read waits for the next frame the
    /// test releases through the returned sender, so the Hub acks only when
    /// the test lets it. With the sender dropped, reads pend forever.
    pub fn then_held(mut self) -> (Self, mpsc::UnboundedSender<SocketReceived>) {
        let (release, held) = mpsc::unbounded_channel();
        self.held = Some(held);
        (self, release)
    }

    /// Every send is [`SocketGone`] and recorded nowhere.
    pub fn refusing_sends(mut self) -> Self {
        self.sends = Sends::Gone;
        self
    }

    /// Every send pends forever and is recorded nowhere, as to a Hub that
    /// reads nothing.
    pub fn stalling_sends(mut self) -> Self {
        self.sends = Sends::Stalled;
        self
    }

    /// `close` records its code, then pends forever, as to a Hub that reads
    /// nothing.
    pub fn stalling_close(mut self) -> Self {
        self.stalling_close = true;
        self
    }

    pub fn log(&self) -> Arc<ConnectionLog> {
        self.log.clone()
    }
}

#[async_trait]
impl HubSocketConnection for ScriptedConnection {
    async fn received(&mut self) -> SocketReceived {
        let next = match (self.script.pop_front(), self.held.as_mut()) {
            (Some(next), _) => next,
            (None, Some(held)) => match held.recv().await {
                Some(next) => next,
                None => std::future::pending().await,
            },
            (None, None) if self.silent_after => std::future::pending().await,
            (None, None) => {
                self.log.unscripted.fetch_add(1, Ordering::SeqCst);
                SocketReceived::Closed(None)
            }
        };
        if matches!(next, SocketReceived::Closed(_)) {
            self.gone = true;
        }
        locked(&self.log.exchanges).push(Exchange::Received(next.clone()));
        next
    }

    async fn send_text(&mut self, text: String) -> Result<(), SocketGone> {
        if self.gone {
            return Err(SocketGone);
        }
        match self.sends {
            Sends::Accepted => {}
            Sends::Gone => return Err(SocketGone),
            Sends::Stalled => std::future::pending().await,
        }
        locked(&self.log.exchanges).push(Exchange::Sent(text));
        self.log.sends.notify_waiters();
        Ok(())
    }

    async fn close(self: Box<Self>, code: u16) {
        *locked(&self.log.closed_with) = Some(code);
        if self.stalling_close {
            std::future::pending::<()>().await;
        }
    }
}

/// A run log that answers every call with an `Err`, and counts the
/// [`RunEventsPort::list_after`] calls it refused.
#[derive(Default)]
pub struct UnreadableRunEvents {
    reads: AtomicUsize,
}

impl UnreadableRunEvents {
    pub const ERROR: &'static str = "the run log is unreadable";

    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl RunEventsPort for UnreadableRunEvents {
    fn append(
        &self,
        _run_id: &str,
        _kind: &str,
        _payload_json: Option<&str>,
        _now: i64,
    ) -> Result<i64, String> {
        Err(Self::ERROR.to_owned())
    }

    fn list_since(&self, _run_id: &str, _from_offset: i64) -> Result<Vec<RunEvent>, String> {
        Err(Self::ERROR.to_owned())
    }

    fn list_after(&self, _from_offset: i64, _limit: usize) -> Result<Vec<RunEvent>, String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Err(Self::ERROR.to_owned())
    }
}

/// A snapshot source that answers each call with the next scripted outcome,
/// then with the snapshot [`Self::set`] last chose, and otherwise with
/// [`Self::UNSCRIPTED`], counted by [`Self::unscripted`].
#[derive(Default)]
pub struct ScriptedSnapshotSource {
    script: Mutex<VecDeque<Result<Snapshot, String>>>,
    current: Mutex<Option<Snapshot>>,
    calls: AtomicUsize,
    unscripted: AtomicUsize,
}

impl ScriptedSnapshotSource {
    pub const UNSCRIPTED: &'static str = "unscripted snapshot";

    pub fn push(&self, snapshot: Snapshot) {
        locked(&self.script).push_back(Ok(snapshot));
    }

    pub fn fail(&self, error: &str) {
        locked(&self.script).push_back(Err(error.to_owned()));
    }

    /// What every call past the script answers, until set again.
    pub fn set(&self, snapshot: Snapshot) {
        *locked(&self.current) = Some(snapshot);
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn unscripted(&self) -> usize {
        self.unscripted.load(Ordering::SeqCst)
    }
}

impl HubSnapshotSource for ScriptedSnapshotSource {
    fn snapshot(&self) -> Result<Snapshot, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(next) = locked(&self.script).pop_front() {
            return next;
        }
        locked(&self.current).clone().ok_or_else(|| {
            self.unscripted.fetch_add(1, Ordering::SeqCst);
            Self::UNSCRIPTED.to_owned()
        })
    }
}

/// A `gate_decide` call: the step execution id, the decision and the feedback.
pub type GateCall = (String, String, Option<String>);

/// A gate presenter that accepts the `gate_decide` it was scripted with, the
/// first time it is made, and answers every other call with an `Err` counted
/// by [`Self::unexpected`]. A refused call leaves the script in place, so the
/// scripted call can still follow one.
pub struct ScriptedGatePresenter {
    script: Mutex<Option<GateCall>>,
    calls: Mutex<Vec<GateCall>>,
    unexpected: AtomicUsize,
}

impl ScriptedGatePresenter {
    pub const UNSCRIPTED: &'static str = "unscripted gate presenter call";

    pub fn expecting(step_execution_id: &str, decision: &str, feedback: Option<&str>) -> Self {
        Self::scripted(Some((
            step_execution_id.to_owned(),
            decision.to_owned(),
            feedback.map(str::to_owned),
        )))
    }

    pub fn refusing_all() -> Self {
        Self::scripted(None)
    }

    fn scripted(script: Option<GateCall>) -> Self {
        Self {
            script: Mutex::new(script),
            calls: Mutex::new(Vec::new()),
            unexpected: AtomicUsize::new(0),
        }
    }

    pub fn calls(&self) -> Vec<GateCall> {
        locked(&self.calls).clone()
    }

    pub fn unexpected(&self) -> usize {
        self.unexpected.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl GatePresenter for ScriptedGatePresenter {
    async fn gate_pending_for_run(&self, feature_id: &str) -> Result<Option<GateDecision>, String> {
        self.unexpected.fetch_add(1, Ordering::SeqCst);
        Err(format!(
            "{}: gate_pending_for_run({feature_id})",
            Self::UNSCRIPTED
        ))
    }

    async fn gate_decide(
        &self,
        step_execution_id: &str,
        decision: &str,
        feedback: Option<&str>,
    ) -> Result<(), AppError> {
        let call = (
            step_execution_id.to_owned(),
            decision.to_owned(),
            feedback.map(str::to_owned),
        );
        let scripted = locked(&self.script).take_if(|script| *script == call);
        if scripted.is_none() {
            self.unexpected.fetch_add(1, Ordering::SeqCst);
            return Err(AppError::validation(format!(
                "{}: gate_decide{call:?}",
                Self::UNSCRIPTED
            )));
        }
        locked(&self.calls).push(call);
        Ok(())
    }
}

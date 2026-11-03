#![cfg(target_os = "linux")]
//! A `demeteo-runner` answering the Hub, from a request the Hub's own key
//! signed to the frame the runner sends back (P2). Each package's unit tests
//! hand the handler a request directly; these reach it the way a runner
//! does: composed by `build_core_context_with_hub_store` in `LocalOnly`,
//! bound by the `HubRequestBinding` it returns, and delivered by the
//! pairing's own supervisor and session over a scripted connection.
//!
//! Two things stand in for the runner, because `src-tauri` cannot depend on
//! the `demeteo-runner` binary crate:
//!
//! - [`RunnerControl`] is `RunnerHubRuns` over `control::cancel_run`, spelled
//!   against the same ports: starts refused, a cancel finds the run by its
//!   Feature id, stops the feature, ends the row and logs `cancelled`.
//! - The socket is configured here rather than by `hub_socket()` in
//!   composition, so it can dial a [`ScriptedSocketDialer`]. It is given
//!   [`RunnerRunKeys`], which is what composition gives a `LocalOnly`
//!   context.
//!
//! Every double answers only what it was scripted with.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use demeteo_core::adapters::database::SqliteAdapter;
use demeteo_core::adapters::git_credentials::KeyringCredentialSource;
use demeteo_core::adapters::hub::device_key::DeviceKey;
use demeteo_core::adapters::hub::run_keys::RunnerRunKeys;
use demeteo_core::adapters::hub::snapshot_source::RepoSnapshotSource;
use demeteo_core::adapters::hub::socket::wake::WakingRunEvents;
use demeteo_core::adapters::hub::socket::{Sleeper, SocketConfig};
use demeteo_core::adapters::hub::testing::{
    ConnectionLog, MemorySecretStore, ScriptedConnection, ScriptedGatePresenter,
    ScriptedRefreshEndpoint, ScriptedSocketDialer,
};
use demeteo_core::adapters::hub::HubPairing;
use demeteo_core::adapters::notification_noop::NoopNotificationAdapter;
use demeteo_core::composition::{build_core_context_with_hub_store, CoreConfig, ExecutionMode};
use demeteo_core::domain::feature_origin::FeatureOrigin;
use demeteo_core::domain::hub_pairing::{
    hub_origin, HubPairingError, HubPins, PairingAnswer, PinnedPasskey,
};
use demeteo_core::domain::hub_request::passkeys::gate_challenge_digest;
use demeteo_core::domain::hub_request::{HubLocalPolicy, LogKind, LogOutcome};
use demeteo_core::domain::ids::{
    FeatureId, GateDecisionId, ProjectId, StepExecutionId, StepId, WorkflowId,
};
use demeteo_core::domain::models::{
    EffortLevel, Feature, FeatureDivergence, FeatureDrift, GateDecision, LaunchedBy, Project,
    StepExecution, Workflow,
};
use demeteo_core::domain::step_assignment::StepAssignment;
use demeteo_core::domain::sync_resolver::SyncResolverChoice;
use demeteo_core::domain::upstream_feature::DivergenceReconcile;
use demeteo_core::error::AppError;
use demeteo_core::ports::hub::{HubRunControl, HubTokenEndpoint, SocketReceived, StartSupport};
use demeteo_core::ports::step_executor::{
    FeatureLaunch, StepExecutor, SyncOutcomeView, SyncResolverView,
};
use demeteo_core::ports::sync_session::SyncSessionView;
use demeteo_core::shared::time::now_secs;
use demeteo_core::state::AppContext;
use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::auth::challenge::base64url_encode;
use demeteo_hub::auth::testing::SoftAuthenticator;
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::pairing::signing_key::SigningKey;
use demeteo_hub::protocol::{
    AttachmentRef, CancelFeature, CursorAck, Decision, GateDecision as GatePayload, RefusalReason,
    RequestOutcome, RequestPayload, RequestResult, RunEvent, RunEventKind, SignedRequest, Snapshot,
    StartFeature, PROTOCOL_VERSION,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::timeout;

const SIGNING_KEY: &str = "request-signing";
const INSTANCE: &str = "i-runner";
const ACCESS_TOKEN: &str = "dha_access-0123456789abcdefghijklmnopqrstuv";
const REFRESH_TOKEN: &str = "dhr_refresh-0123456789abcdefghijklmnopqrstu";
const RUN: &str = "run-1";
const FEATURE: &str = "f-1";
const GATE_STEP: &str = "se-gate";
const BOUND: Duration = Duration::from_secs(10);

fn unexpected<T>(what: &str, calls: &Mutex<Vec<String>>) -> Result<T, String> {
    calls.lock().expect("lock").push(what.to_owned());
    Err(format!("unscripted StepExecutor call: {what}"))
}

/// Records each `feature_cancel`; every other call is refused and counted,
/// so a request that reaches some other effect cannot pass.
#[derive(Default)]
struct CancelOnly {
    cancels: Mutex<Vec<String>>,
    unexpected: Mutex<Vec<String>>,
}

#[async_trait]
impl StepExecutor for CancelOnly {
    async fn feature_start(&self, _: FeatureLaunch) -> Result<Feature, String> {
        unexpected("feature_start", &self.unexpected)
    }
    async fn feature_pause(&self, _: &str) -> Result<(), String> {
        unexpected("feature_pause", &self.unexpected)
    }
    async fn feature_resume(&self, _: &str) -> Result<(), String> {
        unexpected("feature_resume", &self.unexpected)
    }
    async fn feature_cancel(&self, feature_id: &str) -> Result<(), String> {
        self.cancels.lock().expect("lock").push(feature_id.into());
        Ok(())
    }
    async fn step_get(&self, _: &str) -> Result<StepExecution, String> {
        unexpected("step_get", &self.unexpected)
    }
    async fn step_retry(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<EffortLevel>,
    ) -> Result<(), AppError> {
        unexpected("step_retry", &self.unexpected).map_err(AppError::internal)
    }
    async fn step_set_assignment(&self, _: &str, _: StepAssignment) -> Result<(), AppError> {
        unexpected("step_set_assignment", &self.unexpected).map_err(AppError::internal)
    }
    async fn replay_from_step(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<EffortLevel>,
    ) -> Result<(), String> {
        unexpected("replay_from_step", &self.unexpected)
    }
    async fn step_list_for_run(&self, _: &str) -> Result<Vec<StepExecution>, String> {
        unexpected("step_list_for_run", &self.unexpected)
    }
    async fn feature_sync(&self, _: &str) -> Result<SyncOutcomeView, String> {
        unexpected("feature_sync", &self.unexpected)
    }
    async fn feature_drift(&self, _: &str, _: bool) -> Result<FeatureDrift, String> {
        unexpected("feature_drift", &self.unexpected)
    }
    async fn feature_reconcile(
        &self,
        _: &str,
        _: DivergenceReconcile,
    ) -> Result<Option<SyncSessionView>, String> {
        unexpected("feature_reconcile", &self.unexpected)
    }
    async fn feature_divergence(&self, _: &str) -> Result<Option<FeatureDivergence>, String> {
        unexpected("feature_divergence", &self.unexpected)
    }
    async fn feature_resolve_sync_conflicts(
        &self,
        _: &str,
        _: &SyncResolverChoice,
    ) -> Result<SyncOutcomeView, String> {
        unexpected("feature_resolve_sync_conflicts", &self.unexpected)
    }
    async fn feature_continue_sync(&self, _: &str) -> Result<SyncOutcomeView, String> {
        unexpected("feature_continue_sync", &self.unexpected)
    }
    async fn feature_sync_resolver(&self, _: &str) -> Result<SyncResolverView, String> {
        unexpected("feature_sync_resolver", &self.unexpected)
    }
}

/// `RunnerHubRuns` as the module header describes it. A launch is recorded
/// and refused: the handler must never ask.
struct RunnerControl {
    ctx: Arc<AppContext>,
    launches: AtomicUsize,
}

#[async_trait]
impl HubRunControl for RunnerControl {
    fn starts(&self) -> StartSupport {
        StartSupport::Refused
    }

    async fn launch(&self, _: FeatureLaunch) -> Result<FeatureId, AppError> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        Err(AppError::validation("a refused start was launched"))
    }

    async fn cancel(&self, feature_id: &FeatureId) -> Result<(), AppError> {
        let run = self
            .ctx
            .runner_runs
            .list()
            .map_err(AppError::internal)?
            .into_iter()
            .find(|run| run.feature_id.as_deref() == Some(feature_id.as_ref()))
            .ok_or_else(|| AppError::not_found(format!("no run of feature {feature_id}")))?;
        self.ctx
            .executor
            .feature_cancel(feature_id.as_ref())
            .await
            .map_err(AppError::internal)?;
        let now = demeteo_core::paths::now_ms();
        let run = self
            .ctx
            .runner_runs
            .cancel_if_active(&run.run_id, now)
            .map_err(AppError::internal)?
            .ok_or_else(|| AppError::internal("run vanished during cancel"))?;
        if run.status == "cancelled" {
            self.ctx
                .run_events
                .append(&run.run_id, "cancelled", None, now)
                .map_err(AppError::internal)?;
        }
        Ok(())
    }
}

/// Every wait the supervisor makes pends, so a test never redials.
struct Parked;

#[async_trait]
impl Sleeper for Parked {
    async fn sleep(&self, _: Duration) {
        std::future::pending::<()>().await;
    }
}

/// A runner never redeems a code here; it is already paired.
struct NoRedemption;

#[async_trait]
impl HubTokenEndpoint for NoRedemption {
    async fn exchange(
        &self,
        _: &str,
        _: String,
        _: String,
    ) -> Result<PairingAnswer, HubPairingError> {
        Err(HubPairingError::Transport(
            "a paired runner redeemed a code".into(),
        ))
    }
}

/// The Hub's origin: a loopback listener that counts every connection made
/// to it. Nothing in these tests is meant to reach it, so an attachment
/// fetch shows up as a count above zero.
struct Origin {
    url: String,
    connections: Arc<AtomicUsize>,
}

impl Origin {
    async fn listen() -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("a loopback port");
        let port = listener.local_addr().expect("a bound address").port();
        let connections = Arc::new(AtomicUsize::new(0));
        let counted = connections.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                counted.fetch_add(1, Ordering::SeqCst);
                drop(stream);
            }
        });
        let url = hub_origin(&format!("http://localhost:{port}/")).expect("a localhost origin");
        Self { url, connections }
    }

    fn rp(&self) -> RelyingParty {
        RelyingParty {
            rp_id: "localhost".into(),
            origin: self.url.clone(),
        }
    }
}

/// A runner composed as `demeteo-runner` composes itself, paired with a Hub
/// whose signing key this holds, its socket connected and acked.
struct Runner {
    ctx: Arc<AppContext>,
    executor: Arc<CancelOnly>,
    runs: Arc<RunnerControl>,
    presenter: Arc<ScriptedGatePresenter>,
    origin: Origin,
    keys: MemoryKeys,
    signing: SigningKey,
    refresh: Arc<ScriptedRefreshEndpoint>,
    dialer: Arc<ScriptedSocketDialer>,
    log: Arc<ConnectionLog>,
    hub: mpsc::UnboundedSender<SocketReceived>,
    dir: std::path::PathBuf,
}

impl Runner {
    /// `seed` runs on the composed context before the socket starts, so the
    /// first page the session reads already holds what it wrote.
    async fn start(
        tag: &str,
        policy: HubLocalPolicy,
        passkeys: Vec<PinnedPasskey>,
        presenter: ScriptedGatePresenter,
        seed: impl FnOnce(&AppContext),
    ) -> Self {
        let dir = demeteo_core::test_dir::scratch(&format!("demeteo-hub-runner-{tag}"));
        let origin = Origin::listen().await;
        let keys = MemoryKeys::with_keys([SIGNING_KEY]);
        let signing = SigningKey::new(SIGNING_KEY);
        let request_signing_key = signing.public_key(&keys).await.expect("a public key");
        let db = Arc::new(
            SqliteAdapter::new(demeteo_core::db::init_db(dir.clone()).expect("init db"))
                .expect("build sqlite adapter"),
        );
        let device = DeviceKey::generate().expect("a P-256 key");
        let pins = HubPins {
            hub_origin: origin.url.clone(),
            instance_id: INSTANCE.into(),
            request_signing_key: base64url_encode(&request_signing_key),
            rp_id: "localhost".into(),
            scopes: vec!["read".into(), "spend".into(), "gates".into()],
            device_jkt: device.jkt(),
            paired_at: 0,
            passkeys,
        };
        let secrets = Arc::new(
            MemorySecretStore::paired(&*db, &pins, &device, REFRESH_TOKEN).expect("a seeded store"),
        );

        let (mut ctx, writer, binding) = build_core_context_with_hub_store(
            CoreConfig {
                app_data_dir: dir.clone(),
                execution_mode: ExecutionMode::LocalOnly,
            },
            Arc::new(NoopNotificationAdapter),
            tokio::runtime::Handle::current(),
            Arc::new(KeyringCredentialSource),
            secrets.clone(),
        );
        writer.save(&policy).expect("save the policy");
        let executor = Arc::new(CancelOnly::default());
        ctx.executor = executor.clone();
        let presenter = Arc::new(presenter);
        ctx.presenter = presenter.clone();
        let run_events = Arc::new(WakingRunEvents::new(db.clone()));
        ctx.run_events = run_events.clone();

        let refresh = Arc::new(ScriptedRefreshEndpoint::new());
        refresh.answer(ACCESS_TOKEN, REFRESH_TOKEN, 900);
        let dialer = Arc::new(ScriptedSocketDialer::new());
        let run_keys = Arc::new(RunnerRunKeys::new(db.clone(), db.clone()));
        let snapshots =
            RepoSnapshotSource::new(db.clone(), db.clone(), db.clone(), db.clone(), Vec::new())
                .with_run_keys(run_keys.clone());
        let pairing = HubPairing::new(
            db.clone(),
            secrets,
            Arc::new(NoRedemption),
            ctx.hub_policy.clone(),
        )
        .with_refresh_endpoint(refresh.clone())
        .with_socket(SocketConfig {
            dialer: dialer.clone(),
            wake: run_events.wake(),
            run_events,
            snapshots: Arc::new(snapshots),
            snapshot_wake: Arc::new(tokio::sync::Notify::new()),
            run_keys,
            snapshot_debounce: Duration::from_millis(10),
            runtime: tokio::runtime::Handle::current(),
            sleeper: Arc::new(Parked),
            jitter: || 1.0,
        });
        let pairing = Arc::new(pairing);
        ctx.hub = Some(pairing.clone());
        seed(&ctx);

        let ctx = Arc::new(ctx);
        let runs = Arc::new(RunnerControl {
            ctx: ctx.clone(),
            launches: AtomicUsize::new(0),
        });
        assert!(!pairing.requests_bound());
        binding.bind(ctx.clone(), runs.clone());
        assert!(pairing.requests_bound());

        let ack = CursorAck {
            v: PROTOCOL_VERSION,
            offset: 0,
        };
        let (conn, hub) = ScriptedConnection::new([SocketReceived::Text(
            serde_json::to_string(&ack).expect("a serialisable ack"),
        )])
        .then_held();
        let log = conn.log();
        dialer.connect(conn);
        pairing.start_socket();
        timeout(BOUND, dialer.dialed_times(1))
            .await
            .expect("the runner dialed the Hub");

        Self {
            ctx,
            executor,
            runs,
            presenter,
            origin,
            keys,
            signing,
            refresh,
            dialer,
            log,
            hub,
            dir,
        }
    }

    /// `payload`, signed by the Hub's key as `requests::signed` signs one,
    /// and serialized as the socket carries it.
    async fn signed(&self, request_id: &str, payload: RequestPayload, expires_at: u64) -> String {
        let mut request = SignedRequest {
            v: 1,
            request_id: request_id.into(),
            instance_id: INSTANCE.into(),
            issued_at: now_secs(),
            expires_at,
            payload,
            signature: String::new(),
        };
        let message = request.signing_bytes().expect("encode the request");
        let signature = self
            .signing
            .sign(&self.keys, &message)
            .await
            .expect("sign the request");
        request.signature = base64url_encode(&signature);
        serde_json::to_string(&request).expect("serialize the request")
    }

    /// Hands `text` to the runner over the socket, and returns the result it
    /// sent back for `request_id`.
    async fn deliver(&self, request_id: &str, text: String) -> RequestOutcome {
        self.hub
            .send(SocketReceived::Text(text))
            .expect("the runner's session is reading");
        timeout(BOUND, async {
            loop {
                let sent = self.log.sent();
                let answer = sent
                    .iter()
                    .filter_map(|text| serde_json::from_str::<RequestResult>(text).ok())
                    .find(|result| result.request_id == request_id);
                if let Some(result) = answer {
                    return result.outcome;
                }
                self.log.sent_times(sent.len() + 1).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the runner answered {request_id} within {BOUND:?}"))
    }

    /// The frames of type `T` the runner has sent, once they satisfy `until`.
    async fn sent_until<T: serde::de::DeserializeOwned>(
        &self,
        until: impl Fn(&[T]) -> bool,
    ) -> Vec<T> {
        timeout(BOUND, async {
            loop {
                let sent = self.log.sent();
                let frames: Vec<T> = sent
                    .iter()
                    .filter_map(|text| serde_json::from_str::<T>(text).ok())
                    .collect();
                if until(&frames) {
                    return frames;
                }
                self.log.sent_times(sent.len() + 1).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "the runner sent the expected frames within {BOUND:?}: {:?}",
                self.log.sent()
            )
        })
    }

    fn last_log(&self) -> (LogKind, LogOutcome, Option<String>) {
        let entry = self
            .ctx
            .hub_requests
            .list_log(1)
            .expect("list the log")
            .pop()
            .expect("a log row")
            .entry;
        (entry.kind, entry.outcome, entry.reason)
    }

    /// Nothing reached what a runner must not touch for a Hub request.
    fn assert_untouched(&self) {
        assert_eq!(self.origin.connections.load(Ordering::SeqCst), 0);
        assert_eq!(self.runs.launches.load(Ordering::SeqCst), 0);
        assert!(self.executor.unexpected.lock().expect("lock").is_empty());
        assert_eq!(self.presenter.unexpected(), 0);
        assert_eq!(self.refresh.unscripted(), 0);
        assert_eq!(self.dialer.unscripted(), 0);
        assert_eq!(self.log.unscripted(), 0);
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn policy() -> HubLocalPolicy {
    HubLocalPolicy {
        read_enabled: true,
        spend_enabled: true,
        gates_enabled: true,
        run_ceiling_cents: Some(1_000),
        daily_ceiling_cents: Some(10_000),
    }
}

fn feature(id: &str) -> Feature {
    Feature {
        id: FeatureId::from(id.to_string()),
        project_id: ProjectId::from("p-1".to_string()),
        workflow_id: None,
        workflow_version_id: None,
        title: "Submitted by a desktop".into(),
        description: String::new(),
        status: "running".to_string(),
        total_cost: 0.0,
        duration: String::new(),
        tokens: 0,
        created_at: 1,
        agent_kind: None,
        model: None,
        effort: None,
        mr_url: None,
        mr_state: Some("none".into()),
        pr_title: None,
        pr_body: None,
        commit_artifacts: None,
        loop_iterations: None,
        max_budget_usd: None,
        step_overrides: Vec::new(),
        attachments: Vec::new(),
        harness_baseline: None,
        origin: FeatureOrigin::DefaultBranch,
        diff_base_branch: None,
        resolved_branch: None,
        launched_by: LaunchedBy::Desktop,
    }
}

/// A desktop's submitted run, as the runner holds it once bootstrapped:
/// run `RUN` executing Feature `FEATURE`, its first row logged under the run
/// id the way the runner logs every row.
fn seed_run(ctx: &AppContext) {
    seed_project(ctx);
    ctx.features.add(feature(FEATURE)).expect("add the feature");
    ctx.runner_runs
        .get_or_create(RUN, "{}", "client-a", 1)
        .expect("add the run");
    ctx.runner_runs
        .update_status(RUN, "running", Some("p-1"), Some(FEATURE), None, None, 2)
        .expect("bootstrap the run");
    let payload = json!({"status": "running"}).to_string();
    ctx.run_events
        .append(RUN, "feature_status", Some(&payload), 3)
        .expect("log the run");
}

/// The project and workflow a start names, so a start that got past its
/// kind would go on to fetch.
fn seed_project(ctx: &AppContext) {
    ctx.workflows
        .create(Workflow {
            id: WorkflowId::from("w-1".to_string()),
            name: "W".into(),
            description: String::new(),
            is_starter: false,
            created_at: 0,
            updated_at: 0,
            schedule: None,
        })
        .expect("add the workflow");
    ctx.projects
        .add(Project {
            id: ProjectId::new("p-1"),
            name: "demeteo".into(),
            compute_type: "local".into(),
            remote_host: None,
            status: "idle".into(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .expect("add the project");
}

fn seed_gate(ctx: &AppContext) {
    seed_run(ctx);
    ctx.features
        .step_create(StepExecution {
            id: StepExecutionId::from(GATE_STEP.to_string()),
            feature_id: FeatureId::from(FEATURE.to_string()),
            step_id: StepId::from("s-gate".to_string()),
            step_index: 0,
            step_kind: "gate".into(),
            status: "awaiting_gate".into(),
            cost_usd: None,
            tokens: None,
            wall_clock_secs: None,
            artifact_path: None,
            artifact_paths: Vec::new(),
            error_message: None,
            iteration_count: 0,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            last_failure_fingerprint: None,
            created_at: 40,
            updated_at: 40,
        })
        .expect("add the gate step");
    ctx.gates
        .create(GateDecision {
            id: GateDecisionId::new("gd-1"),
            step_execution_id: StepExecutionId::new(GATE_STEP),
            decision: None,
            feedback: None,
            created_at: 40,
            auto_approved: false,
        })
        .expect("add the undecided gate");
}

fn kinds_of(events: &[RunEvent]) -> Vec<(&str, RunEventKind)> {
    events
        .iter()
        .map(|event| (event.feature_id.as_str(), event.kind))
        .collect()
}

/// D1: the Hub cannot start a run on a runner. The id is spent and the start
/// refused `unknown_kind` before either attachment is fetched, since a fetch
/// deletes the Hub's copy.
#[tokio::test]
async fn a_signed_start_to_a_runner_is_unknown_kind_and_fetches_nothing() {
    let runner = Runner::start(
        "start",
        policy(),
        Vec::new(),
        ScriptedGatePresenter::refusing_all(),
        seed_project,
    )
    .await;
    let attachment = |id: &str| AttachmentRef {
        id: id.into(),
        size: 12,
        sha256: "0".repeat(64),
    };
    let start = RequestPayload::StartFeature(StartFeature {
        project_id: "p-1".into(),
        workflow_id: "w-1".into(),
        title: "From the Hub".into(),
        description: "Started from a phone".into(),
        agent_kind: None,
        model: None,
        effort: None,
        step_overrides: Vec::new(),
        max_budget_cents: Some(250),
        max_wall_clock_secs: None,
        attachments: vec![
            attachment("0b5e4c1a-2f7d-4e8b-9a3c-6d1f0e2b7a94"),
            attachment("1c6f5d2b-3a8e-4f9c-8b4d-7e2a1f3c8ba5"),
        ],
        runner: None,
    });
    let text = runner.signed("r-start", start, now_secs() + 600).await;

    let outcome = runner.deliver("r-start", text).await;

    assert_eq!(runner.origin.connections.load(Ordering::SeqCst), 0);
    assert_eq!(
        outcome,
        RequestOutcome::Refused {
            reason: RefusalReason::UnknownKind
        }
    );
    assert_eq!(
        runner.last_log(),
        (
            LogKind::StartFeature,
            LogOutcome::Refused,
            Some("unknown_kind".into())
        )
    );
    assert!(runner.executor.cancels.lock().expect("lock").is_empty());
    runner.assert_untouched();
}

/// D2/D6: a cancel naming the Feature id the runner reports lands in the
/// runner's own control, which ends the row and logs `cancelled`. Named by
/// the run id instead, the cancel finds nothing. The `cancelled` row is the
/// desktop's lifecycle event and is not on the Hub's allowlist; the Hub
/// hears of the cancel from the `feature_status` row the run's driver
/// writes as it stops, which this executor double has no driver to write.
#[tokio::test]
async fn a_signed_cancel_by_feature_id_ends_the_runners_run() {
    let runner = Runner::start(
        "cancel",
        policy(),
        Vec::new(),
        ScriptedGatePresenter::refusing_all(),
        seed_run,
    )
    .await;
    let cancel = |feature_id: &str| {
        RequestPayload::CancelFeature(CancelFeature {
            feature_id: feature_id.into(),
        })
    };

    let by_run = runner
        .signed("r-cancel-run", cancel(RUN), now_secs() + 600)
        .await;
    assert_eq!(
        runner.deliver("r-cancel-run", by_run).await,
        RequestOutcome::Refused {
            reason: RefusalReason::UnknownFeature
        }
    );
    assert!(runner.executor.cancels.lock().expect("lock").is_empty());

    let by_feature = runner
        .signed("r-cancel", cancel(FEATURE), now_secs() + 600)
        .await;
    assert_eq!(
        runner.deliver("r-cancel", by_feature).await,
        RequestOutcome::Accepted
    );

    assert_eq!(*runner.executor.cancels.lock().expect("lock"), [FEATURE]);
    let row = runner
        .ctx
        .runner_runs
        .get(RUN)
        .expect("read the run")
        .expect("the run");
    assert_eq!(row.status, "cancelled");
    assert_eq!(
        runner.last_log(),
        (LogKind::CancelFeature, LogOutcome::Accepted, None)
    );
    let logged: Vec<String> = runner
        .ctx
        .run_events
        .list_since(RUN, 0)
        .expect("read the run log")
        .into_iter()
        .map(|row| row.kind)
        .collect();
    assert_eq!(logged, ["feature_status", "cancelled"]);
    let events = runner
        .sent_until(|events: &[RunEvent]| !events.is_empty())
        .await;
    assert_eq!(kinds_of(&events), [(FEATURE, RunEventKind::FeatureStarted)]);
    runner.assert_untouched();
}

/// A Feature the runner holds with no run under it has nothing to cancel,
/// which is the Feature's run being unknown here, not a malformed request.
#[tokio::test]
async fn a_signed_cancel_of_a_feature_with_no_run_is_unknown_feature() {
    let runner = Runner::start(
        "cancel-no-run",
        policy(),
        Vec::new(),
        ScriptedGatePresenter::refusing_all(),
        |ctx| {
            seed_project(ctx);
            ctx.features.add(feature(FEATURE)).expect("add the feature");
        },
    )
    .await;
    let cancel = RequestPayload::CancelFeature(CancelFeature {
        feature_id: FEATURE.into(),
    });
    let text = runner.signed("r-cancel", cancel, now_secs() + 600).await;

    assert_eq!(
        runner.deliver("r-cancel", text).await,
        RequestOutcome::Refused {
            reason: RefusalReason::UnknownFeature
        }
    );
    assert_eq!(
        runner.last_log(),
        (
            LogKind::CancelFeature,
            LogOutcome::Refused,
            Some("unknown_feature".into())
        )
    );
    assert!(runner.executor.cancels.lock().expect("lock").is_empty());
    runner.assert_untouched();
}

/// A gate of a run the runner owns, named by the Feature id it reports the
/// run under, is decided: the runner has no mirror row, so it is neither
/// `runner_owned` nor, under that id, `unknown_feature`. The run's rows and
/// its snapshot entry go out under that id and never under the run id.
#[tokio::test]
async fn a_signed_gate_decision_on_a_runners_run_by_feature_id_is_decided() {
    let passkey = SoftAuthenticator::new().with_sign_count(7);
    let runner = Runner::start(
        "gate",
        policy(),
        vec![PinnedPasskey {
            credential_id: passkey.credential_id(),
            public_key: base64url_encode(&passkey.public_key_cose()),
        }],
        ScriptedGatePresenter::expecting(GATE_STEP, "approve", None),
        seed_gate,
    )
    .await;
    let expires_at = now_secs() + 600;
    let challenge = gate_challenge_digest(
        INSTANCE,
        FEATURE,
        GATE_STEP,
        Decision::Approve,
        "r-gate",
        expires_at,
    )
    .expect("encode the gate challenge");
    let gate = RequestPayload::GateDecision(GatePayload {
        feature_id: FEATURE.into(),
        step_execution_id: GATE_STEP.into(),
        decision: Decision::Approve,
        feedback: None,
        assertion: passkey.assert(&runner.origin.rp(), &challenge),
    });
    let text = runner.signed("r-gate", gate, expires_at).await;

    let outcome = runner.deliver("r-gate", text).await;

    assert_eq!(outcome, RequestOutcome::Accepted);
    assert_eq!(
        runner.presenter.calls(),
        [(GATE_STEP.to_owned(), "approve".to_owned(), None)]
    );
    assert_eq!(
        runner.last_log(),
        (LogKind::GateDecision, LogOutcome::Accepted, None)
    );
    let events = runner
        .sent_until(|events: &[RunEvent]| !events.is_empty())
        .await;
    assert_eq!(kinds_of(&events), [(FEATURE, RunEventKind::FeatureStarted)]);
    let reported: BTreeSet<String> = runner
        .sent_until(|snapshots: &[Snapshot]| !snapshots.is_empty())
        .await
        .into_iter()
        .flat_map(|snapshot| snapshot.features.into_iter().map(|feature| feature.id))
        .collect();
    assert_eq!(reported, BTreeSet::from([FEATURE.to_owned()]));
    let sent: Vec<Value> = runner
        .log
        .sent()
        .iter()
        .filter_map(|text| serde_json::from_str(text).ok())
        .collect();
    assert!(
        !sent.iter().any(|frame| frame.to_string().contains(RUN)),
        "nothing names the run id: {sent:?}"
    );
    assert!(runner.executor.cancels.lock().expect("lock").is_empty());
    runner.assert_untouched();
}

#![cfg(target_os = "linux")]
//! The Hub's `/v1` API, asserted from the desktop package: this file is the
//! evidence a Demeteo run has for it. The header of
//! `src-tauri/tests/hub_pairing.rs` says why that evidence has to live here
//! and not beside the code.

use demeteo_core::adapters::run_event_log::run_event_record;
use demeteo_core::domain::hub_socket::hub_event;
use demeteo_core::domain::ids::{FeatureId, StepExecutionId};
use demeteo_core::ports::notification::DomainEvent;
use demeteo_core::ports::run_events::RunEvent as RunEventRow;
use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::api::dispatch::{
    cancel, dispatch, set_step_assignment, AssignmentBody, DispatchBody, EnqueuedView,
};
use demeteo_hub::api::fleet::{fleet, instance_detail};
use demeteo_hub::api::gates::{
    gate_challenge, gate_decision, GateChallengeBody, GateChallengeView, GateDecisionBody,
};
use demeteo_hub::api::request_status::request_status;
use demeteo_hub::api::run_state::fold;
use demeteo_hub::api::runs::{run_detail, runs, RunsFilter};
use demeteo_hub::api::testing::MemoryFleet;
use demeteo_hub::api::{ApiContext, ApiStore, MirrorEvent};
use demeteo_hub::attachments::AttachmentSettings;
use demeteo_hub::auth::ceremonies::{Ceremonies, Ceremony};
use demeteo_hub::auth::challenge::{base64url_decode, base64url_encode};
use demeteo_hub::auth::handlers::ApiError;
use demeteo_hub::auth::testing::{answered, SoftAuthenticator};
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::pairing::signing_key::SigningKey;
use demeteo_hub::protocol::{
    CancelFeature, Decision, Effort, GateDecision, ProjectSnapshot, RefusalReason, RequestOutcome,
    RequestPayload, RunEvent, RunEventKind, RunEventPayload, RunShapeSettings, Scope,
    SetStepAssignment, Snapshot, StartFeature, StepOverride, WebAuthnAssertion,
    WorkflowVersionSnapshot,
};
use demeteo_hub::socket::connections::Connections;
use demeteo_hub::socket::port::SocketStore;
use demeteo_hub::socket::requests::{self, EnqueueError, EnqueueRefusal};
use demeteo_hub::store::requests::RequestStatus;
use demeteo_hub::store::{InstanceId, StoreError, UserId};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SIGNING_KEY: &str = "demeteo-hub-signing";
const NOW: u64 = 1_700_000_000;
const ALL_SCOPES: [Scope; 3] = [Scope::Read, Scope::Spend, Scope::Gates];

#[tokio::test]
async fn a_scope_refusal_names_the_scope_and_nothing_else() {
    for (scope, name) in [
        (Scope::Read, "read"),
        (Scope::Spend, "spend"),
        (Scope::Gates, "gates"),
    ] {
        let refused = EnqueueError::Refused(EnqueueRefusal::ScopeNotGranted(scope));
        let expected = (409, json!({"error": "scope_not_granted", "scope": name}));

        assert_eq!(answered(ApiError::from(refused)).await, expected, "{name}");
        assert_eq!(
            answered(ApiError::scope_not_granted(scope)).await,
            expected,
            "{name}"
        );
    }
}

#[tokio::test]
async fn an_instance_the_store_does_not_hold_is_not_found() {
    let missing = EnqueueError::Store(StoreError::NoSuchInstance);

    assert_eq!(
        answered(ApiError::from(missing)).await,
        (404, json!({"error": "not_found"}))
    );
}

#[tokio::test]
async fn an_error_with_no_scope_is_its_code_alone() {
    assert_eq!(
        answered(ApiError::UNAUTHENTICATED).await,
        (401, json!({"error": "unauthenticated"}))
    );
}

/// The desktop's own `gate_required` row through the Hub's own fold: a
/// pending Gate must name its Step, or hub-web has no node to hang the
/// decision on. Each half passes against a hand-written payload, which is how
/// a gate that named no Step shipped.
#[test]
fn a_gate_the_desktop_records_is_pending_on_its_step_at_the_hub() {
    let record = run_event_record(&DomainEvent::GateRequired {
        feature_id: FeatureId::from("f-1".to_string()),
        step_execution_id: StepExecutionId::new("se-7"),
        step_id: "s-gate".into(),
    })
    .expect("gate_required is recorded");
    let row = RunEventRow {
        offset: 1,
        run_id: record.feature_id,
        kind: record.kind.into(),
        payload_json: Some(record.payload.to_string()),
        created_at: 1_700_000_000_000,
    };

    let state = fold(&[hub_event(&row).expect("allowlisted row")]);

    assert_eq!(state.pending_gates.len(), 1);
    assert_eq!(state.pending_gates[0].step_id.as_deref(), Some("s-gate"));
    assert_eq!(state.pending_gates[0].step_execution_id, "se-7");
}

fn started(feature_id: &str) -> RunEvent {
    RunEvent {
        v: 1,
        feature_id: feature_id.to_owned(),
        offset: 1,
        kind: RunEventKind::FeatureStarted,
        occurred_at: NOW,
        payload: RunEventPayload {
            step_id: None,
            step_execution_id: None,
            ticket_id: None,
        },
    }
}

fn with_project(id: &str) -> Snapshot {
    Snapshot {
        v: 1,
        projects: vec![ProjectSnapshot {
            id: id.to_owned(),
            name: id.to_owned(),
            remote_url: None,
        }],
        workflows: Vec::new(),
        run_shape: Vec::new(),
    }
}

/// Two tenants, each with one instance holding a run, a snapshot and a
/// request queued through the fleet's own `requests_of`.
struct TwoTenants {
    fleet: MemoryFleet,
    a: UserId,
    a_instance: InstanceId,
    b: UserId,
    b_instance: InstanceId,
}

impl TwoTenants {
    async fn new() -> Self {
        let fleet = MemoryFleet::new();
        let (a, b) = (fleet.tenant(), fleet.tenant());
        let a_instance = fleet.paired(a, &ALL_SCOPES);
        let b_instance = fleet.paired(b, &ALL_SCOPES);
        for (user, instance, feature) in [(a, a_instance, "f-a"), (b, b_instance, "f-b")] {
            fleet.mirrored(instance, started(feature)).await;
            fleet.snapshot_sent(instance, with_project(feature)).await;
            enqueue(&fleet, user, instance, feature).await.unwrap();
        }
        Self {
            fleet,
            a,
            a_instance,
            b,
            b_instance,
        }
    }
}

async fn enqueue(
    fleet: &MemoryFleet,
    user: UserId,
    instance: InstanceId,
    feature_id: &str,
) -> Result<requests::Enqueued, EnqueueError> {
    let store = fleet
        .requests_of(user, instance)
        .await
        .expect("a store to queue through");
    requests::enqueued(
        &*store,
        &MemoryKeys::with_keys([SIGNING_KEY]),
        &SigningKey::new(SIGNING_KEY),
        &Connections::default(),
        RequestPayload::CancelFeature(CancelFeature {
            feature_id: feature_id.to_owned(),
        }),
        NOW,
    )
    .await
}

#[tokio::test]
async fn a_request_queued_as_a_minted_id_carries_that_id_and_expiry() {
    let fleet = MemoryFleet::new();
    let user = fleet.tenant();
    let instance = fleet.paired(user, &[Scope::Spend]);
    let store = fleet.requests_of(user, instance).await.unwrap();
    let minted = requests::Minted {
        id: [9; 16],
        expires_at: NOW + 42,
    };

    let queued = requests::enqueued_as(
        &*store,
        &MemoryKeys::with_keys([SIGNING_KEY]),
        &SigningKey::new(SIGNING_KEY),
        &Connections::default(),
        minted,
        RequestPayload::CancelFeature(CancelFeature {
            feature_id: "f-1".to_owned(),
        }),
        NOW,
    )
    .await
    .expect("spend is granted");

    let held = fleet.requests(instance);
    assert_eq!(held.len(), 1);
    let row = &held[0];
    assert_eq!(row.id, [9; 16]);
    assert_eq!(row.envelope.request_id, base64url_encode(&[9; 16]));
    assert_eq!(row.envelope.expires_at, NOW + 42);
    assert_eq!(row.expires_at, NOW + 42);
    assert_eq!(row.envelope.issued_at, NOW);
    assert_eq!((queued.id, queued.expires_at), ([9; 16], NOW + 42));
}

#[tokio::test]
async fn a_tenant_reads_its_own_instance_and_nothing_of_another() {
    let t = TwoTenants::new().await;
    let fleet = &t.fleet;

    let mine = fleet.instances(t.a).await.unwrap();
    assert_eq!(
        mine.iter().map(|instance| instance.id).collect::<Vec<_>>(),
        [t.a_instance]
    );
    assert_eq!(mine[0].user_id, t.a);
    assert!(fleet.instance(t.a, t.a_instance).await.unwrap().is_some());
    assert_eq!(
        fleet.mirror(t.a, t.a_instance, None).await.unwrap().len(),
        1
    );
    assert!(fleet.snapshot(t.a, t.a_instance).await.unwrap().is_some());

    assert!(fleet.instance(t.a, t.b_instance).await.unwrap().is_none());
    assert!(fleet
        .mirror(t.a, t.b_instance, None)
        .await
        .unwrap()
        .is_empty());
    assert!(fleet
        .mirror(t.a, t.b_instance, Some("f-b"))
        .await
        .unwrap()
        .is_empty());
    assert!(fleet.snapshot(t.a, t.b_instance).await.unwrap().is_none());
    let b_request = fleet.requests(t.b_instance)[0].id;
    assert!(fleet
        .request(t.a, t.b_instance, b_request)
        .await
        .unwrap()
        .is_none());
    assert!(fleet
        .request(t.a, t.a_instance, b_request)
        .await
        .unwrap()
        .is_none());

    let never_paired = InstanceId::generate();
    assert!(fleet.instance(t.a, never_paired).await.unwrap().is_none());
    assert!(fleet
        .mirror(t.a, never_paired, None)
        .await
        .unwrap()
        .is_empty());
    assert!(fleet.snapshot(t.a, never_paired).await.unwrap().is_none());
    assert!(fleet.instances(fleet.tenant()).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_request_queued_through_requests_of_is_readable_by_its_owner_only() {
    let t = TwoTenants::new().await;
    let fleet = &t.fleet;
    let queued = enqueue(fleet, t.a, t.a_instance, "f-a-2").await.unwrap();

    let held = fleet.requests(t.a_instance);
    assert_eq!(held.len(), 2);
    assert_eq!(held[1].id, queued.id);

    let read = fleet
        .request(t.a, t.a_instance, queued.id)
        .await
        .unwrap()
        .expect("the owner reads its request");
    assert_eq!(read.id, queued.id);
    assert_eq!(read.kind, "cancel_feature");
    assert_eq!(read.status, RequestStatus::Queued);
    assert_eq!(read.deliveries, 0);
    assert_eq!(read.expires_at, queued.expires_at);
    assert_eq!(read.answered_at, None);

    assert!(fleet
        .request(t.b, t.a_instance, queued.id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(fleet.requests(t.b_instance).len(), 1);
}

#[tokio::test]
async fn queueing_for_another_tenants_instance_is_no_such_instance_and_queues_nothing() {
    let t = TwoTenants::new().await;
    let fleet = &t.fleet;

    for instance in [t.b_instance, InstanceId::generate()] {
        let refused = enqueue(fleet, t.a, instance, "f-b").await;
        assert!(
            matches!(
                refused,
                Err(EnqueueError::Store(StoreError::NoSuchInstance))
            ),
            "{refused:?}"
        );
    }
    assert_eq!(fleet.requests(t.b_instance).len(), 1);
}

#[tokio::test]
async fn what_a_session_writes_through_the_fleets_store_is_what_the_api_reads() {
    let t = TwoTenants::new().await;
    let fleet = &t.fleet;
    let queued = enqueue(fleet, t.a, t.a_instance, "f-a-2").await.unwrap();

    let store = fleet.store(t.a, t.a_instance);
    store.request_sent(queued.id, NOW + 1).await.unwrap();

    let read = fleet.request(t.a, t.a_instance, queued.id).await.unwrap();
    assert_eq!(read.map(|record| record.deliveries), Some(1));
}

#[tokio::test]
async fn greeting_and_scopes_are_what_the_instance_row_reads() {
    let fleet = MemoryFleet::new();
    let user = fleet.tenant();
    let instance = fleet.paired(user, &ALL_SCOPES);

    let paired = fleet.instance(user, instance).await.unwrap().unwrap();
    assert_eq!(paired.connection_id, None);
    assert_eq!(paired.granted_scopes, ["read", "spend", "gates"]);

    fleet.greeted(instance, "linux", "x86_64", "1.4.0", true);
    fleet.scopes(instance, &[Scope::Gates]);
    let greeted = fleet.instance(user, instance).await.unwrap().unwrap();
    assert_eq!(
        (
            greeted.os.as_str(),
            greeted.arch.as_str(),
            greeted.version.as_str()
        ),
        ("linux", "x86_64", "1.4.0")
    );
    assert!(greeted.connection_id.is_some());
    assert_eq!(greeted.granted_scopes, ["gates"]);

    fleet.greeted(instance, "linux", "x86_64", "1.4.0", false);
    let gone = fleet.instance(user, instance).await.unwrap().unwrap();
    assert_eq!(gone.connection_id, None);
}

#[tokio::test]
async fn an_unreadable_mirror_row_reads_as_unreadable_under_its_key() {
    let fleet = MemoryFleet::new();
    let user = fleet.tenant();
    let instance = fleet.paired(user, &ALL_SCOPES);
    fleet.mirrored(instance, started("f-1")).await;
    fleet.mirrored_unreadable(instance, "f-1", 2);
    fleet.mirrored(instance, started("f-2")).await;

    let only_f1 = fleet.mirror(user, instance, Some("f-1")).await.unwrap();
    assert_eq!(
        only_f1,
        [
            MirrorEvent::Readable(started("f-1")),
            MirrorEvent::Unreadable {
                feature_id: "f-1".to_owned(),
                offset: 2,
            },
        ]
    );
    assert_eq!(fleet.mirror(user, instance, None).await.unwrap().len(), 3);
}

#[tokio::test]
async fn trusted_credential_ids_are_the_tenants_own() {
    let fleet = MemoryFleet::new();
    let (a, b) = (fleet.tenant(), fleet.tenant());
    fleet.trusted(a, vec!["cred-a".to_owned()]);
    fleet.trusted(b, vec!["cred-b".to_owned()]);

    assert_eq!(fleet.trusted_credential_ids(a).await.unwrap(), ["cred-a"]);
    assert_eq!(fleet.trusted_credential_ids(b).await.unwrap(), ["cred-b"]);
}

/// What an [`ApiContext`] borrows besides the store, owned by the test.
struct Parts {
    keys: MemoryKeys,
    signing_key: SigningKey,
    connections: Connections,
    ceremonies: Ceremonies,
    rp: RelyingParty,
    settings: AttachmentSettings,
}

impl Parts {
    fn new() -> Self {
        Self {
            keys: MemoryKeys::with_keys([SIGNING_KEY]),
            signing_key: SigningKey::new(SIGNING_KEY),
            connections: Connections::default(),
            ceremonies: Ceremonies::default(),
            rp: RelyingParty {
                rp_id: "hub.example".to_owned(),
                origin: "https://hub.example".to_owned(),
            },
            settings: AttachmentSettings::default(),
        }
    }

    fn over<'a>(&'a self, store: &'a MemoryFleet) -> ApiContext<'a> {
        ApiContext {
            store,
            keys: &self.keys,
            signing_key: &self.signing_key,
            connections: &self.connections,
            ceremonies: &self.ceremonies,
            rp: &self.rp,
            attachments: store,
            transit_key: SIGNING_KEY,
            settings: &self.settings,
        }
    }
}

fn event(
    feature_id: &str,
    offset: u64,
    kind: RunEventKind,
    step_id: Option<&str>,
    step_execution_id: Option<&str>,
    ticket_id: Option<&str>,
) -> RunEvent {
    RunEvent {
        v: 1,
        feature_id: feature_id.to_owned(),
        offset,
        kind,
        occurred_at: NOW + offset,
        payload: RunEventPayload {
            step_id: step_id.map(str::to_owned),
            step_execution_id: step_execution_id.map(str::to_owned),
            ticket_id: ticket_id.map(str::to_owned),
        },
    }
}

/// The spec's three Features, on one table-global offset sequence as the
/// desktop numbers them: F1 is running with gate `e1` waiting, F2 completed,
/// F3 running with its gate `e2` decided. Beside them, an unreadable row of
/// F1 and one of a Feature with no readable event at all.
async fn three_features(fleet: &MemoryFleet, instance: InstanceId) {
    use RunEventKind::*;
    for event in [
        event("F1", 1, FeatureStarted, None, None, None),
        event("F2", 2, FeatureStarted, None, None, None),
        event("F1", 3, StepStarted, Some("s1"), None, None),
        event("F1", 4, TicketStarted, Some("s1"), None, Some("t1")),
        event("F3", 5, FeatureStarted, None, None, None),
        event("F1", 6, TicketCompleted, Some("s1"), None, Some("t1")),
        event("F2", 7, FeatureCompleted, None, None, None),
        event("F1", 8, TicketStarted, Some("s1"), None, Some("t2")),
        event("F3", 9, GatePending, Some("s-review"), Some("e2"), None),
        event("F1", 10, GatePending, Some("s-gate"), Some("e1"), None),
        event("F3", 11, GateDecided, Some("s-review"), Some("e2"), None),
    ] {
        fleet.mirrored(instance, event).await;
    }
    fleet.mirrored_unreadable(instance, "F1", 12);
    fleet.mirrored_unreadable(instance, "F4", 13);
}

fn two_projects() -> Snapshot {
    Snapshot {
        v: 1,
        projects: vec![
            ProjectSnapshot {
                id: "p-1".to_owned(),
                name: "demeteo".to_owned(),
                remote_url: Some("https://git.example/demeteo.git".to_owned()),
            },
            ProjectSnapshot {
                id: "p-2".to_owned(),
                name: "scratch".to_owned(),
                remote_url: None,
            },
        ],
        workflows: Vec::new(),
        run_shape: Vec::new(),
    }
}

#[tokio::test]
async fn the_fleet_counts_live_runs_and_lists_the_one_gate_that_needs_you() {
    let fleet = MemoryFleet::new();
    let parts = Parts::new();
    let user = fleet.tenant();
    let instance = fleet.paired(user, &ALL_SCOPES);
    fleet.greeted(instance, "linux", "x86_64", "1.4.0", true);
    let _seated = parts.connections.register(instance).unwrap();
    three_features(&fleet, instance).await;

    let view = fleet_json(&parts.over(&fleet), user).await;

    let id = instance.0.to_string();
    assert_eq!(
        view,
        json!({"instances": [{
            "id": id,
            "label": "paired-name",
            "present": true,
            "os": "linux",
            "arch": "x86_64",
            "version": "1.4.0",
            "scopes": ["read", "spend", "gates"],
            "read_granted": true,
            "last_seen": null,
            "live_runs": 2,
            "needs_you": [{
                "instance_id": id,
                "feature_id": "F1",
                "step_id": "s-gate",
                "step_execution_id": "e1",
                "since": NOW + 10,
            }],
        }]})
    );
}

#[tokio::test]
async fn an_instance_shows_its_projects_and_live_runs_and_nothing_unreported() {
    let fleet = MemoryFleet::new();
    let parts = Parts::new();
    let user = fleet.tenant();
    let instance = fleet.paired(user, &ALL_SCOPES);
    three_features(&fleet, instance).await;
    fleet.snapshot_sent(instance, two_projects()).await;

    let detail = detail_json(&parts.over(&fleet), user, &instance.0.to_string()).await;

    let id = instance.0.to_string();
    assert_eq!(
        detail["projects"],
        json!([
            {"id": "p-1", "name": "demeteo", "remote_url": "https://git.example/demeteo.git"},
            {"id": "p-2", "name": "scratch", "remote_url": null},
        ])
    );
    assert_eq!(member(&detail, "harnesses"), &Value::Null);
    assert_eq!(
        detail["live_runs"],
        json!([
            {"instance_id": id, "feature_id": "F1", "status": "running",
             "started_at": NOW + 1, "updated_at": NOW + 10, "cost_cents": null},
            {"instance_id": id, "feature_id": "F3", "status": "running",
             "started_at": NOW + 5, "updated_at": NOW + 11, "cost_cents": null},
        ])
    );
    assert_eq!(detail["id"], json!(id));
    assert_eq!(detail["read_granted"], json!(true));
    assert_eq!(detail["scopes"], json!(["read", "spend", "gates"]));
    assert_eq!(detail["present"], json!(false));
}

/// Strings the instance holds that the browser must never read: a node's
/// prompt and command, an edge's guard, the v1 step list and the note.
const WITHHELD: [&str; 5] = [
    "SENTINEL-PROMPT",
    "SENTINEL-COMMAND",
    "SENTINEL-GUARD",
    "SENTINEL-STEPS",
    "SENTINEL-NOTE",
];

fn workflow_version(
    workflow_id: &str,
    version: u32,
    definition_json: Option<String>,
) -> WorkflowVersionSnapshot {
    WorkflowVersionSnapshot {
        id: format!("{workflow_id}@{version}"),
        workflow_id: workflow_id.to_owned(),
        version,
        steps_json: r#"[{"prompt_template":"SENTINEL-STEPS"}]"#.to_owned(),
        definition_json,
        note: Some("SENTINEL-NOTE".to_owned()),
        created_at: 1_700_000_000,
    }
}

fn definition(name: &str, title: &str) -> String {
    json!({
        "schema_version": 2,
        "id": "wf-a",
        "name": name,
        "nodes": [
            {"id": "s-plan", "type": "agent", "type_version": 1, "title": title,
             "config": {"prompt_template": "SENTINEL-PROMPT"},
             "retry": {"verdict": {"max_attempts": 2}}, "join": "all_success",
             "position": {"x": 10.0, "y": 20.5}},
            {"id": "s-build", "type": "command", "title": "Build",
             "config": {"command": "SENTINEL-COMMAND"}},
        ],
        "edges": [{"from": "s-plan", "to": "s-build", "when": "SENTINEL-GUARD"}],
        "defaults": {"join": "all_done"},
    })
    .to_string()
}

/// [`two_projects`] with three versions of `wf-a`, the latest not last; a
/// `wf-b` from before schema v2; a `wf-c` whose document is no JSON; and one
/// project's run shape.
fn two_projects_with_workflows() -> Snapshot {
    Snapshot {
        workflows: vec![
            workflow_version("wf-a", 1, Some(definition("Alpha v1", "Plan v1"))),
            workflow_version("wf-a", 3, Some(definition("Alpha", "Plan"))),
            workflow_version("wf-a", 2, Some(definition("Alpha v2", "Plan v2"))),
            workflow_version("wf-b", 1, None),
            workflow_version("wf-c", 4, Some("not json".to_owned())),
        ],
        run_shape: vec![RunShapeSettings {
            project_id: "p-1".to_owned(),
            default_agent_kind: Some("claude-code".to_owned()),
            default_model: Some("opus".to_owned()),
            default_effort: Some(Effort::High),
            default_workflow_id: Some("wf-a".to_owned()),
            default_loop_iterations: Some(3),
            default_max_budget_cents: Some(500),
        }],
        ..two_projects()
    }
}

#[tokio::test]
async fn an_instance_offers_each_workflow_once_at_its_latest_version_as_a_graph_alone() {
    let fleet = MemoryFleet::new();
    let parts = Parts::new();
    let user = fleet.tenant();
    let instance = fleet.paired(user, &ALL_SCOPES);
    fleet
        .snapshot_sent(instance, two_projects_with_workflows())
        .await;

    let detail = detail_json(&parts.over(&fleet), user, &instance.0.to_string()).await;

    assert_eq!(
        detail["workflows"],
        json!([
            {"workflow_id": "wf-a", "version": 3, "name": "Alpha", "graph": {
                "nodes": [
                    {"id": "s-plan", "type": "agent", "title": "Plan",
                     "position": {"x": 10.0, "y": 20.5}},
                    {"id": "s-build", "type": "command", "title": "Build", "position": null},
                ],
                "edges": [{"from": "s-plan", "to": "s-build"}],
            }},
            {"workflow_id": "wf-b", "version": 1, "name": "wf-b", "graph": null},
            {"workflow_id": "wf-c", "version": 4, "name": "wf-c", "graph": null},
        ])
    );
    assert_eq!(
        detail["run_shape"],
        json!([{
            "project_id": "p-1",
            "default_agent_kind": "claude-code",
            "default_model": "opus",
            "default_effort": "high",
            "default_workflow_id": "wf-a",
            "default_max_budget_cents": 500,
        }])
    );
    assert_eq!(detail["projects"][0]["id"], json!("p-1"));
}

#[tokio::test]
async fn an_instances_workflows_carry_no_prompt_command_guard_or_step_list() {
    let fleet = MemoryFleet::new();
    let parts = Parts::new();
    let user = fleet.tenant();
    let instance = fleet.paired(user, &ALL_SCOPES);
    fleet
        .snapshot_sent(instance, two_projects_with_workflows())
        .await;

    let detail = detail_json(&parts.over(&fleet), user, &instance.0.to_string()).await;

    let text = detail.to_string();
    assert!(text.contains("s-plan"), "the graph is shown: {text}");
    for sentinel in WITHHELD {
        assert!(
            !text.contains(sentinel),
            "{sentinel} reached the browser: {text}"
        );
    }
}

#[tokio::test]
async fn an_instance_with_no_readable_snapshot_offers_no_workflows_and_no_run_shape() {
    let fleet = MemoryFleet::new();
    let parts = Parts::new();
    let user = fleet.tenant();
    let instance = fleet.paired(user, &ALL_SCOPES);
    let api = parts.over(&fleet);

    let unsent = detail_json(&api, user, &instance.0.to_string()).await;
    fleet.snapshot_unreadable(instance);
    let unreadable = detail_json(&api, user, &instance.0.to_string()).await;

    for detail in [unsent, unreadable] {
        assert_eq!(member(&detail, "workflows"), &json!([]));
        assert_eq!(member(&detail, "run_shape"), &json!([]));
        assert_eq!(member(&detail, "projects"), &json!([]));
    }
}

#[tokio::test]
async fn an_instance_without_read_withholds_its_workflows_and_run_shape() {
    let fleet = MemoryFleet::new();
    let parts = Parts::new();
    let user = fleet.tenant();
    let instance = fleet.paired(user, &[Scope::Spend, Scope::Gates]);
    fleet
        .snapshot_sent(instance, two_projects_with_workflows())
        .await;

    let detail = detail_json(&parts.over(&fleet), user, &instance.0.to_string()).await;

    assert_eq!(member(&detail, "workflows"), &Value::Null);
    assert_eq!(member(&detail, "run_shape"), &Value::Null);
}

#[tokio::test]
async fn an_instance_without_read_is_listed_with_its_reported_fields_withheld() {
    let fleet = MemoryFleet::new();
    let parts = Parts::new();
    let user = fleet.tenant();
    let instance = fleet.paired(user, &[Scope::Spend, Scope::Gates]);
    three_features(&fleet, instance).await;
    fleet.snapshot_sent(instance, two_projects()).await;
    let api = parts.over(&fleet);

    let listed = &fleet_json(&api, user).await["instances"][0];
    assert_eq!(listed["read_granted"], json!(false));
    assert_eq!(listed["scopes"], json!(["spend", "gates"]));
    assert_eq!(member(listed, "live_runs"), &Value::Null);
    assert_eq!(member(listed, "needs_you"), &Value::Null);

    let detail = detail_json(&api, user, &instance.0.to_string()).await;
    assert_eq!(detail["read_granted"], json!(false));
    assert_eq!(member(&detail, "projects"), &Value::Null);
    assert_eq!(member(&detail, "live_runs"), &Value::Null);
    assert_eq!(detail["os"], json!("paired-os"));
}

#[tokio::test]
async fn an_instance_is_present_only_through_its_row_and_a_live_socket_together() {
    let fleet = MemoryFleet::new();
    let parts = Parts::new();
    let user = fleet.tenant();
    let instance = fleet.paired(user, &ALL_SCOPES);
    let api = parts.over(&fleet);
    let id = instance.0.to_string();
    let present = |view: Value| view["present"].clone();

    fleet.greeted(instance, "linux", "x86_64", "1.4.0", true);
    assert_eq!(present(detail_json(&api, user, &id).await), json!(false));

    let seated = parts.connections.register(instance).unwrap();
    assert_eq!(present(detail_json(&api, user, &id).await), json!(true));
    assert_eq!(
        fleet_json(&api, user).await["instances"][0]["present"],
        json!(true)
    );

    fleet.greeted(instance, "linux", "x86_64", "1.4.0", false);
    assert_eq!(present(detail_json(&api, user, &id).await), json!(false));

    fleet.greeted(instance, "linux", "x86_64", "1.4.0", true);
    parts.connections.release(instance, seated.connection);
    assert_eq!(present(detail_json(&api, user, &id).await), json!(false));
}

#[tokio::test]
async fn another_tenants_instance_reads_as_one_that_does_not_exist() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);

    for instance_id in [
        t.b_instance.0.to_string(),
        InstanceId::generate().0.to_string(),
        "not-an-instance".to_owned(),
    ] {
        let refused = instance_detail(&api, t.a, &instance_id, NOW)
            .await
            .expect_err("not the tenant's");
        assert_eq!(
            answered(refused).await,
            (404, json!({"error": "not_found"})),
            "{instance_id}"
        );
    }

    let listed = fleet_json(&api, t.a).await;
    let ids: Vec<&Value> = listed["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|instance| &instance["id"])
        .collect();
    assert_eq!(ids, [&json!(t.a_instance.0.to_string())]);
    assert!(!listed.to_string().contains(&t.b_instance.0.to_string()));
    // Not a substring search: A's instance id is random hex, which spells
    // `f-b` across a hyphen in about one run in fifty.
    let mut strings = Vec::new();
    strings_of(&listed, &mut strings);
    assert!(!strings.iter().any(|s| s == "f-b"), "{listed}");
}

/// The tenant's three instances: `x` holds the spec's three Features, `y` one
/// running Feature of its own and `z`, which withholds `read`, one more.
struct ThreeInstances {
    fleet: MemoryFleet,
    user: UserId,
    x: InstanceId,
    y: InstanceId,
    z: InstanceId,
}

impl ThreeInstances {
    async fn new() -> Self {
        let fleet = MemoryFleet::new();
        let user = fleet.tenant();
        let x = fleet.paired(user, &ALL_SCOPES);
        let y = fleet.paired(user, &ALL_SCOPES);
        let z = fleet.paired(user, &[Scope::Spend, Scope::Gates]);
        three_features(&fleet, x).await;
        fleet.mirrored(y, started("F-y")).await;
        fleet.mirrored(z, started("F-z")).await;
        Self {
            fleet,
            user,
            x,
            y,
            z,
        }
    }
}

fn filter(instance: Option<InstanceId>, status: Option<&str>) -> RunsFilter {
    RunsFilter {
        instance: instance.map(|instance| instance.0.to_string()),
        status: status.map(str::to_owned),
    }
}

/// Each run as `(instance, feature_id)`, in the order listed.
async fn listed(api: &ApiContext<'_>, user: UserId, filter: RunsFilter) -> Vec<(String, String)> {
    let view = runs(api, user, filter, NOW).await.expect("the runs");
    let view = serde_json::to_value(view).unwrap();
    view["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| {
            (
                run["instance_id"].as_str().unwrap().to_owned(),
                run["feature_id"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn runs_are_filtered_by_status_and_by_instance() {
    let t = ThreeInstances::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let (x, y) = (t.x.0.to_string(), t.y.0.to_string());
    let run = |instance: &str, feature: &str| (instance.to_owned(), feature.to_owned());

    assert_eq!(
        listed(&api, t.user, filter(None, Some("running"))).await,
        [run(&x, "F1"), run(&x, "F3"), run(&y, "F-y")]
    );
    assert_eq!(
        listed(&api, t.user, filter(Some(t.x), Some("running"))).await,
        [run(&x, "F1"), run(&x, "F3")]
    );
    assert_eq!(
        listed(&api, t.user, filter(None, Some("completed"))).await,
        [run(&x, "F2")]
    );
    assert_eq!(
        listed(&api, t.user, filter(Some(t.y), None)).await,
        [run(&y, "F-y")]
    );
    assert_eq!(
        listed(&api, t.user, filter(None, None)).await,
        [run(&x, "F1"), run(&x, "F2"), run(&x, "F3"), run(&y, "F-y")]
    );
    assert!(listed(&api, t.user, filter(None, Some("failed")))
        .await
        .is_empty());

    let summary = runs(&api, t.user, filter(Some(t.x), Some("completed")), NOW)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(summary).unwrap(),
        json!({"runs": [{"instance_id": x, "feature_id": "F2", "status": "completed",
            "started_at": NOW + 2, "updated_at": NOW + 7, "cost_cents": null}]})
    );
}

#[tokio::test]
async fn a_status_no_feature_can_have_is_an_invalid_request() {
    let t = ThreeInstances::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);

    for status in ["", "Running", "pending", "running "] {
        let refused = runs(&api, t.user, filter(None, Some(status)), NOW)
            .await
            .expect_err("no such status");
        assert_eq!(
            answered(refused).await,
            (400, json!({"error": "invalid_request"})),
            "{status:?}"
        );
    }
}

#[tokio::test]
async fn a_run_shows_its_steps_tickets_pending_gates_and_events_in_offset_order() {
    let t = ThreeInstances::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let x = t.x.0.to_string();

    let detail = run_detail(&api, t.user, &x, "F1", NOW)
        .await
        .expect("the run");
    let detail = serde_json::to_value(detail).unwrap();

    let at = |offset: u64| NOW + offset;
    assert_eq!(
        detail,
        json!({
            "instance_id": x,
            "feature_id": "F1",
            "status": "running",
            "workflow": null,
            "steps": [{"step_id": "s1", "state": "running"}],
            "tickets": {"completed": 1, "failed": 0, "running": 1},
            "pending_gates": [{"feature_id": "F1", "step_execution_id": "e1",
                "step_id": "s-gate", "since": at(10)}],
            "events": [
                {"offset": 1, "kind": "feature_started", "occurred_at": at(1),
                 "step_id": null, "step_execution_id": null, "ticket_id": null},
                {"offset": 3, "kind": "step_started", "occurred_at": at(3),
                 "step_id": "s1", "step_execution_id": null, "ticket_id": null},
                {"offset": 4, "kind": "ticket_started", "occurred_at": at(4),
                 "step_id": "s1", "step_execution_id": null, "ticket_id": "t1"},
                {"offset": 6, "kind": "ticket_completed", "occurred_at": at(6),
                 "step_id": "s1", "step_execution_id": null, "ticket_id": "t1"},
                {"offset": 8, "kind": "ticket_started", "occurred_at": at(8),
                 "step_id": "s1", "step_execution_id": null, "ticket_id": "t2"},
                {"offset": 10, "kind": "gate_pending", "occurred_at": at(10),
                 "step_id": "s-gate", "step_execution_id": "e1", "ticket_id": null},
            ],
        })
    );

    let f2 = run_detail(&api, t.user, &x, "F2", NOW).await.unwrap();
    let f2 = serde_json::to_value(f2).unwrap();
    assert_eq!(f2["status"], json!("completed"));
    assert_eq!(f2["pending_gates"], json!([]));
}

#[tokio::test]
async fn without_read_a_run_and_an_instances_runs_are_refused_by_name_and_nothing_is_queued() {
    let t = ThreeInstances::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    enqueue(&t.fleet, t.user, t.z, "F-z").await.unwrap();
    let queued = t.fleet.requests(t.z);
    let refusal = (409, json!({"error": "scope_not_granted", "scope": "read"}));

    let detail = run_detail(&api, t.user, &t.z.0.to_string(), "F-z", NOW)
        .await
        .expect_err("read is off");
    assert_eq!(answered(detail).await, refusal);
    let unknown = run_detail(&api, t.user, &t.z.0.to_string(), "no-such-feature", NOW)
        .await
        .expect_err("read is off");
    assert_eq!(answered(unknown).await, refusal);
    for status in [None, Some("running")] {
        let listed = runs(&api, t.user, filter(Some(t.z), status), NOW)
            .await
            .expect_err("read is off");
        assert_eq!(answered(listed).await, refusal, "{status:?}");
    }

    let everything = listed(&api, t.user, filter(None, None)).await;
    assert!(
        everything.iter().all(|(_, feature)| feature != "F-z"),
        "{everything:?}"
    );
    assert_eq!(everything.len(), 4);
    assert_eq!(t.fleet.requests(t.z), queued);
}

#[tokio::test]
async fn another_tenants_run_reads_as_one_that_does_not_exist() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let (a_instance, b_instance) = (t.a_instance.0.to_string(), t.b_instance.0.to_string());
    let random = InstanceId::generate().0.to_string();
    let not_found = (404, json!({"error": "not_found"}));

    for (instance_id, feature_id) in [
        (b_instance.as_str(), "f-b"),
        (a_instance.as_str(), "f-b"),
        (a_instance.as_str(), "no-such-feature"),
        (random.as_str(), "f-a"),
        ("not-an-instance", "f-a"),
    ] {
        let refused = run_detail(&api, t.a, instance_id, feature_id, NOW)
            .await
            .expect_err("not the tenant's");
        assert_eq!(
            answered(refused).await,
            not_found,
            "{instance_id} {feature_id}"
        );
    }
    for instance in [b_instance.clone(), random, "not-an-instance".to_owned()] {
        let filter = RunsFilter {
            instance: Some(instance.clone()),
            status: None,
        };
        let refused = runs(&api, t.a, filter, NOW)
            .await
            .expect_err("not the tenant's");
        assert_eq!(answered(refused).await, not_found, "{instance}");
    }

    let everything = runs(&api, t.a, filter(None, None), NOW).await.unwrap();
    let everything = serde_json::to_value(everything).unwrap();
    assert_eq!(
        everything,
        json!({"runs": [{"instance_id": a_instance, "feature_id": "f-a", "status": "running",
            "started_at": NOW, "updated_at": NOW, "cost_cents": null}]})
    );
    assert!(!everything.to_string().contains(&b_instance));
    assert_eq!(t.fleet.requests(t.b_instance).len(), 1);
}

async fn fleet_json(api: &ApiContext<'_>, user: UserId) -> Value {
    let view = fleet(api, user, NOW).await.expect("the fleet");
    serde_json::to_value(view).unwrap()
}

async fn detail_json(api: &ApiContext<'_>, user: UserId, instance_id: &str) -> Value {
    let view = instance_detail(api, user, instance_id, NOW)
        .await
        .expect("the instance");
    serde_json::to_value(view).unwrap()
}

/// A withheld field is answered as `null`; indexing a `Value` would read a
/// missing one as `null` too.
fn member<'a>(view: &'a Value, key: &str) -> &'a Value {
    match view.get(key) {
        Some(value) => value,
        None => panic!("`{key}` is missing from {view}"),
    }
}

/// Every field a dispatch body has, none of them at its default.
fn full_dispatch() -> Value {
    json!({
        "project_id": "p-1",
        "workflow_id": "w-1",
        "title": "Add a fleet view",
        "description": "List every instance.",
        "agent_kind": "claude-code",
        "model": "opus",
        "effort": "high",
        "step_overrides": [
            {"step_id": "s-review", "agent_kind": "codex", "model": "gpt", "effort": "low"},
            {"step_id": "s-plan"},
        ],
        "max_budget_cents": 1250,
        "max_wall_clock_secs": 3600,
        "attachment_ids": [],
    })
}

fn dispatch_body(body: Value) -> DispatchBody {
    serde_json::from_value(body).expect("a dispatch body")
}

#[tokio::test]
async fn a_dispatch_queues_one_start_feature_that_is_the_body_field_for_field() {
    let fleet = MemoryFleet::new();
    let parts = Parts::new();
    let user = fleet.tenant();
    let instance = fleet.paired(user, &[Scope::Spend]);

    let answer = dispatch(
        &parts.over(&fleet),
        user,
        &instance.0.to_string(),
        dispatch_body(full_dispatch()),
        NOW,
    )
    .await
    .expect("spend is granted");

    let queued = fleet.requests(instance);
    assert_eq!(queued.len(), 1);
    let row = &queued[0];
    assert_eq!(row.kind, "start_feature");
    assert_eq!(
        row.envelope.payload,
        RequestPayload::StartFeature(StartFeature {
            project_id: "p-1".to_owned(),
            workflow_id: "w-1".to_owned(),
            title: "Add a fleet view".to_owned(),
            description: "List every instance.".to_owned(),
            agent_kind: Some("claude-code".to_owned()),
            model: Some("opus".to_owned()),
            effort: Some(Effort::High),
            step_overrides: vec![
                StepOverride {
                    step_id: "s-review".to_owned(),
                    agent_kind: Some("codex".to_owned()),
                    model: Some("gpt".to_owned()),
                    effort: Some(Effort::Low),
                },
                StepOverride {
                    step_id: "s-plan".to_owned(),
                    agent_kind: None,
                    model: None,
                    effort: None,
                },
            ],
            max_budget_cents: Some(1250),
            max_wall_clock_secs: Some(3600),
            attachments: Vec::new(),
        })
    );
    assert_eq!(answer.request_id, base64url_encode(&row.id));
    assert_eq!(answer.request_id, row.envelope.request_id);
    assert_eq!(answer.expires_at, row.expires_at);
    assert_eq!(
        serde_json::to_value(&answer).unwrap(),
        json!({"request_id": row.envelope.request_id, "expires_at": row.expires_at})
    );
}

#[tokio::test]
async fn a_dispatch_of_only_the_required_fields_queues_the_defaults() {
    let fleet = MemoryFleet::new();
    let parts = Parts::new();
    let user = fleet.tenant();
    let instance = fleet.paired(user, &ALL_SCOPES);
    let body = json!({"project_id": "p", "workflow_id": "w", "title": "t", "description": ""});

    dispatch(
        &parts.over(&fleet),
        user,
        &instance.0.to_string(),
        dispatch_body(body),
        NOW,
    )
    .await
    .expect("spend is granted");

    let queued = fleet.requests(instance);
    assert_eq!(queued.len(), 1);
    let RequestPayload::StartFeature(start) = &queued[0].envelope.payload else {
        panic!("{:?}", queued[0].envelope.payload);
    };
    assert_eq!(
        (&start.agent_kind, &start.model, start.effort),
        (&None, &None, None)
    );
    assert!(start.step_overrides.is_empty());
    assert!(start.attachments.is_empty());
    assert_eq!(
        (start.max_budget_cents, start.max_wall_clock_secs),
        (None, None)
    );
}

#[test]
fn a_dispatch_body_with_a_field_it_does_not_have_is_not_one() {
    let mut body = full_dispatch();
    body["attachments"] = json!([]);
    assert!(serde_json::from_value::<DispatchBody>(body).is_err());
}

#[tokio::test]
async fn without_spend_a_dispatch_is_refused_by_name_and_nothing_is_queued() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    t.fleet.scopes(t.a_instance, &[Scope::Read, Scope::Gates]);
    let before = t.fleet.requests(t.a_instance);
    let mut with_attachment = full_dispatch();
    with_attachment["attachment_ids"] = json!(["att-1"]);

    for body in [full_dispatch(), with_attachment] {
        let refused = dispatch(
            &api,
            t.a,
            &t.a_instance.0.to_string(),
            dispatch_body(body),
            NOW,
        )
        .await
        .expect_err("spend is off");
        assert_eq!(
            answered(refused).await,
            (409, json!({"error": "scope_not_granted", "scope": "spend"}))
        );
    }
    assert_eq!(t.fleet.requests(t.a_instance), before);
}

#[tokio::test]
async fn a_dispatch_with_an_attachment_or_a_nul_is_refused_and_nothing_is_queued() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let before = t.fleet.requests(t.a_instance);

    let mut with_attachment = full_dispatch();
    with_attachment["attachment_ids"] = json!(["att-1"]);
    let mut nul_title = full_dispatch();
    nul_title["title"] = json!("a\u{0}b");
    let mut nul_override = full_dispatch();
    nul_override["step_overrides"][0]["model"] = json!("\u{0}");
    let mut long_project = full_dispatch();
    long_project["project_id"] = json!("p".repeat(257));

    let invalid = (400, json!({"error": "invalid_request"}));
    for (body, expected) in [
        (with_attachment, (404, json!({"error": "not_found"}))),
        (nul_title, invalid.clone()),
        (nul_override, invalid.clone()),
        (long_project, invalid),
    ] {
        let refused = dispatch(
            &api,
            t.a,
            &t.a_instance.0.to_string(),
            dispatch_body(body),
            NOW,
        )
        .await
        .expect_err("the body is refused");
        assert_eq!(answered(refused).await, expected, "{expected:?}");
    }
    assert_eq!(t.fleet.requests(t.a_instance), before);
}

#[tokio::test]
async fn a_dispatch_to_another_tenants_instance_reads_as_one_that_does_not_exist() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let b_before = t.fleet.requests(t.b_instance);
    let a_before = t.fleet.requests(t.a_instance);

    for instance_id in [
        t.b_instance.0.to_string(),
        InstanceId::generate().0.to_string(),
        "not-an-instance".to_owned(),
    ] {
        let refused = dispatch(&api, t.a, &instance_id, dispatch_body(full_dispatch()), NOW)
            .await
            .expect_err("not the tenant's");
        assert_eq!(
            answered(refused).await,
            (404, json!({"error": "not_found"})),
            "{instance_id}"
        );
    }
    assert_eq!(t.fleet.requests(t.b_instance), b_before);
    assert_eq!(t.fleet.requests(t.a_instance), a_before);
}

fn assignment_body(body: Value) -> AssignmentBody {
    serde_json::from_value(body).expect("an assignment body")
}

fn full_assignment() -> Value {
    json!({"agent_kind": "codex", "model": "gpt", "effort": "medium"})
}

#[tokio::test]
async fn a_cancel_queues_one_cancel_feature_for_the_feature_in_the_path() {
    let fleet = MemoryFleet::new();
    let parts = Parts::new();
    let user = fleet.tenant();
    let instance = fleet.paired(user, &[Scope::Spend]);

    let answer = cancel(
        &parts.over(&fleet),
        user,
        &instance.0.to_string(),
        "f-1",
        NOW,
    )
    .await
    .expect("spend is granted");

    let queued = fleet.requests(instance);
    assert_eq!(queued.len(), 1);
    let row = &queued[0];
    assert_eq!(row.kind, "cancel_feature");
    assert_eq!(
        row.envelope.payload,
        RequestPayload::CancelFeature(CancelFeature {
            feature_id: "f-1".to_owned(),
        })
    );
    assert_eq!(answer.request_id, row.envelope.request_id);
    assert_eq!(answer.expires_at, row.expires_at);
}

#[tokio::test]
async fn an_assignment_queues_one_set_step_assignment_that_is_the_path_and_body() {
    let fleet = MemoryFleet::new();
    let parts = Parts::new();
    let user = fleet.tenant();
    let instance = fleet.paired(user, &[Scope::Spend]);
    let api = parts.over(&fleet);
    let instance_id = instance.0.to_string();

    for (body, agent_kind, model, effort) in [
        (
            full_assignment(),
            Some("codex".to_owned()),
            Some("gpt".to_owned()),
            Some(Effort::Medium),
        ),
        (json!({}), None, None, None),
    ] {
        let before = fleet.requests(instance).len();
        let answer = set_step_assignment(
            &api,
            user,
            &instance_id,
            "f-1",
            "s-review",
            assignment_body(body),
            NOW,
        )
        .await
        .expect("spend is granted");

        let queued = fleet.requests(instance);
        assert_eq!(queued.len(), before + 1);
        let row = queued
            .iter()
            .find(|row| row.envelope.request_id == answer.request_id)
            .expect("the answer names the queued request");
        assert_eq!(row.kind, "set_step_assignment");
        assert_eq!(
            row.envelope.payload,
            RequestPayload::SetStepAssignment(SetStepAssignment {
                feature_id: "f-1".to_owned(),
                step_id: "s-review".to_owned(),
                agent_kind,
                model,
                effort,
            })
        );
        assert_eq!(answer.expires_at, row.expires_at);
    }
}

#[test]
fn an_assignment_body_with_a_field_it_does_not_have_is_not_one() {
    let mut body = full_assignment();
    body["step_id"] = json!("s-other");
    assert!(serde_json::from_value::<AssignmentBody>(body).is_err());
}

#[tokio::test]
async fn without_spend_a_cancel_or_an_assignment_is_refused_by_name_and_nothing_is_queued() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    t.fleet.scopes(t.a_instance, &[Scope::Read, Scope::Gates]);
    let before = t.fleet.requests(t.a_instance);
    let instance_id = t.a_instance.0.to_string();

    for feature_id in ["f-a", "a\u{0}b"] {
        let refused = cancel(&api, t.a, &instance_id, feature_id, NOW)
            .await
            .expect_err("spend is off");
        assert_eq!(
            answered(refused).await,
            (409, json!({"error": "scope_not_granted", "scope": "spend"})),
            "cancel {feature_id:?}"
        );
        let refused = set_step_assignment(
            &api,
            t.a,
            &instance_id,
            feature_id,
            "s-review",
            assignment_body(full_assignment()),
            NOW,
        )
        .await
        .expect_err("spend is off");
        assert_eq!(
            answered(refused).await,
            (409, json!({"error": "scope_not_granted", "scope": "spend"})),
            "assignment {feature_id:?}"
        );
    }
    assert_eq!(t.fleet.requests(t.a_instance), before);
}

#[tokio::test]
async fn a_cancel_or_an_assignment_with_a_nul_or_an_over_long_id_is_refused_and_nothing_is_queued()
{
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let before = t.fleet.requests(t.a_instance);
    let instance_id = t.a_instance.0.to_string();
    let invalid = (400, json!({"error": "invalid_request"}));
    let long = "x".repeat(257);

    for feature_id in ["a\u{0}b", long.as_str()] {
        let refused = cancel(&api, t.a, &instance_id, feature_id, NOW)
            .await
            .expect_err("the feature id is refused");
        assert_eq!(answered(refused).await, invalid, "cancel {feature_id:?}");
    }

    let mut nul_model = full_assignment();
    nul_model["model"] = json!("\u{0}");
    let mut nul_agent = full_assignment();
    nul_agent["agent_kind"] = json!("a\u{0}");
    for (feature_id, step_id, body) in [
        ("a\u{0}b", "s-review", full_assignment()),
        (long.as_str(), "s-review", full_assignment()),
        ("f-a", "s\u{0}", full_assignment()),
        ("f-a", long.as_str(), full_assignment()),
        ("f-a", "s-review", nul_model),
        ("f-a", "s-review", nul_agent),
    ] {
        let refused = set_step_assignment(
            &api,
            t.a,
            &instance_id,
            feature_id,
            step_id,
            assignment_body(body.clone()),
            NOW,
        )
        .await
        .expect_err("the assignment is refused");
        assert_eq!(
            answered(refused).await,
            invalid,
            "{feature_id:?} {step_id:?} {body}"
        );
    }
    assert_eq!(t.fleet.requests(t.a_instance), before);
}

#[tokio::test]
async fn a_cancel_or_an_assignment_for_another_tenants_instance_reads_as_one_that_does_not_exist() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let b_before = t.fleet.requests(t.b_instance);
    let a_before = t.fleet.requests(t.a_instance);
    let not_found = (404, json!({"error": "not_found"}));

    for instance_id in [
        t.b_instance.0.to_string(),
        InstanceId::generate().0.to_string(),
        "not-an-instance".to_owned(),
    ] {
        let refused = cancel(&api, t.a, &instance_id, "f-b", NOW)
            .await
            .expect_err("not the tenant's");
        assert_eq!(answered(refused).await, not_found, "cancel {instance_id}");
        let refused = set_step_assignment(
            &api,
            t.a,
            &instance_id,
            "f-b",
            "s-review",
            assignment_body(full_assignment()),
            NOW,
        )
        .await
        .expect_err("not the tenant's");
        assert_eq!(
            answered(refused).await,
            not_found,
            "assignment {instance_id}"
        );
    }
    assert_eq!(t.fleet.requests(t.b_instance), b_before);
    assert_eq!(t.fleet.requests(t.a_instance), a_before);
}

async fn status_json(
    api: &ApiContext<'_>,
    user: UserId,
    instance: InstanceId,
    request_id: &str,
    now: u64,
) -> Value {
    let view = request_status(api, user, &instance.0.to_string(), request_id, now)
        .await
        .expect("the tenant's request");
    serde_json::to_value(view).unwrap()
}

async fn outcome(
    fleet: &MemoryFleet,
    user: UserId,
    instance: InstanceId,
    id: &str,
    outcome: RequestOutcome,
) {
    let id = requests::named_request(id).expect("an id the dispatch answered");
    let decide = move |state| requests::outcome_change(state, outcome);
    let recorded = fleet
        .store(user, instance)
        .outcome_recorded(id, &decide, NOW + 2)
        .await
        .unwrap();
    assert!(recorded);
}

#[tokio::test]
async fn a_request_reads_queued_then_delivered_then_its_outcome_or_expired() {
    let fleet = MemoryFleet::new();
    let parts = Parts::new();
    let api = parts.over(&fleet);
    let user = fleet.tenant();
    let instance = fleet.paired(user, &ALL_SCOPES);
    let mut queued = Vec::new();
    for _ in 0..4 {
        let answer = dispatch(
            &api,
            user,
            &instance.0.to_string(),
            dispatch_body(full_dispatch()),
            NOW,
        )
        .await
        .expect("spend is granted");
        queued.push(answer);
    }
    let [first, second, third, fourth] = &queued[..] else {
        unreachable!()
    };
    let expires_at = first.expires_at;
    let read = |id: &str, deliveries: u32, status: Value, answered_at: Option<u64>| {
        let mut view = json!({"request_id": id, "kind": "start_feature",
            "deliveries": deliveries, "expires_at": expires_at, "answered_at": answered_at});
        for (key, value) in status.as_object().unwrap() {
            view[key] = value.clone();
        }
        view
    };

    assert_eq!(
        status_json(&api, user, instance, &first.request_id, NOW).await,
        read(&first.request_id, 0, json!({"status": "queued"}), None)
    );
    let first_id = requests::named_request(&first.request_id).unwrap();
    let store = fleet.store(user, instance);
    store.request_sent(first_id, NOW + 1).await.unwrap();
    assert_eq!(
        status_json(&api, user, instance, &first.request_id, NOW + 1).await,
        read(&first.request_id, 1, json!({"status": "delivered"}), None)
    );
    outcome(
        &fleet,
        user,
        instance,
        &first.request_id,
        RequestOutcome::Accepted,
    )
    .await;
    assert_eq!(
        status_json(&api, user, instance, &first.request_id, NOW + 2).await,
        read(
            &first.request_id,
            1,
            json!({"status": "accepted"}),
            Some(NOW + 2)
        )
    );

    let scope_off = RequestOutcome::Refused {
        reason: RefusalReason::ScopeOff,
    };
    outcome(&fleet, user, instance, &second.request_id, scope_off).await;
    assert_eq!(
        status_json(&api, user, instance, &second.request_id, NOW + 2).await,
        read(
            &second.request_id,
            0,
            json!({"status": "refused", "reason": "scope_off"}),
            Some(NOW + 2)
        )
    );

    for now in [expires_at, expires_at + 1] {
        assert_eq!(
            status_json(&api, user, instance, &third.request_id, now).await,
            read(&third.request_id, 0, json!({"status": "expired"}), None),
            "{now}"
        );
    }
    assert_eq!(
        status_json(&api, user, instance, &third.request_id, expires_at - 1).await,
        read(&third.request_id, 0, json!({"status": "queued"}), None)
    );

    let fourth_id = requests::named_request(&fourth.request_id).unwrap();
    store.request_expired(fourth_id).await.unwrap();
    assert_eq!(
        status_json(&api, user, instance, &fourth.request_id, NOW + 2).await,
        read(&fourth.request_id, 0, json!({"status": "expired"}), None)
    );
}

#[tokio::test]
async fn another_tenants_request_reads_as_one_that_does_not_exist() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let a_request = base64url_encode(&t.fleet.requests(t.a_instance)[0].id);
    let b_request = base64url_encode(&t.fleet.requests(t.b_instance)[0].id);
    let not_found = (404, json!({"error": "not_found"}));

    assert_eq!(
        status_json(&api, t.a, t.a_instance, &a_request, NOW).await["status"],
        "queued"
    );
    for (user, instance_id, request_id) in [
        (t.b, t.b_instance.0.to_string(), a_request.clone()),
        (t.b, t.a_instance.0.to_string(), a_request.clone()),
        (t.a, t.a_instance.0.to_string(), b_request.clone()),
        (t.a, t.a_instance.0.to_string(), base64url_encode(&[9; 16])),
        (t.a, t.a_instance.0.to_string(), "not-a-request".to_owned()),
        (t.a, t.a_instance.0.to_string(), base64url_encode(&[9; 15])),
        (t.a, InstanceId::generate().0.to_string(), a_request.clone()),
        (t.a, "not-an-instance".to_owned(), a_request.clone()),
    ] {
        let refused = request_status(&api, user, &instance_id, &request_id, NOW)
            .await
            .expect_err("not the tenant's");
        assert_eq!(
            answered(refused).await,
            not_found,
            "{instance_id} {request_id}"
        );
    }
}

const A_HOLDER: &[u8] = b"the passkey a signed in with";

fn gate_body(body: Value) -> GateChallengeBody {
    serde_json::from_value(body).expect("a gate challenge body")
}

fn approve_e1() -> Value {
    json!({"feature_id": "f-a", "step_execution_id": "e1", "decision": "approve"})
}

#[tokio::test]
async fn a_gate_challenge_is_the_protocols_pre_image_and_its_digest_is_the_webauthn_challenge() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    t.fleet.trusted(t.a, vec!["cred-a".to_owned()]);
    let before = t.fleet.requests(t.a_instance);

    for (decision, spelled) in [(Decision::Approve, "approve"), (Decision::Cancel, "cancel")] {
        let mut body = approve_e1();
        body["decision"] = json!(spelled);
        let view = gate_challenge(
            &api,
            t.a,
            A_HOLDER.to_vec(),
            &t.a_instance.0.to_string(),
            gate_body(body),
            NOW,
        )
        .await
        .expect("a challenge");

        let request_id = base64url_decode(&view.request_id).expect("base64url");
        assert_eq!(request_id.len(), 16, "{spelled}");
        assert_eq!(
            view.expires_at,
            NOW + demeteo_hub::socket::REQUEST_TTL_SECS,
            "{spelled}"
        );
        let pre_image = demeteo_hub::protocol::gate_challenge(
            &t.a_instance.0.to_string(),
            "f-a",
            "e1",
            decision,
            &view.request_id,
            view.expires_at,
        )
        .expect("a pre-image");
        assert_eq!(
            base64url_decode(&view.gate_challenge).expect("base64url"),
            pre_image,
            "{spelled}"
        );
        let digest: [u8; 32] = Sha256::digest(&pre_image).into();
        assert_eq!(
            view.public_key,
            json!({
                "challenge": base64url_encode(&digest),
                "rpId": "hub.example",
                "allowCredentials": [{"type": "public-key", "id": "cred-a"}],
                "userVerification": "required",
                "timeout": 300_000,
            }),
            "{spelled}"
        );
    }
    assert_eq!(parts.ceremonies.held_under(A_HOLDER), 2);
    assert_eq!(t.fleet.requests(t.a_instance), before);
}

#[tokio::test]
async fn without_gates_a_challenge_is_refused_by_name_and_nothing_is_held_or_queued() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    t.fleet.trusted(t.a, vec!["cred-a".to_owned()]);
    t.fleet.scopes(t.a_instance, &[Scope::Read, Scope::Spend]);
    let before = t.fleet.requests(t.a_instance);
    let mut nul_feature = approve_e1();
    nul_feature["feature_id"] = json!("a\u{0}b");

    for body in [approve_e1(), nul_feature] {
        let refused = gate_challenge(
            &api,
            t.a,
            A_HOLDER.to_vec(),
            &t.a_instance.0.to_string(),
            gate_body(body),
            NOW,
        )
        .await
        .expect_err("gates is off");
        assert_eq!(
            answered(refused).await,
            (409, json!({"error": "scope_not_granted", "scope": "gates"}))
        );
    }
    assert_eq!(parts.ceremonies.held_under(A_HOLDER), 0);
    assert_eq!(t.fleet.requests(t.a_instance), before);
}

#[tokio::test]
async fn a_gate_challenge_with_a_nul_or_an_over_long_id_is_refused_and_nothing_is_held() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    t.fleet.trusted(t.a, vec!["cred-a".to_owned()]);
    let mut nul_feature = approve_e1();
    nul_feature["feature_id"] = json!("a\u{0}b");
    let mut nul_execution = approve_e1();
    nul_execution["step_execution_id"] = json!("\u{0}");
    let mut long_execution = approve_e1();
    long_execution["step_execution_id"] = json!("e".repeat(257));

    for body in [nul_feature, nul_execution, long_execution] {
        let refused = gate_challenge(
            &api,
            t.a,
            A_HOLDER.to_vec(),
            &t.a_instance.0.to_string(),
            gate_body(body),
            NOW,
        )
        .await
        .expect_err("the body is refused");
        assert_eq!(
            answered(refused).await,
            (400, json!({"error": "invalid_request"}))
        );
    }
    assert_eq!(parts.ceremonies.held_under(A_HOLDER), 0);
}

#[test]
fn a_gate_challenge_body_with_a_field_it_does_not_have_is_not_one() {
    let mut body = approve_e1();
    body["feedback"] = json!("looks good");
    assert!(serde_json::from_value::<GateChallengeBody>(body).is_err());
    let mut body = approve_e1();
    body["decision"] = json!("redirect");
    assert!(serde_json::from_value::<GateChallengeBody>(body).is_err());
}

#[tokio::test]
async fn a_gate_challenge_for_another_tenants_instance_reads_as_one_that_does_not_exist() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    t.fleet.trusted(t.a, vec!["cred-a".to_owned()]);
    let b_before = t.fleet.requests(t.b_instance);

    for instance_id in [
        t.b_instance.0.to_string(),
        InstanceId::generate().0.to_string(),
        "not-an-instance".to_owned(),
    ] {
        let refused = gate_challenge(
            &api,
            t.a,
            A_HOLDER.to_vec(),
            &instance_id,
            gate_body(approve_e1()),
            NOW,
        )
        .await
        .expect_err("not the tenant's");
        assert_eq!(
            answered(refused).await,
            (404, json!({"error": "not_found"})),
            "{instance_id}"
        );
    }
    assert_eq!(parts.ceremonies.held_under(A_HOLDER), 0);
    assert_eq!(t.fleet.requests(t.b_instance), b_before);
}

/// A challenge held for A's approval of `e1` on A's instance, under [`A_HOLDER`].
async fn challenged(t: &TwoTenants, api: &ApiContext<'_>) -> GateChallengeView {
    t.fleet.trusted(t.a, vec!["cred-a".to_owned()]);
    gate_challenge(
        api,
        t.a,
        A_HOLDER.to_vec(),
        &t.a_instance.0.to_string(),
        gate_body(approve_e1()),
        NOW,
    )
    .await
    .expect("a challenge")
}

/// The digest a passkey is asked to sign, read back from what the browser was given.
fn digest_of(view: &GateChallengeView) -> [u8; 32] {
    let challenge = view.public_key["challenge"].as_str().expect("a challenge");
    base64url_decode(challenge)
        .expect("base64url")
        .try_into()
        .expect("32 bytes")
}

fn decision(view: &GateChallengeView, assertion: WebAuthnAssertion) -> GateDecisionBody {
    GateDecisionBody {
        ceremony_id: view.ceremony_id.clone(),
        request_id: view.request_id.clone(),
        assertion,
    }
}

fn signed_decision(parts: &Parts, view: &GateChallengeView) -> GateDecisionBody {
    decision(
        view,
        SoftAuthenticator::new().assert(&parts.rp, &digest_of(view)),
    )
}

#[tokio::test]
async fn a_gate_decision_queues_one_request_whose_envelope_recomputes_the_challenged_bytes() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let before = t.fleet.requests(t.a_instance);
    let view = challenged(&t, &api).await;
    let body = signed_decision(&parts, &view);
    let assertion = body.assertion.clone();

    let answer = gate_decision(
        &api,
        t.a,
        A_HOLDER.to_vec(),
        &t.a_instance.0.to_string(),
        body,
        NOW + 1,
    )
    .await
    .expect("a decision");

    let after = t.fleet.requests(t.a_instance);
    assert_eq!(after.len(), before.len() + 1);
    let row = after.last().expect("the queued decision");
    assert_eq!(row.kind, "gate_decision");
    let envelope = &row.envelope;
    assert_eq!(envelope.request_id, view.request_id);
    assert_eq!(envelope.instance_id, t.a_instance.0.to_string());
    assert_eq!(envelope.expires_at, view.expires_at);
    assert_eq!(
        envelope.payload,
        RequestPayload::GateDecision(GateDecision {
            feature_id: "f-a".to_owned(),
            step_execution_id: "e1".to_owned(),
            decision: Decision::Approve,
            feedback: None,
            assertion,
        })
    );
    let RequestPayload::GateDecision(queued) = &envelope.payload else {
        unreachable!("asserted above");
    };
    let recomputed = demeteo_hub::protocol::gate_challenge(
        &envelope.instance_id,
        &queued.feature_id,
        &queued.step_execution_id,
        queued.decision,
        &envelope.request_id,
        envelope.expires_at,
    )
    .expect("a pre-image");
    assert_eq!(
        recomputed,
        base64url_decode(&view.gate_challenge).expect("base64url")
    );
    assert_eq!(
        answer,
        EnqueuedView {
            request_id: view.request_id.clone(),
            expires_at: view.expires_at,
        }
    );
    assert_eq!(parts.ceremonies.held_under(A_HOLDER), 0);
}

#[tokio::test]
async fn a_mismatched_request_id_is_refused_and_spends_the_challenge() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let before = t.fleet.requests(t.a_instance);
    let view = challenged(&t, &api).await;
    let mut other = base64url_decode(&view.request_id).expect("base64url");
    other[0] ^= 0x01;
    let mut mismatched = signed_decision(&parts, &view);
    mismatched.request_id = base64url_encode(&other);

    let instance_id = t.a_instance.0.to_string();
    let decide = |body| gate_decision(&api, t.a, A_HOLDER.to_vec(), &instance_id, body, NOW + 1);
    let refused = decide(mismatched).await.expect_err("another request");
    assert_eq!(
        answered(refused).await,
        (409, json!({"error": "challenge_mismatch"}))
    );
    assert_eq!(t.fleet.requests(t.a_instance), before);

    let resubmitted = decide(signed_decision(&parts, &view))
        .await
        .expect_err("the challenge was spent");
    assert_eq!(
        answered(resubmitted).await,
        (400, json!({"error": "ceremony_expired"}))
    );
    assert_eq!(t.fleet.requests(t.a_instance), before);
}

/// Assertions over `digest` an instance could not accept whoever signed them,
/// each with a valid signature over what it claims.
fn malformed_over(rp: &RelyingParty, digest: &[u8; 32]) -> Vec<(&'static str, WebAuthnAssertion)> {
    let authenticator = SoftAuthenticator::new();
    let valid = authenticator.assert(rp, digest);
    let data = base64url_decode(&valid.authenticator_data).expect("base64url");
    let client = |kind: &str, challenge: &[u8]| {
        json!({"type": kind, "challenge": base64url_encode(challenge), "origin": rp.origin})
            .to_string()
            .into_bytes()
    };
    let padded = |field: fn(&mut WebAuthnAssertion) -> &mut String| {
        let mut assertion = valid.clone();
        field(&mut assertion).push('=');
        assertion
    };
    let mut other = *digest;
    other[0] ^= 0x01;
    vec![
        ("credential_id padded", padded(|a| &mut a.credential_id)),
        (
            "authenticator_data padded",
            padded(|a| &mut a.authenticator_data),
        ),
        (
            "client_data_json padded",
            padded(|a| &mut a.client_data_json),
        ),
        ("signature padded", padded(|a| &mut a.signature)),
        (
            "authenticator data of 36 bytes",
            authenticator.sign(&data[..36], &client("webauthn.get", digest)),
        ),
        (
            "client data that is not json",
            authenticator.sign(&data, b"webauthn.get"),
        ),
        (
            "client data of another type",
            authenticator.sign(&data, &client("webauthn.create", digest)),
        ),
        (
            "client data over another challenge",
            authenticator.sign(&data, &client("webauthn.get", &other)),
        ),
    ]
}

#[tokio::test]
async fn a_malformed_assertion_is_refused_and_nothing_is_queued() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let before = t.fleet.requests(t.a_instance);

    let variants = malformed_over(&parts.rp, &[0; 32]).len();
    assert_eq!(variants, 8);
    for variant in 0..variants {
        let view = challenged(&t, &api).await;
        let (name, assertion) = malformed_over(&parts.rp, &digest_of(&view)).swap_remove(variant);
        let refused = gate_decision(
            &api,
            t.a,
            A_HOLDER.to_vec(),
            &t.a_instance.0.to_string(),
            decision(&view, assertion),
            NOW + 1,
        )
        .await
        .expect_err(name);
        assert_eq!(
            answered(refused).await,
            (400, json!({"error": "assertion_malformed"})),
            "{name}"
        );
    }
    assert_eq!(t.fleet.requests(t.a_instance), before);
}

#[tokio::test]
async fn a_challenge_held_for_another_holder_user_or_instance_reads_as_expired() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let a_second = t.fleet.paired(t.a, &ALL_SCOPES);
    let before = [t.a_instance, t.b_instance, a_second].map(|i| t.fleet.requests(i));

    let cases: [(&str, UserId, &[u8], InstanceId); 4] = [
        ("another holder", t.a, b"another passkey of a", t.a_instance),
        ("another user", t.b, A_HOLDER, t.b_instance),
        ("another instance", t.a, A_HOLDER, a_second),
        ("past its expiry", t.a, A_HOLDER, t.a_instance),
    ];
    for (name, user, holder, instance) in cases {
        let view = challenged(&t, &api).await;
        let now = match name {
            "past its expiry" => view.expires_at,
            _ => NOW + 1,
        };
        let refused = gate_decision(
            &api,
            user,
            holder.to_vec(),
            &instance.0.to_string(),
            signed_decision(&parts, &view),
            now,
        )
        .await
        .expect_err(name);
        assert_eq!(
            answered(refused).await,
            (400, json!({"error": "ceremony_expired"})),
            "{name}"
        );
        let retried = gate_decision(
            &api,
            t.a,
            A_HOLDER.to_vec(),
            &t.a_instance.0.to_string(),
            signed_decision(&parts, &view),
            NOW + 1,
        )
        .await
        .expect_err("the challenge was spent");
        assert_eq!(
            answered(retried).await,
            (400, json!({"error": "ceremony_expired"})),
            "{name}"
        );
    }
    assert_eq!(
        [t.a_instance, t.b_instance, a_second].map(|i| t.fleet.requests(i)),
        before
    );
}

#[tokio::test]
async fn a_gate_decision_for_an_instance_not_the_tenants_is_not_found_and_spends_nothing() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let b_before = t.fleet.requests(t.b_instance);
    let view = challenged(&t, &api).await;

    for instance_id in [
        t.b_instance.0.to_string(),
        InstanceId::generate().0.to_string(),
        "not-an-instance".to_owned(),
    ] {
        let refused = gate_decision(
            &api,
            t.a,
            A_HOLDER.to_vec(),
            &instance_id,
            signed_decision(&parts, &view),
            NOW + 1,
        )
        .await
        .expect_err("not the tenant's");
        assert_eq!(
            answered(refused).await,
            (404, json!({"error": "not_found"})),
            "{instance_id}"
        );
    }
    assert_eq!(t.fleet.requests(t.b_instance), b_before);
    assert_eq!(parts.ceremonies.held_under(A_HOLDER), 1);
}

#[tokio::test]
async fn without_gates_a_decision_is_refused_by_name_and_nothing_is_queued() {
    let t = TwoTenants::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let view = challenged(&t, &api).await;
    t.fleet.scopes(t.a_instance, &[Scope::Read, Scope::Spend]);
    let before = t.fleet.requests(t.a_instance);

    let refused = gate_decision(
        &api,
        t.a,
        A_HOLDER.to_vec(),
        &t.a_instance.0.to_string(),
        signed_decision(&parts, &view),
        NOW + 1,
    )
    .await
    .expect_err("gates is off");
    assert_eq!(
        answered(refused).await,
        (409, json!({"error": "scope_not_granted", "scope": "gates"}))
    );
    assert_eq!(t.fleet.requests(t.a_instance), before);
    assert_eq!(parts.ceremonies.held_under(A_HOLDER), 1);
}

#[test]
fn a_gate_decision_body_carries_no_gate_of_its_own() {
    let body = json!({
        "ceremony_id": "c",
        "request_id": "r",
        "assertion": {"credential_id": "a", "authenticator_data": "b",
            "client_data_json": "c", "signature": "d"},
    });
    assert!(serde_json::from_value::<GateDecisionBody>(body.clone()).is_ok());
    for (field, value) in [
        ("feature_id", json!("f-b")),
        ("step_execution_id", json!("e2")),
        ("decision", json!("cancel")),
    ] {
        let mut smuggled = body.clone();
        smuggled[field] = value;
        assert!(
            serde_json::from_value::<GateDecisionBody>(smuggled).is_err(),
            "{field}"
        );
    }
}

/// Two tenants whose instances hold everything a read view can show: a
/// running Feature with a step, tickets and a pending gate, a completed one,
/// projects, a hello, and a request queued and then refused. Every id is
/// spelled with its tenant's tag, so one tenant's id cannot be a substring of
/// the other's.
struct Populated {
    fleet: MemoryFleet,
    a: UserId,
    a_instance: InstanceId,
    a_request: String,
    b: UserId,
    b_instance: InstanceId,
    b_request: String,
}

impl Populated {
    async fn new() -> Self {
        use RunEventKind::*;
        let fleet = MemoryFleet::new();
        let (a, b) = (fleet.tenant(), fleet.tenant());
        let a_instance = fleet.paired(a, &ALL_SCOPES);
        let b_instance = fleet.paired(b, &ALL_SCOPES);
        let mut queued = Vec::new();
        for (user, instance, tag) in [(a, a_instance, "of-a"), (b, b_instance, "of-b")] {
            let (running, done) = (format!("run-{tag}-1"), format!("run-{tag}-2"));
            let (step, execution) = (format!("step-{tag}"), format!("exec-{tag}-1"));
            for event in [
                event(&running, 1, FeatureStarted, None, None, None),
                event(&done, 2, FeatureStarted, None, None, None),
                event(&running, 3, StepStarted, Some(&step), None, None),
                event(&running, 4, TicketStarted, Some(&step), None, Some("t1")),
                event(&running, 5, TicketCompleted, Some(&step), None, Some("t1")),
                event(&done, 6, FeatureCompleted, None, None, None),
                event(
                    &running,
                    7,
                    GatePending,
                    Some(&step),
                    Some(&execution),
                    None,
                ),
            ] {
                fleet.mirrored(instance, event).await;
            }
            fleet.greeted(instance, "linux", "x86_64", "1.4.0", true);
            fleet
                .snapshot_sent(
                    instance,
                    Snapshot {
                        v: 1,
                        projects: vec![ProjectSnapshot {
                            id: format!("project-{tag}"),
                            name: format!("name-{tag}"),
                            remote_url: Some(format!("https://git.example/{tag}.git")),
                        }],
                        workflows: Vec::new(),
                        run_shape: Vec::new(),
                    },
                )
                .await;
            let queued_one = enqueue(&fleet, user, instance, &done).await.unwrap();
            let id = base64url_encode(&queued_one.id);
            let refused = RequestOutcome::Refused {
                reason: RefusalReason::ScopeOff,
            };
            outcome(&fleet, user, instance, &id, refused).await;
            queued.push(id);
        }
        let [a_request, b_request] = <[String; 2]>::try_from(queued).unwrap();
        Self {
            fleet,
            a,
            a_instance,
            a_request,
            b,
            b_instance,
            b_request,
        }
    }

    /// Every id of B's a view could carry.
    fn b_ids(&self) -> [String; 6] {
        [
            self.b_instance.0.to_string(),
            "run-of-b-1".to_owned(),
            "run-of-b-2".to_owned(),
            "exec-of-b-1".to_owned(),
            "project-of-b".to_owned(),
            self.b_request.clone(),
        ]
    }
}

/// What each of the eight use-cases that name an instance answers when A
/// calls it with `instance_id`, every other argument naming B's data.
async fn addressed_as_a(
    p: &Populated,
    parts: &Parts,
    view: &GateChallengeView,
    instance_id: &str,
) -> Vec<(&'static str, ApiError)> {
    let api = parts.over(&p.fleet);
    let (a, now) = (p.a, NOW + 1);
    let gate = json!({"feature_id": "run-of-b-1", "step_execution_id": "exec-of-b-1",
        "decision": "approve"});
    vec![
        (
            "instance_detail",
            instance_detail(&api, a, instance_id, now)
                .await
                .expect_err("instance_detail"),
        ),
        (
            "run_detail",
            run_detail(&api, a, instance_id, "run-of-b-1", now)
                .await
                .expect_err("run_detail"),
        ),
        (
            "request_status",
            request_status(&api, a, instance_id, &p.b_request, now)
                .await
                .expect_err("request_status"),
        ),
        (
            "dispatch",
            dispatch(&api, a, instance_id, dispatch_body(full_dispatch()), now)
                .await
                .expect_err("dispatch"),
        ),
        (
            "cancel",
            cancel(&api, a, instance_id, "run-of-b-1", now)
                .await
                .expect_err("cancel"),
        ),
        (
            "set_step_assignment",
            set_step_assignment(
                &api,
                a,
                instance_id,
                "run-of-b-1",
                "step-of-b",
                assignment_body(full_assignment()),
                now,
            )
            .await
            .expect_err("set_step_assignment"),
        ),
        (
            "gate_challenge",
            gate_challenge(
                &api,
                a,
                A_HOLDER.to_vec(),
                instance_id,
                gate_body(gate),
                now,
            )
            .await
            .expect_err("gate_challenge"),
        ),
        (
            "gate_decision",
            gate_decision(
                &api,
                a,
                A_HOLDER.to_vec(),
                instance_id,
                signed_decision(parts, view),
                now,
            )
            .await
            .expect_err("gate_decision"),
        ),
    ]
}

#[tokio::test]
async fn every_use_case_called_as_one_tenant_neither_shows_nor_touches_another_tenants_data() {
    let p = Populated::new().await;
    let parts = Parts::new();
    let api = parts.over(&p.fleet);
    p.fleet.trusted(p.a, vec!["cred-a".to_owned()]);
    p.fleet.trusted(p.b, vec!["cred-b".to_owned()]);
    let view = gate_challenge(
        &api,
        p.a,
        A_HOLDER.to_vec(),
        &p.a_instance.0.to_string(),
        gate_body(
            json!({"feature_id": "run-of-a-1", "step_execution_id": "exec-of-a-1",
            "decision": "approve"}),
        ),
        NOW,
    )
    .await
    .expect("a challenge on A's own gate");
    let b_before = p.fleet.requests(p.b_instance);
    let a_before = p.fleet.requests(p.a_instance);

    let listed = fleet_json(&api, p.a).await.to_string();
    let everything = runs(&api, p.a, RunsFilter::default(), NOW)
        .await
        .expect("A's runs");
    let everything = serde_json::to_value(everything).unwrap().to_string();
    for (name, text) in [("fleet", &listed), ("runs", &everything)] {
        for own in [p.a_instance.0.to_string(), "run-of-a-1".to_owned()] {
            assert!(text.contains(&own), "{name} lacks A's {own}: {text}");
        }
        for b_id in p.b_ids() {
            assert!(!text.contains(&b_id), "{name} shows B's {b_id}: {text}");
        }
    }
    assert!(listed.contains("exec-of-a-1"), "{listed}");

    let not_found = (404, json!({"error": "not_found"}));
    let at_b = addressed_as_a(&p, &parts, &view, &p.b_instance.0.to_string()).await;
    let never_paired =
        addressed_as_a(&p, &parts, &view, &InstanceId::generate().0.to_string()).await;
    assert_eq!(at_b.len(), 8);
    for ((name, at_b), (_, never_paired)) in at_b.into_iter().zip(never_paired) {
        let (status, body) = answered(at_b).await;
        assert_eq!((status, &body), (not_found.0, &not_found.1), "{name}");
        assert_eq!(body.as_object().map(|body| body.len()), Some(1), "{name}");
        assert_eq!(answered(never_paired).await, (status, body), "{name}");
    }

    assert_eq!(p.fleet.requests(p.b_instance), b_before);
    assert_eq!(p.fleet.requests(p.a_instance), a_before);
    assert_eq!(parts.ceremonies.held_under(A_HOLDER), 1);
    match parts.ceremonies.take(NOW + 1, &view.ceremony_id) {
        Some(Ceremony::Gate { instance, .. }) => assert_eq!(instance, p.a_instance),
        held => panic!("A's one held ceremony is not its own gate: {held:?}"),
    }
}

/// Every object key in `value`, at any depth.
fn keys_of(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::Object(members) => {
            for (key, member) in members {
                into.push(key.clone());
                keys_of(member, into);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| keys_of(item, into)),
        _ => {}
    }
}

/// Every string value in `value`, at any depth.
fn strings_of(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::String(text) => into.push(text.clone()),
        Value::Object(members) => members.values().for_each(|member| strings_of(member, into)),
        Value::Array(items) => items.iter().for_each(|item| strings_of(item, into)),
        _ => {}
    }
}

#[tokio::test]
async fn no_read_view_carries_a_key_named_like_a_local_path() {
    let p = Populated::new().await;
    let parts = Parts::new();
    let api = parts.over(&p.fleet);
    let instance_id = p.a_instance.0.to_string();
    let views = [
        ("fleet", fleet_json(&api, p.a).await),
        (
            "instance_detail",
            detail_json(&api, p.a, &instance_id).await,
        ),
        (
            "runs",
            serde_json::to_value(runs(&api, p.a, RunsFilter::default(), NOW).await.unwrap())
                .unwrap(),
        ),
        (
            "run_detail",
            serde_json::to_value(
                run_detail(&api, p.a, &instance_id, "run-of-a-1", NOW)
                    .await
                    .unwrap(),
            )
            .unwrap(),
        ),
        (
            "request_status",
            status_json(&api, p.a, p.a_instance, &p.a_request, NOW + 2).await,
        ),
    ];

    let mut seen = Vec::new();
    for (name, view) in &views {
        let mut keys = Vec::new();
        keys_of(view, &mut keys);
        assert!(!keys.is_empty(), "{name}: {view}");
        for key in &keys {
            let key = key.to_ascii_lowercase();
            for local in ["path", "cwd", "dir", "directory", "root", "worktree"] {
                assert!(!key.ends_with(local), "{name} has `{key}`: {view}");
            }
        }
        seen.extend(keys);
    }
    for nested in [
        "needs_you",
        "projects",
        "remote_url",
        "pending_gates",
        "events",
        "reason",
    ] {
        assert!(
            seen.iter().any(|key| key == nested),
            "no view walked `{nested}`"
        );
    }
}

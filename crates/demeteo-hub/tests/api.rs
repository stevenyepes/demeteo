//! The `/v1` fleet API over HTTP against a real Postgres: the tenant filter
//! in the SQL, which the `MemoryFleet` tests in `src-tauri/tests/hub_api.rs`
//! cannot reach. Needs the `postgres` service from `deploy/hub/compose.yaml`
//! as `deploy/hub/README.md` describes, and `DEMETEO_HUB_TEST_DATABASE_URL`
//! pointing at it.

mod support;

use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::auth::challenge::base64url_decode;
use demeteo_hub::auth::policy::Status;
use demeteo_hub::auth::session::SessionSecret;
use demeteo_hub::auth::testing::SoftAuthenticator;
use demeteo_hub::protocol::{
    gate_challenge, Decision, GateDecision, RequestPayload, RunEvent, RunEventKind,
    RunEventPayload, WebAuthnAssertion,
};
use demeteo_hub::routes::AppState;
use demeteo_hub::store::instances::{insert_instance, NewInstance};
use demeteo_hub::store::pool::create_user;
use demeteo_hub::store::requests::{list_queued_requests, QueuedEnvelope};
use demeteo_hub::store::run_mirror::{mirror_event, NewEvent};
use demeteo_hub::store::{Db, InstanceId, UserId};
use serde_json::{json, Value};
use std::sync::Arc;
use support::http::{enrol, get_as, open_session, post_as, rp, serve_state};
use support::in_fresh_schema;

const SIGNING_KEY: &str = "demeteo-hub-signing";

fn state_over(db: Db) -> AppState {
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);
    AppState::new(db, rp(), Arc::new(keys), SIGNING_KEY.to_owned()).unwrap()
}

/// A tenant signed in with its one trusted passkey, and one instance of its
/// own granted `scopes`.
struct Tenant {
    user: UserId,
    key: SoftAuthenticator,
    session: SessionSecret,
    instance: InstanceId,
}

impl Tenant {
    async fn new(db: &Db, scopes: &[&str]) -> Self {
        let user = create_user(db).await.unwrap();
        let key = SoftAuthenticator::new();
        enrol(db, user, &key, Status::Trusted).await;
        let session = open_session(db, user, &key).await;
        let new = NewInstance {
            id: InstanceId::generate(),
            name: "laptop".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            version: "1.2.0".to_owned(),
            granted_scopes: scopes.iter().map(|s| (*s).to_owned()).collect(),
        };
        insert_instance(user, db, &new).await.unwrap();
        Self {
            user,
            key,
            session,
            instance: new.id,
        }
    }

    fn at(&self, rest: &str) -> String {
        format!("/v1/instances/{}{rest}", self.instance.0)
    }
}

const ALL_SCOPES: [&str; 3] = ["read", "spend", "gates"];

fn minimal_dispatch() -> Value {
    json!({"project_id": "p", "workflow_id": "w", "title": "t", "description": ""})
}

async fn request_rows(db: &Db) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM instance_requests")
        .fetch_one(db)
        .await
        .unwrap()
}

async fn started(db: &Db, tenant: &Tenant, feature_id: &str) {
    let event = RunEvent {
        v: 1,
        feature_id: feature_id.to_owned(),
        offset: 1,
        kind: RunEventKind::FeatureStarted,
        occurred_at: 1_700_000_000,
        payload: RunEventPayload {
            step_id: None,
            step_execution_id: None,
            ticket_id: None,
        },
    };
    let new = NewEvent {
        feature_id: feature_id.to_owned(),
        event_offset: event.offset,
        event: serde_json::to_string(&event).unwrap(),
    };
    mirror_event(tenant.user, db, tenant.instance, &new, u64::max)
        .await
        .unwrap()
        .unwrap();
}

fn listed_ids(view: &Value, list: &str, field: &str) -> Vec<String> {
    view[list]
        .as_array()
        .unwrap_or_else(|| panic!("no `{list}` in {view}"))
        .iter()
        .map(|item| item[field].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn every_read_of_another_tenants_instance_run_or_request_is_not_found() {
    in_fresh_schema(|db| async move {
        let a = Tenant::new(&db, &ALL_SCOPES).await;
        let b = Tenant::new(&db, &ALL_SCOPES).await;
        started(&db, &a, "f-a").await;
        started(&db, &b, "f-b").await;
        let base = serve_state(state_over(db.clone())).await;
        let (status, queued) =
            post_as(&base, &b.at("/runs"), &b.session, &minimal_dispatch()).await;
        assert_eq!(status, 202, "{queued}");
        let b_request = queued["request_id"].as_str().unwrap().to_owned();
        let of_b = [
            b.at(""),
            b.at("/runs/f-b"),
            b.at(&format!("/requests/{b_request}")),
        ];

        for path in &of_b {
            let (status, body) = get_as(&base, path, &b.session).await;
            assert_eq!(status, 200, "{path} as its owner: {body}");
        }
        for path in &of_b {
            let (status, body) = get_as(&base, path, &a.session).await;
            assert_eq!(
                (status, body),
                (404, json!({"error": "not_found"})),
                "{path}"
            );
        }

        for (tenant, run) in [(&a, "f-a"), (&b, "f-b")] {
            let (status, fleet) = get_as(&base, "/v1/fleet", &tenant.session).await;
            assert_eq!(status, 200, "{fleet}");
            let own = tenant.instance.0.to_string();
            assert_eq!(
                listed_ids(&fleet, "instances", "id"),
                std::slice::from_ref(&own)
            );

            let (status, runs) = get_as(&base, "/v1/runs", &tenant.session).await;
            assert_eq!(status, 200, "{runs}");
            assert_eq!(listed_ids(&runs, "runs", "feature_id"), [run]);
            assert_eq!(listed_ids(&runs, "runs", "instance_id"), [own]);
        }
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn without_spend_a_dispatch_is_refused_by_name_and_inserts_no_row() {
    in_fresh_schema(|db| async move {
        let a = Tenant::new(&db, &["read", "gates"]).await;
        let base = serve_state(state_over(db.clone())).await;

        let refused = post_as(&base, &a.at("/runs"), &a.session, &minimal_dispatch()).await;

        assert_eq!(
            refused,
            (409, json!({"error": "scope_not_granted", "scope": "spend"}))
        );
        assert_eq!(request_rows(&db).await, 0);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_dispatch_with_spend_is_accepted_and_reads_back_as_queued() {
    in_fresh_schema(|db| async move {
        let a = Tenant::new(&db, &["spend"]).await;
        let base = serve_state(state_over(db.clone())).await;

        let (status, queued) =
            post_as(&base, &a.at("/runs"), &a.session, &minimal_dispatch()).await;
        assert_eq!(status, 202, "{queued}");
        assert_eq!(request_rows(&db).await, 1);

        let request_id = queued["request_id"].as_str().unwrap();
        let (status, read) =
            get_as(&base, &a.at(&format!("/requests/{request_id}")), &a.session).await;
        assert_eq!(status, 200, "{read}");
        assert_eq!(read["request_id"], request_id);
        assert_eq!(read["kind"], "start_feature");
        assert_eq!(read["status"], "queued");
        assert_eq!(read["expires_at"], queued["expires_at"]);
    })
    .await;
}

fn digest_in(challenge: &Value) -> [u8; 32] {
    challenge["public_key"]["challenge"]
        .as_str()
        .and_then(base64url_decode)
        .and_then(|bytes| bytes.try_into().ok())
        .expect("a 32-byte base64url challenge")
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_gate_decision_queues_one_request_whose_envelope_recomputes_the_challenged_bytes() {
    in_fresh_schema(|db| async move {
        let a = Tenant::new(&db, &["gates"]).await;
        let base = serve_state(state_over(db.clone())).await;
        let asked = json!({"feature_id": "f-a", "step_execution_id": "e1", "decision": "approve"});

        let (status, challenge) =
            post_as(&base, &a.at("/gates/challenge"), &a.session, &asked).await;
        assert_eq!(status, 200, "{challenge}");
        let assertion: WebAuthnAssertion = a.key.assert(&rp(), &digest_in(&challenge));
        let decided = json!({
            "ceremony_id": challenge["ceremony_id"],
            "request_id": challenge["request_id"],
            "assertion": assertion,
            "feedback": "ship it",
        });
        let answer = post_as(&base, &a.at("/gates/decision"), &a.session, &decided).await;
        assert_eq!(
            answer,
            (
                202,
                json!({
                    "request_id": challenge["request_id"],
                    "expires_at": challenge["expires_at"],
                })
            )
        );

        let queued = list_queued_requests(a.user, &db, a.instance).await.unwrap();
        assert_eq!(queued.len(), 1);
        let QueuedEnvelope::Readable(envelope) = &queued[0].envelope else {
            panic!("{:?}", queued[0].envelope);
        };
        assert_eq!(
            envelope.payload,
            RequestPayload::GateDecision(GateDecision {
                feature_id: "f-a".to_owned(),
                step_execution_id: "e1".to_owned(),
                decision: Decision::Approve,
                feedback: Some("ship it".to_owned()),
                assertion,
            })
        );
        let RequestPayload::GateDecision(decision) = &envelope.payload else {
            unreachable!("asserted above");
        };
        let recomputed = gate_challenge(
            &envelope.instance_id,
            &decision.feature_id,
            &decision.step_execution_id,
            decision.decision,
            &envelope.request_id,
            envelope.expires_at,
        )
        .unwrap();
        let challenged = challenge["gate_challenge"]
            .as_str()
            .and_then(base64url_decode)
            .expect("a base64url pre-image");
        assert_eq!(recomputed, challenged);

        let request_id = challenge["request_id"].as_str().unwrap();
        let (status, read) =
            get_as(&base, &a.at(&format!("/requests/{request_id}")), &a.session).await;
        assert_eq!(status, 200, "{read}");
        assert_eq!(
            (&read["kind"], &read["status"]),
            (&json!("gate_decision"), &json!("queued"))
        );
    })
    .await;
}

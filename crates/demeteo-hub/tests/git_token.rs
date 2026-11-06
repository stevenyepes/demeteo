//! `POST /git-token` over the real router and real rows. Every status the
//! route can answer is held to its verdict in `src/git_token/` without a
//! database; what is left for this file is what only Postgres and the router
//! can show: that a runner's access token and proof are admitted for this
//! URL and no other caller is, that the binding a fetch writes is the one the
//! next fetch is judged on, that of two first fetches at once exactly one
//! binds, and that no cell holds the token.
//!
//! The tests are ignored for the reason `crates/demeteo-hub/tests/pairing.rs`
//! gives in its header, and an ignored run shows nothing of them.

mod support;

use async_trait::async_trait;
use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::attachments::AttachmentSettings;
use demeteo_hub::pairing::grant::FreshPair;
use demeteo_hub::pairing::jwk::DeviceJwk;
use demeteo_hub::pairing::testing::SoftDeviceKey;
use demeteo_hub::ports::{GitTokenMinter, GithubRepository, InstallationToken, MintError};
use demeteo_hub::routes::AppState;
use demeteo_hub::store::device_keys::NewDeviceKey;
use demeteo_hub::store::instances::{pair_instance, InstanceKind, NewInstanceLabels, NewPairing};
use demeteo_hub::store::pool::create_user;
use demeteo_hub::store::requests::{
    git_token_problem_of, insert_request, record_outcome, NewRequest,
};
use demeteo_hub::store::run_mirror::{mirror_event, NewEvent};
use demeteo_hub::store::{Db, InstanceId, UserId};
use demeteo_hub_protocol::{
    RequestOutcome, RequestPayload, RunEvent, RunEventKind, RunEventPayload, RunnerRepository,
    RunnerStart, RunnerWorkflow, Scope, SignedRequest, StartFeature, PROTOCOL_VERSION,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use support::http::{post_with_dpop, rp, serve_state, unix_now};
use support::in_fresh_schema;
use uuid::Uuid;

const PATH: &str = "/git-token";
const TOKEN: &str = "ghs_handed_over_and_never_kept";
const REQUEST: [u8; 16] = [7; 16];
const REQUEST_TEXT: &str = "BwcHBwcHBwcHBwcHBwcHBw";

/// Answers every mint with [`TOKEN`], or with "not installed" while
/// `uninstalled` is set, and keeps what it was asked for.
#[derive(Default)]
struct FakeGithub {
    asked: Mutex<Vec<GithubRepository>>,
    uninstalled: Mutex<bool>,
}

#[async_trait]
impl GitTokenMinter for FakeGithub {
    async fn mint(
        &self,
        _user: UserId,
        repository: &GithubRepository,
        now: u64,
    ) -> Result<InstallationToken, MintError> {
        self.asked.lock().unwrap().push(repository.clone());
        if *self.uninstalled.lock().unwrap() {
            return Err(MintError::NotInstalled);
        }
        Ok(InstallationToken::new(TOKEN.to_owned(), now + 3600))
    }
}

fn state_over(db: Db, github: Arc<FakeGithub>) -> AppState {
    AppState::new(
        db,
        rp(),
        Arc::new(MemoryKeys::with_keys(["signing", "transit"])),
        "signing".to_owned(),
        "transit".to_owned(),
        AttachmentSettings::default(),
    )
    .unwrap()
    .with_git_tokens(github)
}

/// A paired instance and what it presents to the Hub.
struct Paired {
    user: UserId,
    instance: InstanceId,
    key: SoftDeviceKey,
    access_token: String,
}

impl Paired {
    async fn new(db: &Db, user: UserId, kind: InstanceKind) -> Self {
        let key = SoftDeviceKey::new();
        let jwk = DeviceJwk::parse(&key.jwk()).unwrap();
        let device_key = NewDeviceKey {
            public_jwk: jwk.canonical_json(),
            jkt: jwk.thumbprint(),
        };
        let family_id = Uuid::new_v4();
        let pair = FreshPair::generate().unwrap();
        let rows = pair.rows(family_id, &device_key.jkt, unix_now()).unwrap();
        let new = NewPairing {
            kind,
            instance: NewInstanceLabels {
                name: "runner".to_owned(),
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
                version: "1.2.0".to_owned(),
            },
            device_key,
            scopes: vec![Scope::Read, Scope::Spend],
            family_id,
            access: rows.access,
            refresh: rows.refresh,
        };
        let instance = pair_instance(user, db, &new).await.unwrap().instance_id;
        Self {
            user,
            instance,
            key,
            access_token: pair.access.text(),
        }
    }

    /// A start with a runner part queued for this instance, answered
    /// `accepted` when `accepted` says so.
    async fn started(&self, db: &Db, accepted: bool) {
        let now = unix_now();
        let envelope = SignedRequest {
            v: PROTOCOL_VERSION,
            request_id: REQUEST_TEXT.to_owned(),
            instance_id: self.instance.0.to_string(),
            issued_at: now,
            expires_at: now + 600,
            payload: RequestPayload::StartFeature(StartFeature {
                project_id: "p-1".to_owned(),
                workflow_id: "w-1".to_owned(),
                title: "T".to_owned(),
                description: "D".to_owned(),
                agent_kind: None,
                model: None,
                effort: None,
                step_overrides: Vec::new(),
                max_budget_cents: None,
                max_wall_clock_secs: None,
                attachments: Vec::new(),
                runner: Some(Box::new(RunnerStart {
                    source_instance_id: Uuid::new_v4().to_string(),
                    repository: RunnerRepository {
                        provider_kind: "github".to_owned(),
                        host: "github.com".to_owned(),
                        full_name: "acme/widgets".to_owned(),
                    },
                    workflow: RunnerWorkflow {
                        workflow_id: "w-1".to_owned(),
                        version: 1,
                        name: "Standard".to_owned(),
                        steps_json: "[]".to_owned(),
                    },
                })),
            }),
            signature: "not checked by the hub".to_owned(),
        };
        let new = NewRequest {
            id: REQUEST,
            kind: "start_feature".to_owned(),
            envelope,
            expires_at: now + 600,
        };
        insert_request(self.user, db, self.instance, &new)
            .await
            .unwrap();
        if accepted {
            let found = record_outcome(self.user, db, self.instance, &REQUEST, now, |_| {
                Some(RequestOutcome::Accepted)
            })
            .await
            .unwrap();
            assert!(found);
        }
    }

    async fn ended(&self, db: &Db, feature_id: &str) {
        let event = RunEvent {
            v: 1,
            feature_id: feature_id.to_owned(),
            offset: 1,
            kind: RunEventKind::FeatureCompleted,
            occurred_at: unix_now(),
            payload: RunEventPayload::default(),
        };
        let new = NewEvent {
            feature_id: feature_id.to_owned(),
            event_offset: 1,
            event: serde_json::to_string(&event).unwrap(),
        };
        mirror_event(self.user, db, self.instance, &new, |_, offset| offset)
            .await
            .unwrap()
            .unwrap();
    }

    async fn ask(&self, base: &str, feature_id: &str) -> (u16, Option<String>, Value) {
        let body = json!({"request_id": REQUEST_TEXT, "feature_id": feature_id});
        post_with_dpop(base, PATH, &self.key, &self.access_token, &body).await
    }
}

async fn bound_feature(db: &Db) -> Option<String> {
    sqlx::query_scalar("SELECT git_token_feature_id FROM instance_requests")
        .fetch_one(db)
        .await
        .unwrap()
}

/// Whether any text cell of the request queue holds the token: the one table
/// a careless implementation would write it to.
async fn token_stored(db: &Db) -> bool {
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM instance_requests r \
         WHERE strpos(row_to_json(r)::text, $1) > 0",
    )
    .bind(TOKEN)
    .fetch_one(db)
    .await
    .unwrap();
    rows > 0
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_runner_is_handed_a_token_for_its_accepted_start_and_its_feature_is_bound() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let runner = Paired::new(&db, user, InstanceKind::Runner).await;
        runner.started(&db, true).await;
        let github = Arc::new(FakeGithub::default());
        let base = serve_state(state_over(db.clone(), Arc::clone(&github))).await;

        let (status, cache_control, body) = runner.ask(&base, "f-1").await;

        assert_eq!(status, 200, "{body}");
        assert_eq!(cache_control.as_deref(), Some("no-store"));
        assert_eq!(body["token"], TOKEN);
        assert!(body["expires_at"].as_u64().unwrap() > unix_now());
        assert_eq!(bound_feature(&db).await.as_deref(), Some("f-1"));
        assert_eq!(
            github.asked.lock().unwrap().clone(),
            vec![GithubRepository {
                host: "github.com".to_owned(),
                owner: "acme".to_owned(),
                name: "widgets".to_owned(),
            }]
        );

        assert_eq!(runner.ask(&base, "f-1").await.0, 200, "a refresh");
        let (status, _, body) = runner.ask(&base, "f-2").await;
        assert_eq!((status, body), (409, json!({"error": "feature_mismatch"})));
        assert_eq!(bound_feature(&db).await.as_deref(), Some("f-1"));
        assert!(!token_stored(&db).await);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_app_not_installed_is_recorded_for_the_run_until_a_token_is_minted() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let runner = Paired::new(&db, user, InstanceKind::Runner).await;
        runner.started(&db, true).await;
        let github = Arc::new(FakeGithub::default());
        *github.uninstalled.lock().unwrap() = true;
        let base = serve_state(state_over(db.clone(), Arc::clone(&github))).await;
        let problem = || git_token_problem_of(user, &db, runner.instance, "f-1");

        let (status, _, body) = runner.ask(&base, "f-1").await;
        assert_eq!(
            (status, body),
            (503, json!({"error": "github_app_not_installed"}))
        );
        assert_eq!(
            problem().await.unwrap().as_deref(),
            Some("github_app_not_installed")
        );

        *github.uninstalled.lock().unwrap() = false;
        assert_eq!(runner.ask(&base, "f-1").await.0, 200);
        assert_eq!(problem().await.unwrap(), None);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn of_two_first_fetches_at_once_exactly_one_binds_its_feature() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let runner = Paired::new(&db, user, InstanceKind::Runner).await;
        runner.started(&db, true).await;
        let base = serve_state(state_over(db.clone(), Arc::default())).await;

        for round in 0..5 {
            sqlx::query("UPDATE instance_requests SET git_token_feature_id = NULL")
                .execute(&db)
                .await
                .unwrap();
            let (a, b) = tokio::join!(runner.ask(&base, "f-a"), runner.ask(&base, "f-b"));
            let mut statuses = [a.0, b.0];
            statuses.sort_unstable();
            assert_eq!(statuses, [200, 409], "round {round}");
            let winner = if a.0 == 200 { "f-a" } else { "f-b" };
            assert_eq!(bound_feature(&db).await.as_deref(), Some(winner));
        }
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn no_token_for_an_unanswered_start_an_ended_run_a_desktop_or_no_credentials() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let github = Arc::new(FakeGithub::default());
        let base = serve_state(state_over(db.clone(), Arc::clone(&github))).await;

        let unanswered = Paired::new(&db, user, InstanceKind::Runner).await;
        unanswered.started(&db, false).await;
        assert_eq!(unanswered.ask(&base, "f-1").await.0, 404);

        let ended = Paired::new(&db, user, InstanceKind::Runner).await;
        ended.started(&db, true).await;
        ended.ended(&db, "f-1").await;
        let (status, _, body) = ended.ask(&base, "f-1").await;
        assert_eq!((status, body), (410, json!({"error": "run_ended"})));

        let desktop = Paired::new(&db, user, InstanceKind::Desktop).await;
        desktop.started(&db, true).await;
        assert_eq!(desktop.ask(&base, "f-1").await.0, 404);

        let other_tenant = create_user(&db).await.unwrap();
        let stranger = Paired::new(&db, other_tenant, InstanceKind::Runner).await;
        assert_eq!(stranger.ask(&base, "f-1").await.0, 404);

        let response = reqwest::Client::new()
            .post(format!("{base}{PATH}"))
            .json(&json!({"request_id": REQUEST_TEXT, "feature_id": "f-1"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 401);
        assert_eq!(
            response
                .headers()
                .get("www-authenticate")
                .and_then(|v| v.to_str().ok()),
            Some("DPoP")
        );

        assert!(github.asked.lock().unwrap().is_empty());
        let bound: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM instance_requests WHERE git_token_feature_id IS NOT NULL",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(bound, 0);
    })
    .await;
}

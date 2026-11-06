//! Connecting a GitHub App over the real router and real rows: the session
//! starts a manifest, GitHub's redirect back (here a fake GitHub) is admitted
//! by its `state` alone, the key lands in the key service and the row in
//! Postgres, and a replayed redirect finds nothing. The rules themselves are
//! held without a database in `src/git_hosts/`.
//!
//! The tests are ignored for the reason `crates/demeteo-hub/tests/pairing.rs`
//! gives in its header, and an ignored run shows nothing of them.

mod support;

use aws_lc_rs::encoding::{AsDer, Pkcs8V1Der};
use aws_lc_rs::rsa::{KeyPair, KeySize};
use axum::extract::Path;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::attachments::AttachmentSettings;
use demeteo_hub::auth::policy::Status;
use demeteo_hub::auth::testing::SoftAuthenticator;
use demeteo_hub::git_hosts::manifest::{ConnectableHost, GitHostsSetup};
use demeteo_hub::git_hosts::postgres::PostgresGitHosts;
use demeteo_hub::ports::RsaSigner;
use demeteo_hub::routes::AppState;
use demeteo_hub::store::pool::create_user;
use demeteo_hub::store::Db;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use support::http::{enrol, get_as, open_session, post_as, rp, serve_state};
use support::in_fresh_schema;

const APP_ID: u64 = 5_647_010;

/// GitHub's conversion: each code converts once, to an App with a fresh
/// PKCS#8 key, and is a 404 after.
async fn fake_github() -> String {
    let used: Arc<Mutex<HashSet<String>>> = Arc::default();
    let app = Router::new().route(
        "/app-manifests/{code}/conversions",
        post(move |Path(code): Path<String>| {
            let used = Arc::clone(&used);
            async move {
                if !used.lock().unwrap().insert(code) {
                    return (StatusCode::NOT_FOUND, Json(json!({})));
                }
                let key = KeyPair::generate(KeySize::Rsa2048).unwrap();
                let der = AsDer::<Pkcs8V1Der>::as_der(&key).unwrap();
                let pem = format!(
                    "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
                    STANDARD.encode(der.as_ref())
                );
                (
                    StatusCode::CREATED,
                    Json(json!({
                        "id": APP_ID,
                        "slug": "demeteo-hub-test",
                        "owner": {"login": "octo-org"},
                        "client_secret": "never kept",
                        "webhook_secret": "never kept",
                        "pem": pem,
                    })),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn state_over(db: Db, keys: Arc<MemoryKeys>) -> AppState {
    let setup = GitHostsSetup {
        store: Arc::new(PostgresGitHosts::new(db.clone())),
        importer: keys,
        http: reqwest::Client::new(),
        hosts: vec![ConnectableHost {
            host: "github.com".to_owned(),
            api_base: fake_github().await,
            web_base: "https://github.com".to_owned(),
        }],
    };
    AppState::new(
        db,
        rp(),
        Arc::new(MemoryKeys::with_keys(["signing", "transit"])),
        "signing".to_owned(),
        "transit".to_owned(),
        AttachmentSettings::default(),
    )
    .unwrap()
    .with_git_hosts(Arc::new(setup))
}

/// GitHub's redirect, as the browser follows it: no cookie, no `Origin`.
async fn redirected(base: &str, code: &str, state: &str) -> (u16, String) {
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(format!(
            "{base}/github/manifest/callback?code={code}&state={state}"
        ))
        .send()
        .await
        .unwrap();
    let location = response.headers()["location"].to_str().unwrap().to_owned();
    assert_eq!(response.headers()["cache-control"], "no-store");
    (response.status().as_u16(), location)
}

fn state_of(form: &Value) -> String {
    form["state"].as_str().unwrap().to_owned()
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_manifest_round_trip_connects_the_app_once() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let key = SoftAuthenticator::new();
        enrol(&db, user, &key, Status::Trusted).await;
        let session = open_session(&db, user, &key).await;
        let keys = Arc::new(MemoryKeys::with_keys(["signing"]));
        let base = serve_state(state_over(db.clone(), Arc::clone(&keys)).await).await;

        let (status, form) =
            post_as(&base, "/v1/git-hosts/github/manifest", &session, &json!({})).await;
        assert_eq!(status, 200, "{form}");
        assert_eq!(form["action_url"], "https://github.com/settings/apps/new");
        let manifest: Value = serde_json::from_str(form["manifest"].as_str().unwrap()).unwrap();
        assert_eq!(
            manifest["redirect_url"],
            format!("{}/github/manifest/callback", rp().origin)
        );

        let (status, location) = redirected(&base, "c0de1", &state_of(&form)).await;
        assert_eq!(status, 303);
        let id = location
            .strip_prefix("/#/git-hosts/connected/")
            .unwrap_or_else(|| panic!("{location}"));

        let (status, listed) = get_as(&base, "/v1/git-hosts", &session).await;
        assert_eq!(status, 200);
        assert_eq!(listed["connectable"], json!(["github.com"]));
        let host = &listed["hosts"][0];
        assert_eq!(host["id"], id);
        assert_eq!(host["app_id"], APP_ID);
        assert_eq!(host["account"], "octo-org");
        assert_eq!(
            host["install_url"],
            "https://github.com/apps/demeteo-hub-test/installations/new"
        );
        assert!(keys
            .sign_rs256(&format!("demeteo-hub-gh-{APP_ID}"), b"a.b")
            .await
            .is_ok());

        let (_, replayed) = redirected(&base, "c0de1", &state_of(&form)).await;
        assert_eq!(replayed, "/#/git-hosts/failed/expired");
        let (_, listed) = get_as(&base, "/v1/git-hosts", &session).await;
        assert_eq!(listed["hosts"].as_array().unwrap().len(), 1);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn another_tenant_neither_sees_nor_removes_the_app() {
    in_fresh_schema(|db| async move {
        let (owner, other) = (
            create_user(&db).await.unwrap(),
            create_user(&db).await.unwrap(),
        );
        let (owner_key, other_key) = (SoftAuthenticator::new(), SoftAuthenticator::new());
        enrol(&db, owner, &owner_key, Status::Trusted).await;
        enrol(&db, other, &other_key, Status::Trusted).await;
        let owner_session = open_session(&db, owner, &owner_key).await;
        let other_session = open_session(&db, other, &other_key).await;
        let keys = Arc::new(MemoryKeys::with_keys(["signing"]));
        let base = serve_state(state_over(db.clone(), keys).await).await;

        let (_, form) = post_as(
            &base,
            "/v1/git-hosts/github/manifest",
            &owner_session,
            &json!({}),
        )
        .await;
        let (_, location) = redirected(&base, "c0de2", &state_of(&form)).await;
        let id = location.rsplit('/').next().unwrap().to_owned();

        let (_, seen) = get_as(&base, "/v1/git-hosts", &other_session).await;
        assert_eq!(seen["hosts"], json!([]));
        let path = format!("/v1/git-hosts/{id}/remove");
        let (status, _) = post_as(&base, &path, &other_session, &json!({})).await;
        assert_eq!(status, 404);
        let (status, removed) = post_as(&base, &path, &owner_session, &json!({})).await;
        assert_eq!((status, removed), (200, json!({"removed": true})));
        let (_, seen) = get_as(&base, "/v1/git-hosts", &owner_session).await;
        assert_eq!(seen["hosts"], json!([]));
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_redirect_with_no_live_state_connects_nothing() {
    in_fresh_schema(|db| async move {
        let keys = Arc::new(MemoryKeys::with_keys(["signing"]));
        let base = serve_state(state_over(db, keys).await).await;
        let (status, location) = redirected(&base, "c0de3", "never-issued").await;
        assert_eq!(
            (status, location.as_str()),
            (303, "/#/git-hosts/failed/expired")
        );
    })
    .await;
}

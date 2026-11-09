//! Hub Projects and the starter Workflows over the real router and real rows:
//! the tenant filter and the uniqueness in the SQL, which the
//! `MemoryProjects` tests in `src/projects/` cannot reach. The rules
//! themselves are held without a database in `src/projects/verdict.rs`.
//!
//! The tests are ignored for the reason `crates/demeteo-hub/tests/pairing.rs`
//! gives in its header, and an ignored run shows nothing of them.

mod support;

use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::attachments::AttachmentSettings;
use demeteo_hub::auth::policy::Status;
use demeteo_hub::auth::session::SessionSecret;
use demeteo_hub::auth::testing::SoftAuthenticator;
use demeteo_hub::routes::AppState;
use demeteo_hub::store::hub_projects::{
    insert_hub_project, list_hub_projects, HubProjectFields, HubProjectId,
};
use demeteo_hub::store::pool::create_user;
use demeteo_hub::store::{Db, StoreError, UserId};
use demeteo_hub::workflows::starters;
use serde_json::{json, Value};
use std::sync::Arc;
use support::http::{enrol, get_as, open_session, post_as, rp, serve_state};
use support::in_fresh_schema;

fn state_over(db: Db) -> AppState {
    AppState::new(
        db,
        rp(),
        Arc::new(MemoryKeys::with_keys(["signing", "transit"])),
        "signing".to_owned(),
        "transit".to_owned(),
        AttachmentSettings::default(),
    )
    .unwrap()
}

async fn signed_in(db: &Db) -> (UserId, SessionSecret) {
    let user = create_user(db).await.unwrap();
    let key = SoftAuthenticator::new();
    enrol(db, user, &key, Status::Trusted).await;
    (user, open_session(db, user, &key).await)
}

fn body(name: &str, full_name: &str) -> Value {
    json!({
        "name": name,
        "provider_kind": "github",
        "host": "GitHub.com",
        "full_name": full_name,
        "default_agent_kind": "claude-code",
        "default_model": "opus",
        "default_effort": "high",
        "default_workflow_id": starters()[0].id,
        "default_max_budget_cents": 250,
    })
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_project_is_created_listed_replaced_and_removed() {
    in_fresh_schema(|db| async move {
        let (_, session) = signed_in(&db).await;
        let base = serve_state(state_over(db.clone())).await;

        let (status, created) = post_as(
            &base,
            "/v1/projects",
            &session,
            &body("Widgets", "acme/widgets"),
        )
        .await;
        assert_eq!(status, 201, "{created}");
        assert_eq!(created["host"], "github.com");
        assert_eq!(created["remote_url"], "https://github.com/acme/widgets");
        assert_eq!(created["runner_ready"], true);
        assert_eq!(created["default_effort"], "high");
        assert_eq!(created["default_max_budget_cents"], 250);
        assert_eq!(created["default_workflow_id"], json!(starters()[0].id));
        let id = created["id"].as_str().unwrap().to_owned();

        post_as(
            &base,
            "/v1/projects",
            &session,
            &body("Another", "acme/another"),
        )
        .await;
        let (status, listed) = get_as(&base, "/v1/projects", &session).await;
        assert_eq!(status, 200);
        let names: Vec<&str> = listed["projects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["Another", "Widgets"]);

        let mut replaced = body("Gadgets", "acme/gadgets");
        replaced["default_effort"] = Value::Null;
        replaced["provider_kind"] = json!("gitlab");
        let (status, updated) =
            post_as(&base, &format!("/v1/projects/{id}"), &session, &replaced).await;
        assert_eq!(status, 200, "{updated}");
        assert_eq!(updated["id"], id.as_str());
        assert_eq!(updated["full_name"], "acme/gadgets");
        assert_eq!(updated["default_effort"], Value::Null);
        assert_eq!(updated["runner_ready"], false);
        assert_eq!(updated["created_at"], created["created_at"]);

        let (status, gone) = post_as(
            &base,
            &format!("/v1/projects/{id}/remove"),
            &session,
            &json!({}),
        )
        .await;
        assert_eq!(status, 204, "{gone}");
        let (status, _) = post_as(
            &base,
            &format!("/v1/projects/{id}/remove"),
            &session,
            &json!({}),
        )
        .await;
        assert_eq!(status, 404);
        let (_, listed) = get_as(&base, "/v1/projects", &session).await;
        assert_eq!(listed["projects"].as_array().unwrap().len(), 1);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn another_tenant_neither_sees_nor_changes_the_project() {
    in_fresh_schema(|db| async move {
        let (_, owner) = signed_in(&db).await;
        let (_, other) = signed_in(&db).await;
        let base = serve_state(state_over(db.clone())).await;
        let (_, created) = post_as(
            &base,
            "/v1/projects",
            &owner,
            &body("Widgets", "acme/widgets"),
        )
        .await;
        let id = created["id"].as_str().unwrap().to_owned();

        let (_, seen) = get_as(&base, "/v1/projects", &other).await;
        assert_eq!(seen["projects"], json!([]));
        let (status, _) = post_as(
            &base,
            &format!("/v1/projects/{id}"),
            &other,
            &body("Mine", "acme/mine"),
        )
        .await;
        assert_eq!(status, 404);
        let (status, _) = post_as(
            &base,
            &format!("/v1/projects/{id}/remove"),
            &other,
            &json!({}),
        )
        .await;
        assert_eq!(status, 404);

        let (status, created_by_other) = post_as(
            &base,
            "/v1/projects",
            &other,
            &body("Widgets", "acme/widgets"),
        )
        .await;
        assert_eq!(status, 201, "{created_by_other}");
        let (_, kept) = get_as(&base, "/v1/projects", &owner).await;
        assert_eq!(kept["projects"][0]["name"], "Widgets");
        assert_eq!(kept["projects"][0]["id"], id.as_str());
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_second_project_on_one_repository_is_project_exists() {
    in_fresh_schema(|db| async move {
        let (_, session) = signed_in(&db).await;
        let base = serve_state(state_over(db.clone())).await;
        post_as(
            &base,
            "/v1/projects",
            &session,
            &body("Widgets", "acme/widgets"),
        )
        .await;
        let (_, other) = post_as(
            &base,
            "/v1/projects",
            &session,
            &body("Other", "acme/other"),
        )
        .await;

        let (status, refused) = post_as(
            &base,
            "/v1/projects",
            &session,
            &body("Again", "acme/widgets"),
        )
        .await;
        assert_eq!((status, refused), (409, json!({"error": "project_exists"})));
        let other_id = other["id"].as_str().unwrap();
        let (status, refused) = post_as(
            &base,
            &format!("/v1/projects/{other_id}"),
            &session,
            &body("Other", "acme/widgets"),
        )
        .await;
        assert_eq!((status, refused), (409, json!({"error": "project_exists"})));
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn removing_the_user_removes_their_projects() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let fields = HubProjectFields {
            name: "Widgets".to_owned(),
            provider_kind: "github".to_owned(),
            host: "github.com".to_owned(),
            full_name: "acme/widgets".to_owned(),
            default_agent_kind: None,
            default_model: None,
            default_effort: None,
            default_workflow_id: Some("wf-starter-dropped-by-a-later-build".to_owned()),
            default_max_budget_cents: None,
        };
        insert_hub_project(user, &db, HubProjectId::generate(), &fields)
            .await
            .unwrap();
        let again = insert_hub_project(user, &db, HubProjectId::generate(), &fields).await;
        assert!(matches!(again, Err(StoreError::ProjectExists)), "{again:?}");

        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user)
            .execute(&db)
            .await
            .unwrap();

        assert!(list_hub_projects(user, &db).await.unwrap().is_empty());
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM hub_projects")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(count, 0);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn the_starters_are_listed_to_a_session_and_to_nobody_else() {
    in_fresh_schema(|db| async move {
        let (_, session) = signed_in(&db).await;
        let base = serve_state(state_over(db.clone())).await;

        let (status, listed) = get_as(&base, "/v1/workflows", &session).await;
        assert_eq!(status, 200);
        let workflows = listed["workflows"].as_array().unwrap();
        assert_eq!(workflows.len(), starters().len());
        assert!(workflows.iter().all(|w| w.get("steps_json").is_none()));

        let anonymous = reqwest::get(format!("{base}/v1/workflows")).await.unwrap();
        assert_eq!(anonymous.status().as_u16(), 401);
        let anonymous = reqwest::get(format!("{base}/v1/projects")).await.unwrap();
        assert_eq!(anonymous.status().as_u16(), 401);
    })
    .await;
}

//! The project routes. Each handler is a thin shell over a use-case that
//! takes the store and the tenant, so the rules are reachable from a test
//! without Postgres. The tenant is the session's, never a path's: another
//! tenant's project answers `404` as one that does not exist does.

use super::port::HubProjectStore;
use super::postgres::PostgresProjects;
use super::verdict::{project_verdict, remote_url, runner_ready, ProjectBody, ProjectRefusal};
use crate::auth::handlers::{store_failed, ApiError, CurrentSession, JsonBody};
use crate::routes::AppState;
use crate::store::hub_projects::{HubProjectId, HubProjectRow};
use crate::store::{StoreError, UserId};
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use demeteo_hub_protocol::Effort;
use serde::Serialize;
use uuid::Uuid;

const PROJECT_EXISTS: ApiError = ApiError::new(StatusCode::CONFLICT, "project_exists");

#[derive(Debug, Serialize)]
pub struct ProjectsView {
    projects: Vec<ProjectView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectView {
    id: Uuid,
    name: String,
    provider_kind: String,
    host: String,
    full_name: String,
    /// Derived from `host` and `full_name`: the URL a runner clones.
    remote_url: String,
    /// False for a project a runner cannot be dispatched against yet.
    runner_ready: bool,
    default_agent_kind: Option<String>,
    default_model: Option<String>,
    default_effort: Option<Effort>,
    default_workflow_id: Option<String>,
    default_max_budget_cents: Option<u64>,
    /// Unix seconds.
    created_at: i64,
    /// Unix seconds.
    updated_at: i64,
}

impl From<HubProjectRow> for ProjectView {
    fn from(row: HubProjectRow) -> Self {
        let fields = row.fields;
        Self {
            id: row.id.0,
            remote_url: remote_url(&fields.host, &fields.full_name),
            runner_ready: runner_ready(&fields.provider_kind),
            name: fields.name,
            provider_kind: fields.provider_kind,
            host: fields.host,
            full_name: fields.full_name,
            default_agent_kind: fields.default_agent_kind,
            default_model: fields.default_model,
            default_effort: fields.default_effort,
            default_workflow_id: fields.default_workflow_id,
            default_max_budget_cents: fields.default_max_budget_cents,
            created_at: row.created_at.unix_timestamp(),
            updated_at: row.updated_at.unix_timestamp(),
        }
    }
}

impl From<ProjectRefusal> for ApiError {
    fn from(_: ProjectRefusal) -> Self {
        ApiError::INVALID_REQUEST
    }
}

fn write_failed(error: StoreError) -> ApiError {
    match error {
        StoreError::ProjectExists => PROJECT_EXISTS,
        other => store_failed(other),
    }
}

pub async fn list_projects(
    store: &dyn HubProjectStore,
    user: UserId,
) -> Result<ProjectsView, ApiError> {
    let rows = store.projects(user).await.map_err(store_failed)?;
    Ok(ProjectsView {
        projects: rows.into_iter().map(ProjectView::from).collect(),
    })
}

pub async fn create_project(
    store: &dyn HubProjectStore,
    user: UserId,
    body: ProjectBody,
) -> Result<ProjectView, ApiError> {
    let fields = project_verdict(body)?;
    let row = store
        .insert(user, HubProjectId::generate(), &fields)
        .await
        .map_err(write_failed)?;
    Ok(row.into())
}

pub async fn update_project(
    store: &dyn HubProjectStore,
    user: UserId,
    id: HubProjectId,
    body: ProjectBody,
) -> Result<ProjectView, ApiError> {
    let fields = project_verdict(body)?;
    store
        .update(user, id, &fields)
        .await
        .map_err(write_failed)?
        .map(ProjectView::from)
        .ok_or(ApiError::NOT_FOUND)
}

pub async fn remove_project(
    store: &dyn HubProjectStore,
    user: UserId,
    id: HubProjectId,
) -> Result<(), ApiError> {
    if store.remove(user, id).await.map_err(store_failed)? {
        Ok(())
    } else {
        Err(ApiError::NOT_FOUND)
    }
}

fn project_id(path: Result<Path<Uuid>, PathRejection>) -> Result<HubProjectId, ApiError> {
    let Path(id) = path.map_err(|_| ApiError::NOT_FOUND)?;
    Ok(HubProjectId(id))
}

/// `GET /v1/projects`.
pub async fn get_projects(
    State(state): State<AppState>,
    current: CurrentSession,
) -> Result<Json<ProjectsView>, ApiError> {
    let store = PostgresProjects::new(state.db().clone());
    list_projects(&store, current.user).await.map(Json)
}

/// `POST /v1/projects`.
pub async fn post_project(
    State(state): State<AppState>,
    current: CurrentSession,
    JsonBody(body): JsonBody<ProjectBody>,
) -> Result<(StatusCode, Json<ProjectView>), ApiError> {
    let store = PostgresProjects::new(state.db().clone());
    let view = create_project(&store, current.user, body).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

/// `POST /v1/projects/{id}`: a full replace.
pub async fn post_project_update(
    State(state): State<AppState>,
    current: CurrentSession,
    id: Result<Path<Uuid>, PathRejection>,
    JsonBody(body): JsonBody<ProjectBody>,
) -> Result<Json<ProjectView>, ApiError> {
    let id = project_id(id)?;
    let store = PostgresProjects::new(state.db().clone());
    update_project(&store, current.user, id, body)
        .await
        .map(Json)
}

/// `POST /v1/projects/{id}/remove`. A run already queued or started from the
/// project keeps what it was signed with.
pub async fn post_project_remove(
    State(state): State<AppState>,
    current: CurrentSession,
    id: Result<Path<Uuid>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let id = project_id(id)?;
    let store = PostgresProjects::new(state.db().clone());
    remove_project(&store, current.user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::testing::MemoryProjects;
    use serde_json::json;

    fn body(name: &str, provider_kind: &str, full_name: &str) -> ProjectBody {
        serde_json::from_value(json!({
            "name": name,
            "provider_kind": provider_kind,
            "host": "GitHub.com",
            "full_name": full_name,
            "default_effort": "high",
            "default_max_budget_cents": 250,
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn a_created_project_is_listed_with_its_derived_fields() {
        let store = MemoryProjects::default();
        let user = UserId::generate();

        let created = create_project(&store, user, body("Widgets", "github", "acme/widgets"))
            .await
            .unwrap();
        let listed = list_projects(&store, user).await.unwrap();

        let view = serde_json::to_value(&listed).unwrap();
        assert_eq!(view["projects"][0]["id"], json!(created.id));
        assert_eq!(view["projects"][0]["host"], "github.com");
        assert_eq!(
            view["projects"][0]["remote_url"],
            "https://github.com/acme/widgets"
        );
        assert_eq!(view["projects"][0]["runner_ready"], true);
        assert_eq!(view["projects"][0]["default_effort"], "high");
        assert_eq!(view["projects"][0]["default_max_budget_cents"], 250);
    }

    #[tokio::test]
    async fn a_gitlab_project_is_kept_and_not_runner_ready() {
        let store = MemoryProjects::default();
        let created = create_project(
            &store,
            UserId::generate(),
            body("Group", "gitlab", "group/sub/repo"),
        )
        .await
        .unwrap();
        assert!(!created.runner_ready);
    }

    #[tokio::test]
    async fn a_second_project_on_one_repository_is_project_exists() {
        let store = MemoryProjects::default();
        let user = UserId::generate();
        create_project(&store, user, body("One", "github", "acme/widgets"))
            .await
            .unwrap();
        let other = create_project(&store, user, body("Other", "github", "acme/other"))
            .await
            .unwrap();

        let again = create_project(&store, user, body("Two", "github", "acme/widgets")).await;
        assert_eq!(again, Err(PROJECT_EXISTS));
        let moved = update_project(
            &store,
            user,
            HubProjectId(other.id),
            body("Other", "github", "acme/widgets"),
        )
        .await;
        assert_eq!(moved, Err(PROJECT_EXISTS));
        let elsewhere = create_project(
            &store,
            UserId::generate(),
            body("Two", "github", "acme/widgets"),
        )
        .await;
        assert!(elsewhere.is_ok());
    }

    #[tokio::test]
    async fn another_tenants_project_is_not_found_on_update_and_remove() {
        let store = MemoryProjects::default();
        let owner = UserId::generate();
        let created = create_project(&store, owner, body("Widgets", "github", "acme/widgets"))
            .await
            .unwrap();
        let id = HubProjectId(created.id);
        let stranger = UserId::generate();

        let updated =
            update_project(&store, stranger, id, body("Mine", "github", "acme/widgets")).await;
        assert_eq!(updated, Err(ApiError::NOT_FOUND));
        assert_eq!(
            remove_project(&store, stranger, id).await,
            Err(ApiError::NOT_FOUND)
        );
        assert!(list_projects(&store, stranger)
            .await
            .unwrap()
            .projects
            .is_empty());

        let renamed = update_project(&store, owner, id, body("Renamed", "github", "acme/widgets"))
            .await
            .unwrap();
        assert_eq!(renamed.name, "Renamed");
        assert_eq!(remove_project(&store, owner, id).await, Ok(()));
        assert_eq!(
            remove_project(&store, owner, id).await,
            Err(ApiError::NOT_FOUND)
        );
    }

    #[tokio::test]
    async fn a_refused_body_writes_nothing() {
        let store = MemoryProjects::default();
        let user = UserId::generate();
        let refused = create_project(&store, user, body("Widgets", "github", "widgets")).await;
        assert_eq!(refused, Err(ApiError::INVALID_REQUEST));
        assert!(list_projects(&store, user)
            .await
            .unwrap()
            .projects
            .is_empty());
    }
}

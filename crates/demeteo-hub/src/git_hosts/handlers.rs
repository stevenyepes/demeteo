//! The git-host routes: a list and a removal under the session, the start of
//! a manifest round trip under the session, and GitHub's redirect back,
//! which carries no session and is admitted by its single-use `state`
//! ([`manifest::complete`]).

use super::manifest::{
    self, connected_page, failed_page, install_url, CallbackFailure, GitHostsSetup, ManifestForm,
    StartRefusal, StartRequest,
};
use crate::auth::handlers::{
    ceremony_not_held, store_failed, unix_now, ApiError, CurrentSession, JsonBody,
};
use crate::ports::{ReachableRepository, UnlistedApp};
use crate::routes::AppState;
use crate::store::git_hosts::GitHostId;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::header::CACHE_CONTROL;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const GIT_HOSTS_UNAVAILABLE: ApiError =
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "git_hosts_unavailable");

#[derive(Serialize)]
pub struct HostsView {
    /// The hosts an App can be connected on, github.com first.
    connectable: Vec<String>,
    hosts: Vec<HostView>,
}

#[derive(Serialize)]
struct HostView {
    id: Uuid,
    host: String,
    app_id: i64,
    app_slug: String,
    account: Option<String>,
    install_url: String,
    /// Unix seconds.
    connected_at: i64,
}

#[derive(Serialize)]
pub struct RepositoriesView {
    repositories: Vec<ReachableRepository>,
    /// Apps that did not answer: their repositories are missing from the
    /// list, and the page says so instead of implying the list is whole.
    unlisted: Vec<UnlistedApp>,
    truncated: bool,
}

/// `GET /v1/git-hosts/repositories`: what the project picker offers.
/// Never cached, since installing an App on one more repository should
/// show the next time the page asks.
pub async fn get_repositories(
    State(state): State<AppState>,
    current: CurrentSession,
) -> Result<Response, ApiError> {
    let lister = state.repositories().ok_or(GIT_HOSTS_UNAVAILABLE)?;
    let listing = lister
        .repositories(current.user, unix_now())
        .await
        .map_err(|error| {
            tracing::error!(%error, "the connected git hosts could not be read");
            ApiError::INTERNAL
        })?;
    let view = RepositoriesView {
        repositories: listing.repositories,
        unlisted: listing.unlisted,
        truncated: listing.truncated,
    };
    Ok(([(CACHE_CONTROL, "no-store")], Json(view)).into_response())
}

fn setup(state: &AppState) -> Result<&GitHostsSetup, ApiError> {
    state.git_hosts().ok_or(GIT_HOSTS_UNAVAILABLE)
}

/// `GET /v1/git-hosts`.
pub async fn get_git_hosts(
    State(state): State<AppState>,
    current: CurrentSession,
) -> Result<Json<HostsView>, ApiError> {
    let setup = setup(&state)?;
    let rows = setup
        .store
        .hosts(current.user)
        .await
        .map_err(store_failed)?;
    Ok(Json(HostsView {
        connectable: setup.hosts.iter().map(|h| h.host.clone()).collect(),
        hosts: rows
            .into_iter()
            .map(|row| HostView {
                id: row.id.0,
                install_url: install_url(&row.web_base, &row.app_slug),
                host: row.host,
                app_id: row.app_id,
                app_slug: row.app_slug,
                account: row.account,
                connected_at: row.created_at.unix_timestamp(),
            })
            .collect(),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestBody {
    host: Option<String>,
    organization: Option<String>,
}

/// `POST /v1/git-hosts/github/manifest`: the form hub-web posts to GitHub.
pub async fn post_manifest(
    State(state): State<AppState>,
    current: CurrentSession,
    JsonBody(body): JsonBody<ManifestBody>,
) -> Result<Json<ManifestForm>, ApiError> {
    let setup = setup(&state)?;
    let organization = body.organization.as_deref().filter(|o| !o.is_empty());
    let request = StartRequest {
        user: current.user,
        holder: current.credential_id.clone(),
        host: body.host.as_deref(),
        organization,
    };
    manifest::start(setup, state.ceremonies(), state.rp(), unix_now(), request)
        .map(Json)
        .map_err(|refusal| match refusal {
            StartRefusal::UnknownHost | StartRefusal::BadOrganization => ApiError::INVALID_REQUEST,
            StartRefusal::Ceremony(error) => ceremony_not_held(error),
        })
}

/// `POST /v1/git-hosts/{id}/remove`. The App stays on GitHub and its key in
/// Transit; the Hub stops minting with it.
pub async fn post_remove(
    State(state): State<AppState>,
    current: CurrentSession,
    id: Result<Path<Uuid>, PathRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::INVALID_REQUEST)?;
    let removed = setup(&state)?
        .store
        .remove(current.user, GitHostId(id))
        .await
        .map_err(store_failed)?;
    if removed {
        Ok(Json(serde_json::json!({"removed": true})))
    } else {
        Err(ApiError::NOT_FOUND)
    }
}

#[derive(Deserialize)]
pub struct CallbackParams {
    code: String,
    state: String,
}

/// `GET` [`MANIFEST_CALLBACK_PATH`](manifest::MANIFEST_CALLBACK_PATH): always
/// a redirect to hub-web's page, which says how it went. Never cached, since
/// the URL carries a one-time code.
pub async fn manifest_callback(
    State(state): State<AppState>,
    query: Result<Query<CallbackParams>, QueryRejection>,
) -> Response {
    let page = match (query, state.git_hosts()) {
        (Ok(Query(params)), Some(setup)) => manifest::complete(
            setup,
            state.ceremonies(),
            unix_now(),
            &params.code,
            &params.state,
        )
        .await
        .map_or_else(failed_page, connected_page),
        (Err(_), _) => failed_page(CallbackFailure::Expired),
        (_, None) => failed_page(CallbackFailure::KeyServiceUnavailable),
    };
    ([(CACHE_CONTROL, "no-store")], Redirect::to(&page)).into_response()
}

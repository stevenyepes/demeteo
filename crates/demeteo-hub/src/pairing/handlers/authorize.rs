//! `GET /authorize`: where a desktop sends the user's browser to start a
//! pairing.
//!
//! Every refusal is answered here, as a `400`, and never as the redirect to
//! the client that RFC 6749 §4.1.2.1 has an authorization server send. That
//! redirect goes to a `redirect_uri` the server already holds for the client.
//! The Hub registers no client, so the only one it has is the one in the
//! request being refused: redirecting there would send a browser to an
//! address chosen by whoever built the link, before anything about the link
//! was accepted.

use crate::auth::handlers::ApiError;
use crate::pairing::authorize::{encode, validate, AuthorizeParams};
use axum::extract::rejection::QueryRejection;
use axum::extract::Query;
use axum::http::header::CACHE_CONTROL;
use axum::response::{IntoResponse, Redirect, Response};

/// hub-web's route for the approval page, up to the segment it is handed.
const APPROVAL_PAGE: &str = "/#/pair/";

/// The query is taken as a `Result` because axum's own rejection answers in
/// prose: a parameter sent twice is refused by the extractor before
/// [`validate`] is asked anything, and is told the same as any other refusal.
pub async fn authorize(
    query: Result<Query<AuthorizeParams>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(params) = query.map_err(|_| ApiError::INVALID_REQUEST)?;
    let request = validate(&params).map_err(|_| ApiError::INVALID_REQUEST)?;
    let approval_page = format!("{APPROVAL_PAGE}{}", encode(&request));
    Ok(([(CACHE_CONTROL, "no-store")], Redirect::to(&approval_page)).into_response())
}

//! An instance fetching an attachment a request it was sent names. The
//! fetch-once rule it keeps is [the module's](super); `docs/HUB.md` §6.3 is
//! the lifecycle.
//!
//! The tenant and the instance are the [`Admission`]'s and nothing else's:
//! the id is the only thing the caller chooses, and every id that is not
//! this instance's to fetch now (another instance's, another tenant's,
//! unbound, its request answered or expired, never minted, not a UUID) is
//! the one [`ApiError::NOT_FOUND`]. A refusal per reason would tell a holder
//! of one token what the others hold.
//!
//! The row is deleted only after its content has opened, so a key service
//! that is away or a row that does not authenticate leaves it to be fetched
//! again or swept. Past the delete there is no retry: a response lost in
//! transit loses the attachment.
//!
//! [`get_attachment`] is the route. Like the socket it is called by an
//! instance with its access token, so it is admitted by
//! [`pairing::access`](crate::pairing::access); this module only says which
//! method and URL its proofs are made for.

use super::envelope::{self, EnvelopeError};
use super::port::AttachmentStore;
use super::postgres::PostgresAttachments;
use crate::auth::handlers::{header_text, store_failed, unix_now, ApiError};
use crate::pairing::access::{self, refusal, AccessStore, Admission, PostgresAccess};
use crate::pairing::dpop::Expected;
use crate::pairing::handlers::one_proof;
use crate::ports::KeyService;
use crate::routes::AppState;
use axum::extract::{Path, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

/// The route [`get_attachment`] is mounted at. A proof is made for the URL
/// its request is sent to, so [`fetch_htu`] has to spell the same path.
pub const FETCH_ROUTE: &str = "/attachments/{attachment_id}";
const FETCH_METHOD: &str = "GET";

/// The `htu` a proof for fetching `attachment_id` has to name, built by
/// [`access::htu`] and so from the configured origin. One per id: a proof
/// made for one attachment is no proof for another.
pub fn fetch_htu(state: &AppState, attachment_id: &str) -> String {
    access::htu(state, &format!("/attachments/{attachment_id}"))
}

/// [`access::admitted`] for a fetch of `attachment_id`.
pub async fn admitted(
    state: &AppState,
    attachment_id: &str,
    authorization: Option<&str>,
    proof: Option<&str>,
    now: u64,
    store: &impl AccessStore,
) -> Result<Admission, ApiError> {
    let htu = fetch_htu(state, attachment_id);
    let expected = Expected {
        htm: FETCH_METHOD,
        htu: &htu,
    };
    access::admitted(state, authorization, proof, &expected, now, store).await
}

/// The content of attachment `attachment_id`, which this call has deleted.
/// `now` is Unix seconds, judged against the bound request's `expires_at`.
pub async fn fetch(
    store: &dyn AttachmentStore,
    keys: &dyn KeyService,
    transit_key: &str,
    admission: &Admission,
    attachment_id: &str,
    now: u64,
) -> Result<Vec<u8>, ApiError> {
    let id = Uuid::parse_str(attachment_id).map_err(|_| ApiError::NOT_FOUND)?;
    let row = store
        .fetchable(admission.user, admission.instance, id, now)
        .await
        .map_err(store_failed)?
        .ok_or(ApiError::NOT_FOUND)?;

    let data_key = keys.decrypt(transit_key, &row.wrapped_key).await?;
    let aad = envelope::aad(id, admission.user, admission.instance);
    let content = envelope::open(&data_key, &aad, &row.ciphertext).map_err(open_failed)?;
    drop(data_key);

    // Of two fetches that both opened the row, only the one whose delete
    // took it hands the content over; neither does if the request ended
    // while it was being opened.
    let consumed = store
        .consumed(admission.user, admission.instance, id, now)
        .await
        .map_err(store_failed)?;
    if !consumed {
        return Err(ApiError::NOT_FOUND);
    }
    Ok(content)
}

/// [`get_attachment`] past its extractors. The id is admitted before it is
/// parsed, so a caller with no credentials is told it is nobody and not what
/// is wrong with the id it named.
async fn fetched(
    state: &AppState,
    attachment_id: &str,
    headers: &HeaderMap,
) -> Result<Vec<u8>, ApiError> {
    let db = state.db();
    let now = unix_now();
    let admission = admitted(
        state,
        attachment_id,
        header_text(headers, AUTHORIZATION),
        one_proof(headers),
        now,
        &PostgresAccess(db),
    )
    .await?;
    let attachments = PostgresAttachments::new(db.clone());
    fetch(
        &attachments,
        state.keys(),
        state.transit_key(),
        &admission,
        attachment_id,
        now,
    )
    .await
}

/// `GET` [`FETCH_ROUTE`]: the content, once, as `application/octet-stream`.
pub async fn get_attachment(
    State(state): State<AppState>,
    Path(attachment_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match fetched(&state, &attachment_id, &headers).await {
        Ok(content) => ([(CONTENT_TYPE, "application/octet-stream")], content).into_response(),
        Err(refused) => refusal(refused),
    }
}

fn open_failed(error: EnvelopeError) -> ApiError {
    tracing::error!(%error, "an attachment did not open under its own key");
    ApiError::INTERNAL
}

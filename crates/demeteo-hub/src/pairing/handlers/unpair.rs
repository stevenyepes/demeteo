//! `POST /unpair`: an instance removing itself from the Hub as it unpairs —
//! the desktop's disconnect and `demeteo-runner hub disconnect` call it
//! before they delete their own secrets.
//!
//! It is admitted as the socket is, by [`pairing::access`](crate::pairing::access)
//! with a proof for this one method and URL, and it removes the instance
//! that admission names and no other: the request has no body, so nothing it
//! says can choose an instance. What it writes is
//! [`remove_instance`]'s, the same as a removal from the browser; the
//! instance's own socket, if one is still open, is closed `4003`.
//!
//! No passkey is asked. A removal from the browser needs one because a
//! session can reach every instance of the tenant; this request can reach
//! only the instance whose device key signed its proof, and that instance
//! could as well stop answering. An attacker who holds the key and a live
//! token can do no more with this than make the instance pair again.
//!
//! An instance already removed is refused by admission, `401 invalid_token`,
//! as any revoked key is: its caller is told it is nobody.

use crate::auth::handlers::{header_text, store_failed, unix_now, ApiError};
use crate::pairing::access::{self, refusal, PostgresAccess};
use crate::pairing::dpop::Expected;
use crate::pairing::handlers::one_proof;
use crate::routes::AppState;
use crate::store::instances::remove_instance;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

/// The route [`unpair`] is mounted at, and the path of the `htu` its proofs
/// are made for.
pub const UNPAIR_PATH: &str = "/unpair";
const UNPAIR_METHOD: &str = "POST";

async fn unpaired(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let db = state.db();
    let now = unix_now();
    let htu = access::htu(state, UNPAIR_PATH);
    let expected = Expected {
        htm: UNPAIR_METHOD,
        htu: &htu,
    };
    let admission = access::admitted(
        state,
        header_text(headers, AUTHORIZATION),
        one_proof(headers),
        &expected,
        now,
        &PostgresAccess(db),
    )
    .await?;
    let removed = remove_instance(admission.user, db, admission.instance, now)
        .await
        .map_err(store_failed)?;
    if !removed {
        // Admission read the key as live, and a removal committed since.
        return Err(ApiError::INVALID_TOKEN);
    }
    state.connections().remove(admission.instance);
    Ok(())
}

/// `POST` [`UNPAIR_PATH`]: `204`, or a refusal with admission's answers.
pub async fn unpair(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match unpaired(&state, &headers).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(refused) => refusal(refused),
    }
}

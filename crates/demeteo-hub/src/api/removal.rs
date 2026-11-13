//! Removing an instance from the Hub:
//! `POST /v1/instances/{instance_id}/remove/challenge` asks a trusted passkey
//! to sign the removal, and `DELETE /v1/instances/{instance_id}` carries the
//! assertion and removes the instance.
//!
//! **The Hub is the authority here, not the instance.** A gate decision is
//! verified by the instance against the passkeys it pinned (`docs/HUB.md`
//! §8), because the instance is the party that acts on it. A removal acts on
//! the Hub's own rows and is aimed at the instance, so the instance cannot be
//! the one asked: the Hub verifies the assertion against the tenant's trusted
//! passkeys itself, as it does for a runner's device approval
//! (`pairing/handlers/device_consent.rs`).
//!
//! **Why a passkey and not the session alone.** Removing an instance retires
//! its pairing, which only pairing again — a fresh consent at the machine —
//! undoes. A session cookie is a bearer credential; a passkey assertion needs
//! the human at an authenticator, so a stolen session cannot empty a fleet.
//! Removing a Hub Project takes the session alone because nothing is retired
//! by it.
//!
//! What a removal writes is
//! [`remove_instance`](crate::store::instances::remove_instance)'s; this
//! module then closes the instance's live socket with `4003`.

use super::runs::owned_live;
use super::{ApiContext, PostgresApi};
use crate::attachments::postgres::PostgresAttachments;
use crate::auth::assertion;
use crate::auth::ceremonies::Ceremony;
use crate::auth::challenge::{base64url_encode, instance_removal_challenge, ChallengeBinding};
use crate::auth::handlers::{
    ceremony_not_held, public_key_options, store_failed, unix_now, ApiError, CurrentSession,
    JsonBody,
};
use crate::auth::policy::Status;
use crate::routes::AppState;
use crate::store::passkeys::{self, Passkey};
use crate::store::{InstanceId, UserId};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use demeteo_hub_protocol::WebAuthnAssertion;
use serde::{Deserialize, Serialize};

/// A held removal challenge.
#[derive(Debug, Clone, Serialize)]
pub struct RemovalChallengeView {
    pub ceremony_id: String,
    /// `navigator.credentials.get`'s `publicKey`, allowing any of the
    /// tenant's trusted passkeys.
    pub public_key: serde_json::Value,
}

/// The assertion over a held challenge. It names no instance: what is
/// removed is what the challenge was computed over, and the path must name
/// the same one.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemovalBody {
    pub ceremony_id: String,
    pub assertion: WebAuthnAssertion,
}

/// Holds a challenge to remove the tenant's instance under `holder`. A
/// removed instance, like one that is not the tenant's, is `404 not_found`.
pub async fn removal_challenge(
    api: &ApiContext<'_>,
    user: UserId,
    holder: Vec<u8>,
    instance_id: &str,
    now: u64,
) -> Result<RemovalChallengeView, ApiError> {
    let row = owned_live(api, user, instance_id).await?;
    let binding = ChallengeBinding::mint(now).map_err(|failed| ceremony_not_held(failed.into()))?;
    let challenge =
        instance_removal_challenge(&binding, row.id).map_err(|_| ApiError::INVALID_REQUEST)?;
    let trusted = api
        .store
        .trusted_credential_ids(user)
        .await
        .map_err(store_failed)?;
    let public_key = public_key_options(api.rp, &challenge, trusted.iter().map(String::as_str));
    let ceremony = Ceremony::RemoveInstance {
        user,
        holder,
        instance: row.id,
        challenge,
    };
    let ceremony_id = api
        .ceremonies
        .insert(now, ceremony)
        .map_err(ceremony_not_held)?;
    Ok(RemovalChallengeView {
        ceremony_id,
        public_key,
    })
}

/// Removes the instance the held challenge names, once `body`'s assertion
/// verifies under one of `passkeys` that is trusted, then asks its live
/// socket to close.
///
/// The challenge is taken before it is judged, so a refused assertion spends
/// it and the page asks for another. One refused because the instance is
/// not found does not, since nothing was taken yet.
pub async fn remove(
    api: &ApiContext<'_>,
    user: UserId,
    holder: &[u8],
    instance_id: &str,
    body: RemovalBody,
    passkeys: &[Passkey],
    now: u64,
) -> Result<(), ApiError> {
    let row = owned_live(api, user, instance_id).await?;
    let taken = api.ceremonies.take(now, &body.ceremony_id);
    let challenge = held_removal(taken, user, holder, row.id)?;
    let signer = passkeys
        .iter()
        .filter(|passkey| passkey.status == Status::Trusted)
        .find(|passkey| base64url_encode(&passkey.credential_id) == body.assertion.credential_id)
        .ok_or(ApiError::ASSERTION_INVALID)?;
    assertion::verify(api.rp, &signer.public_key, &challenge, &body.assertion)
        .map_err(|_| ApiError::ASSERTION_INVALID)?;
    let removed = api
        .store
        .remove_instance(user, row.id, now)
        .await
        .map_err(store_failed)?;
    if !removed {
        return Err(ApiError::NOT_FOUND);
    }
    api.connections.remove(row.id);
    Ok(())
}

/// The challenge `taken` holds for `user`'s session `holder` to remove
/// `instance`. One held for anyone or anything else is as unknown as one
/// never held.
fn held_removal(
    taken: Option<Ceremony>,
    user: UserId,
    holder: &[u8],
    instance: InstanceId,
) -> Result<[u8; 32], ApiError> {
    match taken {
        Some(Ceremony::RemoveInstance {
            user: started_by,
            holder: held_by,
            instance: held_for,
            challenge,
        }) if started_by == user && held_by == holder && held_for == instance => Ok(challenge),
        _ => Err(ApiError::CEREMONY_EXPIRED),
    }
}

pub async fn post_removal_challenge(
    State(state): State<AppState>,
    current: CurrentSession,
    Path(instance_id): Path<String>,
) -> Result<Json<RemovalChallengeView>, ApiError> {
    let store = PostgresApi::new(state.db().clone());
    let attachments = PostgresAttachments::new(state.db().clone());
    let api = ApiContext::of(&state, &store, &attachments);
    let view = removal_challenge(
        &api,
        current.user,
        current.credential_id,
        &instance_id,
        unix_now(),
    )
    .await?;
    Ok(Json(view))
}

pub async fn delete_instance(
    State(state): State<AppState>,
    current: CurrentSession,
    Path(instance_id): Path<String>,
    JsonBody(body): JsonBody<RemovalBody>,
) -> Result<StatusCode, ApiError> {
    let passkeys = passkeys::list_passkeys(current.user, state.db())
        .await
        .map_err(store_failed)?;
    let store = PostgresApi::new(state.db().clone());
    let attachments = PostgresAttachments::new(state.db().clone());
    let api = ApiContext::of(&state, &store, &attachments);
    remove(
        &api,
        current.user,
        &current.credential_id,
        &instance_id,
        body,
        &passkeys,
        unix_now(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_removal_body_names_a_ceremony_and_an_assertion_and_nothing_else() {
        let assertion = serde_json::json!({
            "credential_id": "a", "authenticator_data": "b",
            "client_data_json": "c", "signature": "d",
        });
        let parses = |body: serde_json::Value| serde_json::from_value::<RemovalBody>(body).is_ok();
        assert!(parses(
            serde_json::json!({ "ceremony_id": "x", "assertion": assertion })
        ));
        assert!(!parses(serde_json::json!({
            "ceremony_id": "x", "assertion": assertion, "instance_id": "i"
        })));
        assert!(!parses(serde_json::json!({ "ceremony_id": "x" })));
    }
}

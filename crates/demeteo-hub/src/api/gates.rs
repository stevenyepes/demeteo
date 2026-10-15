//! A tenant's decision on an instance's gate:
//! `POST /v1/instances/{instance_id}/gates/challenge` asks a trusted passkey
//! to sign it, and `…/gates/decision` queues the signed decision as a
//! `gate_decision`.
//!
//! The challenge fixes the request the decision will travel as — its id and
//! expiry are minted here, not when it is queued — because the instance
//! recomputes the signed challenge from that request's envelope. Everything
//! the decision is built from is held as a [`Ceremony::Gate`] under the
//! passkey that signed the session in, so what is queued is what was signed
//! and never what a later body says.

use super::dispatch::{queue_as, strings_verdict, EnqueuedView};
use super::fleet::granted;
use super::runs::owned;
use super::{ApiContext, PostgresApi};
use crate::auth::assertion::shaped;
use crate::auth::ceremonies::Ceremony;
use crate::auth::challenge::{
    base64url_decode, base64url_encode, gate_challenge_digest, gate_challenge_pre_image,
};
use crate::auth::handlers::{
    ceremony_not_held, public_key_options, store_failed, unix_now, ApiError, CurrentSession,
    JsonBody,
};
use crate::routes::AppState;
use crate::socket::requests::{delivery_verdict, fresh_minted, Delivery, Minted};
use crate::store::instances::Instance;
use crate::store::requests::RequestId;
use crate::store::{InstanceId, UserId};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use demeteo_hub_protocol::{
    Decision, EncodeError, GateDecision, RequestPayload, Scope, WebAuthnAssertion,
};
use serde::{Deserialize, Serialize};

/// The gate and the decision a passkey is asked to sign.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateChallengeBody {
    pub feature_id: String,
    pub step_execution_id: String,
    pub decision: Decision,
}

/// A signed decision on a held challenge. It names no gate: what is decided
/// is what the challenge held.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateDecisionBody {
    pub ceremony_id: String,
    /// Unpadded base64url; the challenge's own, or the decision is refused.
    pub request_id: String,
    pub assertion: WebAuthnAssertion,
    /// Passed through as the protocol's unbound note.
    #[serde(default)]
    pub feedback: Option<String>,
}

/// A held gate challenge. `request_id` and `gate_challenge` are unpadded
/// base64url; `gate_challenge` is the pre-image, `public_key.challenge` its
/// SHA-256, so a client can show what it is signing and check that it is.
#[derive(Debug, Clone, Serialize)]
pub struct GateChallengeView {
    pub ceremony_id: String,
    pub request_id: String,
    /// Unix seconds: when the Hub stops offering the request, not when the
    /// ceremony is dropped, which is sooner.
    pub expires_at: u64,
    pub gate_challenge: String,
    /// `navigator.credentials.get`'s `publicKey`, allowing any of the
    /// tenant's trusted passkeys.
    pub public_key: serde_json::Value,
}

/// Holds a challenge for `body`'s decision on the tenant's instance under
/// `holder`. An instance that does not report `gates` is refused by name
/// before the body is judged, and nothing is held.
pub async fn gate_challenge(
    api: &ApiContext<'_>,
    user: UserId,
    holder: Vec<u8>,
    instance_id: &str,
    body: GateChallengeBody,
    now: u64,
) -> Result<GateChallengeView, ApiError> {
    let row = owned(api, user, instance_id).await?;
    gates_verdict(&row)?;
    strings_verdict(
        [body.feature_id.as_str(), body.step_execution_id.as_str()].into_iter(),
        std::iter::empty(),
    )?;
    let minted = fresh_minted(now).map_err(|failed| ceremony_not_held(failed.into()))?;
    let pre_image = gate_challenge_pre_image(
        row.id,
        &body.feature_id,
        &body.step_execution_id,
        body.decision,
        &minted.id,
        minted.expires_at,
    )
    .map_err(too_long)?;
    let challenge = gate_challenge_digest(
        row.id,
        &body.feature_id,
        &body.step_execution_id,
        body.decision,
        &minted.id,
        minted.expires_at,
    )
    .map_err(too_long)?;
    let trusted = api
        .store
        .trusted_credential_ids(user)
        .await
        .map_err(store_failed)?;
    let public_key = public_key_options(api.rp, &challenge, trusted.iter().map(String::as_str));
    let ceremony = Ceremony::Gate {
        user,
        holder,
        instance: row.id,
        feature_id: body.feature_id,
        step_execution_id: body.step_execution_id,
        decision: body.decision,
        request_id: minted.id,
        expires_at: minted.expires_at,
        challenge,
    };
    let ceremony_id = api
        .ceremonies
        .insert(now, ceremony)
        .map_err(ceremony_not_held)?;
    Ok(GateChallengeView {
        ceremony_id,
        request_id: base64url_encode(&minted.id),
        expires_at: minted.expires_at,
        gate_challenge: base64url_encode(&pre_image),
        public_key,
    })
}

/// Queues the decision `body` signs on the challenge it names, under the
/// request id and expiry the challenge minted. The feature, step execution
/// and decision are the held challenge's, never the body's.
///
/// The challenge is taken before it is judged, so a submission refused for
/// its request id, its assertion or its expiry spends it: the next one is
/// `ceremony_expired`. One refused for its instance or its scope does not,
/// since nothing was taken yet.
pub async fn gate_decision(
    api: &ApiContext<'_>,
    user: UserId,
    holder: Vec<u8>,
    instance_id: &str,
    body: GateDecisionBody,
    now: u64,
) -> Result<EnqueuedView, ApiError> {
    let row = owned(api, user, instance_id).await?;
    gates_verdict(&row)?;
    let taken = api.ceremonies.take(now, &body.ceremony_id);
    let held = held_gate(taken, user, &holder, row.id)?;
    decision_verdict(&held, &body, now)?;
    let payload = RequestPayload::GateDecision(GateDecision {
        feature_id: held.feature_id,
        step_execution_id: held.step_execution_id,
        decision: held.decision,
        feedback: body.feedback,
        assertion: body.assertion,
    });
    queue_as(api, user, &row, held.minted, payload, now).await
}

/// What a [`Ceremony::Gate`] fixed, taken back for the decision on it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HeldGate {
    feature_id: String,
    step_execution_id: String,
    decision: Decision,
    minted: Minted,
    challenge: [u8; 32],
}

/// The gate challenge `taken` holds for `user`'s session `holder` on
/// `instance`. One held for anyone or anything else is as unknown as one
/// never held, so a ceremony id says nothing about another session's.
fn held_gate(
    taken: Option<Ceremony>,
    user: UserId,
    holder: &[u8],
    instance: InstanceId,
) -> Result<HeldGate, ApiError> {
    match taken {
        Some(Ceremony::Gate {
            user: started_by,
            holder: held_by,
            instance: held_for,
            feature_id,
            step_execution_id,
            decision,
            request_id,
            expires_at,
            challenge,
        }) if started_by == user && held_by == holder && held_for == instance => Ok(HeldGate {
            feature_id,
            step_execution_id,
            decision,
            minted: Minted {
                id: request_id,
                expires_at,
            },
            challenge,
        }),
        _ => Err(ApiError::CEREMONY_EXPIRED),
    }
}

/// Whether `body` may be queued as the decision on `held` at `now`: its
/// request id is the held one, its assertion is shaped like one over the
/// held digest, its note could be stored, and the request it would travel
/// as has not expired.
fn decision_verdict(held: &HeldGate, body: &GateDecisionBody, now: u64) -> Result<(), ApiError> {
    if !held_matches(&held.minted.id, &body.request_id) {
        return Err(ApiError::CHALLENGE_MISMATCH);
    }
    check_assertion_shape(&body.assertion, &held.challenge)
        .map_err(|Malformed| ApiError::ASSERTION_MALFORMED)?;
    strings_verdict(std::iter::empty(), body.feedback.as_deref().into_iter())?;
    match delivery_verdict(held.minted.expires_at, now) {
        Delivery::Deliver => Ok(()),
        Delivery::Expire => Err(ApiError::CEREMONY_EXPIRED),
    }
}

/// `gates` is what a `GateDecision` needs (`socket::requests::required_scope`).
/// A challenge has no payload yet to ask that of, so the scope is named here.
fn gates_verdict(row: &Instance) -> Result<(), ApiError> {
    if granted(row).contains(&Scope::Gates) {
        Ok(())
    } else {
        Err(ApiError::scope_not_granted(Scope::Gates))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the assertion is not a well-formed webauthn.get over the held challenge")]
pub struct Malformed;

/// Refuses an assertion an instance could not accept whoever signed it: a
/// field that is not unpadded base64url, authenticator data too short to
/// hold a counter, or client data that is not a `webauthn.get` over
/// `challenge`.
///
/// This is an early filter and never the authority. The instance verifies
/// the assertion against the passkeys it pinned (`docs/HUB.md` §8, "What
/// the desktop verifies"), so the Hub checks nothing that needs a key: the
/// passkeys it holds are those it signs tenants in with, not an instance's
/// pinned set, and a signature it accepted would prove nothing to the
/// instance. An assertion this passes may still be refused there.
pub fn check_assertion_shape(
    assertion: &WebAuthnAssertion,
    challenge: &[u8; 32],
) -> Result<(), Malformed> {
    base64url_decode(&assertion.credential_id).ok_or(Malformed)?;
    shaped(challenge, assertion).map_err(|_| Malformed)?;
    Ok(())
}

/// Whether a body's base64url `request_id` names the request a challenge was
/// held for. Text that is not that id's one unpadded base64url spelling does
/// not.
pub fn held_matches(held: &RequestId, body_request_id: &str) -> bool {
    base64url_decode(body_request_id).as_deref() == Some(held.as_slice())
}

fn too_long(error: EncodeError) -> ApiError {
    match error {
        EncodeError::TooLong => ApiError::INVALID_REQUEST,
    }
}

pub async fn post_gate_challenge(
    State(state): State<AppState>,
    current: CurrentSession,
    Path(instance_id): Path<String>,
    JsonBody(body): JsonBody<GateChallengeBody>,
) -> Result<Json<GateChallengeView>, ApiError> {
    let store = PostgresApi::new(state.db().clone());
    let api = ApiContext::of(&state, &store);
    let view = gate_challenge(
        &api,
        current.user,
        current.credential_id,
        &instance_id,
        body,
        unix_now(),
    )
    .await?;
    Ok(Json(view))
}

pub async fn post_gate_decision(
    State(state): State<AppState>,
    current: CurrentSession,
    Path(instance_id): Path<String>,
    JsonBody(body): JsonBody<GateDecisionBody>,
) -> Result<(StatusCode, Json<EnqueuedView>), ApiError> {
    let store = PostgresApi::new(state.db().clone());
    let api = ApiContext::of(&state, &store);
    let queued = gate_decision(
        &api,
        current.user,
        current.credential_id,
        &instance_id,
        body,
        unix_now(),
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(queued)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::testing::SoftAuthenticator;
    use crate::auth::RelyingParty;

    const DIGEST: [u8; 32] = [0x3c; 32];
    const REQUEST_ID: RequestId = [7; 16];

    fn rp() -> RelyingParty {
        RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: "https://hub.example".to_owned(),
        }
    }

    fn signed(authenticator_data: &[u8], client_data: &[u8]) -> WebAuthnAssertion {
        SoftAuthenticator::new().sign(authenticator_data, client_data)
    }

    fn get_over(challenge: &[u8]) -> Vec<u8> {
        serde_json::json!({
            "type": "webauthn.get",
            "challenge": base64url_encode(challenge),
            "origin": rp().origin,
        })
        .to_string()
        .into_bytes()
    }

    fn authenticator_data() -> Vec<u8> {
        let assertion = SoftAuthenticator::new().assert(&rp(), &DIGEST);
        base64url_decode(&assertion.authenticator_data).unwrap()
    }

    #[test]
    fn an_assertion_over_the_held_digest_passes() {
        let assertion = SoftAuthenticator::new().assert(&rp(), &DIGEST);
        assert_eq!(check_assertion_shape(&assertion, &DIGEST), Ok(()));
    }

    #[test]
    fn a_field_that_is_not_unpadded_base64url_is_malformed() {
        type Field = fn(&mut WebAuthnAssertion) -> &mut String;
        let fields: [(&str, Field); 4] = [
            ("credential_id", |a| &mut a.credential_id),
            ("authenticator_data", |a| &mut a.authenticator_data),
            ("client_data_json", |a| &mut a.client_data_json),
            ("signature", |a| &mut a.signature),
        ];
        for (name, field) in fields {
            for spoil in ["=", "+", "/"] {
                let mut assertion = SoftAuthenticator::new().assert(&rp(), &DIGEST);
                field(&mut assertion).push_str(spoil);
                assert_eq!(
                    check_assertion_shape(&assertion, &DIGEST),
                    Err(Malformed),
                    "{name} ending {spoil:?}"
                );
            }
        }
    }

    #[test]
    fn authenticator_data_of_36_bytes_is_malformed() {
        let assertion = signed(&authenticator_data()[..36], &get_over(&DIGEST));
        assert_eq!(check_assertion_shape(&assertion, &DIGEST), Err(Malformed));
    }

    #[test]
    fn client_data_that_is_not_json_is_malformed() {
        let assertion = signed(&authenticator_data(), b"webauthn.get");
        assert_eq!(check_assertion_shape(&assertion, &DIGEST), Err(Malformed));
    }

    #[test]
    fn client_data_of_another_type_is_malformed() {
        let client_data = serde_json::json!({
            "type": "webauthn.create",
            "challenge": base64url_encode(&DIGEST),
            "origin": rp().origin,
        });
        let assertion = signed(&authenticator_data(), client_data.to_string().as_bytes());
        assert_eq!(check_assertion_shape(&assertion, &DIGEST), Err(Malformed));
    }

    #[test]
    fn client_data_over_another_challenge_is_malformed() {
        let mut other = DIGEST;
        other[31] ^= 0x01;
        for challenge in [&other[..], &DIGEST[..31]] {
            let assertion = signed(&authenticator_data(), &get_over(challenge));
            assert_eq!(
                check_assertion_shape(&assertion, &DIGEST),
                Err(Malformed),
                "{} bytes",
                challenge.len()
            );
        }
    }

    #[test]
    fn only_the_held_request_id_matches() {
        let held = base64url_encode(&REQUEST_ID);
        assert!(held_matches(&REQUEST_ID, &held));
        let mut other = REQUEST_ID;
        other[15] ^= 0x01;
        for body in [
            base64url_encode(&other),
            base64url_encode(&REQUEST_ID[..15]),
            format!("{held}=="),
            String::new(),
        ] {
            assert!(!held_matches(&REQUEST_ID, &body), "{body:?}");
        }
    }

    const EXPIRES: u64 = 1_700_000_600;

    fn held() -> HeldGate {
        HeldGate {
            feature_id: "f-1".to_owned(),
            step_execution_id: "e1".to_owned(),
            decision: Decision::Approve,
            minted: Minted {
                id: REQUEST_ID,
                expires_at: EXPIRES,
            },
            challenge: DIGEST,
        }
    }

    fn ceremony(user: UserId, holder: &[u8], instance: InstanceId) -> Ceremony {
        Ceremony::Gate {
            user,
            holder: holder.to_vec(),
            instance,
            feature_id: "f-1".to_owned(),
            step_execution_id: "e1".to_owned(),
            decision: Decision::Approve,
            request_id: REQUEST_ID,
            expires_at: EXPIRES,
            challenge: DIGEST,
        }
    }

    fn body(request_id: &RequestId, assertion: WebAuthnAssertion) -> GateDecisionBody {
        GateDecisionBody {
            ceremony_id: "c".to_owned(),
            request_id: base64url_encode(request_id),
            assertion,
            feedback: None,
        }
    }

    #[test]
    fn only_the_gate_held_for_this_user_holder_and_instance_is_taken_back() {
        let (user, instance) = (UserId::generate(), InstanceId(uuid::Uuid::new_v4()));
        assert_eq!(
            held_gate(Some(ceremony(user, b"h", instance)), user, b"h", instance),
            Ok(held())
        );
        let others = [
            Some(ceremony(UserId::generate(), b"h", instance)),
            Some(ceremony(user, b"g", instance)),
            Some(ceremony(user, b"h", InstanceId(uuid::Uuid::new_v4()))),
            Some(Ceremony::Revoke {
                user,
                holder: b"h".to_vec(),
                binding: crate::auth::challenge::ChallengeBinding::mint(0).expect("a binding"),
                target_credential_id: "t".to_owned(),
            }),
            None,
        ];
        for (n, taken) in others.into_iter().enumerate() {
            assert_eq!(
                held_gate(taken, user, b"h", instance),
                Err(ApiError::CEREMONY_EXPIRED),
                "{n}"
            );
        }
    }

    #[test]
    fn a_decision_is_judged_by_request_id_then_shape_then_note_then_expiry() {
        let valid = SoftAuthenticator::new().assert(&rp(), &DIGEST);
        let mut other = REQUEST_ID;
        other[0] ^= 0x01;
        let mut malformed = valid.clone();
        malformed.signature.push('=');
        let mut noted = body(&REQUEST_ID, valid.clone());
        noted.feedback = Some("a\0b".to_owned());

        let late = EXPIRES;
        assert_eq!(
            decision_verdict(&held(), &body(&other, malformed.clone()), late),
            Err(ApiError::CHALLENGE_MISMATCH)
        );
        assert_eq!(
            decision_verdict(&held(), &body(&REQUEST_ID, malformed), late),
            Err(ApiError::ASSERTION_MALFORMED)
        );
        assert_eq!(
            decision_verdict(&held(), &noted, late),
            Err(ApiError::INVALID_REQUEST)
        );
        assert_eq!(
            decision_verdict(&held(), &body(&REQUEST_ID, valid.clone()), late),
            Err(ApiError::CEREMONY_EXPIRED)
        );
        assert_eq!(
            decision_verdict(&held(), &body(&REQUEST_ID, valid), EXPIRES - 1),
            Ok(())
        );
    }
}

//! Listing a tenant's passkeys, and the two ceremonies by which one of them
//! vouches for or removes another.

use super::{
    ceremony_not_held, store_failed, unix_now, ApiError, CeremonyStarted, CurrentSession, JsonBody,
};
use crate::auth::ceremonies::Ceremony;
use crate::auth::chain::{self, ChainError, StoredKey};
use crate::auth::challenge::{
    base64url_encode, endorsement_challenge, revocation_challenge, ChallengeBinding,
    CHALLENGE_TTL_SECS,
};
use crate::auth::policy::{self, KeyView, Refusal, Status};
use crate::auth::RelyingParty;
use crate::routes::AppState;
use crate::store::passkeys::{self, Passkey, TrustChangeError};
use crate::store::UserId;
use axum::extract::State;
use axum::Json;
use demeteo_hub_protocol::WebAuthnAssertion;
use serde::{Deserialize, Serialize};

/// What a browser is told to wait for an authenticator, in the milliseconds
/// WebAuthn counts in: as long as the ceremony is held.
const CEREMONY_TIMEOUT_MS: u64 = CHALLENGE_TTL_SECS * 1000;

fn chain_refused(error: ChainError) -> ApiError {
    match error {
        ChainError::Expired => ApiError::CEREMONY_EXPIRED,
        ChainError::Policy(refusal) => refusal.into(),
        ChainError::Assertion(_) => ApiError::ASSERTION_INVALID,
        ChainError::Encode => {
            tracing::error!(%error, "a stored passkey gives no challenge");
            ApiError::INTERNAL
        }
    }
}

fn trust_change_failed(error: TrustChangeError<ChainError>) -> ApiError {
    match error {
        TrustChangeError::Refused(refusal) => chain_refused(refusal),
        TrustChangeError::Store(error) => store_failed(error),
    }
}

/// A passkey as every route that returns one spells it, so the page reads one
/// shape whether it listed the passkey or just changed it.
#[derive(Serialize)]
pub struct PasskeyView {
    credential_id: String,
    label: String,
    status: &'static str,
    created_at: i64,
    current: bool,
}

impl PasskeyView {
    pub(crate) fn of(passkey: Passkey, signed_in_with: &[u8]) -> Self {
        Self {
            current: passkey.credential_id == signed_in_with,
            credential_id: base64url_encode(&passkey.credential_id),
            label: passkey.label,
            status: passkey.status.as_str(),
            created_at: passkey.created_at.unix_timestamp(),
        }
    }
}

#[derive(Serialize)]
pub struct PasskeyList {
    passkeys: Vec<PasskeyView>,
}

pub async fn list_passkeys(
    State(state): State<AppState>,
    current: CurrentSession,
) -> Result<Json<PasskeyList>, ApiError> {
    let passkeys = passkeys::list_passkeys(current.user, state.db())
        .await
        .map_err(store_failed)?
        .into_iter()
        .map(|passkey| PasskeyView::of(passkey, &current.credential_id))
        .collect();
    Ok(Json(PasskeyList { passkeys }))
}

/// What starts a ceremony over one of the tenant's passkeys.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetCredential {
    credential_id: String,
}

/// What finishes one. `assertion.credential_id` says which passkey signed;
/// which passkey it signed *for* is not a field, and is read from the
/// ceremony the Hub held.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CeremonyAssertion {
    ceremony_id: String,
    assertion: WebAuthnAssertion,
}

#[derive(Serialize)]
pub struct ChangedPasskey {
    pub(super) passkey: PasskeyView,
}

fn stored_keys(passkeys: &[Passkey]) -> Vec<StoredKey> {
    passkeys
        .iter()
        .map(|passkey| StoredKey {
            credential_id: base64url_encode(&passkey.credential_id),
            public_key: passkey.public_key.clone(),
            status: passkey.status,
        })
        .collect()
}

fn key_views(keys: &[StoredKey]) -> Vec<KeyView<'_>> {
    keys.iter()
        .map(|key| KeyView {
            credential_id: &key.credential_id,
            status: key.status,
        })
        .collect()
}

/// `navigator.credentials.get` options, built by hand rather than by a
/// WebAuthn library: the challenge is a digest the Hub computed, and a
/// library's request options carry a random one of its own.
fn assertion_options<'a>(
    rp: &RelyingParty,
    challenge: &[u8; 32],
    allowed: impl Iterator<Item = &'a str>,
) -> serde_json::Value {
    let allowed: Vec<serde_json::Value> = allowed
        .map(|id| serde_json::json!({ "type": "public-key", "id": id }))
        .collect();
    serde_json::json!({
        "publicKey": {
            "challenge": base64url_encode(challenge),
            "rpId": rp.rp_id,
            "allowCredentials": allowed,
            "userVerification": "required",
            "timeout": CEREMONY_TIMEOUT_MS,
        }
    })
}

/// The options that ask a trusted passkey to endorse `pending_credential_id`,
/// or why nothing is asked. The challenge is over that passkey as `keys` holds
/// it, which is how [`chain::endorse`] computes it again at the finish.
fn endorsement_options(
    rp: &RelyingParty,
    keys: &[StoredKey],
    binding: &ChallengeBinding,
    pending_credential_id: &str,
) -> Result<serde_json::Value, ChainError> {
    policy::may_be_endorsed(&key_views(keys), pending_credential_id)?;
    let pending = keys
        .iter()
        .find(|key| key.credential_id == pending_credential_id)
        .ok_or(Refusal::UnknownCredential)?;
    let challenge = endorsement_challenge(
        binding,
        &pending.credential_id,
        &base64url_encode(&pending.public_key),
    )
    .map_err(|_| ChainError::Encode)?;
    let trusted = keys
        .iter()
        .filter(|key| key.status == Status::Trusted)
        .map(|key| key.credential_id.as_str());
    Ok(assertion_options(rp, &challenge, trusted))
}

pub async fn start_endorsement(
    State(state): State<AppState>,
    current: CurrentSession,
    JsonBody(target): JsonBody<TargetCredential>,
) -> Result<Json<CeremonyStarted>, ApiError> {
    let passkeys = passkeys::list_passkeys(current.user, state.db())
        .await
        .map_err(store_failed)?;
    let now = unix_now();
    let binding = ChallengeBinding::mint(now).map_err(|failed| ceremony_not_held(failed.into()))?;
    let options = endorsement_options(
        state.rp(),
        &stored_keys(&passkeys),
        &binding,
        &target.credential_id,
    )
    .map_err(chain_refused)?;
    let ceremony = Ceremony::Endorse {
        user: current.user,
        holder: current.credential_id,
        binding,
        pending_credential_id: target.credential_id,
    };
    let ceremony_id = state
        .ceremonies()
        .insert(now, ceremony)
        .map_err(ceremony_not_held)?;
    Ok(Json(CeremonyStarted {
        ceremony_id,
        options,
    }))
}

/// A ceremony of another kind, or one another tenant started, is as unknown
/// as one that was never held.
fn held_endorsement(
    taken: Option<Ceremony>,
    user: UserId,
) -> Result<(ChallengeBinding, String), ApiError> {
    match taken {
        Some(Ceremony::Endorse {
            user: started_by,
            holder: _,
            binding,
            pending_credential_id,
        }) if started_by == user => Ok((binding, pending_credential_id)),
        _ => Err(ApiError::CEREMONY_EXPIRED),
    }
}

pub async fn finish_endorsement(
    State(state): State<AppState>,
    current: CurrentSession,
    JsonBody(finish): JsonBody<CeremonyAssertion>,
) -> Result<Json<ChangedPasskey>, ApiError> {
    let now = unix_now();
    let taken = state.ceremonies().take(now, &finish.ceremony_id);
    let (binding, pending_credential_id) = held_endorsement(taken, current.user)?;
    let endorsed = passkeys::endorse_passkey(current.user, state.db(), |keys| {
        chain::endorse(
            state.rp(),
            now,
            keys,
            &binding,
            &pending_credential_id,
            &finish.assertion,
        )
    })
    .await
    .map_err(trust_change_failed)?;

    Ok(Json(ChangedPasskey {
        passkey: PasskeyView::of(endorsed.passkey, &current.credential_id),
    }))
}

/// The options that ask a trusted passkey to revoke `target_credential_id`,
/// or why nothing is asked. The target is not among the passkeys asked: its
/// own assertion is the one [`chain::revoke`] refuses.
fn revocation_options(
    rp: &RelyingParty,
    keys: &[StoredKey],
    binding: &ChallengeBinding,
    target_credential_id: &str,
) -> Result<serde_json::Value, ChainError> {
    policy::may_be_revoked(&key_views(keys), target_credential_id)?;
    let challenge =
        revocation_challenge(binding, target_credential_id).map_err(|_| ChainError::Encode)?;
    let revokers = keys
        .iter()
        .filter(|key| key.status == Status::Trusted && key.credential_id != target_credential_id)
        .map(|key| key.credential_id.as_str());
    Ok(assertion_options(rp, &challenge, revokers))
}

pub async fn start_revocation(
    State(state): State<AppState>,
    current: CurrentSession,
    JsonBody(target): JsonBody<TargetCredential>,
) -> Result<Json<CeremonyStarted>, ApiError> {
    let passkeys = passkeys::list_passkeys(current.user, state.db())
        .await
        .map_err(store_failed)?;
    let now = unix_now();
    let binding = ChallengeBinding::mint(now).map_err(|failed| ceremony_not_held(failed.into()))?;
    let options = revocation_options(
        state.rp(),
        &stored_keys(&passkeys),
        &binding,
        &target.credential_id,
    )
    .map_err(chain_refused)?;
    let ceremony = Ceremony::Revoke {
        user: current.user,
        holder: current.credential_id,
        binding,
        target_credential_id: target.credential_id,
    };
    let ceremony_id = state
        .ceremonies()
        .insert(now, ceremony)
        .map_err(ceremony_not_held)?;
    Ok(Json(CeremonyStarted {
        ceremony_id,
        options,
    }))
}

/// [`held_endorsement`] for a revocation.
fn held_revocation(
    taken: Option<Ceremony>,
    user: UserId,
) -> Result<(ChallengeBinding, String), ApiError> {
    match taken {
        Some(Ceremony::Revoke {
            user: started_by,
            holder: _,
            binding,
            target_credential_id,
        }) if started_by == user => Ok((binding, target_credential_id)),
        _ => Err(ApiError::CEREMONY_EXPIRED),
    }
}

/// The session that asked may be one the revoked passkey signed in. It is
/// answered all the same, and is gone by its next request.
pub async fn finish_revocation(
    State(state): State<AppState>,
    current: CurrentSession,
    JsonBody(finish): JsonBody<CeremonyAssertion>,
) -> Result<Json<ChangedPasskey>, ApiError> {
    let now = unix_now();
    let taken = state.ceremonies().take(now, &finish.ceremony_id);
    let (binding, target_credential_id) = held_revocation(taken, current.user)?;
    let revoked = passkeys::revoke_passkey(current.user, state.db(), |keys| {
        chain::revoke(
            state.rp(),
            now,
            keys,
            &binding,
            &target_credential_id,
            &finish.assertion,
        )
    })
    .await
    .map_err(trust_change_failed)?;

    Ok(Json(ChangedPasskey {
        passkey: PasskeyView::of(revoked.passkey, &current.credential_id),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::assertion::AssertionError;
    use crate::auth::challenge::{base64url_decode, HUB_SCOPE};
    use crate::auth::testing::SoftAuthenticator;
    use crate::store::StoreError;
    use axum::body::Body;
    use axum::extract::FromRequest;
    use axum::http::header::CONTENT_TYPE;
    use axum::http::{Request, StatusCode};
    use serde::de::DeserializeOwned;

    const NOW: u64 = 1_700_000_000;

    fn rp() -> RelyingParty {
        RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: "https://hub.example".to_owned(),
        }
    }

    fn binding() -> ChallengeBinding {
        ChallengeBinding {
            instance_id: HUB_SCOPE.to_owned(),
            request_id: "AAECAwQFBgcICQoLDA0ODw".to_owned(),
            expires_at: NOW + CHALLENGE_TTL_SECS,
        }
    }

    fn passkey(credential_id: &[u8], status: Status) -> Passkey {
        Passkey {
            user_id: UserId::generate(),
            credential_id: credential_id.to_vec(),
            public_key: b"a cose key".to_vec(),
            label: "laptop".to_owned(),
            sign_count: 9,
            status,
            credential: "{}".to_owned(),
            created_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        }
    }

    /// The whole object, so a field a client must not see — the public key,
    /// the verifier's credential, the tenant — would show here as an extra.
    #[test]
    fn a_passkey_is_shown_as_its_five_fields_and_current_only_to_its_own_session() {
        let id = [0xfb, 0xff, 0xfe];
        let shown = |passkey, signed_in_with: &[u8]| {
            serde_json::to_value(PasskeyView::of(passkey, signed_in_with)).unwrap()
        };

        assert_eq!(
            shown(passkey(&id, Status::Pending), &id),
            serde_json::json!({
                "credential_id": "-__-",
                "label": "laptop",
                "status": "pending",
                "created_at": 1_700_000_000,
                "current": true,
            })
        );
        for status in [Status::Pending, Status::Trusted, Status::Revoked] {
            let view = shown(passkey(&id, status), &[0xfb, 0xff]);
            assert_eq!(view["status"], status.as_str());
            assert_eq!(view["current"], false, "{status:?}");
        }
    }

    #[test]
    fn every_refusal_has_the_status_and_the_code_the_api_names() {
        for (refusal, status, code) in [
            (
                Refusal::SelfEndorsement,
                StatusCode::FORBIDDEN,
                "self_endorsement",
            ),
            (
                Refusal::EndorserNotTrusted,
                StatusCode::FORBIDDEN,
                "endorser_not_trusted",
            ),
            (Refusal::NotPending, StatusCode::CONFLICT, "not_pending"),
            (
                Refusal::UnknownCredential,
                StatusCode::NOT_FOUND,
                "not_found",
            ),
            (
                Refusal::SelfRevocation,
                StatusCode::FORBIDDEN,
                "self_revocation",
            ),
            (
                Refusal::RevokerNotTrusted,
                StatusCode::FORBIDDEN,
                "revoker_not_trusted",
            ),
            (Refusal::LastPasskey, StatusCode::CONFLICT, "last_passkey"),
            (
                Refusal::TooManyPasskeys,
                StatusCode::CONFLICT,
                "too_many_passkeys",
            ),
            (
                Refusal::AlreadyRevoked,
                StatusCode::CONFLICT,
                "already_revoked",
            ),
        ] {
            let expected = ApiError::new(status, code);
            assert_eq!(ApiError::from(refusal), expected, "{refusal:?}");
            assert_eq!(
                chain_refused(ChainError::Policy(refusal)),
                expected,
                "{refusal:?}"
            );
            assert_eq!(
                trust_change_failed(TrustChangeError::Refused(ChainError::Policy(refusal))),
                expected,
                "{refusal:?}"
            );
        }
    }

    #[test]
    fn an_assertion_that_fails_any_check_is_403_under_one_code() {
        for failed in [
            AssertionError::Malformed,
            AssertionError::WrongType,
            AssertionError::WrongChallenge,
            AssertionError::WrongOrigin,
            AssertionError::WrongRpId,
            AssertionError::UserNotPresent,
            AssertionError::UserNotVerified,
            AssertionError::UnsupportedKey,
            AssertionError::BadSignature,
        ] {
            assert_eq!(
                chain_refused(ChainError::Assertion(failed)),
                ApiError::new(StatusCode::FORBIDDEN, "assertion_invalid"),
                "{failed:?}"
            );
        }
    }

    #[test]
    fn an_expired_challenge_is_400_and_one_that_cannot_be_computed_is_500() {
        assert_eq!(
            chain_refused(ChainError::Expired),
            ApiError::new(StatusCode::BAD_REQUEST, "ceremony_expired")
        );
        assert_eq!(chain_refused(ChainError::Encode), ApiError::INTERNAL);
    }

    #[test]
    fn a_trust_change_the_store_could_not_make_is_500() {
        let failed = TrustChangeError::<ChainError>::Store(StoreError::RecordNamesNoPasskey(
            Status::Trusted,
        ));
        assert_eq!(trust_change_failed(failed), ApiError::INTERNAL);
    }

    async fn parsed<T: DeserializeOwned>(
        content_type: Option<&str>,
        body: &str,
    ) -> Result<T, ApiError> {
        let mut request = Request::builder().method("POST").uri("/v1/a-route");
        if let Some(content_type) = content_type {
            request = request.header(CONTENT_TYPE, content_type);
        }
        let request = request.body(Body::from(body.to_owned())).unwrap();
        JsonBody::<T>::from_request(request, &())
            .await
            .map(|JsonBody(body)| body)
    }

    #[tokio::test]
    async fn a_body_that_does_not_parse_is_400_invalid_request() {
        const JSON: Option<&str> = Some("application/json");
        let invalid = ApiError::new(StatusCode::BAD_REQUEST, "invalid_request");

        for (content_type, body) in [
            (JSON, ""),
            (JSON, "{"),
            (JSON, "{}"),
            (JSON, r#"{"credential_id":7}"#),
            (JSON, r#"{"credential_id":"AQID","status":"trusted"}"#),
            (None, r#"{"credential_id":"AQID"}"#),
            (Some("text/plain"), r#"{"credential_id":"AQID"}"#),
        ] {
            let refused = parsed::<TargetCredential>(content_type, body).await.err();
            assert_eq!(refused, Some(invalid), "{content_type:?} {body:?}");
        }
        let assertion =
            serde_json::to_value(SoftAuthenticator::new().assert(&rp(), &[7; 32])).unwrap();
        for body in [
            serde_json::json!({ "ceremony_id": "AAAA" }),
            serde_json::json!({ "ceremony_id": "AAAA", "assertion": { "credential_id": "AQID" } }),
            serde_json::json!({
                "ceremony_id": "AAAA",
                "assertion": assertion,
                "credential_id": "AQID",
            }),
        ] {
            let refused = parsed::<CeremonyAssertion>(JSON, &body.to_string())
                .await
                .err();
            assert_eq!(refused, Some(invalid), "{body}");
        }
        let finish = serde_json::json!({ "ceremony_id": "AAAA", "assertion": assertion });
        let finish = parsed::<CeremonyAssertion>(JSON, &finish.to_string())
            .await
            .unwrap();
        assert_eq!(finish.ceremony_id, "AAAA");

        let target = parsed::<TargetCredential>(JSON, r#"{"credential_id":"AQID"}"#)
            .await
            .unwrap();
        assert_eq!(target.credential_id, "AQID");
    }

    fn challenge_of(options: &serde_json::Value) -> [u8; 32] {
        options["publicKey"]["challenge"]
            .as_str()
            .and_then(base64url_decode)
            .and_then(|bytes| bytes.try_into().ok())
            .expect("a 32-byte base64url challenge")
    }

    #[test]
    fn the_endorsement_options_ask_the_trusted_passkeys_for_the_challenge_over_the_stored_key() {
        let trusted = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        let revoked = SoftAuthenticator::new();
        let keys = [
            revoked.stored(Status::Revoked),
            trusted.stored(Status::Trusted),
            pending.stored(Status::Pending),
        ];
        let challenge = endorsement_challenge(
            &binding(),
            &pending.credential_id(),
            &base64url_encode(&pending.public_key_cose()),
        )
        .unwrap();

        let options =
            endorsement_options(&rp(), &keys, &binding(), &pending.credential_id()).unwrap();

        assert_eq!(
            options,
            serde_json::json!({
                "publicKey": {
                    "challenge": base64url_encode(&challenge),
                    "rpId": "hub.example",
                    "allowCredentials": [
                        { "type": "public-key", "id": trusted.credential_id() },
                    ],
                    "userVerification": "required",
                    "timeout": 300_000,
                }
            })
        );
    }

    /// What ties a start to its finish: the challenge the options carry is the
    /// one the chain accepts for that binding, and for no other pending key.
    #[test]
    fn an_assertion_over_the_challenge_in_the_options_is_the_endorsement_the_chain_accepts() {
        let trusted = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        let other = SoftAuthenticator::new();
        let keys = [
            trusted.stored(Status::Trusted),
            pending.stored(Status::Pending),
            other.stored(Status::Pending),
        ];
        let options =
            endorsement_options(&rp(), &keys, &binding(), &pending.credential_id()).unwrap();
        let assertion = trusted.assert(&rp(), &challenge_of(&options));
        let endorsing = |target: &SoftAuthenticator| {
            chain::endorse(
                &rp(),
                NOW,
                &keys,
                &binding(),
                &target.credential_id(),
                &assertion,
            )
            .map(|record| record.body.new_credential_id)
        };

        assert_eq!(endorsing(&pending), Ok(pending.credential_id()));
        assert_eq!(
            endorsing(&other),
            Err(ChainError::Assertion(AssertionError::WrongChallenge))
        );
    }

    #[test]
    fn no_options_are_made_for_a_passkey_that_is_not_the_tenants_or_not_pending() {
        let trusted = SoftAuthenticator::new();
        let revoked = SoftAuthenticator::new();
        let stranger = SoftAuthenticator::new();
        let keys = [
            trusted.stored(Status::Trusted),
            revoked.stored(Status::Revoked),
        ];
        let refused =
            |target: &str| endorsement_options(&rp(), &keys, &binding(), target).unwrap_err();

        for unknown in [stranger.credential_id(), String::new()] {
            assert_eq!(
                refused(&unknown),
                ChainError::Policy(Refusal::UnknownCredential),
                "{unknown:?}"
            );
        }
        for settled in [&trusted, &revoked] {
            assert_eq!(
                refused(&settled.credential_id()),
                ChainError::Policy(Refusal::NotPending)
            );
        }
    }

    #[test]
    fn the_revocation_options_ask_every_trusted_passkey_but_the_target() {
        let target = SoftAuthenticator::new();
        let other = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        let revoked = SoftAuthenticator::new();
        let keys = [
            revoked.stored(Status::Revoked),
            target.stored(Status::Trusted),
            pending.stored(Status::Pending),
            other.stored(Status::Trusted),
        ];
        let challenge = revocation_challenge(&binding(), &target.credential_id()).unwrap();

        let options =
            revocation_options(&rp(), &keys, &binding(), &target.credential_id()).unwrap();

        assert_eq!(
            options,
            serde_json::json!({
                "publicKey": {
                    "challenge": base64url_encode(&challenge),
                    "rpId": "hub.example",
                    "allowCredentials": [
                        { "type": "public-key", "id": other.credential_id() },
                    ],
                    "userVerification": "required",
                    "timeout": 300_000,
                }
            })
        );
    }

    /// As for an endorsement: the challenge the options carry is the one the
    /// chain accepts for that binding, and for no other target.
    #[test]
    fn an_assertion_over_the_challenge_in_the_options_is_the_revocation_the_chain_accepts() {
        let revoker = SoftAuthenticator::new();
        let target = SoftAuthenticator::new();
        let other = SoftAuthenticator::new();
        let keys = [
            revoker.stored(Status::Trusted),
            target.stored(Status::Trusted),
            other.stored(Status::Pending),
        ];
        let options =
            revocation_options(&rp(), &keys, &binding(), &target.credential_id()).unwrap();
        let assertion = revoker.assert(&rp(), &challenge_of(&options));
        let revoking = |target: &SoftAuthenticator| {
            chain::revoke(
                &rp(),
                NOW,
                &keys,
                &binding(),
                &target.credential_id(),
                &assertion,
            )
            .map(|record| record.body.credential_id)
        };

        assert_eq!(revoking(&target), Ok(target.credential_id()));
        assert_eq!(
            revoking(&other),
            Err(ChainError::Assertion(AssertionError::WrongChallenge))
        );
    }

    #[test]
    fn no_assertion_is_asked_for_a_revocation_no_assertion_could_make() {
        let only = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        let revoked = SoftAuthenticator::new();
        let stranger = SoftAuthenticator::new();
        let keys = [
            only.stored(Status::Trusted),
            pending.stored(Status::Pending),
            revoked.stored(Status::Revoked),
        ];
        let refused = |target: &str| {
            revocation_options(&rp(), &keys, &binding(), target).map_err(chain_refused)
        };

        assert_eq!(
            refused(&only.credential_id()),
            Err(ApiError::new(StatusCode::CONFLICT, "last_passkey"))
        );
        assert_eq!(
            refused(&revoked.credential_id()),
            Err(ApiError::new(StatusCode::CONFLICT, "already_revoked"))
        );
        for unknown in [stranger.credential_id(), String::new()] {
            assert_eq!(
                refused(&unknown),
                Err(ApiError::new(StatusCode::NOT_FOUND, "not_found")),
                "{unknown:?}"
            );
        }
        assert!(refused(&pending.credential_id()).is_ok());
    }

    #[test]
    fn only_the_tenants_own_revocation_is_taken_up_at_the_finish() {
        let user = UserId::generate();
        let revoke = |user| Ceremony::Revoke {
            user,
            holder: b"the session's passkey".to_vec(),
            binding: binding(),
            target_credential_id: "target".to_owned(),
        };
        let expired = ApiError::new(StatusCode::BAD_REQUEST, "ceremony_expired");

        assert_eq!(
            held_revocation(Some(revoke(user)), user),
            Ok((binding(), "target".to_owned()))
        );
        assert_eq!(held_revocation(None, user), Err(expired));
        assert_eq!(
            held_revocation(Some(revoke(UserId::generate())), user),
            Err(expired)
        );
        let endorsement = Ceremony::Endorse {
            user,
            holder: b"the session's passkey".to_vec(),
            binding: binding(),
            pending_credential_id: "target".to_owned(),
        };
        assert_eq!(held_revocation(Some(endorsement), user), Err(expired));
    }

    #[test]
    fn a_stored_passkey_is_decided_on_by_the_text_form_of_its_id() {
        let stored = stored_keys(&[passkey(&[0xfb, 0xff, 0xfe], Status::Pending)]);
        assert_eq!(
            stored,
            [StoredKey {
                credential_id: "-__-".to_owned(),
                public_key: b"a cose key".to_vec(),
                status: Status::Pending,
            }]
        );
    }

    #[test]
    fn only_the_tenants_own_endorsement_is_taken_up_at_the_finish() {
        let user = UserId::generate();
        let endorse = |user| Ceremony::Endorse {
            user,
            holder: b"the session's passkey".to_vec(),
            binding: binding(),
            pending_credential_id: "pending".to_owned(),
        };
        let expired = ApiError::new(StatusCode::BAD_REQUEST, "ceremony_expired");

        assert_eq!(
            held_endorsement(Some(endorse(user)), user),
            Ok((binding(), "pending".to_owned()))
        );
        assert_eq!(held_endorsement(None, user), Err(expired));
        assert_eq!(
            held_endorsement(Some(endorse(UserId::generate())), user),
            Err(expired)
        );
        let revocation = Ceremony::Revoke {
            user,
            holder: b"the session's passkey".to_vec(),
            binding: binding(),
            target_credential_id: "pending".to_owned(),
        };
        assert_eq!(held_endorsement(Some(revocation), user), Err(expired));
    }
}

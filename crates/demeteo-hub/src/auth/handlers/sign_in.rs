//! Signing in with a passkey the Hub already holds. Neither route has a
//! session to ask and the start names nobody: the tenant is whichever one
//! holds the credential the finish presents.
//!
//! Every refusal of that credential is the one `401`. An answer that told an
//! unknown credential from a pending or a revoked one would tell whoever
//! holds a credential id what the Hub decided about it.

use super::{
    ceremony_not_held, session_expiry, session_not_opened, session_started, store_failed, unix_now,
    ApiError, CeremonyCredential, CeremonyStarted, JsonBody,
};
use crate::auth::ceremonies::Ceremony;
use crate::auth::policy;
use crate::auth::session::SessionSecret;
use crate::auth::webauthn::{sign_in_credential_id, SignInError, SignInState, SignedIn};
use crate::routes::AppState;
use crate::store::passkeys::{find_passkey, update_sign_count, Passkey};
use crate::store::sessions::{insert_session, NewSession};
use crate::store::StoreError;
use axum::extract::State;
use axum::response::Response;
use axum::Json;
use std::future::Future;

/// The two `500`s are faults of the Hub's own, which nothing a caller sends
/// brings about; everything a caller can bring about is the one refusal.
fn sign_in_refused(error: SignInError) -> ApiError {
    match error {
        SignInError::Malformed
        | SignInError::WrongChallenge
        | SignInError::WrongOrigin
        | SignInError::UserNotVerified
        | SignInError::WrongCredential
        | SignInError::BadSignature
        | SignInError::Refused => ApiError::UNAUTHENTICATED,
        SignInError::StoredCredential | SignInError::Library => {
            tracing::error!(%error, "a sign-in ceremony failed in the verifier");
            ApiError::INTERNAL
        }
    }
}

pub async fn start_sign_in(
    State(state): State<AppState>,
) -> Result<Json<CeremonyStarted>, ApiError> {
    let (options, sign_in) = state.verifier().start_sign_in().map_err(sign_in_refused)?;
    let ceremony_id = state
        .ceremonies()
        .insert(unix_now(), Ceremony::SignIn { sign_in })
        .map_err(ceremony_not_held)?;
    Ok(Json(CeremonyStarted {
        ceremony_id,
        options,
    }))
}

fn held_sign_in(taken: Option<Ceremony>) -> Result<SignInState, ApiError> {
    match taken {
        Some(Ceremony::SignIn { sign_in }) => Ok(sign_in),
        _ => Err(ApiError::CEREMONY_EXPIRED),
    }
}

/// The passkey a sign-in may be verified against. This is asked of the row
/// alone and before its assertion is read: were a pending key refused only
/// because some later check happened to fail, a valid assertion would sign it
/// in, and an endorsement would be optional.
fn admitted(found: Option<Passkey>) -> Result<Passkey, ApiError> {
    found
        .filter(|passkey| policy::may_sign_in(passkey.status))
        .ok_or(ApiError::UNAUTHENTICATED)
}

/// Everything a sign-in's finish does before it writes: the ceremony is
/// spent, and the passkey its credential names is looked up, admitted and
/// verified against, in that order. `lookup` is the one thing this asks of a
/// store, so the admission is reachable from a test that has no database;
/// [`finish_sign_in`] passes [`find_passkey`].
pub async fn verified_sign_in<L, F>(
    state: &AppState,
    finish: CeremonyCredential,
    lookup: L,
) -> Result<(Passkey, SignedIn), ApiError>
where
    L: FnOnce(Vec<u8>) -> F,
    F: Future<Output = Result<Option<Passkey>, StoreError>>,
{
    let taken = state.ceremonies().take(unix_now(), &finish.ceremony_id);
    let sign_in = held_sign_in(taken)?;
    let credential_id = sign_in_credential_id(&finish.credential).map_err(sign_in_refused)?;
    let found = lookup(credential_id).await.map_err(store_failed)?;
    let passkey = admitted(found)?;
    let signed_in = state
        .verifier()
        .finish_sign_in(
            &finish.credential,
            sign_in,
            &passkey.credential,
            passkey.sign_count,
        )
        .map_err(sign_in_refused)?;
    Ok((passkey, signed_in))
}

pub async fn finish_sign_in(
    State(state): State<AppState>,
    JsonBody(finish): JsonBody<CeremonyCredential>,
) -> Result<Response, ApiError> {
    let db = state.db();
    let (passkey, signed_in) = verified_sign_in(&state, finish, |credential_id| async move {
        find_passkey(db, &credential_id).await
    })
    .await?;
    if signed_in.counter_regressed {
        tracing::warn!(
            user = %passkey.user_id.handle(),
            "a passkey signed in with a signature counter that did not advance"
        );
    }
    update_sign_count(
        passkey.user_id,
        state.db(),
        &passkey.credential_id,
        signed_in.sign_count,
    )
    .await
    .map_err(store_failed)?;
    let secret = SessionSecret::generate().map_err(session_not_opened)?;
    let session = NewSession {
        hash: secret.hash(),
        credential_id: passkey.credential_id,
        expires_at: session_expiry(),
    };
    insert_session(passkey.user_id, state.db(), &session)
        .await
        .map_err(store_failed)?;
    Ok(session_started(&secret, &session.credential_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::ceremonies::MAX_LIVE_CEREMONIES;
    use crate::auth::challenge::{ChallengeBinding, CHALLENGE_TTL_SECS};
    use crate::auth::policy::Status;
    use crate::auth::testing::{lazy_state, SoftAuthenticator};
    use crate::auth::RelyingParty;
    use crate::store::UserId;
    use axum::http::StatusCode;
    use serde_json::{json, Value};
    use time::OffsetDateTime;

    const ORIGIN: &str = "https://hub.example";
    const EXPIRED: ApiError = ApiError::new(StatusCode::BAD_REQUEST, "ceremony_expired");

    /// The database is never dialled: a lookup made for any request here
    /// would answer `500`.
    fn state() -> AppState {
        let rp = RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: ORIGIN.to_owned(),
        };
        lazy_state("postgres://hub@db.invalid/hub", rp).unwrap()
    }

    async fn started(state: &AppState) -> Result<(String, Value), ApiError> {
        start_sign_in(State(state.clone()))
            .await
            .map(|Json(started)| (started.ceremony_id, started.options))
    }

    async fn finished(state: &AppState, ceremony_id: &str, credential: Value) -> Option<ApiError> {
        let finish = CeremonyCredential {
            ceremony_id: ceremony_id.to_owned(),
            credential,
        };
        finish_sign_in(State(state.clone()), JsonBody(finish))
            .await
            .err()
    }

    fn passkey(status: Status) -> Passkey {
        Passkey {
            user_id: UserId::generate(),
            credential_id: vec![1, 2, 3],
            public_key: Vec::new(),
            label: status.as_str().to_owned(),
            sign_count: 0,
            status,
            credential: "{}".to_owned(),
            created_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn whatever_a_caller_can_get_wrong_is_the_one_401_and_a_fault_of_the_hubs_is_500() {
        for failure in [
            SignInError::Malformed,
            SignInError::WrongChallenge,
            SignInError::WrongOrigin,
            SignInError::UserNotVerified,
            SignInError::WrongCredential,
            SignInError::BadSignature,
            SignInError::Refused,
        ] {
            assert_eq!(
                sign_in_refused(failure),
                ApiError::new(StatusCode::UNAUTHORIZED, "unauthenticated"),
                "{failure:?}"
            );
        }
        for fault in [SignInError::StoredCredential, SignInError::Library] {
            assert_eq!(sign_in_refused(fault), ApiError::INTERNAL, "{fault:?}");
        }
    }

    #[test]
    fn only_a_trusted_passkey_is_admitted_and_every_other_is_the_same_401() {
        let trusted = passkey(Status::Trusted);
        assert_eq!(admitted(Some(trusted.clone())), Ok(trusted));
        for found in [
            None,
            Some(passkey(Status::Pending)),
            Some(passkey(Status::Revoked)),
        ] {
            assert_eq!(
                admitted(found.clone()),
                Err(ApiError::new(StatusCode::UNAUTHORIZED, "unauthenticated")),
                "{found:?}"
            );
        }
    }

    #[tokio::test]
    async fn only_a_sign_in_is_taken_up_at_the_finish() {
        let state = state();
        let (ceremony_id, _) = started(&state).await.unwrap();
        let endorse = Ceremony::Endorse {
            user: UserId::generate(),
            holder: b"the session's passkey".to_vec(),
            binding: ChallengeBinding {
                instance_id: "*".to_owned(),
                request_id: "AAECAwQFBgcICQoLDA0ODw".to_owned(),
                expires_at: unix_now() + CHALLENGE_TTL_SECS,
            },
            pending_credential_id: "pending".to_owned(),
        };

        let held = state.ceremonies().take(unix_now(), &ceremony_id);
        assert!(held_sign_in(held).is_ok());
        assert_eq!(held_sign_in(None).err(), Some(EXPIRED));
        assert_eq!(held_sign_in(Some(endorse)).err(), Some(EXPIRED));
    }

    #[tokio::test]
    async fn a_sign_in_is_not_started_once_the_ceremony_map_is_full() {
        let state = state();
        for _ in 0..MAX_LIVE_CEREMONIES {
            started(&state).await.unwrap();
        }

        assert_eq!(
            started(&state).await.err(),
            Some(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "too_many_ceremonies"
            ))
        );
    }

    #[tokio::test]
    async fn the_clone_signal_is_judged_against_the_rows_sign_count() {
        let state = state();
        let authenticator = SoftAuthenticator::new().with_sign_count(5);
        let row = Passkey {
            sign_count: 100,
            ..authenticator.enrolled(&state, Status::Trusted)
        };
        let authenticator = authenticator.with_sign_count(51);
        let (ceremony_id, options) = started(&state).await.unwrap();
        let finish = CeremonyCredential {
            ceremony_id,
            credential: authenticator.get(ORIGIN, &options, None),
        };

        let (_, signed_in) = verified_sign_in(&state, finish, |_| async { Ok(Some(row)) })
            .await
            .unwrap();

        assert_eq!(
            signed_in,
            SignedIn {
                sign_count: 51,
                counter_regressed: true,
            }
        );
    }

    #[tokio::test]
    async fn a_finish_that_names_no_credential_is_401_and_spends_the_ceremony() {
        let state = state();
        let (ceremony_id, options) = started(&state).await.unwrap();
        let assertion = SoftAuthenticator::new().get(ORIGIN, &options, None);

        assert_eq!(
            finished(&state, &ceremony_id, json!({ "id": "AQID" })).await,
            Some(ApiError::new(StatusCode::UNAUTHORIZED, "unauthenticated"))
        );
        assert_eq!(
            finished(&state, &ceremony_id, assertion.clone()).await,
            Some(EXPIRED)
        );
        assert_eq!(
            finished(&state, "AAAAAAAAAAAAAAAAAAAAAA", assertion).await,
            Some(EXPIRED)
        );
    }
}

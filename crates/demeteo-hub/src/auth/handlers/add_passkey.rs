//! A further passkey for a tenant that already has one. The session that asks
//! is not what makes it trusted: whoever held a hijacked session, or the Hub
//! itself, could then enrol a key of their own. It is stored with the status
//! `policy::admission` gives it and waits for the endorsement ceremony, unless
//! `policy::may_be_added` says the tenant already holds as many as it may.

use super::passkeys::{ChangedPasskey, PasskeyView};
use super::register::{
    accepted_label, passkey_not_stored, registration_refused, verified_passkey, NewLabel,
};
use super::{
    ceremony_not_held, store_failed, unix_now, ApiError, CeremonyCredential, CeremonyStarted,
    CurrentSession, JsonBody,
};
use crate::auth::ceremonies::Ceremony;
use crate::auth::policy::{self, Refusal, Status};
use crate::auth::webauthn::{RegistrationError, RegistrationState, Verifier};
use crate::routes::AppState;
use crate::store::passkeys::{self, NewPasskey, Passkey, TrustChangeError};
use crate::store::UserId;
use axum::extract::State;
use axum::Json;
use std::future::Future;

/// A registration under the tenant's own user handle that excludes every
/// passkey it has ever had, revoked ones included: a revoked credential that
/// registered again would otherwise fail only at the insert.
fn addition(
    verifier: &Verifier,
    user: UserId,
    label: &str,
    existing: &[Passkey],
) -> Result<(serde_json::Value, RegistrationState), RegistrationError> {
    let held: Vec<Vec<u8>> = existing
        .iter()
        .map(|passkey| passkey.credential_id.clone())
        .collect();
    verifier.start_registration(user.handle(), label, &held)
}

pub async fn start_addition(
    State(state): State<AppState>,
    current: CurrentSession,
    JsonBody(start): JsonBody<NewLabel>,
) -> Result<Json<CeremonyStarted>, ApiError> {
    let label = accepted_label(&start.label).ok_or(ApiError::INVALID_REQUEST)?;
    let existing = passkeys::list_passkeys(current.user, state.db())
        .await
        .map_err(store_failed)?;
    // Advisory: nothing is locked here, and the finish decides again on what
    // it reads under the tenant's row locks. Refusing now is what keeps a
    // browser from creating a discoverable credential the Hub would then
    // refuse to store.
    let held: Vec<Status> = existing.iter().map(|passkey| passkey.status).collect();
    policy::may_be_added(&held)?;
    let (options, registration) =
        addition(state.verifier(), current.user, label, &existing).map_err(registration_refused)?;
    let ceremony = Ceremony::Add {
        user: current.user,
        holder: current.credential_id,
        label: label.to_owned(),
        registration,
    };
    let ceremony_id = state
        .ceremonies()
        .insert(unix_now(), ceremony)
        .map_err(ceremony_not_held)?;
    Ok(Json(CeremonyStarted {
        ceremony_id,
        options,
    }))
}

fn held_addition(
    taken: Option<Ceremony>,
    user: UserId,
) -> Result<(String, RegistrationState), ApiError> {
    match taken {
        Some(Ceremony::Add {
            user: started_by,
            holder: _,
            label,
            registration,
        }) if started_by == user => Ok((label, registration)),
        _ => Err(ApiError::CEREMONY_EXPIRED),
    }
}

/// Whether the new passkey is stored, and with what status, given the
/// statuses of the passkeys its tenant holds. `existing` is never empty for a
/// request that got this far — a session is opened only by a passkey of its
/// tenant, and no passkey row is ever deleted — so the status is always
/// `pending`. That is the policy's answer and not a literal here, so that the
/// rule stays in one place.
fn admitted_as(existing: &[Status]) -> Result<Status, Refusal> {
    policy::may_be_added(existing)?;
    Ok(policy::admission(existing))
}

fn addition_refused(error: TrustChangeError<Refusal>) -> ApiError {
    match error {
        TrustChangeError::Refused(refusal) => refusal.into(),
        TrustChangeError::Store(error) => passkey_not_stored(error),
    }
}

/// Everything an addition's finish does: the ceremony is spent and must be
/// `user`'s own addition, its registration is verified, and the verified
/// passkey is handed to `store` together with the decision of whether it is
/// stored and as what. `store` is the one thing this asks of a store, and it
/// is asked to decide nothing: it shows the decision the tenant's statuses
/// and writes what the decision answers. That keeps the decision the handler
/// stores on reachable from a test that has no database; [`finish_addition`]
/// passes [`passkeys::add_passkey`].
pub async fn stored_addition<S, F>(
    state: &AppState,
    user: UserId,
    finish: CeremonyCredential,
    store: S,
) -> Result<Passkey, ApiError>
where
    S: FnOnce(NewPasskey, fn(&[Status]) -> Result<Status, Refusal>) -> F,
    F: Future<Output = Result<Passkey, TrustChangeError<Refusal>>>,
{
    let taken = state.ceremonies().take(unix_now(), &finish.ceremony_id);
    let (label, registration) = held_addition(taken, user)?;
    let registered = state
        .verifier()
        .finish_registration(&finish.credential, registration)
        .map_err(registration_refused)?;
    store(verified_passkey(label, registered), admitted_as)
        .await
        .map_err(addition_refused)
}

pub async fn finish_addition(
    State(state): State<AppState>,
    current: CurrentSession,
    JsonBody(finish): JsonBody<CeremonyCredential>,
) -> Result<Json<ChangedPasskey>, ApiError> {
    let (user, db) = (current.user, state.db());
    let added = stored_addition(&state, user, finish, |new, decide| async move {
        passkeys::add_passkey(user, db, &new, decide).await
    })
    .await?;
    Ok(Json(ChangedPasskey {
        passkey: PasskeyView::of(added, &current.credential_id),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::challenge::{base64url_decode, base64url_encode};
    use crate::auth::policy::MAX_ACTIVE_PASSKEYS;
    use crate::auth::session::SessionSecret;
    use crate::auth::testing::{lazy_state, SoftAuthenticator};
    use crate::auth::RelyingParty;
    use crate::store::pool::ReservedUser;
    use axum::http::StatusCode;
    use serde_json::{json, Value};
    use std::future::{ready, Ready};

    const ORIGIN: &str = "https://hub.example";

    fn rp() -> RelyingParty {
        RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: ORIGIN.to_owned(),
        }
    }

    /// The database is never dialled: a request that got as far as a query
    /// would answer `500`, which no case here expects.
    fn state() -> AppState {
        lazy_state("postgres://hub@db.invalid/hub", rp()).unwrap()
    }

    fn signed_in(user: UserId) -> CurrentSession {
        CurrentSession {
            secret: SessionSecret::generate().unwrap(),
            user,
            credential_id: vec![1, 2, 3],
        }
    }

    fn passkey(key: &SoftAuthenticator, status: Status) -> Passkey {
        Passkey {
            user_id: UserId::generate(),
            credential_id: base64url_decode(&key.credential_id()).unwrap(),
            public_key: key.public_key_cose(),
            label: "laptop".to_owned(),
            sign_count: 0,
            status,
            credential: "{}".to_owned(),
            created_at: time::OffsetDateTime::UNIX_EPOCH,
        }
    }

    fn held(state: &AppState, user: UserId, label: &str) -> (String, Value) {
        let (options, registration) = addition(state.verifier(), user, label, &[]).unwrap();
        let ceremony = Ceremony::Add {
            user,
            holder: b"the session's passkey".to_vec(),
            label: label.to_owned(),
            registration,
        };
        let id = state.ceremonies().insert(unix_now(), ceremony).unwrap();
        (id, options)
    }

    async fn finished(
        state: &AppState,
        user: UserId,
        ceremony_id: &str,
        credential: Value,
    ) -> Option<ApiError> {
        let finish = CeremonyCredential {
            ceremony_id: ceremony_id.to_owned(),
            credential,
        };
        finish_addition(State(state.clone()), signed_in(user), JsonBody(finish))
            .await
            .err()
    }

    #[test]
    fn the_addition_is_for_the_tenants_own_handle_and_excludes_every_passkey_it_holds() {
        let verifier = Verifier::new(&rp()).unwrap();
        let user = UserId::generate();
        let (trusted, pending, revoked) = (
            SoftAuthenticator::new(),
            SoftAuthenticator::new(),
            SoftAuthenticator::new(),
        );
        let existing = [
            passkey(&trusted, Status::Trusted),
            passkey(&pending, Status::Pending),
            passkey(&revoked, Status::Revoked),
        ];

        let (options, _) = addition(&verifier, user, "phone", &existing).unwrap();

        let public_key = &options["publicKey"];
        assert_eq!(
            public_key["user"]["id"],
            base64url_encode(user.handle().as_bytes())
        );
        assert_eq!(public_key["user"]["name"], "phone");
        assert_eq!(
            public_key["excludeCredentials"],
            json!([
                { "type": "public-key", "id": trusted.credential_id() },
                { "type": "public-key", "id": pending.credential_id() },
                { "type": "public-key", "id": revoked.credential_id() },
            ])
        );
    }

    #[test]
    fn a_passkey_the_tenant_already_holds_is_409_whatever_its_status() {
        let verifier = Verifier::new(&rp()).unwrap();
        let user = UserId::generate();
        let fresh = SoftAuthenticator::new();
        for status in [Status::Trusted, Status::Pending, Status::Revoked] {
            let held = SoftAuthenticator::new();
            let existing = [passkey(&held, status)];
            let finishing = |key: &SoftAuthenticator| {
                let (options, registration) =
                    addition(&verifier, user, "phone", &existing).unwrap();
                verifier
                    .finish_registration(&key.create(ORIGIN, &options), registration)
                    .map(|registered| base64url_encode(&registered.credential_id))
                    .map_err(registration_refused)
            };

            assert_eq!(
                finishing(&held),
                Err(ApiError::new(StatusCode::CONFLICT, "credential_exists")),
                "{status:?}"
            );
            assert_eq!(finishing(&fresh), Ok(fresh.credential_id()), "{status:?}");
        }
    }

    #[test]
    fn a_further_passkey_is_admitted_pending_whatever_the_tenant_already_holds() {
        for holding in [
            vec![Status::Trusted],
            vec![Status::Pending],
            vec![Status::Revoked],
            vec![Status::Revoked, Status::Trusted, Status::Pending],
        ] {
            assert_eq!(admitted_as(&holding), Ok(Status::Pending), "{holding:?}");
        }
    }

    type Decision = fn(&[Status]) -> Result<Status, Refusal>;
    type Stored = Ready<Result<Passkey, TrustChangeError<Refusal>>>;

    /// The store of a tenant holding `existing`, as the one call
    /// [`stored_addition`] makes of it: it decides nothing, and the row it
    /// answers with carries whatever status the decision gave.
    fn store_holding(existing: Vec<Status>) -> impl FnOnce(NewPasskey, Decision) -> Stored {
        move |new, decide| {
            ready(match decide(&existing) {
                Ok(status) => Ok(Passkey {
                    user_id: UserId::generate(),
                    credential_id: new.credential_id,
                    public_key: new.public_key,
                    label: new.label,
                    sign_count: new.sign_count,
                    status,
                    credential: new.credential,
                    created_at: time::OffsetDateTime::UNIX_EPOCH,
                }),
                Err(refusal) => Err(TrustChangeError::Refused(refusal)),
            })
        }
    }

    #[tokio::test]
    async fn the_store_is_handed_a_decision_that_admits_pending_until_the_tenant_holds_the_most_it_may(
    ) {
        let state = state();
        let user = UserId::generate();
        let added_to = |existing: Vec<Status>| {
            let key = SoftAuthenticator::new();
            let (ceremony_id, options) = held(&state, user, "phone");
            let finish = CeremonyCredential {
                ceremony_id,
                credential: key.create(ORIGIN, &options),
            };
            let state = &state;
            async move {
                stored_addition(state, user, finish, store_holding(existing))
                    .await
                    .map(|added| (added.label, added.status))
            }
        };
        let pending = Ok(("phone".to_owned(), Status::Pending));

        assert_eq!(added_to(vec![Status::Trusted]).await, pending);
        assert_eq!(
            added_to(vec![Status::Trusted; MAX_ACTIVE_PASSKEYS - 1]).await,
            pending
        );
        assert_eq!(
            added_to(vec![Status::Trusted; MAX_ACTIVE_PASSKEYS]).await,
            Err(ApiError::new(StatusCode::CONFLICT, "too_many_passkeys"))
        );
    }

    #[test]
    fn a_refused_addition_is_the_refusals_own_answer_and_a_held_credential_is_409() {
        assert_eq!(
            addition_refused(TrustChangeError::Refused(Refusal::TooManyPasskeys)),
            ApiError::new(StatusCode::CONFLICT, "too_many_passkeys")
        );
        assert_eq!(
            addition_refused(TrustChangeError::Store(
                crate::store::StoreError::CredentialExists
            )),
            ApiError::new(StatusCode::CONFLICT, "credential_exists")
        );
    }

    #[test]
    fn only_the_tenants_own_addition_is_taken_up_at_the_finish() {
        let verifier = Verifier::new(&rp()).unwrap();
        let expired = ApiError::new(StatusCode::BAD_REQUEST, "ceremony_expired");
        let user = UserId::generate();
        let add = |user| {
            let (_, registration) = addition(&verifier, user, "phone", &[]).unwrap();
            Ceremony::Add {
                user,
                holder: b"the session's passkey".to_vec(),
                label: "phone".to_owned(),
                registration,
            }
        };

        let (label, _) = held_addition(Some(add(user)), user).unwrap();
        assert_eq!(label, "phone");
        assert_eq!(held_addition(None, user).err(), Some(expired));
        assert_eq!(
            held_addition(Some(add(UserId::generate())), user).err(),
            Some(expired)
        );

        let reserved = ReservedUser::generate();
        let (_, registration) = verifier
            .start_registration(reserved.handle(), "phone", &[])
            .unwrap();
        let first_registration = Ceremony::Register {
            user: reserved,
            label: "phone".to_owned(),
            registration,
        };
        assert_eq!(
            held_addition(Some(first_registration), user).err(),
            Some(expired)
        );
    }

    #[tokio::test]
    async fn a_label_that_is_not_accepted_is_400_before_the_tenants_passkeys_are_read() {
        let state = state();
        for label in ["", "   ", &"x".repeat(65), "a\0b"] {
            let start = NewLabel {
                label: label.to_owned(),
            };
            let refused = start_addition(
                State(state.clone()),
                signed_in(UserId::generate()),
                JsonBody(start),
            )
            .await
            .err();
            assert_eq!(
                refused,
                Some(ApiError::new(StatusCode::BAD_REQUEST, "invalid_request")),
                "{label:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_finish_is_refused_before_anything_is_stored_and_spends_its_ceremony() {
        let state = state();
        let user = UserId::generate();
        let key = SoftAuthenticator::new();
        let expired = Some(ApiError::new(StatusCode::BAD_REQUEST, "ceremony_expired"));

        let (ceremony_id, options) = held(&state, user, "phone");
        let elsewhere = key.create("https://evil.example", &options);
        let here = key.create(ORIGIN, &options);
        assert_eq!(
            finished(&state, user, &ceremony_id, elsewhere).await,
            Some(ApiError::new(StatusCode::BAD_REQUEST, "invalid_request"))
        );
        assert_eq!(
            finished(&state, user, &ceremony_id, here.clone()).await,
            expired
        );

        let (anothers, options) = held(&state, UserId::generate(), "phone");
        let theirs = key.create(ORIGIN, &options);
        assert_eq!(finished(&state, user, &anothers, theirs).await, expired);
        assert_eq!(
            finished(&state, user, "AAAAAAAAAAAAAAAAAAAAAA", here).await,
            expired
        );
    }
}

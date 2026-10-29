//! The first passkey of a tenant that does not exist yet. Neither route has a
//! session to ask: the `Origin` check in `crate::routes` is all that guards
//! them, and what the finish creates is read from the ceremony the start held.

use super::{
    ceremony_not_held, session_expiry, session_not_opened, session_started, store_failed, unix_now,
    ApiError, CeremonyCredential, CeremonyStarted, JsonBody,
};
use crate::auth::ceremonies::Ceremony;
use crate::auth::policy;
use crate::auth::session::SessionSecret;
use crate::auth::webauthn::{Registered, RegistrationError, RegistrationState};
use crate::routes::AppState;
use crate::store::passkeys::NewPasskey;
use crate::store::pool::{register_user, ReservedUser};
use crate::store::StoreError;
use axum::extract::State;
use axum::response::Response;
use axum::Json;
use serde::Deserialize;

pub(super) const MAX_LABEL_CHARS: usize = 64;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewLabel {
    pub(super) label: String,
}

/// The label as it is stored and shown, or `None` for one that, once trimmed,
/// is empty, longer than [`MAX_LABEL_CHARS`] or holds a control character.
/// The bound is in characters, as `hub-web/src/lib/passkeyLabel.ts` counts
/// it: in bytes, a label the page let through would be refused here.
///
/// Postgres stores no U+0000 in `TEXT`, so a label holding one would be held
/// by its ceremony and fail the finish's `INSERT` as a `500`. The rest of
/// general category Cc is refused with it: none belongs in a name that is
/// listed and sent to the authenticator as `user.name`.
pub(super) fn accepted_label(typed: &str) -> Option<&str> {
    let label = typed.trim();
    let storable =
        (1..=MAX_LABEL_CHARS).contains(&label.chars().count()) && !label.contains(char::is_control);
    storable.then_some(label)
}

/// Which check a credential failed is not told: every fault of the browser's
/// answer is one `invalid_request`, apart from the two a page can act on.
pub(super) fn registration_refused(error: RegistrationError) -> ApiError {
    match error {
        RegistrationError::UnsupportedAlgorithm => ApiError::UNSUPPORTED_ALGORITHM,
        RegistrationError::CredentialExcluded => ApiError::CREDENTIAL_EXISTS,
        RegistrationError::Malformed
        | RegistrationError::WrongChallenge
        | RegistrationError::WrongOrigin
        | RegistrationError::UserNotVerified
        | RegistrationError::Refused => ApiError::INVALID_REQUEST,
        RegistrationError::Library => {
            tracing::error!(%error, "a registration ceremony failed in the verifier");
            ApiError::INTERNAL
        }
    }
}

pub(super) fn passkey_not_stored(error: StoreError) -> ApiError {
    match error {
        StoreError::CredentialExists => ApiError::CREDENTIAL_EXISTS,
        other => store_failed(other),
    }
}

/// The row a verified registration becomes, under the label its ceremony held.
pub(super) fn verified_passkey(label: String, registered: Registered) -> NewPasskey {
    NewPasskey {
        credential_id: registered.credential_id,
        public_key: registered.public_key.to_vec(),
        label,
        sign_count: registered.sign_count,
        credential: registered.credential,
    }
}

pub async fn start_registration(
    State(state): State<AppState>,
    JsonBody(start): JsonBody<NewLabel>,
) -> Result<Json<CeremonyStarted>, ApiError> {
    let label = accepted_label(&start.label).ok_or(ApiError::INVALID_REQUEST)?;
    let user = ReservedUser::generate();
    let (options, registration) = state
        .verifier()
        .start_registration(user.handle(), label, &[])
        .map_err(registration_refused)?;
    let ceremony = Ceremony::Register {
        user,
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

fn held_registration(
    taken: Option<Ceremony>,
) -> Result<(ReservedUser, String, RegistrationState), ApiError> {
    match taken {
        Some(Ceremony::Register {
            user,
            label,
            registration,
        }) => Ok((user, label, registration)),
        _ => Err(ApiError::CEREMONY_EXPIRED),
    }
}

/// The tenant is created by this request, so its passkey is admitted against
/// no existing key.
pub async fn finish_registration(
    State(state): State<AppState>,
    JsonBody(finish): JsonBody<CeremonyCredential>,
) -> Result<Response, ApiError> {
    let taken = state.ceremonies().take(unix_now(), &finish.ceremony_id);
    let (user, label, registration) = held_registration(taken)?;
    let registered = state
        .verifier()
        .finish_registration(&finish.credential, registration)
        .map_err(registration_refused)?;
    let passkey = verified_passkey(label, registered);
    let secret = SessionSecret::generate().map_err(session_not_opened)?;
    register_user(
        user,
        state.db(),
        &passkey,
        policy::admission(&[]),
        secret.hash(),
        session_expiry(),
    )
    .await
    .map_err(passkey_not_stored)?;
    Ok(session_started(&secret, &passkey.credential_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::ceremonies::MAX_LIVE_CEREMONIES;
    use crate::auth::challenge::{base64url_encode, ChallengeBinding, CHALLENGE_TTL_SECS};
    use crate::auth::testing::{lazy_state, SoftAuthenticator};
    use crate::auth::webauthn::Verifier;
    use crate::auth::RelyingParty;
    use crate::store::UserId;
    use axum::http::StatusCode;
    use serde_json::{json, Value};

    const ORIGIN: &str = "https://hub.example";

    fn rp() -> RelyingParty {
        RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: ORIGIN.to_owned(),
        }
    }

    /// The database is never dialled: nothing here gets as far as storing.
    fn state() -> AppState {
        lazy_state("postgres://hub@db.invalid/hub", rp()).unwrap()
    }

    fn verifier() -> Verifier {
        Verifier::new(&rp()).unwrap()
    }

    async fn started(state: &AppState, label: &str) -> Result<(String, Value), ApiError> {
        let start = NewLabel {
            label: label.to_owned(),
        };
        start_registration(State(state.clone()), JsonBody(start))
            .await
            .map(|Json(started)| (started.ceremony_id, started.options))
    }

    async fn finished(state: &AppState, ceremony_id: &str, credential: Value) -> Option<ApiError> {
        let finish = CeremonyCredential {
            ceremony_id: ceremony_id.to_owned(),
            credential,
        };
        finish_registration(State(state.clone()), JsonBody(finish))
            .await
            .err()
    }

    #[test]
    fn a_label_is_kept_trimmed_when_it_has_1_to_64_characters() {
        let longest = "é".repeat(64);
        for (typed, kept) in [
            ("Work laptop", Some("Work laptop")),
            ("  Work laptop\t\n", Some("Work laptop")),
            ("x", Some("x")),
            (longest.as_str(), Some(longest.as_str())),
            ("", None),
            (" \t\n", None),
        ] {
            assert_eq!(accepted_label(typed), kept, "{typed:?}");
        }
        assert_eq!(accepted_label(&"x".repeat(65)), None);
        assert_eq!(accepted_label(&format!(" {longest} ")), Some(&*longest));
        assert_eq!(accepted_label(&format!("{longest}é")), None);
    }

    #[test]
    fn a_label_holding_a_control_character_is_not_accepted() {
        for typed in ["\0", "a\0b", "a\u{7f}b", "line\nbreak"] {
            assert_eq!(accepted_label(typed), None, "{typed:?}");
        }
        let longest = "x".repeat(64);
        for (typed, kept) in [
            ("\n Work laptop\t", "Work laptop"),
            ("📱", "📱"),
            (longest.as_str(), longest.as_str()),
        ] {
            assert_eq!(accepted_label(typed), Some(kept), "{typed:?}");
        }
    }

    #[test]
    fn every_registration_failure_has_the_status_and_the_code_the_api_names() {
        let invalid = ApiError::new(StatusCode::BAD_REQUEST, "invalid_request");
        for (failure, expected) in [
            (
                RegistrationError::UnsupportedAlgorithm,
                ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "unsupported_algorithm"),
            ),
            (
                RegistrationError::CredentialExcluded,
                ApiError::new(StatusCode::CONFLICT, "credential_exists"),
            ),
            (RegistrationError::Malformed, invalid),
            (RegistrationError::WrongChallenge, invalid),
            (RegistrationError::WrongOrigin, invalid),
            (RegistrationError::UserNotVerified, invalid),
            (RegistrationError::Refused, invalid),
            (RegistrationError::Library, ApiError::INTERNAL),
        ] {
            assert_eq!(registration_refused(failure), expected, "{failure:?}");
        }
    }

    #[test]
    fn a_credential_another_tenant_holds_is_409_and_any_other_store_failure_is_500() {
        assert_eq!(
            passkey_not_stored(StoreError::CredentialExists),
            ApiError::new(StatusCode::CONFLICT, "credential_exists")
        );
        assert_eq!(
            passkey_not_stored(StoreError::TooLarge { bytes: 4242 }),
            ApiError::INTERNAL
        );
    }

    #[test]
    fn a_body_with_a_member_the_route_does_not_take_does_not_parse() {
        let label = |body| serde_json::from_value::<NewLabel>(body).map(|new| new.label);
        assert_eq!(label(json!({ "label": "laptop" })).unwrap(), "laptop");
        for body in [
            json!({}),
            json!({ "label": 7 }),
            json!({ "label": "laptop", "status": "trusted" }),
        ] {
            assert!(label(body.clone()).is_err(), "{body}");
        }

        let finish = |body| serde_json::from_value::<CeremonyCredential>(body);
        assert!(finish(json!({ "ceremony_id": "AAAA", "credential": {} })).is_ok());
        for body in [
            json!({ "ceremony_id": "AAAA" }),
            json!({ "credential": {} }),
            json!({ "ceremony_id": "AAAA", "credential": {}, "label": "laptop" }),
        ] {
            assert!(finish(body.clone()).is_err(), "{body}");
        }
    }

    #[tokio::test]
    async fn the_started_ceremony_is_for_a_fresh_user_under_the_trimmed_label() {
        let state = state();

        let (first_id, first) = started(&state, "  Work laptop ").await.unwrap();
        let (second_id, second) = started(&state, "Phone").await.unwrap();

        assert_eq!(first["publicKey"]["user"]["name"], "Work laptop");
        assert_eq!(first["publicKey"]["excludeCredentials"], json!([]));
        assert_ne!(
            first["publicKey"]["user"]["id"],
            second["publicKey"]["user"]["id"]
        );
        let held = state.ceremonies().take(unix_now(), &first_id);
        let (user, label, _) = held_registration(held).unwrap();
        assert_eq!(label, "Work laptop");
        assert_eq!(
            first["publicKey"]["user"]["id"],
            base64url_encode(user.handle().as_bytes())
        );
        assert!(state.ceremonies().take(unix_now(), &second_id).is_some());
    }

    #[tokio::test]
    async fn a_label_that_is_not_accepted_starts_no_ceremony() {
        let state = state();
        for _ in 1..MAX_LIVE_CEREMONIES {
            started(&state, "filler").await.unwrap();
        }

        for label in ["", "   ", &"x".repeat(65)] {
            assert_eq!(
                started(&state, label).await.err(),
                Some(ApiError::new(StatusCode::BAD_REQUEST, "invalid_request")),
                "{label:?}"
            );
        }

        assert!(started(&state, "the last place").await.is_ok());
        assert_eq!(
            started(&state, "one too many").await.err(),
            Some(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "too_many_ceremonies"
            ))
        );
    }

    #[test]
    fn only_a_registration_is_taken_up_at_the_finish() {
        let expired = ApiError::new(StatusCode::BAD_REQUEST, "ceremony_expired");
        let user = ReservedUser::generate();
        let handle = user.handle();
        let (_, registration) = verifier()
            .start_registration(handle, "laptop", &[])
            .unwrap();
        let register = Ceremony::Register {
            user,
            label: "laptop".to_owned(),
            registration,
        };
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

        let (held_user, label, _) = held_registration(Some(register)).unwrap();
        assert_eq!((held_user.handle(), label.as_str()), (handle, "laptop"));
        assert_eq!(held_registration(None).err(), Some(expired));
        assert_eq!(held_registration(Some(endorse)).err(), Some(expired));
    }

    #[test]
    fn a_verified_registration_is_stored_as_the_verifier_read_it_under_the_held_label() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new().with_sign_count(7);
        let (options, registration) = verifier
            .start_registration(ReservedUser::generate().handle(), "typed", &[])
            .unwrap();
        let registered = verifier
            .finish_registration(&authenticator.create(ORIGIN, &options), registration)
            .unwrap();

        let stored = verified_passkey("held".to_owned(), registered.clone());

        assert_eq!(
            base64url_encode(&stored.credential_id),
            authenticator.credential_id()
        );
        assert_eq!(stored.public_key, authenticator.public_key_cose());
        assert_eq!(stored.label, "held");
        assert_eq!(stored.sign_count, 7);
        assert_eq!(stored.credential, registered.credential);
    }

    #[tokio::test]
    async fn a_finish_the_verifier_refuses_is_400_and_spends_the_ceremony() {
        let state = state();
        let authenticator = SoftAuthenticator::new();
        let (ceremony_id, options) = started(&state, "laptop").await.unwrap();
        let elsewhere = authenticator.create("https://evil.example", &options);
        let here = authenticator.create(ORIGIN, &options);

        assert_eq!(
            finished(&state, &ceremony_id, elsewhere).await,
            Some(ApiError::new(StatusCode::BAD_REQUEST, "invalid_request"))
        );
        assert_eq!(
            finished(&state, &ceremony_id, here.clone()).await,
            Some(ApiError::new(StatusCode::BAD_REQUEST, "ceremony_expired"))
        );
        assert_eq!(
            finished(&state, "AAAAAAAAAAAAAAAAAAAAAA", here).await,
            Some(ApiError::new(StatusCode::BAD_REQUEST, "ceremony_expired"))
        );
    }
}

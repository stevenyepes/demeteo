//! What a request presenting a DPoP-bound access token has to show before it
//! is acted on, and who it is admitted as. Every route an instance calls with
//! its access token is admitted here, each with the method and URL its proofs
//! are made for, so there is one sequence and not one per route.
//!
//! [`admitted`] is the whole of admission past the extractors, and the order
//! of its steps is what it guarantees:
//!
//! 1. One proof and one token that reads as a token, before anything is
//!    verified. A request that names no key or no token costs no ECDSA.
//! 2. The proof is verified and then remembered before any row is read, the
//!    order `/token` keeps and for its reason: a proof is spent by being
//!    presented, whatever becomes of the request it came with.
//! 3. `ath` before the store is asked. A proof that does not name the token
//!    beside it was made for another request, and what that token's row says
//!    is not this caller's to learn.
//! 4. The token's row, then [`access_verdict`](grant::access_verdict) over it,
//!    then the instance of the key. The tenant and the instance are the row's
//!    and the key's: nothing the request says chooses either.
//! 5. The token's first use is recorded, last, so that only a request that
//!    was admitted makes its pair one a lost-answer replay cannot replace.
//!
//! # One answer
//!
//! Every credential failure is [`ApiError::INVALID_TOKEN`], a missing header
//! as much as a retired token, for the reason the refusals of a code share
//! one in `pairing/handlers/token.rs`. A captured request is exactly a token
//! with something missing, and an answer per fault would say what.
//!
//! `ProofError` and `RememberRefusal` already convert into `ApiError`, with
//! `/token`'s `400`. **A bare `?` on either compiles here and answers that
//! `400`.** The mappings below are therefore functions, applied by name at
//! every site, and none has a wildcard arm: a variant added to a refusal has
//! to be given this answer before the crate compiles.

use crate::auth::challenge::base64url_encode;
use crate::auth::handlers::{store_failed, ApiError};
use crate::pairing::dpop::{self, Expected, ProofError, VerifiedProof};
use crate::pairing::grant::{self, AccessRefusal, TokenSecret};
use crate::pairing::pending::RememberRefusal;
use crate::routes::AppState;
use crate::store::tokens::{self, TokenHash, TokenRecord};
use crate::store::{device_keys, Db, InstanceId, StoreError, UserId};
use aws_lc_rs::constant_time::verify_slices_are_equal;
use axum::http::header::WWW_AUTHENTICATE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use std::future::Future;
use uuid::Uuid;

const TOKEN_SCHEME: &str = "DPoP";

/// Who a request was admitted as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admission {
    pub user: UserId,
    pub instance: InstanceId,
    /// Unix seconds: the `expires_at` of the token presented. It is carried
    /// out because admission happens once and a socket outlives it. A
    /// connection held open past this would outlive the bound an expiry is
    /// there to put on whoever holds the token.
    pub token_expires_at: u64,
}

/// The `htu` a proof for `path` has to name: the configured origin and the
/// path. `htu` is compared as a whole string, so there has to be one
/// spelling, and the one built from configuration is the one `/token`
/// already has a client build. Never from `Host`, for the reason given where
/// `/token` builds its own.
pub fn htu(state: &AppState, path: &str) -> String {
    format!("{}{path}", state.rp().origin)
}

/// The access token of an `Authorization` header, in both the forms admission
/// needs: the text for `ath`, the hash for the lookup. Deliberately not
/// `Debug`: `text` is the token.
struct PresentedToken<'a> {
    text: &'a str,
    hash: TokenHash,
}

/// `DPoP <token>`, with exactly one space. The scheme is matched in any case
/// (RFC 9110 §11.1); the token is not trimmed or normalised, since `ath` is
/// a digest of its text and there is one text a client could have hashed.
fn presented_token(authorization: &str) -> Option<PresentedToken<'_>> {
    let (scheme, text) = authorization.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case(TOKEN_SCHEME) {
        return None;
    }
    let hash = TokenSecret::parse(text)?.hash();
    Some(PresentedToken { text, hash })
}

/// Whether a proof's `ath` is the one RFC 9449 §4.2 defines for `token`:
/// unpadded base64url of SHA-256 over the token's ASCII text. Not
/// [`TokenHash`], which is a digest of the 32 bytes that text encodes; the
/// two never agree.
fn names_token(ath: Option<&str>, token: &str) -> bool {
    let expected = base64url_encode(&Sha256::digest(token.as_bytes()));
    ath.is_some_and(|ath| verify_slices_are_equal(ath.as_bytes(), expected.as_bytes()).is_ok())
}

fn proof_refused(error: ProofError) -> ApiError {
    match error {
        ProofError::Malformed
        | ProofError::Type
        | ProofError::Algorithm
        | ProofError::Key
        | ProofError::Method
        | ProofError::Uri
        | ProofError::Stale
        | ProofError::Signature => ApiError::INVALID_TOKEN,
    }
}

/// `Full` is about the Hub and not about the caller, so it keeps the answer
/// `/token` gives it: a client that retries a `503` is right to, and one told
/// its token was invalid would refresh a token that was fine.
fn proof_not_remembered(refusal: RememberRefusal) -> ApiError {
    match refusal {
        RememberRefusal::Replayed => ApiError::INVALID_TOKEN,
        RememberRefusal::Full => ApiError::TOO_MANY_REQUESTS,
    }
}

fn token_refused(refusal: AccessRefusal) -> ApiError {
    match refusal {
        AccessRefusal::NotAccess
        | AccessRefusal::KeyMismatch
        | AccessRefusal::Retired
        | AccessRefusal::Expired => ApiError::INVALID_TOKEN,
    }
}

/// The proof a request carried, verified against `expected` and spent.
fn spent_proof(
    state: &AppState,
    presented: &str,
    expected: &Expected<'_>,
    now: u64,
) -> Result<VerifiedProof, ApiError> {
    let proof = dpop::verify(presented, expected, now).map_err(proof_refused)?;
    state
        .pairings()
        .remember_proof(&proof, now)
        .map_err(proof_not_remembered)?;
    Ok(proof)
}

/// What admission asks of a store, so the whole of it is reachable from a
/// test that has no database; a route passes [`PostgresAccess`].
pub trait AccessStore: Sync {
    /// The row a presented token's hash finds, whatever its kind or state,
    /// as [`find_token_by_hash`](tokens::find_token_by_hash) reads it.
    fn find_token(
        &self,
        hash: TokenHash,
    ) -> impl Future<Output = Result<Option<TokenRecord>, StoreError>> + Send;

    /// The instance of the tenant's device key `jkt`, as
    /// [`instance_of_key`](device_keys::instance_of_key) reads it.
    fn instance_of_key(
        &self,
        user: UserId,
        jkt: String,
    ) -> impl Future<Output = Result<Option<InstanceId>, StoreError>> + Send;

    /// Records the token `id` as used, and answers whether it was still
    /// unretired, as [`mark_first_use`](tokens::mark_first_use) does.
    fn mark_first_use(
        &self,
        user: UserId,
        id: Uuid,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;
}

/// [`AccessStore`] over the Hub's database.
pub struct PostgresAccess<'a>(pub &'a Db);

impl AccessStore for PostgresAccess<'_> {
    async fn find_token(&self, hash: TokenHash) -> Result<Option<TokenRecord>, StoreError> {
        tokens::find_token_by_hash(self.0, &hash).await
    }

    async fn instance_of_key(
        &self,
        user: UserId,
        jkt: String,
    ) -> Result<Option<InstanceId>, StoreError> {
        device_keys::instance_of_key(user, self.0, &jkt).await
    }

    async fn mark_first_use(&self, user: UserId, id: Uuid) -> Result<bool, StoreError> {
        tokens::mark_first_use(user, self.0, id).await
    }
}

/// Everything admission does. `authorization` is the request's
/// `Authorization` header and `proof` its one `DPoP` header, `None` when it
/// carried none or more than one. `expected` is the method and URL the proof
/// has to be made for, its `htu` built by [`htu`].
///
/// The instance of the key is asked for only for a token that passed: its
/// `None` is a key that was revoked, or whose instance was deleted, under a
/// token that has not yet expired.
///
/// The token is recorded as used last, once everything else admitted it,
/// and only the first time: a token the store already holds as used is not
/// written again. That record is what keeps a lost-answer replay from
/// replacing this token's pair ([`refresh_change`](grant::refresh_change)),
/// and a replay that got there first has retired the token, which refuses
/// the request here as any retired token is refused.
pub async fn admitted(
    state: &AppState,
    authorization: Option<&str>,
    proof: Option<&str>,
    expected: &Expected<'_>,
    now: u64,
    store: &impl AccessStore,
) -> Result<Admission, ApiError> {
    let presented = proof.ok_or(ApiError::INVALID_TOKEN)?;
    let token = authorization
        .and_then(presented_token)
        .ok_or(ApiError::INVALID_TOKEN)?;
    let proof = spent_proof(state, presented, expected, now)?;
    if !names_token(proof.ath.as_deref(), token.text) {
        return Err(ApiError::INVALID_TOKEN);
    }
    let row = store
        .find_token(token.hash)
        .await
        .map_err(store_failed)?
        .ok_or(ApiError::INVALID_TOKEN)?;
    grant::access_verdict(&row, &proof.jkt, now).map_err(token_refused)?;
    let token_expires_at =
        u64::try_from(row.expires_at.unix_timestamp()).map_err(|_| ApiError::INVALID_TOKEN)?;
    let instance = store
        .instance_of_key(row.user_id, proof.jkt)
        .await
        .map_err(store_failed)?
        .ok_or(ApiError::INVALID_TOKEN)?;
    if row.used_at.is_none() {
        let unretired = store
            .mark_first_use(row.user_id, row.id)
            .await
            .map_err(store_failed)?;
        if !unretired {
            return Err(ApiError::INVALID_TOKEN);
        }
    }
    Ok(Admission {
        user: row.user_id,
        instance,
        token_expires_at,
    })
}

/// What a refused request is answered. A `401` names the scheme it wanted
/// (RFC 9110 §15.5.2 requires a challenge of one, RFC 9449 §7.1 says which),
/// and it is added here, by status, so that no refusal of [`admitted`] can
/// be a `401` without it.
///
/// **Rejected: a challenge on `ApiError` itself.** The `/v1` routes answer
/// `401` to a browser with no session, and a challenge there would name a
/// scheme those routes do not take.
pub fn refusal(error: ApiError) -> Response {
    let mut response = error.into_response();
    if response.status() == StatusCode::UNAUTHORIZED {
        response
            .headers_mut()
            .insert(WWW_AUTHENTICATE, HeaderValue::from_static(TOKEN_SCHEME));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::testing::lazy_state;
    use crate::auth::RelyingParty;
    use crate::pairing::grant::{FamilyChange, FreshPair};
    use crate::pairing::testing::{claims, MemoryFamily, SoftDeviceKey};
    use serde_json::json;

    const NOW: u64 = 1_700_000_000;
    const PATH: &str = "/socket";
    const URL: &str = "https://hub.example/socket";

    const INVALID_TOKEN: ApiError = ApiError::new(StatusCode::UNAUTHORIZED, "invalid_token");

    #[test]
    fn every_credential_refusal_is_the_one_401_and_a_full_window_is_503() {
        for fault in [
            ProofError::Malformed,
            ProofError::Type,
            ProofError::Algorithm,
            ProofError::Key,
            ProofError::Method,
            ProofError::Uri,
            ProofError::Stale,
            ProofError::Signature,
        ] {
            assert_eq!(proof_refused(fault), INVALID_TOKEN, "{fault:?}");
        }
        for refusal in [
            AccessRefusal::NotAccess,
            AccessRefusal::KeyMismatch,
            AccessRefusal::Retired,
            AccessRefusal::Expired,
        ] {
            assert_eq!(token_refused(refusal), INVALID_TOKEN, "{refusal:?}");
        }
        assert_eq!(
            proof_not_remembered(RememberRefusal::Replayed),
            INVALID_TOKEN
        );
        assert_eq!(
            proof_not_remembered(RememberRefusal::Full),
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "too_many_requests")
        );
    }

    #[test]
    fn a_token_is_read_from_one_spelling_of_the_header_and_the_scheme_in_any_case() {
        let token = TokenSecret::generate().unwrap();
        let text = token.text();
        let read = |authorization: String| {
            presented_token(&authorization).map(|token| (token.text.to_owned(), token.hash))
        };

        for scheme in ["DPoP", "dpop", "DPOP"] {
            assert_eq!(
                read(format!("{scheme} {text}")),
                Some((text.clone(), token.hash())),
                "{scheme}"
            );
        }
        for unreadable in [
            text.clone(),
            format!("Bearer {text}"),
            format!("DPoP{text}"),
            format!("DPoP  {text}"),
            format!("DPoP {text} "),
            format!("DPoP {text}="),
            format!("DPoP {}", &text[1..]),
            "DPoP ".to_owned(),
            "DPoP".to_owned(),
            String::new(),
        ] {
            assert!(read(unreadable.clone()).is_none(), "{unreadable:?}");
        }
    }

    #[test]
    fn a_401_carries_the_dpop_challenge_and_no_other_refusal_does() {
        let challenges = |error: ApiError| {
            let response = refusal(error);
            let challenges: Vec<_> = response
                .headers()
                .get_all(WWW_AUTHENTICATE)
                .iter()
                .cloned()
                .collect();
            (response.status(), challenges)
        };

        assert_eq!(
            challenges(ApiError::INVALID_TOKEN),
            (
                StatusCode::UNAUTHORIZED,
                vec![HeaderValue::from_static("DPoP")]
            )
        );
        for (other, status) in [
            (ApiError::INVALID_REQUEST, StatusCode::BAD_REQUEST),
            (ApiError::TOO_MANY_REQUESTS, StatusCode::SERVICE_UNAVAILABLE),
            (
                ApiError::TOO_MANY_CONNECTIONS,
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (ApiError::INTERNAL, StatusCode::INTERNAL_SERVER_ERROR),
        ] {
            assert_eq!(challenges(other), (status, vec![]), "{other:?}");
        }
    }

    /// The store is never dialled: a [`MemoryAccess`] stands where it would be.
    fn state() -> AppState {
        let rp = RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: "https://hub.example".to_owned(),
        };
        lazy_state("postgres://hub@db.invalid/hub", rp).unwrap()
    }

    /// The store admission asks, over a [`MemoryFamily`]. `failing` names
    /// the one call that fails, if any.
    struct MemoryAccess {
        family: std::sync::Mutex<MemoryFamily>,
        instance: InstanceId,
        failing: Option<&'static str>,
    }

    impl MemoryAccess {
        fn family(&self) -> std::sync::MutexGuard<'_, MemoryFamily> {
            self.family.lock().unwrap()
        }

        fn fails(&self, call: &str) -> Result<(), StoreError> {
            match self.failing {
                Some(failing) if failing == call => Err(StoreError::TooLarge { bytes: 0 }),
                _ => Ok(()),
            }
        }
    }

    impl AccessStore for MemoryAccess {
        async fn find_token(&self, hash: TokenHash) -> Result<Option<TokenRecord>, StoreError> {
            self.fails("find_token")?;
            Ok(self.family().find(&hash).cloned())
        }

        async fn instance_of_key(
            &self,
            _: UserId,
            _: String,
        ) -> Result<Option<InstanceId>, StoreError> {
            self.fails("instance_of_key")?;
            Ok(Some(self.instance))
        }

        async fn mark_first_use(&self, _: UserId, id: Uuid) -> Result<bool, StoreError> {
            self.fails("mark_first_use")?;
            let mut family = self.family();
            let Some(row) = family.rows().iter().find(|row| row.id == id) else {
                return Ok(false);
            };
            let hash = row.hash.clone();
            Ok(family.mark_used(&hash, NOW))
        }
    }

    struct Caller {
        key: SoftDeviceKey,
        store: MemoryAccess,
        authorization: String,
        token: String,
    }

    impl Caller {
        fn paired() -> Self {
            let key = SoftDeviceKey::new();
            let pair = FreshPair::generate().unwrap();
            let family = MemoryFamily::paired(&pair, &key.thumbprint(), NOW);
            let token = pair.access.text();
            Self {
                authorization: format!("DPoP {token}"),
                key,
                store: MemoryAccess {
                    family: std::sync::Mutex::new(family),
                    instance: InstanceId::generate(),
                    failing: None,
                },
                token,
            }
        }

        fn failing(mut self, call: &'static str) -> Self {
            self.store.failing = Some(call);
            self
        }

        fn proof(&self, htm: &str, htu: &str, jti: &str) -> String {
            let mut claims = claims(htm, htu, NOW, jti);
            claims["ath"] = json!(base64url_encode(&Sha256::digest(self.token.as_bytes())));
            self.key.proof_with(&self.key.header(), &claims)
        }

        /// The presented token's row as the store now holds it.
        fn presented(&self) -> TokenRecord {
            let Some(secret) = TokenSecret::parse(&self.token) else {
                panic!("the caller's own token does not parse");
            };
            let found = self.store.family().find(&secret.hash()).cloned();
            found.unwrap()
        }

        async fn admitted(
            &self,
            state: &AppState,
            proof: &str,
            expected: &Expected<'_>,
        ) -> Result<Admission, ApiError> {
            admitted(
                state,
                Some(&self.authorization),
                Some(proof),
                expected,
                NOW,
                &self.store,
            )
            .await
        }
    }

    const SOCKET: Expected<'static> = Expected {
        htm: "GET",
        htu: URL,
    };

    #[tokio::test]
    async fn the_htu_is_the_configured_origin_and_the_path() {
        let state = state();
        assert_eq!(htu(&state, PATH), URL);
        assert_eq!(
            htu(&state, "/attachments/a1"),
            "https://hub.example/attachments/a1"
        );
    }

    #[tokio::test]
    async fn a_proof_is_admitted_only_for_the_method_and_url_it_was_expected_for() {
        let state = state();
        let caller = Caller::paired();
        let other_url = htu(&state, "/attachments/a1");

        for (jti, htm, url) in [("jti-1", "POST", URL), ("jti-2", "GET", other_url.as_str())] {
            let proof = caller.proof(htm, url, jti);
            assert_eq!(
                caller.admitted(&state, &proof, &SOCKET).await,
                Err(INVALID_TOKEN),
                "{htm} {url}"
            );
        }

        let elsewhere = Expected {
            htm: "GET",
            htu: &other_url,
        };
        let proof = caller.proof("GET", &other_url, "jti-3");
        let admission = caller.admitted(&state, &proof, &elsewhere).await;
        assert_eq!(
            admission.map(|admission| admission.instance),
            Ok(caller.store.instance)
        );
    }

    #[tokio::test]
    async fn a_store_that_fails_any_call_is_500_and_the_same_calls_answered_admit() {
        let state = state();
        let caller = Caller::paired();
        let proof = caller.proof("GET", URL, "jti-1");
        let admission = caller.admitted(&state, &proof, &SOCKET).await;
        assert_eq!(
            admission.map(|admission| admission.instance),
            Ok(caller.store.instance)
        );

        for (n, call) in ["find_token", "instance_of_key", "mark_first_use"]
            .into_iter()
            .enumerate()
        {
            let caller = Caller::paired().failing(call);
            let proof = caller.proof("GET", URL, &format!("jti-failing-{n}"));
            let admission = caller.admitted(&state, &proof, &SOCKET).await;
            assert_eq!(admission, Err(ApiError::INTERNAL), "{call}");
        }
    }

    #[tokio::test]
    async fn a_token_is_recorded_as_used_by_its_first_admission_and_never_written_again() {
        let state = state();
        let caller = Caller::paired();
        assert_eq!(caller.presented().used_at, None);

        let misdirected = caller.proof("GET", "https://hub.example/elsewhere", "jti-0");
        let refused = caller.admitted(&state, &misdirected, &SOCKET).await;
        assert_eq!(refused, Err(INVALID_TOKEN));
        assert_eq!(caller.presented().used_at, None);

        let first = caller.proof("GET", URL, "jti-1");
        assert!(caller.admitted(&state, &first, &SOCKET).await.is_ok());
        let used_at = caller.presented().used_at;
        assert!(used_at.is_some());

        // A store whose write fails admits only a token it is not asked to
        // write: one already recorded as used.
        let caller = caller.failing("mark_first_use");
        let second = caller.proof("GET", URL, "jti-2");
        assert!(caller.admitted(&state, &second, &SOCKET).await.is_ok());
        assert_eq!(caller.presented().used_at, used_at);
    }

    /// The row admission read passed every check, and a lost-answer replay
    /// retired the token before its use was recorded: the record finds it
    /// retired, and the request is refused as any retired token is.
    #[tokio::test]
    async fn a_token_retired_before_its_first_use_is_recorded_is_refused() {
        struct RetiredOnWrite(MemoryAccess);

        impl AccessStore for RetiredOnWrite {
            async fn find_token(&self, hash: TokenHash) -> Result<Option<TokenRecord>, StoreError> {
                self.0.find_token(hash).await
            }

            async fn instance_of_key(
                &self,
                user: UserId,
                jkt: String,
            ) -> Result<Option<InstanceId>, StoreError> {
                self.0.instance_of_key(user, jkt).await
            }

            async fn mark_first_use(&self, user: UserId, id: Uuid) -> Result<bool, StoreError> {
                let replayed = FamilyChange {
                    retire: vec![id],
                    insert: Vec::new(),
                };
                self.0.family().apply(&replayed, NOW);
                self.0.mark_first_use(user, id).await
            }
        }

        let state = state();
        let caller = Caller::paired();
        let proof = caller.proof("GET", URL, "jti-1");
        let store = RetiredOnWrite(caller.store);
        let admission = admitted(
            &state,
            Some(&caller.authorization),
            Some(&proof),
            &SOCKET,
            NOW,
            &store,
        )
        .await;
        assert_eq!(admission, Err(INVALID_TOKEN));
    }
}

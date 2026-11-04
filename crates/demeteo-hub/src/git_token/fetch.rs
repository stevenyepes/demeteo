//! `POST /git-token`: a runner asking for a fresh installation token for a
//! run the Hub started on it. The rules it keeps are [the module's](super).
//!
//! It is admitted as the attachment fetch is, by
//! [`pairing::access`](crate::pairing::access) with a proof for this one
//! method and URL, and the body is read only once the caller is known: one
//! with no credentials is told it is nobody, not what is wrong with its body.

use super::port::GitTokenStore;
use super::postgres::PostgresGitTokens;
use super::verdict::{
    body_verdict, bound_verdict, liveness_verdict, mint_verdict, repository_verdict, start_verdict,
    TokenRefusal,
};
use crate::auth::handlers::{header_text, store_failed, unix_now, ApiError};
use crate::pairing::access::{self, refusal, Admission};
use crate::pairing::dpop::Expected;
use crate::pairing::handlers::one_proof;
use crate::ports::{GitTokenMinter, InstallationToken};
use crate::routes::AppState;
use crate::socket::requests::named_request;
use crate::store::{device_keys, tokens};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;

/// The route [`post_git_token`] is mounted at, and the path of the `htu`
/// its proofs are made for.
pub const GIT_TOKEN_PATH: &str = "/git-token";
const GIT_TOKEN_METHOD: &str = "POST";

/// The token for the run `body` names, minted now for the instance
/// `admission` admitted. `now` is Unix seconds.
///
/// The Feature is bound before the token is minted, so that a mint that
/// fails leaves the binding the runner will ask again under. The binding is
/// judged twice: once over the row as read, which answers a later fetch, and
/// once over what the conditional `UPDATE` left, which settles two first
/// fetches at once.
pub async fn issue(
    store: &dyn GitTokenStore,
    minter: Option<&dyn GitTokenMinter>,
    admission: &Admission,
    body: &[u8],
    now: u64,
) -> Result<InstallationToken, ApiError> {
    let body = body_verdict(body)?;
    let id = named_request(&body.request_id).ok_or(TokenRefusal::NoStart)?;
    let (user, instance) = (admission.user, admission.instance);

    let kind = store
        .instance_kind(user, instance)
        .await
        .map_err(store_failed)?;
    let record = store
        .start(user, instance, id)
        .await
        .map_err(store_failed)?;
    let repository = start_verdict(kind, record.as_ref(), &body.feature_id)?;
    let events = store
        .events(user, instance, &body.feature_id)
        .await
        .map_err(store_failed)?;
    liveness_verdict(&events)?;
    let repository = repository_verdict(&repository)?;
    let minter = minter.ok_or(TokenRefusal::Unavailable)?;

    let bound = store
        .bind(user, instance, id, &body.feature_id)
        .await
        .map_err(store_failed)?
        .ok_or(TokenRefusal::NoStart)?;
    bound_verdict(Some(&bound), &body.feature_id)?;

    minter.mint(&repository, now).await.map_err(|error| {
        tracing::warn!(%error, "no git token was minted");
        ApiError::from(mint_verdict(&error))
    })
}

async fn issued(
    state: &AppState,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<InstallationToken, ApiError> {
    let db = state.db();
    let now = unix_now();
    let htu = access::htu(state, GIT_TOKEN_PATH);
    let expected = Expected {
        htm: GIT_TOKEN_METHOD,
        htu: &htu,
    };
    let admission = access::admitted(
        state,
        header_text(headers, AUTHORIZATION),
        one_proof(headers),
        &expected,
        now,
        |hash| async move { tokens::find_token_by_hash(db, &hash).await },
        |user, jkt| async move { device_keys::instance_of_key(user, db, &jkt).await },
    )
    .await?;
    let store = PostgresGitTokens::new(db.clone());
    issue(&store, state.git_tokens(), &admission, body, now).await
}

/// `POST` [`GIT_TOKEN_PATH`]: `{"token", "expires_at"}`, never cached.
pub async fn post_git_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match issued(&state, &headers, &body).await {
        Ok(token) => (
            [(CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({
                "token": token.secret(),
                "expires_at": token.expires_at,
            })),
        )
            .into_response(),
        Err(refused) => refusal(refused),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{GithubRepository, MintError};
    use crate::store::instances::InstanceKind;
    use crate::store::requests::{QueuedEnvelope, RequestId, StartRecord};
    use crate::store::{InstanceId, StoreError, UserId};
    use async_trait::async_trait;
    use axum::http::StatusCode;
    use demeteo_hub_protocol::{
        RequestOutcome, RequestPayload, RunEvent, RunEventKind, RunEventPayload, RunnerRepository,
        RunnerStart, RunnerWorkflow, SignedRequest, StartFeature, PROTOCOL_VERSION,
    };
    use std::sync::Mutex;

    const NOW: u64 = 1_700_000_000;
    const REQUEST: RequestId = [7; 16];
    const REQUEST_TEXT: &str = "BwcHBwcHBwcHBwcHBwcHBw";

    /// One tenant's one runner and its one start. Answers only for that
    /// tenant, instance and request id, and errors on every other one: a use
    /// case that reached for the wrong row fails instead of reading a
    /// default.
    struct Fleet {
        user: UserId,
        instance: InstanceId,
        kind: Option<InstanceKind>,
        start: Option<StartRecord>,
        events: Vec<RunEvent>,
        bound: Mutex<Option<String>>,
        /// A binding another fetch committed between this one's read and its
        /// `UPDATE`.
        raced: Option<String>,
    }

    impl Fleet {
        fn holding(&self, user: UserId, instance: InstanceId) -> Result<(), StoreError> {
            assert_eq!((user, instance), (self.user, self.instance));
            Ok(())
        }
    }

    #[async_trait]
    impl GitTokenStore for Fleet {
        async fn instance_kind(
            &self,
            user: UserId,
            instance: InstanceId,
        ) -> Result<Option<InstanceKind>, StoreError> {
            self.holding(user, instance)?;
            Ok(self.kind)
        }

        async fn start(
            &self,
            user: UserId,
            instance: InstanceId,
            id: RequestId,
        ) -> Result<Option<StartRecord>, StoreError> {
            self.holding(user, instance)?;
            if id != REQUEST {
                return Ok(None);
            }
            Ok(self.start.clone().map(|start| StartRecord {
                git_token_feature_id: self.bound.lock().unwrap().clone(),
                ..start
            }))
        }

        async fn bind(
            &self,
            user: UserId,
            instance: InstanceId,
            id: RequestId,
            feature_id: &str,
        ) -> Result<Option<String>, StoreError> {
            self.holding(user, instance)?;
            assert_eq!(id, REQUEST);
            let mut bound = self.bound.lock().unwrap();
            if bound.is_none() {
                *bound = Some(self.raced.clone().unwrap_or_else(|| feature_id.to_owned()));
            }
            Ok(bound.clone())
        }

        async fn events(
            &self,
            user: UserId,
            instance: InstanceId,
            feature_id: &str,
        ) -> Result<Vec<RunEvent>, StoreError> {
            self.holding(user, instance)?;
            Ok(self
                .events
                .iter()
                .filter(|event| event.feature_id == feature_id)
                .cloned()
                .collect())
        }
    }

    /// Answers each mint with the next scripted result, and records what it
    /// was asked for. Panics if asked more often than scripted.
    struct Minter {
        answers: Mutex<Vec<Result<InstallationToken, MintError>>>,
        asked: Mutex<Vec<(GithubRepository, u64)>>,
    }

    impl Minter {
        fn answering(answers: Vec<Result<InstallationToken, MintError>>) -> Self {
            Self {
                answers: Mutex::new(answers),
                asked: Mutex::new(Vec::new()),
            }
        }

        fn asked(&self) -> Vec<(GithubRepository, u64)> {
            self.asked.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl GitTokenMinter for Minter {
        async fn mint(
            &self,
            repository: &GithubRepository,
            now: u64,
        ) -> Result<InstallationToken, MintError> {
            self.asked.lock().unwrap().push((repository.clone(), now));
            self.answers.lock().unwrap().remove(0)
        }
    }

    fn minted() -> Result<InstallationToken, MintError> {
        Ok(InstallationToken::new("ghs_minted".to_owned(), NOW + 3600))
    }

    fn accepted_start(provider_kind: &str) -> StartRecord {
        StartRecord {
            kind: "start_feature".to_owned(),
            outcome: Some(RequestOutcome::Accepted),
            envelope: QueuedEnvelope::Readable(Box::new(SignedRequest {
                v: PROTOCOL_VERSION,
                request_id: REQUEST_TEXT.to_owned(),
                instance_id: "i".to_owned(),
                issued_at: NOW,
                expires_at: NOW + 600,
                payload: RequestPayload::StartFeature(StartFeature {
                    project_id: "p-1".to_owned(),
                    workflow_id: "w-1".to_owned(),
                    title: "T".to_owned(),
                    description: "D".to_owned(),
                    agent_kind: None,
                    model: None,
                    effort: None,
                    step_overrides: Vec::new(),
                    max_budget_cents: None,
                    max_wall_clock_secs: None,
                    attachments: Vec::new(),
                    runner: Some(Box::new(RunnerStart {
                        source_instance_id: "src".to_owned(),
                        repository: RunnerRepository {
                            provider_kind: provider_kind.to_owned(),
                            host: "github.com".to_owned(),
                            full_name: "acme/widgets".to_owned(),
                        },
                        workflow: RunnerWorkflow {
                            workflow_id: "w-1".to_owned(),
                            version: 1,
                            name: "Standard".to_owned(),
                            steps_json: "[]".to_owned(),
                        },
                    })),
                }),
                signature: "s".to_owned(),
            })),
            git_token_feature_id: None,
        }
    }

    fn fleet() -> Fleet {
        Fleet {
            user: UserId::generate(),
            instance: InstanceId::generate(),
            kind: Some(InstanceKind::Runner),
            start: Some(accepted_start("github")),
            events: Vec::new(),
            bound: Mutex::new(None),
            raced: None,
        }
    }

    fn admission(fleet: &Fleet) -> Admission {
        Admission {
            user: fleet.user,
            instance: fleet.instance,
            token_expires_at: NOW + 600,
        }
    }

    fn body(feature_id: &str) -> Vec<u8> {
        format!(r#"{{"request_id":"{REQUEST_TEXT}","feature_id":"{feature_id}"}}"#).into_bytes()
    }

    async fn status_of(
        fleet: &Fleet,
        minter: Option<&dyn GitTokenMinter>,
        feature_id: &str,
    ) -> Result<String, StatusCode> {
        issue(fleet, minter, &admission(fleet), &body(feature_id), NOW)
            .await
            .map(|token| token.secret().to_owned())
            .map_err(|refused| refused.into_response().status())
    }

    #[tokio::test]
    async fn the_first_fetch_binds_its_feature_and_is_handed_the_token() {
        let fleet = fleet();
        let minter = Minter::answering(vec![minted(), minted()]);

        assert_eq!(
            status_of(&fleet, Some(&minter), "f-1").await,
            Ok("ghs_minted".to_owned())
        );
        assert_eq!(fleet.bound.lock().unwrap().as_deref(), Some("f-1"));
        assert_eq!(
            status_of(&fleet, Some(&minter), "f-1").await,
            Ok("ghs_minted".to_owned()),
            "a refresh for the bound Feature"
        );
        assert_eq!(
            minter.asked(),
            vec![
                (
                    GithubRepository {
                        host: "github.com".to_owned(),
                        owner: "acme".to_owned(),
                        name: "widgets".to_owned(),
                    },
                    NOW
                );
                2
            ]
        );
    }

    #[tokio::test]
    async fn another_feature_after_the_binding_is_409_and_mints_nothing() {
        let fleet = fleet();
        *fleet.bound.lock().unwrap() = Some("f-1".to_owned());
        let minter = Minter::answering(Vec::new());

        assert_eq!(
            status_of(&fleet, Some(&minter), "f-2").await,
            Err(StatusCode::CONFLICT)
        );
        assert!(minter.asked().is_empty());
    }

    #[tokio::test]
    async fn the_loser_of_two_first_fetches_is_409_and_mints_nothing() {
        let fleet = Fleet {
            raced: Some("f-other".to_owned()),
            ..fleet()
        };
        let minter = Minter::answering(Vec::new());

        assert_eq!(
            status_of(&fleet, Some(&minter), "f-1").await,
            Err(StatusCode::CONFLICT)
        );
        assert!(minter.asked().is_empty());
    }

    #[tokio::test]
    async fn no_accepted_runner_start_is_404_and_binds_nothing() {
        let not_accepted = Fleet {
            start: Some(StartRecord {
                outcome: None,
                ..accepted_start("github")
            }),
            ..fleet()
        };
        let no_row = Fleet {
            start: None,
            ..fleet()
        };
        let desktop = Fleet {
            kind: Some(InstanceKind::Desktop),
            ..fleet()
        };
        for fleet in [not_accepted, no_row, desktop] {
            let minter = Minter::answering(Vec::new());
            assert_eq!(
                status_of(&fleet, Some(&minter), "f-1").await,
                Err(StatusCode::NOT_FOUND)
            );
            assert!(fleet.bound.lock().unwrap().is_none());
        }
        let fleet = fleet();
        let other_id = br#"{"request_id":"AAAAAAAAAAAAAAAAAAAAAA","feature_id":"f-1"}"#;
        let refused = issue(&fleet, None, &admission(&fleet), other_id, NOW).await;
        assert_eq!(refused.unwrap_err(), ApiError::NOT_FOUND);
        let not_an_id = br#"{"request_id":"not base64url!","feature_id":"f-1"}"#;
        let refused = issue(&fleet, None, &admission(&fleet), not_an_id, NOW).await;
        assert_eq!(refused.unwrap_err(), ApiError::NOT_FOUND);
    }

    fn event(kind: RunEventKind) -> RunEvent {
        RunEvent {
            v: 1,
            feature_id: "f-1".to_owned(),
            offset: 1,
            kind,
            occurred_at: NOW,
            payload: RunEventPayload::default(),
        }
    }

    #[tokio::test]
    async fn an_ended_run_is_410_and_another_features_end_is_not() {
        let fleet = Fleet {
            events: vec![event(RunEventKind::FeatureCompleted)],
            ..fleet()
        };
        let minter = Minter::answering(vec![minted()]);

        assert_eq!(
            status_of(&fleet, Some(&minter), "f-1").await,
            Err(StatusCode::GONE)
        );
        assert!(fleet.bound.lock().unwrap().is_none());
        assert_eq!(
            status_of(&fleet, Some(&minter), "f-2").await,
            Ok("ghs_minted".to_owned())
        );
    }

    #[tokio::test]
    async fn a_gitlab_start_is_422_and_so_is_a_github_host_with_no_api() {
        let gitlab = Fleet {
            start: Some(accepted_start("gitlab")),
            ..fleet()
        };
        let minter = Minter::answering(Vec::new());
        assert_eq!(
            status_of(&gitlab, Some(&minter), "f-1").await,
            Err(StatusCode::UNPROCESSABLE_ENTITY)
        );
        assert!(minter.asked().is_empty());

        let minter = Minter::answering(vec![Err(MintError::UnknownHost("ghe".to_owned()))]);
        assert_eq!(
            status_of(&fleet(), Some(&minter), "f-1").await,
            Err(StatusCode::UNPROCESSABLE_ENTITY)
        );
    }

    #[tokio::test]
    async fn no_app_or_a_failed_mint_is_503_with_the_binding_kept() {
        let fleet = fleet();
        assert_eq!(
            status_of(&fleet, None, "f-1").await,
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );

        let minter = Minter::answering(vec![Err(MintError::NotInstalled)]);
        assert_eq!(
            status_of(&fleet, Some(&minter), "f-1").await,
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
        assert_eq!(fleet.bound.lock().unwrap().as_deref(), Some("f-1"));
    }

    #[tokio::test]
    async fn a_body_that_is_no_request_is_400() {
        let fleet = fleet();
        let refused = issue(&fleet, None, &admission(&fleet), b"{}", NOW).await;
        assert_eq!(refused.unwrap_err(), ApiError::INVALID_REQUEST);
    }
}

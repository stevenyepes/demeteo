//! Which answer a git-token fetch gets, decided over what the store read and
//! nothing else, so every status is reachable from a test with no double.

use crate::api::run_state::fold;
use crate::auth::handlers::ApiError;
use crate::ports::{GithubRepository, MintError};
use crate::store::instances::InstanceKind;
use crate::store::requests::{QueuedEnvelope, StartRecord};
use axum::http::StatusCode;
use demeteo_hub_protocol::{RequestOutcome, RequestPayload, RunEvent, RunnerRepository};
use serde::Deserialize;

/// The wire tag of a `start_feature`, as `instance_requests.kind` holds it.
const START_FEATURE: &str = "start_feature";
const GITHUB: &str = "github";
/// Longer than any Feature id an instance mints, and short enough that the
/// column bound to it holds no more than an id.
pub const MAX_FEATURE_ID_CHARS: usize = 128;

/// The body of `POST /git-token`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitTokenBody {
    /// The `start_feature`'s `request_id`, as the envelope spelled it.
    pub request_id: String,
    /// The Feature the runner created for that start.
    pub feature_id: String,
}

/// Why no token is handed over, past admission. Each is one answer the
/// runner acts on: [`NoStart`](Self::NoStart) and every `503` it retries, the
/// rest it stops on. The three App refusals are `503`s with codes of their
/// own: a human can fix each while the run waits, and the run's page says
/// which ([`problem`](Self::problem)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenRefusal {
    /// The body does not parse, or its `feature_id` is no id.
    Malformed,
    /// No accepted `start_feature` with a runner part has this id under this
    /// instance, or the instance is not a runner. A start not yet answered
    /// is this too: the runner can ask before its own `accepted` lands.
    NoStart,
    /// The start is bound to another Feature.
    FeatureMismatch,
    /// The bound run's mirror says it has ended.
    RunEnded,
    /// The start's repository is not one a GitHub App can be installed on:
    /// another provider, or a name that is not `owner/repo`.
    NotGithub,
    /// No App is connected or configured for the repository's host.
    NoApp,
    /// No connected App is installed on the repository.
    NotInstalled,
    /// More than one connected App is installed on the repository.
    Ambiguous,
    /// No token can be minted now: the key service or GitHub could not mint
    /// one.
    Unavailable,
}

impl TokenRefusal {
    /// The code a mint failure is recorded under for the run's page, and
    /// answered with. `None` for a refusal of the request itself, which no
    /// human on the Hub's side can act on.
    pub fn problem(self) -> Option<&'static str> {
        match self {
            TokenRefusal::NoApp => Some("github_app_missing"),
            TokenRefusal::NotInstalled => Some("github_app_not_installed"),
            TokenRefusal::Ambiguous => Some("github_app_ambiguous"),
            TokenRefusal::Unavailable => Some("git_token_unavailable"),
            TokenRefusal::Malformed
            | TokenRefusal::NoStart
            | TokenRefusal::FeatureMismatch
            | TokenRefusal::RunEnded
            | TokenRefusal::NotGithub => None,
        }
    }
}

impl From<TokenRefusal> for ApiError {
    fn from(refusal: TokenRefusal) -> Self {
        match refusal {
            TokenRefusal::Malformed => ApiError::INVALID_REQUEST,
            TokenRefusal::NoStart => ApiError::NOT_FOUND,
            TokenRefusal::FeatureMismatch => {
                ApiError::new(StatusCode::CONFLICT, "feature_mismatch")
            }
            TokenRefusal::RunEnded => ApiError::new(StatusCode::GONE, "run_ended"),
            TokenRefusal::NotGithub => {
                ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "repository_not_github")
            }
            TokenRefusal::NoApp
            | TokenRefusal::NotInstalled
            | TokenRefusal::Ambiguous
            | TokenRefusal::Unavailable => match refusal.problem() {
                Some(code) => ApiError::new(StatusCode::SERVICE_UNAVAILABLE, code),
                None => ApiError::INTERNAL,
            },
        }
    }
}

/// The body, or [`TokenRefusal::Malformed`]. A `feature_id` is bound to the
/// request and compared on every later fetch, so it has to be an id: not
/// empty, not long, no control character (`TEXT` stores no U+0000).
pub fn body_verdict(body: &[u8]) -> Result<GitTokenBody, TokenRefusal> {
    let body: GitTokenBody = serde_json::from_slice(body).map_err(|_| TokenRefusal::Malformed)?;
    let id = &body.feature_id;
    let an_id = !id.is_empty()
        && id.chars().count() <= MAX_FEATURE_ID_CHARS
        && !id.chars().any(char::is_control);
    if !an_id {
        return Err(TokenRefusal::Malformed);
    }
    Ok(body)
}

/// The repository of the start `record` is, for a fetch naming
/// `feature_id` by an instance of `kind`. Every way it is not an accepted
/// runner start of this instance is the one [`TokenRefusal::NoStart`].
pub fn start_verdict(
    kind: Option<InstanceKind>,
    record: Option<&StartRecord>,
    feature_id: &str,
) -> Result<RunnerRepository, TokenRefusal> {
    if kind != Some(InstanceKind::Runner) {
        return Err(TokenRefusal::NoStart);
    }
    let record = record.ok_or(TokenRefusal::NoStart)?;
    if record.kind != START_FEATURE || record.outcome != Some(RequestOutcome::Accepted) {
        return Err(TokenRefusal::NoStart);
    }
    let QueuedEnvelope::Readable(envelope) = &record.envelope else {
        return Err(TokenRefusal::NoStart);
    };
    let RequestPayload::StartFeature(start) = &envelope.payload else {
        return Err(TokenRefusal::NoStart);
    };
    let runner = start.runner.as_ref().ok_or(TokenRefusal::NoStart)?;
    bound_verdict(record.git_token_feature_id.as_deref(), feature_id)?;
    Ok(runner.repository.clone())
}

/// Whether a fetch naming `feature_id` may go on once `bound` is what the
/// request holds: unbound, or bound to the same Feature.
pub fn bound_verdict(bound: Option<&str>, feature_id: &str) -> Result<(), TokenRefusal> {
    match bound {
        Some(bound) if bound != feature_id => Err(TokenRefusal::FeatureMismatch),
        _ => Ok(()),
    }
}

/// [`TokenRefusal::RunEnded`] once the Feature's mirrored events end it, as
/// the run views fold them. A Feature with no event yet is live: a runner
/// asks for its first token before it has reported anything. A terminal
/// event followed by a `feature_started` is a run that was retried, and
/// live again.
pub fn liveness_verdict(events: &[RunEvent]) -> Result<(), TokenRefusal> {
    if fold(events).status.is_terminal() {
        return Err(TokenRefusal::RunEnded);
    }
    Ok(())
}

/// How long past its budget a run may still get tokens: a run that stops at
/// its wall clock still pushes what it has, and opens its pull request.
pub const WINDOW_GRACE_SECS: u64 = 3_600;

/// [`TokenRefusal::RunEnded`] once `now` is past the start's window: from
/// its acceptance, for its own wall-clock budget or, without one,
/// `default_secs`, plus [`WINDOW_GRACE_SECS`].
///
/// This is the bound the Feature-id binding cannot give. The Hub cannot tell
/// that the Feature a runner binds is the one the start created, so a
/// runner could bind one that never reports an end; its window ends anyway.
/// The run itself is held to the same budget, so an honest run never
/// outlives it. A `record` that is not an accepted start answers
/// [`TokenRefusal::NoStart`], as [`start_verdict`] does.
pub fn window_verdict(
    record: &StartRecord,
    default_secs: u64,
    now: u64,
) -> Result<(), TokenRefusal> {
    let QueuedEnvelope::Readable(envelope) = &record.envelope else {
        return Err(TokenRefusal::NoStart);
    };
    let RequestPayload::StartFeature(start) = &envelope.payload else {
        return Err(TokenRefusal::NoStart);
    };
    let accepted_at = record.answered_at.ok_or(TokenRefusal::NoStart)?;
    let budget = start.max_wall_clock_secs.unwrap_or(default_secs);
    let closes_at = accepted_at
        .saturating_add(budget)
        .saturating_add(WINDOW_GRACE_SECS);
    if now > closes_at {
        return Err(TokenRefusal::RunEnded);
    }
    Ok(())
}

/// The GitHub repository `repository` names. `owner` and `name` become path
/// segments of a GitHub API URL, so each is held to GitHub's own alphabet
/// for them, which has no `/`, `?`, `#` or `%`.
pub fn repository_verdict(repository: &RunnerRepository) -> Result<GithubRepository, TokenRefusal> {
    if repository.provider_kind != GITHUB {
        return Err(TokenRefusal::NotGithub);
    }
    let (owner, name) = repository
        .full_name
        .split_once('/')
        .ok_or(TokenRefusal::NotGithub)?;
    if !is_github_name(owner) || !is_github_name(name) {
        return Err(TokenRefusal::NotGithub);
    }
    Ok(GithubRepository {
        host: repository.host.clone(),
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

fn is_github_name(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

/// What a failed mint answers. Every one is retried: the three about Apps
/// wait for a human to connect, install or remove one, the rest for a
/// service.
pub fn mint_verdict(error: &MintError) -> TokenRefusal {
    match error {
        MintError::UnknownHost(_) => TokenRefusal::NoApp,
        MintError::NotInstalled => TokenRefusal::NotInstalled,
        MintError::Ambiguous(_) => TokenRefusal::Ambiguous,
        MintError::Hosts(_)
        | MintError::Signing(_)
        | MintError::Transport(_)
        | MintError::Refused { .. }
        | MintError::Malformed(_) => TokenRefusal::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::KeyServiceError;
    use demeteo_hub_protocol::{
        CancelFeature, RefusalReason, RunEventKind, RunEventPayload, RunnerStart, RunnerWorkflow,
        SignedRequest, StartFeature, PROTOCOL_VERSION,
    };

    fn repository(provider_kind: &str, full_name: &str) -> RunnerRepository {
        RunnerRepository {
            provider_kind: provider_kind.to_owned(),
            host: "github.com".to_owned(),
            full_name: full_name.to_owned(),
        }
    }

    fn start(runner: bool) -> RequestPayload {
        RequestPayload::StartFeature(StartFeature {
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
            runner: runner.then(|| {
                Box::new(RunnerStart {
                    source_instance_id: "src".to_owned(),
                    repository: repository("github", "acme/widgets"),
                    workflow: RunnerWorkflow {
                        workflow_id: "w-1".to_owned(),
                        version: 1,
                        name: "Standard".to_owned(),
                        steps_json: "[]".to_owned(),
                    },
                    settings: None,
                })
            }),
        })
    }

    fn record(payload: RequestPayload) -> StartRecord {
        let kind = match &payload {
            RequestPayload::StartFeature(_) => "start_feature",
            _ => "cancel_feature",
        };
        StartRecord {
            kind: kind.to_owned(),
            outcome: Some(RequestOutcome::Accepted),
            envelope: QueuedEnvelope::Readable(Box::new(SignedRequest {
                v: PROTOCOL_VERSION,
                request_id: "AAECAwQFBgcICQoLDA0ODw".to_owned(),
                instance_id: "i".to_owned(),
                issued_at: 1,
                expires_at: 2,
                payload,
                signature: "s".to_owned(),
            })),
            git_token_feature_id: None,
            answered_at: Some(ACCEPTED_AT),
        }
    }

    const ACCEPTED_AT: u64 = 1_700_000_000;
    const DEFAULT_WINDOW: u64 = 86_400;

    fn with_wall_clock(secs: Option<u64>) -> StartRecord {
        let mut payload = start(true);
        if let RequestPayload::StartFeature(start) = &mut payload {
            start.max_wall_clock_secs = secs;
        }
        record(payload)
    }

    #[test]
    fn a_start_with_a_wall_clock_gets_tokens_for_it_and_the_grace_and_no_longer() {
        let start = with_wall_clock(Some(7_200));
        let closes_at = ACCEPTED_AT + 7_200 + WINDOW_GRACE_SECS;

        assert_eq!(window_verdict(&start, DEFAULT_WINDOW, ACCEPTED_AT), Ok(()));
        assert_eq!(window_verdict(&start, DEFAULT_WINDOW, closes_at), Ok(()));
        assert_eq!(
            window_verdict(&start, DEFAULT_WINDOW, closes_at + 1),
            Err(TokenRefusal::RunEnded)
        );
    }

    #[test]
    fn a_start_without_a_wall_clock_gets_the_default_window() {
        let start = with_wall_clock(None);
        let closes_at = ACCEPTED_AT + DEFAULT_WINDOW + WINDOW_GRACE_SECS;

        assert_eq!(window_verdict(&start, DEFAULT_WINDOW, closes_at), Ok(()));
        assert_eq!(
            window_verdict(&start, DEFAULT_WINDOW, closes_at + 1),
            Err(TokenRefusal::RunEnded)
        );
    }

    #[test]
    fn a_window_that_would_overflow_saturates_instead_of_closing() {
        let start = with_wall_clock(Some(u64::MAX));
        assert_eq!(window_verdict(&start, DEFAULT_WINDOW, u64::MAX), Ok(()));
    }

    #[test]
    fn a_start_never_answered_has_no_window() {
        let unanswered = StartRecord {
            answered_at: None,
            ..record(start(true))
        };
        assert_eq!(
            window_verdict(&unanswered, DEFAULT_WINDOW, ACCEPTED_AT),
            Err(TokenRefusal::NoStart)
        );
    }

    const RUNNER: Option<InstanceKind> = Some(InstanceKind::Runner);

    #[test]
    fn an_accepted_runner_start_answers_its_signed_repository() {
        assert_eq!(
            start_verdict(RUNNER, Some(&record(start(true))), "f-1"),
            Ok(repository("github", "acme/widgets"))
        );
    }

    #[test]
    fn everything_short_of_an_accepted_runner_start_of_a_runner_is_no_start() {
        let accepted = record(start(true));
        let unanswered = StartRecord {
            outcome: None,
            ..accepted.clone()
        };
        let refused = StartRecord {
            outcome: Some(RequestOutcome::Refused {
                reason: RefusalReason::Expired,
            }),
            ..accepted.clone()
        };
        let unreadable = StartRecord {
            envelope: QueuedEnvelope::Unreadable,
            ..accepted.clone()
        };
        let other_kind = StartRecord {
            kind: "cancel_feature".to_owned(),
            ..accepted.clone()
        };
        let cancel = record(RequestPayload::CancelFeature(CancelFeature {
            feature_id: "f-1".to_owned(),
        }));
        let desktop_start = record(start(false));
        let cases: [(&str, Option<InstanceKind>, Option<&StartRecord>); 9] = [
            ("a desktop", Some(InstanceKind::Desktop), Some(&accepted)),
            ("no instance kind", None, Some(&accepted)),
            ("no request", RUNNER, None),
            ("not answered", RUNNER, Some(&unanswered)),
            ("refused", RUNNER, Some(&refused)),
            ("unreadable", RUNNER, Some(&unreadable)),
            ("another kind column", RUNNER, Some(&other_kind)),
            ("another payload", RUNNER, Some(&cancel)),
            ("no runner part", RUNNER, Some(&desktop_start)),
        ];
        for (name, kind, record) in cases {
            assert_eq!(
                start_verdict(kind, record, "f-1"),
                Err(TokenRefusal::NoStart),
                "{name}"
            );
        }
    }

    #[test]
    fn a_start_bound_to_another_feature_is_a_mismatch_and_to_this_one_is_not() {
        let bound = StartRecord {
            git_token_feature_id: Some("f-1".to_owned()),
            ..record(start(true))
        };

        assert!(start_verdict(RUNNER, Some(&bound), "f-1").is_ok());
        assert_eq!(
            start_verdict(RUNNER, Some(&bound), "f-2"),
            Err(TokenRefusal::FeatureMismatch)
        );
        assert_eq!(bound_verdict(None, "f-2"), Ok(()));
    }

    fn event(offset: u64, kind: RunEventKind) -> RunEvent {
        RunEvent {
            v: 1,
            feature_id: "f-1".to_owned(),
            offset,
            kind,
            occurred_at: 1_700_000_000 + offset,
            payload: RunEventPayload::default(),
        }
    }

    #[test]
    fn a_run_is_live_until_its_latest_feature_event_ends_it() {
        use RunEventKind::*;
        assert_eq!(liveness_verdict(&[]), Ok(()));
        assert_eq!(
            liveness_verdict(&[event(1, FeatureStarted), event(2, StepStarted)]),
            Ok(())
        );
        for end in [FeatureCompleted, FeatureFailed, FeatureCancelled] {
            assert_eq!(
                liveness_verdict(&[event(1, FeatureStarted), event(2, end)]),
                Err(TokenRefusal::RunEnded),
                "{end:?}"
            );
            assert_eq!(
                liveness_verdict(&[event(1, end), event(2, FeatureStarted)]),
                Ok(()),
                "{end:?} then a retry"
            );
        }
    }

    #[test]
    fn only_a_github_owner_and_repo_is_a_repository_to_mint_for() {
        assert_eq!(
            repository_verdict(&repository("github", "acme/wid.gets_2-x")),
            Ok(GithubRepository {
                host: "github.com".to_owned(),
                owner: "acme".to_owned(),
                name: "wid.gets_2-x".to_owned(),
            })
        );
        for (kind, full_name) in [
            ("gitlab", "acme/widgets"),
            ("GitHub", "acme/widgets"),
            ("", "acme/widgets"),
            ("github", "acme"),
            ("github", "acme/sub/widgets"),
            ("github", "/widgets"),
            ("github", "acme/"),
            ("github", "acme/.."),
            ("github", "acme/wid?gets"),
            ("github", "acme/wid#gets"),
            ("github", "acme/wid%2Fgets"),
            ("github", "ac me/widgets"),
        ] {
            assert_eq!(
                repository_verdict(&repository(kind, full_name)),
                Err(TokenRefusal::NotGithub),
                "{kind} {full_name}"
            );
        }
    }

    #[test]
    fn a_body_names_a_request_and_an_id_and_nothing_else() {
        assert_eq!(
            body_verdict(br#"{"request_id":"r","feature_id":"f-1"}"#),
            Ok(GitTokenBody {
                request_id: "r".to_owned(),
                feature_id: "f-1".to_owned(),
            })
        );
        let long = "f".repeat(MAX_FEATURE_ID_CHARS + 1);
        let longest = "f".repeat(MAX_FEATURE_ID_CHARS);
        assert!(body_verdict(
            format!(r#"{{"request_id":"r","feature_id":"{longest}"}}"#).as_bytes()
        )
        .is_ok());
        for body in [
            "".to_owned(),
            "{}".to_owned(),
            r#"{"request_id":"r"}"#.to_owned(),
            r#"{"request_id":"r","feature_id":"f","repository":"x/y"}"#.to_owned(),
            r#"{"request_id":"r","feature_id":""}"#.to_owned(),
            r#"{"request_id":"r","feature_id":"f\u0000"}"#.to_owned(),
            r#"{"request_id":"r","feature_id":"f\n"}"#.to_owned(),
            format!(r#"{{"request_id":"r","feature_id":"{long}"}}"#),
        ] {
            assert_eq!(
                body_verdict(body.as_bytes()),
                Err(TokenRefusal::Malformed),
                "{body}"
            );
        }
    }

    #[test]
    fn each_app_problem_has_its_own_refusal_and_the_rest_are_unavailable() {
        assert_eq!(
            mint_verdict(&MintError::UnknownHost("ghe.example".to_owned())),
            TokenRefusal::NoApp
        );
        assert_eq!(
            mint_verdict(&MintError::NotInstalled),
            TokenRefusal::NotInstalled
        );
        assert_eq!(
            mint_verdict(&MintError::Ambiguous(2)),
            TokenRefusal::Ambiguous
        );
        for error in [
            MintError::Hosts("down".to_owned()),
            MintError::Signing(KeyServiceError::Sealed),
            MintError::Transport("down".to_owned()),
            MintError::Refused {
                step: "token request",
                status: 403,
            },
            MintError::Malformed("token request"),
        ] {
            assert_eq!(mint_verdict(&error), TokenRefusal::Unavailable, "{error}");
        }
    }

    #[test]
    fn each_refusal_is_the_status_the_runner_acts_on() {
        let status = |refusal| ApiError::from(refusal);
        assert_eq!(status(TokenRefusal::Malformed), ApiError::INVALID_REQUEST);
        assert_eq!(status(TokenRefusal::NoStart), ApiError::NOT_FOUND);
        for (refusal, code) in [
            (TokenRefusal::FeatureMismatch, StatusCode::CONFLICT),
            (TokenRefusal::RunEnded, StatusCode::GONE),
            (TokenRefusal::NotGithub, StatusCode::UNPROCESSABLE_ENTITY),
            (TokenRefusal::NoApp, StatusCode::SERVICE_UNAVAILABLE),
            (TokenRefusal::NotInstalled, StatusCode::SERVICE_UNAVAILABLE),
            (TokenRefusal::Ambiguous, StatusCode::SERVICE_UNAVAILABLE),
            (TokenRefusal::Unavailable, StatusCode::SERVICE_UNAVAILABLE),
        ] {
            let response = axum::response::IntoResponse::into_response(status(refusal));
            assert_eq!(response.status(), code, "{refusal:?}");
        }
        assert_eq!(
            status(TokenRefusal::NotInstalled),
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "github_app_not_installed")
        );
    }
}

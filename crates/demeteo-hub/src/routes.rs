use crate::api;
use crate::auth::ceremonies::Ceremonies;
use crate::auth::handlers::{self, cookie_text, header_text, ApiError};
use crate::auth::session::{self, Rejection};
use crate::auth::webauthn::{RelyingPartyError, Verifier};
use crate::auth::RelyingParty;
use crate::pairing;
use crate::pairing::pending::Pairings;
use crate::pairing::signing_key::SigningKey;
use crate::ports::KeyService;
use crate::socket;
use crate::socket::connections::Connections;
use crate::store::Db;
use axum::extract::{Request, State};
use axum::http::header::ORIGIN;
use axum::http::{Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use std::path::Path;
use std::sync::Arc;
use tower_http::services::ServeDir;

struct Shared {
    rp: RelyingParty,
    verifier: Verifier,
    ceremonies: Ceremonies,
    pairings: Pairings,
    connections: Connections,
    keys: Arc<dyn KeyService>,
    signing_key: SigningKey,
}

/// axum clones the state into every request, so everything but the pool is
/// shared, not copied: a [`Ceremonies`] per clone would hold a ceremony's
/// start where its finish never looks, a [`Pairings`] per clone a consent
/// where its approval never looks, a [`Connections`] per clone a session
/// where no wake-up and no shutdown ever looks, and a [`SigningKey`] per
/// clone a key that is asked for again by every request.
#[derive(Clone)]
pub struct AppState {
    db: Db,
    shared: Arc<Shared>,
}

impl AppState {
    /// `signing_key` is the name `keys` knows the request-signing key by.
    /// Nothing here asks `keys` for it, or for anything else.
    pub fn new(
        db: Db,
        rp: RelyingParty,
        keys: Arc<dyn KeyService>,
        signing_key: String,
    ) -> Result<Self, RelyingPartyError> {
        let verifier = Verifier::new(&rp)?;
        Ok(Self {
            db,
            shared: Arc::new(Shared {
                rp,
                verifier,
                ceremonies: Ceremonies::default(),
                pairings: Pairings::default(),
                connections: Connections::default(),
                keys,
                signing_key: SigningKey::new(signing_key),
            }),
        })
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn rp(&self) -> &RelyingParty {
        &self.shared.rp
    }

    pub fn verifier(&self) -> &Verifier {
        &self.shared.verifier
    }

    pub fn ceremonies(&self) -> &Ceremonies {
        &self.shared.ceremonies
    }

    pub fn pairings(&self) -> &Pairings {
        &self.shared.pairings
    }

    pub fn connections(&self) -> &Connections {
        &self.shared.connections
    }

    pub fn keys(&self) -> &dyn KeyService {
        self.shared.keys.as_ref()
    }

    pub fn signing_key(&self) -> &SigningKey {
        &self.shared.signing_key
    }
}

/// `/healthz` is liveness only: it must stay green while OpenBao is sealed or
/// Postgres is away, or the orchestrator restarts a Hub that is merely waiting
/// for a manual unseal. Its handler therefore extracts no part of `state`,
/// though `/token` and the socket beside it and the `/v1` routes all hold it,
/// and building the router dials nothing. Nor does it wait on a socket: a
/// session runs in a task of its own, and the registry of them is asked
/// nothing here.
///
/// A missing `web_dir` degrades to 404s at request time, never a startup failure.
pub fn router(state: AppState, web_dir: &Path) -> Router {
    let routes = Router::new()
        .route("/healthz", get(healthz))
        .route("/authorize", get(pairing::handlers::authorize))
        // Outside `v1`, and so outside `guard_mutations`: a desktop is not a
        // browser and sends no `Origin`, so in there every token request
        // would be answered `403 origin_mismatch`. A request is held to the
        // DPoP proof it carries instead.
        .route(
            pairing::handlers::TOKEN_PATH,
            post(pairing::handlers::token),
        )
        // Outside `v1` as `/token` is: the desktop that opens it sends no
        // `Origin`, and an upgrade is held to the access token it carries
        // and the proof beside it. `guard_mutations` lets every `GET`
        // through, so in there this would be guarded by nothing and read as
        // if it were.
        .route(socket::handler::SOCKET_PATH, get(socket::handler::socket))
        .with_state(state.clone())
        .merge(v1(state));
    router_with(routes, web_dir)
}

/// The guard is a `route_layer`, so it runs for a path only once a route
/// matched it. As a `layer` it would also answer `403` for a `/v1` path that
/// nothing serves, and a `403` would stop being evidence that a route exists
/// and is guarded.
fn v1(state: AppState) -> Router {
    Router::new()
        .route("/v1/register/start", post(handlers::start_registration))
        .route("/v1/register/finish", post(handlers::finish_registration))
        .route("/v1/session/start", post(handlers::start_sign_in))
        .route("/v1/session/finish", post(handlers::finish_sign_in))
        .route("/v1/session", get(handlers::read_session))
        .route("/v1/session/end", post(handlers::end_session))
        .route("/v1/passkeys", get(handlers::list_passkeys))
        .route("/v1/passkeys/add/start", post(handlers::start_addition))
        .route("/v1/passkeys/add/finish", post(handlers::finish_addition))
        .route(
            "/v1/passkeys/endorse/start",
            post(handlers::start_endorsement),
        )
        .route(
            "/v1/passkeys/endorse/finish",
            post(handlers::finish_endorsement),
        )
        .route(
            "/v1/passkeys/revoke/start",
            post(handlers::start_revocation),
        )
        .route(
            "/v1/passkeys/revoke/finish",
            post(handlers::finish_revocation),
        )
        .route("/v1/pairing/start", post(pairing::handlers::start_pairing))
        .route(
            "/v1/pairing/approve",
            post(pairing::handlers::approve_pairing),
        )
        .route("/v1/pairing/deny", post(pairing::handlers::deny_pairing))
        .route("/v1/fleet", get(api::fleet::get_fleet))
        .route("/v1/instances/{instance_id}", get(api::fleet::get_instance))
        .route("/v1/runs", get(api::runs::get_runs))
        .route(
            "/v1/instances/{instance_id}/runs/{feature_id}",
            get(api::runs::get_run),
        )
        .route(
            "/v1/instances/{instance_id}/runs",
            post(api::dispatch::post_dispatch),
        )
        .route(
            "/v1/instances/{instance_id}/runs/{feature_id}/cancel",
            post(api::dispatch::post_cancel),
        )
        .route(
            "/v1/instances/{instance_id}/runs/{feature_id}/steps/{step_id}/assignment",
            post(api::dispatch::post_assignment),
        )
        .route(
            "/v1/instances/{instance_id}/gates/challenge",
            post(api::gates::post_gate_challenge),
        )
        .route(
            "/v1/instances/{instance_id}/gates/decision",
            post(api::gates::post_gate_decision),
        )
        .route(
            "/v1/instances/{instance_id}/requests/{request_id}",
            get(api::request_status::get_request),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            guard_mutations,
        ))
        .with_state(state)
}

/// The `/v1` routes that change state before there is a session, whose only
/// CSRF defence is therefore the `Origin` check. A route is named here or it
/// needs the token: one added to `v1` and forgotten here is refused, not
/// left open.
///
/// Public for `src-tauri/tests/hub_auth.rs` alone, which holds this list to
/// one written out by hand there: an exemption added here fails that test
/// until someone edits the other list on purpose.
pub const PRE_SESSION_ROUTES: &[&str] = &[
    "/v1/register/start",
    "/v1/register/finish",
    "/v1/session/start",
    "/v1/session/finish",
];

fn needs_session(path: &str) -> bool {
    !PRE_SESSION_ROUTES.contains(&path)
}

/// `expected_origin` is the one in [`AppState`]. Nothing here reads `Host`,
/// which the client chooses.
fn mutation_verdict(expected_origin: &str, request: &Request) -> Result<(), Rejection> {
    if request.method() == Method::GET {
        return Ok(());
    }
    let headers = request.headers();
    let cookie = cookie_text(headers);
    session::guard_mutation(
        expected_origin,
        header_text(headers, ORIGIN),
        cookie.as_deref(),
        header_text(headers, session::CSRF_HEADER),
        needs_session(request.uri().path()),
    )
}

async fn guard_mutations(State(state): State<AppState>, request: Request, next: Next) -> Response {
    match mutation_verdict(&state.rp().origin, &request) {
        Ok(()) => next.run(request).await,
        Err(rejection) => ApiError::from(rejection).into_response(),
    }
}

/// Takes routes whose state is already applied. The dotfile guard wraps the
/// static fallback only: applied to the whole router it would also 404 every
/// `/.well-known/*` route (WebAuthn related origins, passkey endpoints), and
/// those are served by routes, never by files.
fn router_with(routes: Router, web_dir: &Path) -> Router {
    routes.fallback_service(static_files(web_dir))
}

/// `ServeDir` serves dotfiles; a web dir mounted from a checkout would expose
/// `.git` and `.env`.
fn static_files(web_dir: &Path) -> Router {
    Router::new()
        .fallback_service(ServeDir::new(web_dir))
        .layer(middleware::from_fn(reject_hidden_paths))
}

async fn healthz() -> StatusCode {
    StatusCode::OK
}

async fn reject_hidden_paths(request: Request, next: Next) -> Response {
    if is_hidden(request.uri().path()) {
        let mut response = Response::new(axum::body::Body::empty());
        *response.status_mut() = StatusCode::NOT_FOUND;
        return response;
    }
    next.run(request).await
}

/// A percent-encoded dot is refused rather than decoded.
fn is_hidden(path: &str) -> bool {
    path.split('/').any(|segment| segment.starts_with('.'))
        || path.to_ascii_lowercase().contains("%2e")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::testing::{lazy_state, serve};
    use axum::http::header::COOKIE;
    use axum::http::HeaderValue;
    use std::net::Ipv4Addr;
    use std::path::PathBuf;

    fn fixture_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("web")
    }

    fn state() -> AppState {
        let rp = RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: "https://hub.example".to_owned(),
        };
        lazy_state("postgres://hub@db.invalid/hub", rp).unwrap()
    }

    async fn serve_router(web_dir: &Path) -> String {
        serve_app(router(state(), web_dir)).await
    }

    async fn serve_app(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        serve(listener, app);
        format!("http://{addr}")
    }

    async fn status(base: &str, path: &str) -> u16 {
        reqwest::get(format!("{base}{path}"))
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    #[tokio::test]
    async fn healthz_is_200_with_no_backing_service() {
        let base = serve_router(&fixture_dir()).await;
        assert_eq!(status(&base, "/healthz").await, 200);
    }

    #[tokio::test]
    async fn root_serves_the_fixture_index() {
        let base = serve_router(&fixture_dir()).await;
        let body = reqwest::get(format!("{base}/"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(body.contains("demeteo hub fixture"), "{body}");
    }

    #[tokio::test]
    async fn dotfiles_and_traversal_are_not_served() {
        let base = serve_router(&fixture_dir()).await;
        for path in [
            "/.env",
            "/.git/config",
            "/%2e%2e/Cargo.toml",
            "/../Cargo.toml",
        ] {
            assert_eq!(status(&base, path).await, 404, "{path}");
        }
    }

    #[tokio::test]
    async fn a_well_known_route_is_not_caught_by_the_dotfile_guard() {
        let routes = Router::new().route("/.well-known/probe", get(|| async { StatusCode::OK }));
        let base = serve_app(router_with(routes, &fixture_dir())).await;
        assert_eq!(status(&base, "/.well-known/probe").await, 200);
        for path in [
            "/.env",
            "/.git/config",
            "/.well-known/not-a-route",
            "/%2e%2e/Cargo.toml",
        ] {
            assert_eq!(status(&base, path).await, 404, "{path}");
        }
    }

    const ORIGIN_OF_STATE: &str = "https://hub.example";

    fn mutation(method: Method, headers: &[(&str, &str)]) -> Request {
        let mut request = Request::builder().method(method).uri("/v1/session/end");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.body(axum::body::Body::empty()).unwrap()
    }

    #[test]
    fn a_route_needs_the_csrf_token_unless_it_is_listed_as_pre_session() {
        assert!(needs_session("/v1/session/end"));
        assert!(needs_session("/v1/a-route-nobody-listed"));
        for path in PRE_SESSION_ROUTES {
            assert!(!needs_session(path), "{path}");
        }
    }

    #[test]
    fn a_get_is_not_a_mutation_and_every_other_method_is() {
        assert_eq!(
            mutation_verdict(ORIGIN_OF_STATE, &mutation(Method::GET, &[])),
            Ok(())
        );
        for method in [
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::HEAD,
            Method::OPTIONS,
        ] {
            assert_eq!(
                mutation_verdict(ORIGIN_OF_STATE, &mutation(method.clone(), &[])),
                Err(Rejection::OriginMismatch),
                "{method}"
            );
        }
    }

    #[test]
    fn the_guard_reads_the_three_headers_and_never_the_host() {
        let secret = session::SessionSecret::generate().unwrap();
        let cookie = format!("{}={}", session::COOKIE_NAME, secret.cookie_value());
        let token = secret.csrf_token();
        let verdict = |headers: &[(&str, &str)]| {
            mutation_verdict(ORIGIN_OF_STATE, &mutation(Method::POST, headers))
        };

        assert_eq!(
            verdict(&[
                ("host", "evil.example"),
                ("origin", ORIGIN_OF_STATE),
                ("cookie", &cookie),
                ("x-csrf-token", &token),
            ]),
            Ok(())
        );
        assert_eq!(
            verdict(&[
                ("host", "evil.example"),
                ("origin", "https://evil.example"),
                ("cookie", &cookie),
                ("x-csrf-token", &token),
            ]),
            Err(Rejection::OriginMismatch)
        );
        assert_eq!(
            verdict(&[("origin", ORIGIN_OF_STATE), ("cookie", &cookie)]),
            Err(Rejection::CsrfMissing)
        );
        assert_eq!(
            verdict(&[
                ("origin", ORIGIN_OF_STATE),
                ("cookie", &cookie),
                ("x-csrf-token", &secret.cookie_value()),
            ]),
            Err(Rejection::CsrfMismatch)
        );
    }

    #[test]
    fn the_guard_finds_the_session_beside_a_cookie_that_is_not_ascii() {
        let secret = session::SessionSecret::generate().unwrap();
        let another = session::SessionSecret::generate().unwrap();
        let cookie = format!(
            "theme=café; {}={}",
            session::COOKIE_NAME,
            secret.cookie_value()
        );
        let cookie = HeaderValue::from_bytes(cookie.as_bytes()).unwrap();
        let verdict = |token: String| {
            let request = Request::builder()
                .method(Method::POST)
                .uri("/v1/session/end")
                .header(ORIGIN, ORIGIN_OF_STATE)
                .header(COOKIE, &cookie)
                .header(session::CSRF_HEADER, token)
                .body(axum::body::Body::empty())
                .unwrap();
            mutation_verdict(ORIGIN_OF_STATE, &request)
        };

        assert_eq!(verdict(secret.csrf_token()), Ok(()));
        assert_eq!(verdict(another.csrf_token()), Err(Rejection::CsrfMismatch));
    }

    #[test]
    fn the_static_guard_treats_well_known_as_hidden() {
        assert!(is_hidden("/.well-known/webauthn"));
        assert!(!is_hidden("/index.html"));
    }

    #[tokio::test]
    async fn a_missing_web_dir_is_404_not_a_panic() {
        let base = serve_router(&fixture_dir().join("absent")).await;
        assert_eq!(status(&base, "/").await, 404);
        assert_eq!(status(&base, "/healthz").await, 200);
    }
}

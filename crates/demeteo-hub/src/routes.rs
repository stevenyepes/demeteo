use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use std::path::Path;
use tower_http::services::ServeDir;

/// `/healthz` is liveness only: it must stay green while OpenBao is sealed or
/// Postgres is away, or the orchestrator restarts a Hub that is merely waiting
/// for a manual unseal.
///
/// A missing `web_dir` degrades to 404s at request time, never a startup failure.
pub fn router(web_dir: &Path) -> Router {
    router_with(Router::new().route("/healthz", get(healthz)), web_dir)
}

/// The dotfile guard wraps the static fallback only. Applied to the whole
/// router it would also 404 every `/.well-known/*` route (WebAuthn related
/// origins, passkey endpoints): those are served by routes, never by files.
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
    use std::net::Ipv4Addr;
    use std::path::PathBuf;

    fn fixture_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("web")
    }

    async fn serve(web_dir: &Path) -> String {
        serve_app(router(web_dir)).await
    }

    async fn serve_app(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
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
        let base = serve(&fixture_dir()).await;
        assert_eq!(status(&base, "/healthz").await, 200);
    }

    #[tokio::test]
    async fn root_serves_the_fixture_index() {
        let base = serve(&fixture_dir()).await;
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
        let base = serve(&fixture_dir()).await;
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

    #[test]
    fn the_static_guard_treats_well_known_as_hidden() {
        assert!(is_hidden("/.well-known/webauthn"));
        assert!(!is_hidden("/index.html"));
    }

    #[tokio::test]
    async fn a_missing_web_dir_is_404_not_a_panic() {
        let base = serve(&fixture_dir().join("absent")).await;
        assert_eq!(status(&base, "/").await, 404);
        assert_eq!(status(&base, "/healthz").await, 200);
    }
}

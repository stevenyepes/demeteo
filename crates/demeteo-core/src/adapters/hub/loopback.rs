//! The one-shot listener the Hub's `/authorize` redirect lands on.
//!
//! Every request is judged by [`callback_verdict`]; this module only serves
//! it. A refused request is answered and the wait goes on, so a stray or
//! forged request on the port can neither complete nor abort a pairing.
//!
//! The request line carries the authorization `code`: there is no
//! `TraceLayer` on this router and nothing here logs a URI. The listener is
//! owned by the pairing that bound it — a `static` would make concurrent
//! tests in one binary race, as the MCP `LISTENER` once did.

use std::net::SocketAddr;
use std::time::Duration;

use axum::http::{Method, StatusCode, Uri};
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use tokio::sync::{mpsc, oneshot};

use crate::domain::hub_pairing::callback::{callback_verdict, CallbackVerdict};
use crate::domain::hub_pairing::{redirect_uri, HubPairingError};

/// How long the accepted request gets to receive its page before the server
/// is dropped. Graceful shutdown alone would wait on a connection a local
/// process holds open indefinitely.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

pub struct CallbackListener {
    listener: std::net::TcpListener,
    addr: SocketAddr,
    path: String,
}

type Outcome = Result<String, HubPairingError>;

#[derive(Clone)]
struct Expected {
    path: String,
    state: String,
    outcome: mpsc::Sender<Outcome>,
}

fn listener_error(what: &str, e: impl std::fmt::Display) -> HubPairingError {
    HubPairingError::Transport(format!("local callback listener: {what}: {e}"))
}

impl CallbackListener {
    /// Binds `127.0.0.1:0` and returns the listener with the `redirect_uri`
    /// to send to `/authorize`. Port 0 because the Hub accepts any loopback
    /// port and a fixed one could already be taken; never `localhost`, which
    /// may resolve to `::1`, a host the Hub's redirect rule does not name.
    pub fn bind() -> Result<(Self, String), HubPairingError> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
            .map_err(|e| listener_error("could not bind", e))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| listener_error("could not set non-blocking", e))?;
        let addr = listener
            .local_addr()
            .map_err(|e| listener_error("no local address", e))?;
        let path = random_path()?;
        let uri = redirect_uri(addr.port(), &path).ok_or_else(|| {
            HubPairingError::Transport("local callback listener: unusable redirect URI".into())
        })?;
        Ok((
            Self {
                listener,
                addr,
                path,
            },
            uri,
        ))
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Serves until a request with `expected_state` brings a `code` or
    /// `access_denied`, or until `timeout` elapses, then shuts down.
    ///
    /// The tokio listener is made here rather than in [`Self::bind`]:
    /// `from_std` needs a runtime, and an `async fn` is only ever polled
    /// inside one. The socket already queues connections from `bind` on.
    pub async fn wait(self, expected_state: &str, timeout: Duration) -> Outcome {
        let listener = tokio::net::TcpListener::from_std(self.listener)
            .map_err(|e| listener_error("could not register with the runtime", e))?;
        let (outcome, mut outcome_rx) = mpsc::channel(1);
        let app = Router::new().fallback(callback).with_state(Expected {
            path: self.path,
            state: expected_state.to_owned(),
            outcome,
        });
        let (shutdown, shutdown_rx) = oneshot::channel::<()>();
        let mut server = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });

        let result = match tokio::time::timeout(timeout, outcome_rx.recv()).await {
            Ok(Some(outcome)) => outcome,
            Ok(None) => Err(HubPairingError::Transport(
                "local callback listener stopped unexpectedly".into(),
            )),
            Err(_) => Err(HubPairingError::CallbackTimeout),
        };

        let _ = shutdown.send(());
        if tokio::time::timeout(SHUTDOWN_GRACE, &mut server)
            .await
            .is_err()
        {
            server.abort();
        }
        result
    }
}

async fn callback(
    axum::extract::State(expected): axum::extract::State<Expected>,
    method: Method,
    uri: Uri,
) -> (StatusCode, &'static str) {
    if method != Method::GET {
        return (StatusCode::METHOD_NOT_ALLOWED, "");
    }
    let (outcome, page) =
        match callback_verdict(uri.path(), uri.query(), &expected.path, &expected.state) {
            CallbackVerdict::Code(code) => (
                Ok(code),
                "Approved on the Hub. You can close this tab and return to Demeteo.",
            ),
            CallbackVerdict::Denied => (
                Err(HubPairingError::Denied),
                "Pairing was denied on the Hub. You can close this tab.",
            ),
            CallbackVerdict::Refuse(reason) => {
                let status =
                    StatusCode::from_u16(reason.http_status()).unwrap_or(StatusCode::BAD_REQUEST);
                return (status, "");
            }
        };
    // Capacity one: the first verdict that ends the wait is the one returned.
    match expected.outcome.try_send(outcome) {
        Ok(()) => (StatusCode::OK, page),
        Err(_) => (StatusCode::BAD_REQUEST, ""),
    }
}

/// Unreserved bytes only, which [`redirect_uri`] requires. Random so a
/// process that learns the port still has to guess where to send a callback.
fn random_path() -> Result<String, HubPairingError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| listener_error("no OS randomness for the path", e))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

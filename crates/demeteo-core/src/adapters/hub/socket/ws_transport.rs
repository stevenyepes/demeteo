//! [`HubSocketDialer`] over `tokio-tungstenite`: the only file that knows the
//! WebSocket crate, so swapping it touches nothing the session reads.
//!
//! TLS is the same as `/token`'s: aws-lc-rs, which reqwest already links, and
//! the platform verifier, so a Hub behind a private CA the OS trusts is
//! trusted on both paths. The provider is named on the `ClientConfig` rather
//! than taken from rustls's process default, which a second provider anywhere
//! in the build would make ambiguous and panic on first use.
//!
//! As in `token_client.rs`, no error carries the access token, the proof, or
//! the library's own text, which can quote the request.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use rustls::crypto::aws_lc_rs;
use rustls::ClientConfig;
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{
    connect_async_tls_with_config, Connector, MaybeTlsStream, WebSocketStream,
};

use crate::domain::hub_pairing::{hub_error_code, HubPairingError};
use crate::ports::hub::{
    DialFailure, HubSocketConnection, HubSocketDialer, SocketGone, SocketReceived, SocketUpgrade,
};

/// DNS, TCP, TLS and the upgrade together; the same budget as a `/token`
/// request.
const DIAL_TIMEOUT: Duration = Duration::from_secs(30);
/// The close is a courtesy to the Hub, which drops a silent socket anyway;
/// a stop must not wait on it.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

type Stream = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct WsSocketDialer {
    tls: Arc<ClientConfig>,
}

impl WsSocketDialer {
    pub fn new() -> Result<Self, HubPairingError> {
        let refused = |_| HubPairingError::Transport("could not build the socket's TLS".into());
        let tls = ClientConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(refused)?
            .with_platform_verifier()
            .map_err(refused)?
            .with_no_client_auth();
        Ok(Self { tls: Arc::new(tls) })
    }
}

fn unreachable(what: &str) -> DialFailure {
    DialFailure::Unreachable(what.to_owned())
}

/// Everything short of an HTTP answer is `Unreachable`; a `101` whose
/// handshake is wrong is too, since nothing the Hub said can be acted on.
fn dial_failure(e: WsError) -> DialFailure {
    match e {
        WsError::Http(response) => DialFailure::Refused {
            status: response.status().as_u16(),
            error: response.body().as_deref().and_then(hub_error_code),
        },
        WsError::Io(_) => unreachable("could not connect to the Hub socket"),
        WsError::Tls(_) => unreachable("the Hub socket's TLS handshake failed"),
        WsError::Url(_) => unreachable("the Hub socket URL is not a WebSocket URL"),
        _ => unreachable("the Hub socket handshake failed"),
    }
}

#[async_trait]
impl HubSocketDialer for WsSocketDialer {
    async fn dial(
        &self,
        upgrade: SocketUpgrade,
    ) -> Result<Box<dyn HubSocketConnection>, DialFailure> {
        let mut request = upgrade
            .url
            .as_str()
            .into_client_request()
            .map_err(dial_failure)?;
        let invalid = |_| unreachable("the socket credentials are not a valid header");
        let mut authorization =
            HeaderValue::from_str(&format!("DPoP {}", upgrade.access_token.expose()))
                .map_err(invalid)?;
        authorization.set_sensitive(true);
        let mut dpop = HeaderValue::from_str(&upgrade.dpop).map_err(invalid)?;
        dpop.set_sensitive(true);
        let headers = request.headers_mut();
        headers.insert(AUTHORIZATION, authorization);
        headers.insert("DPoP", dpop);

        let connector = Some(Connector::Rustls(Arc::clone(&self.tls)));
        let dialed = timeout(
            DIAL_TIMEOUT,
            connect_async_tls_with_config(request, None, true, connector),
        )
        .await
        .map_err(|_| unreachable("the Hub socket did not answer in time"))?;
        let (stream, _) = dialed.map_err(dial_failure)?;
        let (sink, stream) = stream.split();
        Ok(Box::new(WsConnection { sink, stream }))
    }
}

/// Split so [`HubSocketConnection::received`] is only ever a
/// `stream.next()`, which is cancel-safe; tungstenite answers a ping while
/// that read is polled.
struct WsConnection {
    sink: SplitSink<Stream, Message>,
    stream: SplitStream<Stream>,
}

#[async_trait]
impl HubSocketConnection for WsConnection {
    async fn received(&mut self) -> SocketReceived {
        match self.stream.next().await {
            Some(Ok(Message::Text(text))) => SocketReceived::Text(text.as_str().to_owned()),
            Some(Ok(Message::Close(frame))) => {
                SocketReceived::Closed(frame.map(|frame| u16::from(frame.code)))
            }
            Some(Ok(
                Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_),
            )) => SocketReceived::Alive,
            Some(Err(_)) | None => SocketReceived::Closed(None),
        }
    }

    async fn send_text(&mut self, text: String) -> Result<(), SocketGone> {
        self.sink
            .send(Message::Text(text.into()))
            .await
            .map_err(|_| SocketGone)
    }

    async fn close(mut self: Box<Self>, code: u16) {
        let frame = CloseFrame {
            code: CloseCode::from(code),
            reason: "".into(),
        };
        let _ = timeout(CLOSE_TIMEOUT, async {
            let _ = self.sink.send(Message::Close(Some(frame))).await;
            let _ = self.sink.close().await;
        })
        .await;
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/hub_ws_transport.rs"]
mod tests;

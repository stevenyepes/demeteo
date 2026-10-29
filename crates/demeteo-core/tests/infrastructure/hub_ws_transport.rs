// Tests extracted from `crates/demeteo-core/src/adapters/hub/socket/ws_transport.rs` (mirrored-tests convention). `super` = that module.
//
// Over `ws://` on loopback only: a `wss` handshake against the platform
// verifier needs a certificate the OS trusts, which no test here can install.

use super::*;
use crate::domain::hub_pairing::RefreshAnswer;
use std::net::Ipv4Addr;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};

const ACCESS: &str = "access-7c1d9e";
const PROOF: &str = "proof.eyJodG0iOiJHRVQifQ.sig";

fn upgrade(url: String) -> SocketUpgrade {
    let answer: RefreshAnswer = serde_json::from_value(serde_json::json!({
        "access_token": ACCESS,
        "token_type": "DPoP",
        "expires_in": 900,
        "refresh_token": "dhr_unused",
    }))
    .unwrap();
    SocketUpgrade {
        url,
        access_token: answer.into_tokens().access,
        dpop: PROOF.to_owned(),
    }
}

async fn loopback() -> (TcpListener, String) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("a loopback bind");
    let url = format!("ws://{}/socket", listener.local_addr().unwrap());
    (listener, url)
}

/// What the upgrade carried, as `(authorization values, DPoP values)`.
type Seen = Arc<Mutex<(Vec<String>, Vec<String>)>>;

/// Records the upgrade's headers. A `Callback` impl rather than a closure:
/// the `Err` type is tungstenite's, too large for clippy's `result_large_err`.
struct Record(Seen);

impl Callback for Record {
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        let values = |name: &str| {
            request
                .headers()
                .get_all(name)
                .iter()
                .map(|v| v.to_str().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        };
        *self.0.lock().unwrap() = (values("authorization"), values("dpop"));
        Ok(response)
    }
}

/// A WebSocket server for one connection, the upgrade's headers recorded.
async fn accepting(listener: TcpListener, seen: Seen) -> WebSocketStream<TcpStream> {
    let (tcp, _) = listener.accept().await.expect("a connection");
    tokio_tungstenite::accept_hdr_async(tcp, Record(seen))
        .await
        .expect("a server handshake")
}

/// Answers the first request with `head` and `body` in one write, as a Hub
/// refusing the upgrade does.
async fn refusing(listener: TcpListener, status_line: &'static str, body: &'static str) {
    let (mut tcp, _) = listener.accept().await.expect("a connection");
    let mut request = Vec::new();
    let mut chunk = [0u8; 4096];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = tcp.read(&mut chunk).await.expect("a request");
        assert!(n > 0, "the request ended inside its head");
        request.extend_from_slice(&chunk[..n]);
    }
    let answer = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    tcp.write_all(answer.as_bytes()).await.expect("an answer");
    let _ = tcp.shutdown().await;
}

async fn dial(url: String) -> Result<Box<dyn HubSocketConnection>, DialFailure> {
    WsSocketDialer::new()
        .expect("a TLS config")
        .dial(upgrade(url))
        .await
}

#[tokio::test]
async fn the_upgrade_carries_the_access_token_and_exactly_one_proof() {
    let (listener, url) = loopback().await;
    let seen = Seen::default();
    let server = tokio::spawn(accepting(listener, Arc::clone(&seen)));

    let conn = dial(url).await.expect("an upgrade");
    drop(server.await.expect("the server"));
    drop(conn);

    let (authorization, dpop) = seen.lock().unwrap().clone();
    assert_eq!(authorization, vec![format!("DPoP {ACCESS}")]);
    assert_eq!(dpop, vec![PROOF.to_owned()]);
}

#[tokio::test]
async fn frames_from_the_hub_become_text_alive_and_its_close_code() {
    let (listener, url) = loopback().await;
    let (frames_tx, frames_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut ws = accepting(listener, Seen::default()).await;
        ws.send(Message::Ping(b"hb".to_vec().into())).await.unwrap();
        ws.send(Message::Text("{\"type\":\"cursor_ack\"}".into()))
            .await
            .unwrap();
        let pong = ws.next().await;
        frames_rx.await.unwrap();
        ws.send(Message::Close(Some(CloseFrame {
            code: CloseCode::from(4002),
            reason: "".into(),
        })))
        .await
        .unwrap();
        pong
    });

    let mut conn = dial(url).await.expect("an upgrade");
    assert_eq!(conn.received().await, SocketReceived::Alive);
    assert_eq!(
        conn.received().await,
        SocketReceived::Text("{\"type\":\"cursor_ack\"}".to_owned())
    );
    frames_tx.send(()).unwrap();
    assert_eq!(conn.received().await, SocketReceived::Closed(Some(4002)));

    let pong = server.await.expect("the server");
    assert!(
        matches!(pong, Some(Ok(Message::Pong(ref p))) if p.as_ref() == b"hb"),
        "the ping was not answered: {pong:?}"
    );
}

#[tokio::test]
async fn a_connection_dropped_without_a_close_frame_is_closed_without_a_code() {
    let (listener, url) = loopback().await;
    let server = tokio::spawn(async move { drop(accepting(listener, Seen::default()).await) });

    let mut conn = dial(url).await.expect("an upgrade");
    server.await.expect("the server");

    assert_eq!(conn.received().await, SocketReceived::Closed(None));
    assert_eq!(conn.received().await, SocketReceived::Closed(None));
}

#[tokio::test]
async fn text_sent_and_the_close_code_reach_the_hub() {
    let (listener, url) = loopback().await;
    let server = tokio::spawn(async move {
        let mut ws = accepting(listener, Seen::default()).await;
        let text = ws.next().await;
        let close = ws.next().await;
        (text, close)
    });

    let mut conn = dial(url).await.expect("an upgrade");
    conn.send_text("{\"type\":\"hello\"}".to_owned())
        .await
        .expect("a send");
    conn.close(1000).await;

    let (text, close) = server.await.expect("the server");
    assert!(
        matches!(text, Some(Ok(Message::Text(ref t))) if t.as_str() == "{\"type\":\"hello\"}"),
        "{text:?}"
    );
    assert!(
        matches!(close, Some(Ok(Message::Close(Some(ref f)))) if u16::from(f.code) == 1000),
        "{close:?}"
    );
}

#[tokio::test]
async fn a_refused_upgrade_carries_the_status_and_the_hubs_error_code() {
    let (listener, url) = loopback().await;
    let server = tokio::spawn(refusing(
        listener,
        "401 Unauthorized",
        r#"{"error":"invalid_token"}"#,
    ));

    let refusal = dial(url).await.err();
    server.await.expect("the server");

    assert_eq!(
        refusal,
        Some(DialFailure::Refused {
            status: 401,
            error: Some("invalid_token".to_owned()),
        })
    );
}

#[tokio::test]
async fn a_refusal_whose_body_is_not_a_hub_error_has_no_error_code() {
    let (listener, url) = loopback().await;
    let server = tokio::spawn(refusing(
        listener,
        "502 Bad Gateway",
        r#"{"error":"<b>upstream</b> said no"}"#,
    ));

    let refusal = dial(url).await.err();
    server.await.expect("the server");

    assert_eq!(
        refusal,
        Some(DialFailure::Refused {
            status: 502,
            error: None,
        })
    );
}

#[tokio::test]
async fn a_closed_port_is_unreachable_and_the_error_names_no_credential() {
    let closed = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let url = format!("ws://{}/socket?probe=q", closed.local_addr().unwrap());
    drop(closed);

    let failure = dial(url).await.err();

    let Some(DialFailure::Unreachable(text)) = failure else {
        panic!("not unreachable: {failure:?}");
    };
    for fragment in [ACCESS, PROOF, "127.0.0.1", "probe"] {
        assert!(!text.contains(fragment), "{text}");
    }
}

#![cfg(target_os = "linux")]
//! The Hub's instance socket, asserted from the desktop package: this file is
//! the evidence a Demeteo run has for it. The header of
//! `src-tauri/tests/hub_pairing.rs` says why that evidence has to live here
//! and not beside the code.
//!
//! Nothing here touches Postgres; `crates/demeteo-hub/tests/socket.rs` holds
//! what only real rows can show.

use async_trait::async_trait;
use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::auth::challenge::{base64url_decode, base64url_encode};
use demeteo_hub::auth::handlers::ApiError;
use demeteo_hub::auth::testing::{answered, lazy_state, serve};
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::pairing::grant::{FamilyChange, FreshPair, TokenSecret};
use demeteo_hub::pairing::signing_key::SigningKey;
use demeteo_hub::pairing::testing::{claims, verifies_ed25519, MemoryFamily, SoftDeviceKey};
use demeteo_hub::ports::{KeyService, KeyServiceError};
use demeteo_hub::protocol::{
    Hello, RefusalReason, RequestOutcome, RequestPayload, RequestResult, RunEvent, RunEventKind,
    RunEventPayload, Scope, SignedRequest, Snapshot,
};
use demeteo_hub::routes::{self, AppState};
use demeteo_hub::socket::connections::{Closing, ConnectionId, Connections, Registered};
use demeteo_hub::socket::frame::{self, FrameError, Inbound};
use demeteo_hub::socket::handler::{admitted, Admission};
use demeteo_hub::socket::port::{PeerGone, Received, SocketStore, Transport};
use demeteo_hub::socket::requests::{self, EnqueueError, EnqueueRefusal, Enqueued, OutcomeChange};
use demeteo_hub::socket::session::{self, Ended, Limits, Violation};
use demeteo_hub::socket::testing::{
    FakeInstance, InstanceView, MemoryHub, MemoryStore, MirrorRow, RequestView, SnapshotRow,
};
use demeteo_hub::socket::{MAX_CONNECTIONS, MAX_FEATURE_ID_BYTES, SHUTDOWN_DRAIN_SECS};
use demeteo_hub::startup;
use demeteo_hub::store::requests::{
    NewRequest, QueuedRequest, RequestId, RequestState, RequestStatus,
};
use demeteo_hub::store::tokens::TokenHash;
use demeteo_hub::store::{InstanceId, StoreError, UserId};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::future::{ready, Future};
use std::net::Ipv4Addr;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

const INBOUND_FIXTURES: [&str; 5] = [
    "hello.json",
    "run_event.json",
    "snapshot.json",
    "request_result_accepted.json",
    "request_result_refused.json",
];

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri has a parent directory")
        .join("crates")
        .join("demeteo-hub-protocol")
        .join("tests")
        .join("fixtures")
}

/// Every fixture the protocol crate keeps, by file name. The directory is
/// listed and not spelled here, so a fixture added there is read here too.
fn protocol_fixtures() -> BTreeMap<String, String> {
    let dir = fixture_dir();
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| {
            let path = entry.expect("a directory entry").path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a UTF-8 file name")
                .to_owned();
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            (name, text)
        })
        .collect()
}

fn fixture(name: &str) -> Value {
    let path = fixture_dir().join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn as_written<T: DeserializeOwned>(name: &str, text: &str) -> T {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{name} as its own type: {e}"))
}

/// What the Hub must make of one fixture, decided by its file name alone. A
/// name this does not know is a panic and not a skip: a fixture the protocol
/// crate gains has to be classified here before the pin says anything about
/// it.
fn expected_of(name: &str, text: &str) -> Result<Inbound, FrameError> {
    match name {
        "hello.json" => Ok(Inbound::Hello(as_written::<Hello>(name, text))),
        "run_event.json" => Ok(Inbound::RunEvent(as_written::<RunEvent>(name, text))),
        "snapshot.json" => Ok(Inbound::Snapshot(as_written::<Snapshot>(name, text))),
        "request_result_accepted.json" | "request_result_refused.json" => Ok(
            Inbound::RequestResult(as_written::<RequestResult>(name, text)),
        ),
        "cursor_ack.json" => Err(FrameError::Unrecognised),
        outbound if outbound.starts_with("signed_request_") => Err(FrameError::Unrecognised),
        unknown => panic!("{unknown} is a protocol fixture this test has no expectation for"),
    }
}

#[test]
fn each_protocol_fixture_parses_as_exactly_one_inbound_frame() {
    let fixtures = protocol_fixtures();

    for required in INBOUND_FIXTURES.into_iter().chain(["cursor_ack.json"]) {
        assert!(fixtures.contains_key(required), "{required} is missing");
    }
    assert!(
        fixtures
            .keys()
            .any(|name| name.starts_with("signed_request_")),
        "no signed request fixture was read"
    );

    for (name, text) in &fixtures {
        assert_eq!(frame::parse(text), expected_of(name, text), "{name}");
    }
}

#[test]
fn a_frame_with_an_unknown_field_another_version_an_offset_past_i64_or_no_json_is_refused() {
    for name in INBOUND_FIXTURES {
        let control = fixture(name);
        assert!(frame::parse(&control.to_string()).is_ok(), "{name}");

        let mut with_a_cost = control.clone();
        with_a_cost["cost_usd"] = json!(1.25);
        assert_eq!(
            frame::parse(&with_a_cost.to_string()),
            Err(FrameError::Unrecognised),
            "{name} with a field its type lacks"
        );

        for version in [0, 2] {
            let mut versioned = control.clone();
            versioned["v"] = json!(version);
            assert_eq!(
                frame::parse(&versioned.to_string()),
                Err(FrameError::Version(version)),
                "{name} at version {version}"
            );
        }

        let text = control.to_string();
        assert_eq!(
            frame::parse(&text[..text.len() - 1]),
            Err(FrameError::Unrecognised),
            "{name} cut short"
        );
    }

    let event = fixture("run_event.json");
    let mut with_a_summary = event.clone();
    with_a_summary["payload"]["summary"] = json!("Approve the migration?");
    assert_eq!(
        frame::parse(&with_a_summary.to_string()),
        Err(FrameError::Unrecognised)
    );

    let largest_stored = i64::MAX as u64;
    for offset in [0, largest_stored] {
        let mut within = event.clone();
        within["offset"] = json!(offset);
        match frame::parse(&within.to_string()) {
            Ok(Inbound::RunEvent(parsed)) => assert_eq!(parsed.offset, offset),
            other => panic!("offset {offset}: {other:?}"),
        }
    }
    let mut beyond = event.clone();
    beyond["offset"] = json!(largest_stored + 1);
    assert_eq!(
        frame::parse(&beyond.to_string()),
        Err(FrameError::OffsetTooLarge(largest_stored + 1))
    );

    let with_a_nul = [
        ("hello.json", "/install_id"),
        ("run_event.json", "/feature_id"),
        ("run_event.json", "/payload/step_id"),
        ("snapshot.json", "/workflows/0/note"),
        ("request_result_accepted.json", "/request_id"),
    ];
    for (name, pointer) in with_a_nul {
        let mut frame = fixture(name);
        *frame.pointer_mut(pointer).expect(pointer) = json!("a\u{0}b");
        assert_eq!(
            frame::parse(&frame.to_string()),
            Err(FrameError::Nul),
            "{name} with U+0000 at {pointer}"
        );
    }
    let mut escaped_backslash = event.clone();
    escaped_backslash["feature_id"] = json!(r"f-\u0000");
    let text = escaped_backslash.to_string();
    assert!(text.contains(r"\u0000"), "{text}");
    assert!(frame::parse(&text).is_ok(), "{text}");

    for feature_id in [
        "f".repeat(MAX_FEATURE_ID_BYTES),
        "é".repeat(MAX_FEATURE_ID_BYTES / 2),
    ] {
        let mut at_the_bound = event.clone();
        at_the_bound["feature_id"] = json!(feature_id);
        match frame::parse(&at_the_bound.to_string()) {
            Ok(Inbound::RunEvent(parsed)) => assert_eq!(parsed.feature_id, feature_id),
            other => panic!("a feature id of {} bytes: {other:?}", feature_id.len()),
        }
    }
    for feature_id in [
        "f".repeat(MAX_FEATURE_ID_BYTES + 1),
        "é".repeat(MAX_FEATURE_ID_BYTES / 2 + 1),
    ] {
        let mut over = event.clone();
        over["feature_id"] = json!(feature_id);
        assert_eq!(
            frame::parse(&over.to_string()),
            Err(FrameError::FeatureIdTooLong(feature_id.len())),
            "a feature id of {} chars",
            feature_id.chars().count()
        );
    }

    for not_a_message in ["", "hello", "{", "[]", "null", "{}"] {
        assert_eq!(
            frame::parse(not_a_message),
            Err(FrameError::Unrecognised),
            "{not_a_message:?}"
        );
    }
}

/// When the upgrades below are presented, in Unix seconds.
const NOW: u64 = 1_700_000_000;
const SOCKET_URL: &str = "https://hub.example/socket";
const TOKEN_URL: &str = "https://hub.example/token";

/// What RFC 9449 §4.2 has a proof say of the access token beside it. Computed
/// here over the text, with `sha2` and not through the Hub: a helper of the
/// Hub's would agree with the Hub about a digest both had wrong.
fn ath_of(token: &TokenSecret) -> String {
    base64url_encode(&Sha256::digest(token.text().as_bytes()))
}

fn dpop(token: &TokenSecret) -> Option<String> {
    Some(format!("DPoP {}", token.text()))
}

/// A proof by `key` of one `htm` request to `htu` at `now`, naming `token`.
fn proof_naming(
    key: &SoftDeviceKey,
    token: &TokenSecret,
    htm: &str,
    htu: &str,
    now: u64,
    jti: &str,
) -> Option<String> {
    let mut claims = claims(htm, htu, now, jti);
    claims["ath"] = json!(ath_of(token));
    Some(key.proof_with(&key.header(), &claims))
}

/// What one upgrade request presents, and when.
struct Upgrade {
    authorization: Option<String>,
    proof: Option<String>,
    now: u64,
}

/// A Hub and one tenant with two paired instances, as its store holds them:
/// the token rows of the first, and the device key of each.
///
/// The second instance is there for one case. A proof by its key is a valid
/// proof by a key the tenant really has, so the only thing wrong with it
/// beside the first instance's token is that the token is bound elsewhere.
struct Paired {
    state: AppState,
    key: SoftDeviceKey,
    issued: FreshPair,
    tokens: MemoryFamily,
    instance: InstanceId,
    sibling_key: SoftDeviceKey,
    device_keys: Vec<(UserId, String, InstanceId)>,
}

impl Paired {
    /// Paired at `NOW`. Must be made inside a Tokio runtime, as `lazy_state`
    /// requires; the pool it holds never connects.
    fn new() -> Self {
        let state = unconnected_state();
        let (key, sibling_key) = (SoftDeviceKey::new(), SoftDeviceKey::new());
        let issued = FreshPair::generate().expect("the system random source");
        let tokens = MemoryFamily::paired(&issued, &key.thumbprint(), NOW);
        let user = tokens.rows()[0].user_id;
        let instance = InstanceId::generate();
        let device_keys = vec![
            (user, key.thumbprint(), instance),
            (user, sibling_key.thumbprint(), InstanceId::generate()),
        ];
        Self {
            state,
            key,
            issued,
            tokens,
            instance,
            sibling_key,
            device_keys,
        }
    }

    /// The upgrade nothing is wrong with: the access token, and a proof by
    /// the key it is bound to, for this socket, naming it. Every proof needs
    /// a `jti` of its own, or it is refused as a replay of the one before.
    fn control(&self, now: u64, jti: &str) -> Upgrade {
        let token = &self.issued.access;
        Upgrade {
            authorization: dpop(token),
            proof: proof_naming(&self.key, token, "GET", SOCKET_URL, now, jti),
            now,
        }
    }

    /// `upgrade` put to `admitted` over this store: the admission, or what a
    /// client is sent in its place. The two reads answer what the rows say
    /// and nothing else, so a token or a key nobody wrote is not found.
    async fn answer(&self, upgrade: &Upgrade) -> Result<Admission, (u16, Value)> {
        let find_token =
            |hash: TokenHash| ready(Ok::<_, StoreError>(self.tokens.find(&hash).cloned()));
        let instance_of_key = |user: UserId, jkt: String| {
            let held = self
                .device_keys
                .iter()
                .find(|(owner, key, _)| *owner == user && *key == jkt);
            ready(Ok::<_, StoreError>(held.map(|(_, _, instance)| *instance)))
        };
        let outcome = admitted(
            &self.state,
            upgrade.authorization.as_deref(),
            upgrade.proof.as_deref(),
            upgrade.now,
            find_token,
            instance_of_key,
        )
        .await;
        match outcome {
            Ok(admission) => Ok(admission),
            Err(refusal) => Err(answered(refusal).await),
        }
    }

    /// The admission of the first instance under the token issued at `NOW`,
    /// which is good for 900 seconds.
    fn admission(&self) -> Admission {
        Admission {
            user: self.tokens.rows()[0].user_id,
            instance: self.instance,
            token_expires_at: NOW + 900,
        }
    }
}

fn invalid_token() -> (u16, Value) {
    (401, json!({"error": "invalid_token"}))
}

/// What is wrong with one upgrade: a change to what the store holds, and
/// then the request presented to it.
type Fault = fn(&mut Paired) -> Upgrade;

#[tokio::test]
async fn an_upgrade_without_a_valid_dpop_bound_token_is_refused() {
    const JTI: &str = "jti-refused";
    let faults: [(&str, Fault); 13] = [
        ("no Authorization", |hub| Upgrade {
            authorization: None,
            ..hub.control(NOW, JTI)
        }),
        ("a scheme other than DPoP", |hub| Upgrade {
            authorization: Some(format!("Bearer {}", hub.issued.access.text())),
            ..hub.control(NOW, JTI)
        }),
        ("no DPoP header", |hub| Upgrade {
            proof: None,
            ..hub.control(NOW, JTI)
        }),
        ("a proof for another URL", |hub| Upgrade {
            proof: proof_naming(&hub.key, &hub.issued.access, "GET", TOKEN_URL, NOW, JTI),
            ..hub.control(NOW, JTI)
        }),
        ("a proof for the wss spelling of the URL", |hub| {
            let dialled = "wss://hub.example/socket";
            Upgrade {
                proof: proof_naming(&hub.key, &hub.issued.access, "GET", dialled, NOW, JTI),
                ..hub.control(NOW, JTI)
            }
        }),
        ("a proof for another method", |hub| Upgrade {
            proof: proof_naming(&hub.key, &hub.issued.access, "POST", SOCKET_URL, NOW, JTI),
            ..hub.control(NOW, JTI)
        }),
        ("a proof by a key the token is not bound to", |hub| {
            let token = &hub.issued.access;
            Upgrade {
                proof: proof_naming(&hub.sibling_key, token, "GET", SOCKET_URL, NOW, JTI),
                ..hub.control(NOW, JTI)
            }
        }),
        ("a token no row holds", |hub| {
            hub.tokens = MemoryFamily::empty();
            hub.control(NOW, JTI)
        }),
        ("a refresh token", |hub| {
            let token = &hub.issued.refresh;
            Upgrade {
                authorization: dpop(token),
                proof: proof_naming(&hub.key, token, "GET", SOCKET_URL, NOW, JTI),
                now: NOW,
            }
        }),
        ("a retired token", |hub| {
            let row = hub
                .tokens
                .find(&hub.issued.access.hash())
                .expect("the access token's row");
            let retirement = FamilyChange {
                retire: vec![row.id],
                insert: Vec::new(),
            };
            hub.tokens.apply(&retirement, NOW);
            hub.control(NOW, JTI)
        }),
        ("a token at its expiry", |hub| hub.control(NOW + 900, JTI)),
        ("a token past its expiry", |hub| hub.control(NOW + 901, JTI)),
        ("a key that belongs to no instance", |hub| {
            let jkt = hub.key.thumbprint();
            hub.device_keys.retain(|(_, key, _)| *key != jkt);
            hub.control(NOW, JTI)
        }),
    ];

    for (fault, presented) in faults {
        let mut hub = Paired::new();
        let admission = hub.admission();
        let control = hub.control(NOW, "jti-control");
        assert_eq!(hub.answer(&control).await, Ok(admission), "{fault}");
        let last_second = hub.control(NOW + 899, "jti-last-second");
        assert_eq!(hub.answer(&last_second).await, Ok(admission), "{fault}");

        let refused = presented(&mut hub);

        assert_eq!(hub.answer(&refused).await, Err(invalid_token()), "{fault}");
    }

    let hub = Paired::new();
    let lower_case = Upgrade {
        authorization: Some(format!("dpop {}", hub.issued.access.text())),
        ..hub.control(NOW, "jti-lower-case")
    };
    assert_eq!(hub.answer(&lower_case).await, Ok(hub.admission()));
}

#[tokio::test]
async fn an_upgrade_proof_is_single_use_and_bound_to_the_presented_token() {
    let hub = Paired::new();
    let admission = hub.admission();

    let once = hub.control(NOW, "jti-1");
    assert_eq!(hub.answer(&once).await, Ok(admission));
    assert_eq!(hub.answer(&once).await, Err(invalid_token()));
    let again = hub.control(NOW, "jti-2");
    assert_eq!(hub.answer(&again).await, Ok(admission));

    let without_ath = Upgrade {
        proof: Some(hub.key.proof("GET", SOCKET_URL, NOW, "jti-3")),
        ..hub.control(NOW, "jti-3")
    };
    assert_eq!(hub.answer(&without_ath).await, Err(invalid_token()));

    let mut hub = Paired::new();
    let next = FreshPair::generate().expect("the system random source");
    let rows = next
        .rows(hub.tokens.rows()[0].family_id, &hub.key.thumbprint(), NOW)
        .expect("an expiry within the range of `time`");
    let rotation = FamilyChange {
        retire: Vec::new(),
        insert: vec![rows.access, rows.refresh],
    };
    hub.tokens.apply(&rotation, NOW);
    let admission = hub.admission();
    let naming = |token: &TokenSecret, jti: &str| Upgrade {
        authorization: dpop(token),
        proof: proof_naming(&hub.key, token, "GET", SOCKET_URL, NOW, jti),
        now: NOW,
    };
    for (token, jti) in [(&hub.issued.access, "jti-4"), (&next.access, "jti-5")] {
        assert_eq!(hub.answer(&naming(token, jti)).await, Ok(admission));
    }

    let naming_the_other = Upgrade {
        authorization: dpop(&hub.issued.access),
        ..naming(&next.access, "jti-6")
    };
    assert_eq!(hub.answer(&naming_the_other).await, Err(invalid_token()));
}

/// The Hub's real router over `state` on a loopback port, and the URL its
/// socket route answers at. The web directory does not exist: the socket is a
/// route and no file.
async fn served_socket(state: AppState) -> String {
    let web_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-web-dir");
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("a free loopback port");
    let addr = listener.local_addr().expect("the bound address");
    serve(listener, routes::router(state, &web_dir));
    format!("http://{addr}/socket")
}

/// The four headers RFC 6455 §4.1 asks of a client, set by hand since
/// nothing here speaks WebSocket. They are what makes a request an upgrade
/// the server could act on, so a `401` to one is a refusal of an upgrade and
/// not of a plain `GET`. The key is that section's own example.
const UPGRADE_HEADERS: [(&str, &str); 4] = [
    ("connection", "upgrade"),
    ("upgrade", "websocket"),
    ("sec-websocket-version", "13"),
    ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ=="),
];

/// What a `GET` of `url` carrying `headers` is answered: the status, its
/// `WWW-Authenticate` headers and the body.
async fn socket_answer(url: &str, headers: &[(&str, &str)]) -> (u16, Vec<String>, Value) {
    let mut request = reqwest::Client::new().get(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.send().await.expect("an answer from the Hub");
    let status = response.status().as_u16();
    let challenges = response
        .headers()
        .get_all("www-authenticate")
        .iter()
        .map(|value| value.to_str().expect("a challenge in ASCII").to_owned())
        .collect();
    let body = response.text().await.expect("the body of the answer");
    let body = serde_json::from_str(&body).unwrap_or_else(|e| panic!("{status} {body:?}: {e}"));
    (status, challenges, body)
}

/// The Hub reads its own clock for an upgrade over HTTP, so a proof that is
/// to be fresh there is made at this one.
fn wall_clock() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs()
}

/// The last upgrade is the only one whose credentials are each good: an
/// access token as the Hub writes one, and two proofs by one key, fresh, for
/// this socket and naming that token. What is refused is that there are two.
/// A route that took the first would go on to look the token up, and this Hub
/// has no database to answer it, so it would say `500` and not `401`.
///
/// The request after them is no upgrade at all. It is asked who it is before
/// it is told what it is not, and so gets the same answer.
#[tokio::test]
async fn the_socket_route_answers_401_to_an_upgrade_that_carries_no_credentials() {
    let hub = Paired::new();
    let url = served_socket(hub.state.clone()).await;
    let token = &hub.issued.access;
    let authorization = dpop(token).expect("an Authorization header");
    let fresh = |jti: &str| {
        proof_naming(&hub.key, token, "GET", SOCKET_URL, wall_clock(), jti).expect("a proof")
    };
    let (first, second) = (fresh("jti-route-1"), fresh("jti-route-2"));
    let cases: [(&str, Vec<(&str, &str)>); 4] = [
        ("no credentials", vec![]),
        (
            "a token and no proof",
            vec![("authorization", &authorization)],
        ),
        (
            "a proof that is no proof",
            vec![("authorization", &authorization), ("dpop", "garbage")],
        ),
        (
            "two proofs",
            vec![
                ("authorization", &authorization),
                ("dpop", &first),
                ("dpop", &second),
            ],
        ),
    ];

    for (case, credentials) in cases {
        let headers = [UPGRADE_HEADERS.as_slice(), &credentials].concat();
        let (status, challenges, body) = socket_answer(&url, &headers).await;
        assert_ne!(status, 101, "{case}");
        assert_eq!((status, body), invalid_token(), "{case}");
        assert_eq!(challenges, ["DPoP"], "{case}");
    }

    let (status, challenges, body) = socket_answer(&url, &[]).await;
    assert_eq!((status, body), invalid_token(), "a plain GET");
    assert_eq!(challenges, ["DPoP"], "a plain GET");
}

/// A `CursorAck` as the Hub writes one, spelled here and not built from the
/// type the fake parses it with.
fn ack(offset: u64) -> String {
    json!({"v": 1, "offset": offset}).to_string()
}

/// Long enough for a frame that is already queued, short enough that a test
/// which waits it out costs nothing.
const IMPATIENT: Duration = Duration::from_millis(50);

/// What the Hub end is handed next. Bounded, so that a fake which stops
/// passing frames on fails the test that drives it and does not hang it.
async fn sent_to(hub: &mut impl Transport) -> Received {
    tokio::time::timeout(Duration::from_secs(2), hub.received())
        .await
        .expect("the Hub end to be handed something")
}

/// The control for the `should_panic` tests below: the same fake against a
/// Hub end driven by hand, with nothing wrong between them.
#[tokio::test]
async fn a_script_that_matches_what_the_hub_sent_finishes_cleanly() {
    let (mut instance, mut hub) = FakeInstance::connect();

    let hello: Hello = serde_json::from_value(fixture("hello.json")).expect("the hello fixture");
    instance.send(&hello);
    let Received::Text(text) = sent_to(&mut hub).await else {
        panic!("the hello did not arrive as text");
    };
    assert_eq!(frame::parse(&text), Ok(Inbound::Hello(hello)));
    hub.send_text(ack(0)).await.expect("an instance to send to");
    instance.expect_ack(0).await;

    instance.send_text("{\"v\":1");
    assert_eq!(
        sent_to(&mut hub).await,
        Received::Text("{\"v\":1".to_owned())
    );
    instance.send_binary();
    assert_eq!(sent_to(&mut hub).await, Received::Unsupported);

    let request = fixture("signed_request_cancel_feature.json");
    hub.send_text(request.to_string())
        .await
        .expect("an instance to send to");
    let expected: SignedRequest = serde_json::from_value(request).expect("the request fixture");
    assert_eq!(instance.expect_request().await, expected);

    hub.send_ping().await.expect("an instance to ping");
    assert_eq!(sent_to(&mut hub).await, Received::Alive);

    hub.close(1008).await;
    instance.expect_close(1008).await;
    instance.finish().await;
}

#[tokio::test]
async fn the_fake_instance_closes_from_its_side_and_waits_for_the_hub_end_to_go() {
    let (instance, mut hub) = FakeInstance::connect();
    let hub_end = async move {
        assert_eq!(sent_to(&mut hub).await, Received::Closed);
        assert_eq!(sent_to(&mut hub).await, Received::Closed);
    };
    tokio::join!(instance.finish(), hub_end);
}

#[tokio::test]
async fn a_silent_fake_instance_answers_no_ping() {
    let (instance, mut hub) = FakeInstance::connect();

    hub.send_ping().await.expect("an instance to ping");
    instance.send_text("after the pong");
    assert_eq!(sent_to(&mut hub).await, Received::Alive);
    assert_eq!(
        sent_to(&mut hub).await,
        Received::Text("after the pong".to_owned())
    );

    instance.go_silent();
    hub.send_ping().await.expect("an instance to ping");
    instance.send_text("and no pong");
    assert_eq!(
        sent_to(&mut hub).await,
        Received::Text("and no pong".to_owned())
    );

    drop(hub);
    instance.finish().await;
}

#[tokio::test]
async fn the_hub_end_is_told_when_the_fake_instance_is_gone() {
    let (instance, mut hub) = FakeInstance::connect();
    drop(instance);

    assert_eq!(hub.send_text(ack(0)).await, Err(PeerGone));
    assert_eq!(hub.send_ping().await, Err(PeerGone));
    assert_eq!(sent_to(&mut hub).await, Received::Closed);
    hub.close(1011).await;
}

#[tokio::test]
#[should_panic(expected = "expected a signed request, the Hub sent a cursor ack at offset 0")]
async fn the_fake_instance_fails_on_a_frame_it_was_not_told_to_expect() {
    let (mut instance, mut hub) = FakeInstance::connect();
    hub.send_text(ack(0)).await.expect("an instance to send to");
    instance.expect_request().await;
}

#[tokio::test]
#[should_panic(expected = "expected a cursor ack at offset 7 (protocol version 1), the Hub sent")]
async fn the_fake_instance_fails_on_an_ack_at_another_offset() {
    let (mut instance, mut hub) = FakeInstance::connect();
    hub.send_text(ack(3)).await.expect("an instance to send to");
    instance.expect_ack(7).await;
}

#[tokio::test]
#[should_panic(expected = "expected a close with code 1008, the Hub sent a close with code 1011")]
async fn the_fake_instance_fails_on_a_close_with_another_code() {
    let (mut instance, hub) = FakeInstance::connect();
    hub.close(1011).await;
    instance.expect_close(1008).await;
}

#[tokio::test]
#[should_panic(expected = "1 Hub frame unread")]
async fn the_fake_instance_fails_when_the_hub_leaves_a_frame_unread() {
    let (instance, mut hub) = FakeInstance::connect();
    hub.send_text(ack(0)).await.expect("an instance to send to");
    drop(hub);
    instance.finish().await;
}

#[tokio::test]
#[should_panic(expected = "the Hub sent nothing within")]
async fn the_fake_instance_fails_when_an_expected_frame_never_arrives() {
    let (instance, _hub) = FakeInstance::connect();
    let mut instance = instance.waiting_at_most(IMPATIENT);
    instance.expect_ack(0).await;
}

/// The frame arrives after the script stopped reading, which is the case a
/// fake that checks only what it reads cannot see.
#[tokio::test]
#[should_panic(expected = "1 Hub frame unread")]
async fn the_fake_instance_fails_on_a_frame_sent_after_its_script_ended() {
    let (instance, mut hub) = FakeInstance::connect();
    let hub_end = async move {
        assert_eq!(sent_to(&mut hub).await, Received::Closed);
        hub.send_text(ack(0)).await.expect("an instance to send to");
    };
    tokio::join!(instance.finish(), hub_end);
}

#[tokio::test]
#[should_panic(expected = "1 Hub frame unread")]
async fn a_fake_instance_dropped_without_finishing_still_fails_on_an_unread_frame() {
    let (instance, mut hub) = FakeInstance::connect();
    hub.send_text(ack(0)).await.expect("an instance to send to");
    drop(instance);
}

#[tokio::test]
#[should_panic(expected = "the Hub end was still open")]
async fn the_fake_instance_fails_when_the_hub_end_outlives_its_close() {
    let (instance, _hub) = FakeInstance::connect();
    instance.waiting_at_most(IMPATIENT).finish().await;
}

fn registered(connections: &Connections, instance: InstanceId) -> Registered {
    connections
        .register(instance)
        .expect("a registry with room for one more")
}

/// Whether the connection has a wake-up to take. A wake-up that was sent is
/// there at once, so [`IMPATIENT`] is only how long the answer "no" costs.
async fn woken(connection: &Registered) -> bool {
    tokio::time::timeout(IMPATIENT, connection.wake.woken())
        .await
        .is_ok()
}

/// What the connection was asked to end for, or `None` while it was not.
async fn asked_to_end(connection: &mut Registered) -> Option<Closing> {
    tokio::time::timeout(IMPATIENT, connection.close.closing())
        .await
        .ok()
}

#[tokio::test]
async fn a_later_connection_of_an_instance_displaces_the_earlier_and_survives_its_release() {
    let connections = Connections::default();
    let (instance, sibling) = (InstanceId::generate(), InstanceId::generate());
    let mut earlier = registered(&connections, instance);
    let mut beside = registered(&connections, sibling);
    assert_eq!(asked_to_end(&mut earlier).await, None);

    let mut later = registered(&connections, instance);

    assert_eq!(asked_to_end(&mut earlier).await, Some(Closing::Displaced));
    assert_eq!(asked_to_end(&mut earlier).await, Some(Closing::Displaced));
    assert_ne!(earlier.connection, later.connection);
    assert_eq!(asked_to_end(&mut later).await, None);
    assert_eq!(asked_to_end(&mut beside).await, None);

    connections.release(instance, earlier.connection);
    connections.wake(instance);
    assert!(woken(&later).await);
    assert!(!woken(&earlier).await);

    connections.release(instance, later.connection);
    connections.wake(instance);
    assert!(!woken(&later).await);
}

#[tokio::test]
async fn a_wake_sent_before_the_session_waits_is_still_observed_and_only_once() {
    let connections = Connections::default();
    let (instance, sibling) = (InstanceId::generate(), InstanceId::generate());
    let held = registered(&connections, instance);
    let beside = registered(&connections, sibling);
    assert!(!woken(&held).await);

    connections.wake(instance);
    connections.wake(instance);
    connections.wake(InstanceId::generate());

    assert!(woken(&held).await);
    assert!(!woken(&held).await);
    assert!(!woken(&beside).await);
}

#[tokio::test]
async fn close_all_asks_every_connection_to_end_as_shutting_down() {
    let connections = Connections::default();
    let instance = InstanceId::generate();
    let mut displaced = registered(&connections, instance);
    let mut held = [instance, InstanceId::generate(), InstanceId::generate()]
        .map(|instance| registered(&connections, instance));
    for connection in &mut held {
        assert_eq!(asked_to_end(connection).await, None);
    }

    connections.close_all();

    for connection in &mut held {
        assert_eq!(asked_to_end(connection).await, Some(Closing::ShuttingDown));
        assert!(!woken(connection).await);
    }
    assert_eq!(asked_to_end(&mut displaced).await, Some(Closing::Displaced));

    let mut admitted_too_late = registered(&connections, InstanceId::generate());
    assert_eq!(
        asked_to_end(&mut admitted_too_late).await,
        Some(Closing::ShuttingDown)
    );
}

#[tokio::test]
async fn a_connection_the_registry_no_longer_holds_is_not_left_waiting_to_be_asked() {
    let connections = Connections::default();
    let instance = InstanceId::generate();
    let mut released = registered(&connections, instance);
    let mut held = registered(&connections, InstanceId::generate());
    assert_eq!(asked_to_end(&mut released).await, None);

    connections.release(instance, released.connection);

    assert_eq!(
        asked_to_end(&mut released).await,
        Some(Closing::ShuttingDown)
    );
    assert_eq!(asked_to_end(&mut held).await, None);

    drop(connections);

    assert_eq!(asked_to_end(&mut held).await, Some(Closing::ShuttingDown));
}

#[tokio::test]
async fn a_full_registry_refuses_an_instance_that_is_not_connected_and_closes_no_one() {
    let connections = Connections::default();
    let instance = InstanceId::generate();
    let mut held = registered(&connections, instance);
    for _ in 1..MAX_CONNECTIONS {
        registered(&connections, InstanceId::generate());
    }

    let refused = connections
        .register(InstanceId::generate())
        .expect_err("a registry at its bound");

    assert_eq!(
        answered(ApiError::from(refused)).await,
        (503, json!({"error": "too_many_connections"}))
    );
    assert_eq!(asked_to_end(&mut held).await, None);

    let reconnected = registered(&connections, instance);
    assert_eq!(asked_to_end(&mut held).await, Some(Closing::Displaced));
    assert!(connections.register(InstanceId::generate()).is_err());

    connections.release(instance, reconnected.connection);
    assert!(connections.register(InstanceId::generate()).is_ok());
    assert!(connections.register(InstanceId::generate()).is_err());
}

/// When the token every session here is admitted with expires: the lifetime
/// of an access token after `NOW`.
const TOKEN_EXPIRES_AT: u64 = NOW + 900;

/// For every test that is not about how long a session lives: no ping is
/// sent and no peer is given up on within any wait in this file.
const UNHURRIED: Limits = Limits {
    ping_interval: Duration::from_secs(3600),
    pong_deadline: Duration::from_secs(3600),
};

/// A registry a test made, or the one a Hub's state holds and a stop
/// drains.
enum Registry {
    Own(Arc<Connections>),
    Hub(AppState),
}

impl Deref for Registry {
    type Target = Connections;

    fn deref(&self) -> &Connections {
        match self {
            Self::Own(connections) => connections,
            Self::Hub(state) => state.connections(),
        }
    }
}

/// One paired instance, the registry its connections are seated in and the
/// clock its sessions read.
struct Sessions {
    hub: MemoryHub,
    connections: Registry,
    instance: InstanceId,
    now: Arc<AtomicU64>,
    limits: Limits,
}

impl Sessions {
    /// Paired with every scope approved and never connected, at `NOW`.
    fn new() -> Self {
        Self::seated_in(&Arc::new(Connections::default()))
    }

    /// As [`new`](Self::new), and one more instance of a registry that
    /// others are seated in.
    fn seated_in(connections: &Arc<Connections>) -> Self {
        Self::seated(Registry::Own(Arc::clone(connections)))
    }

    /// As [`new`](Self::new), in the registry of the Hub `state` is.
    fn seated_at(state: &AppState) -> Self {
        Self::seated(Registry::Hub(state.clone()))
    }

    fn seated(connections: Registry) -> Self {
        let hub = MemoryHub::paired(&[Scope::Read, Scope::Spend, Scope::Gates]);
        Self {
            instance: hub.admission(TOKEN_EXPIRES_AT).instance,
            hub,
            connections,
            now: Arc::new(AtomicU64::new(NOW)),
            limits: UNHURRIED,
        }
    }

    fn with(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// A session over the hub's own store, as
    /// [`session_over`](Self::session_over) makes one.
    fn session(&self) -> (FakeInstance, impl Future<Output = Ended> + Send + 'static) {
        self.session_over(self.hub.store())
    }

    /// The one place a session is made: what `run` is handed changes here
    /// and nowhere else. It is registered and does nothing until it is
    /// polled, which a test that has to say when it runs does itself.
    fn session_over(
        &self,
        store: impl SocketStore + 'static,
    ) -> (FakeInstance, impl Future<Output = Ended> + Send + 'static) {
        let (instance, transport) = FakeInstance::connect();
        let connection = registered(&self.connections, self.instance);
        let now = Arc::clone(&self.now);
        let clock = move || now.load(Ordering::SeqCst);
        let session = session::run(
            transport,
            store,
            connection,
            self.hub.admission(TOKEN_EXPIRES_AT),
            clock,
            self.limits,
        );
        (instance, session)
    }

    fn connect(&self) -> (FakeInstance, JoinHandle<Ended>) {
        let (instance, session) = self.session();
        (instance, tokio::spawn(session))
    }

    /// A session whose `hello` was acked with `cursor`.
    async fn greeted(&self, cursor: u64) -> (FakeInstance, JoinHandle<Ended>) {
        let (mut instance, session) = self.connect();
        instance.send(&hello());
        instance.expect_ack(cursor).await;
        (instance, session)
    }

    fn clock_reads(&self, now: u64) {
        self.now.store(now, Ordering::SeqCst);
    }
}

async fn ended(session: JoinHandle<Ended>) -> Ended {
    ended_within(session, Duration::from_secs(2)).await
}

async fn ended_within(session: JoinHandle<Ended>, bound: Duration) -> Ended {
    tokio::time::timeout(bound, session)
        .await
        .expect("the session to end")
        .expect("the session not to panic")
}

/// Spelled here and not read from the fixture, with scopes that are neither
/// what pairing approved nor in its order.
fn hello() -> Hello {
    Hello {
        v: 1,
        install_id: "inst-7f3a9c".to_owned(),
        app_version: "1.4.0".to_owned(),
        os: "linux".to_owned(),
        arch: "x86_64".to_owned(),
        scopes: vec![Scope::Gates, Scope::Read],
    }
}

/// The row as pairing left it, which is also what a session that wrote
/// nothing leaves.
fn never_connected() -> InstanceView {
    InstanceView {
        install_id: None,
        version: None,
        os: None,
        arch: None,
        scopes: vec![Scope::Read, Scope::Spend, Scope::Gates],
        present: false,
        last_seen: None,
    }
}

#[tokio::test]
async fn hello_records_the_instance_and_marks_it_present() {
    let sessions = Sessions::new();
    assert_eq!(sessions.hub.instance(), never_connected());
    let (mut instance, session) = sessions.connect();

    instance.send(&hello());
    instance.expect_ack(0).await;

    assert_eq!(
        sessions.hub.instance(),
        InstanceView {
            install_id: Some("inst-7f3a9c".to_owned()),
            version: Some("1.4.0".to_owned()),
            os: Some("linux".to_owned()),
            arch: Some("x86_64".to_owned()),
            scopes: vec![Scope::Gates, Scope::Read],
            present: true,
            last_seen: Some(NOW),
        }
    );
    assert_eq!(sessions.hub.cursor(), 0);

    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
}

#[tokio::test]
async fn a_disconnect_marks_the_instance_absent_and_records_last_seen() {
    const LATER: u64 = NOW + 75;
    let sessions = Sessions::new();
    let (mut instance, session) = sessions.connect();
    instance.send(&hello());
    instance.expect_ack(0).await;
    let connected = sessions.hub.instance();
    assert!(connected.present);
    assert_eq!(connected.last_seen, Some(NOW));

    sessions.clock_reads(LATER);
    instance.finish().await;

    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(
        sessions.hub.instance(),
        InstanceView {
            present: false,
            last_seen: Some(LATER),
            ..connected
        }
    );
}

/// What an instance can send that is not allowed where it is sent.
enum Sent {
    Frame(Value),
    Text(&'static str),
    Binary,
}

impl Sent {
    fn by(&self, instance: &FakeInstance) {
        match self {
            Self::Frame(frame) => instance.send(frame),
            Self::Text(text) => instance.send_text(text),
            Self::Binary => instance.send_binary(),
        }
    }
}

fn of_version(name: &str, version: u32) -> Value {
    let mut frame = fixture(name);
    frame["v"] = json!(version);
    frame
}

#[tokio::test]
async fn a_frame_before_hello_closes_the_socket_and_writes_nothing() {
    let unrecognised = Violation::Frame(FrameError::Unrecognised);
    let another_version = Violation::Frame(FrameError::Version(2));

    let before_hello = [
        (
            "a run event",
            Sent::Frame(fixture("run_event.json")),
            Violation::BeforeHello,
        ),
        (
            "a snapshot",
            Sent::Frame(fixture("snapshot.json")),
            Violation::BeforeHello,
        ),
        (
            "a request result",
            Sent::Frame(fixture("request_result_accepted.json")),
            Violation::BeforeHello,
        ),
        (
            "a hello of another version",
            Sent::Frame(of_version("hello.json", 2)),
            another_version,
        ),
        (
            "a frame of the Hub's own",
            Sent::Frame(fixture("cursor_ack.json")),
            unrecognised,
        ),
        ("text that is not JSON", Sent::Text("hello"), unrecognised),
        ("a binary frame", Sent::Binary, Violation::Binary),
    ];
    for (what, sent, violation) in before_hello {
        let sessions = Sessions::new();
        let (mut instance, session) = sessions.connect();

        sent.by(&instance);
        instance.expect_close(1008).await;
        instance.finish().await;

        assert_eq!(ended(session).await, Ended::Refused(violation), "{what}");
        assert_eq!(sessions.hub.instance(), never_connected(), "{what}");
        assert_eq!(sessions.hub.cursor(), 0, "{what}");
        assert_eq!(sessions.hub.snapshot(), None, "{what}");
    }

    let after_hello = [
        (
            "a second hello",
            Sent::Frame(fixture("hello.json")),
            Violation::SecondHello,
        ),
        (
            "a hello of another version",
            Sent::Frame(of_version("hello.json", 2)),
            another_version,
        ),
        (
            "a run event of another version",
            Sent::Frame(of_version("run_event.json", 2)),
            another_version,
        ),
        (
            "text that is not JSON",
            Sent::Text("{\"v\":1"),
            unrecognised,
        ),
        ("a binary frame", Sent::Binary, Violation::Binary),
    ];
    for (what, sent, violation) in after_hello {
        let sessions = Sessions::new();
        let (mut instance, session) = sessions.connect();
        instance.send(&hello());
        instance.expect_ack(0).await;
        let greeted = sessions.hub.instance();
        sessions.clock_reads(NOW + 5);

        sent.by(&instance);
        instance.expect_close(1008).await;
        instance.finish().await;

        assert_eq!(ended(session).await, Ended::Refused(violation), "{what}");
        assert_eq!(
            sessions.hub.instance(),
            InstanceView {
                present: false,
                last_seen: Some(NOW + 5),
                ..greeted
            },
            "{what}"
        );
    }
}

#[tokio::test]
async fn a_hello_the_store_cannot_record_closes_the_socket_and_is_not_acked() {
    let sessions = Sessions::new();
    let (mut instance, session) = sessions.connect();
    // Any error will do: a session is told that a call failed and not how.
    sessions.hub.fail_next_call(StoreError::TimeTooLarge(NOW));

    instance.send(&hello());
    instance.expect_close(1011).await;
    instance.finish().await;

    assert_eq!(ended(session).await, Ended::StoreFailed);
    assert_eq!(sessions.hub.instance(), never_connected());
}

fn event(feature_id: &str, offset: u64) -> RunEvent {
    RunEvent {
        v: 1,
        feature_id: feature_id.to_owned(),
        offset,
        kind: RunEventKind::StepStarted,
        occurred_at: NOW,
        payload: RunEventPayload::default(),
    }
}

fn row(event: &RunEvent) -> MirrorRow {
    MirrorRow {
        feature_id: event.feature_id.clone(),
        offset: event.offset,
        event: serde_json::to_string(event).expect("a run event serialises"),
    }
}

fn stored_offsets(hub: &MemoryHub) -> Vec<u64> {
    hub.mirror().into_iter().map(|row| row.offset).collect()
}

#[tokio::test]
async fn hello_is_acked_with_the_stored_cursor() {
    let sessions = Sessions::new();
    let (mut instance, session) = sessions.greeted(0).await;
    for offset in [4, 9] {
        instance.send(&event("f-1", offset));
        instance.expect_ack(offset).await;
    }
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);

    // Twice, because a `hello` that moved the cursor would show on the next.
    for _ in 0..2 {
        let (instance, session) = sessions.greeted(9).await;
        instance.finish().await;
        assert_eq!(ended(session).await, Ended::PeerGone);
        assert_eq!(sessions.hub.cursor(), 9);
    }
}

#[tokio::test]
async fn resent_events_create_no_duplicate_rows_and_each_ack_names_the_highest_stored_offset() {
    let sessions = Sessions::new();
    let (mut instance, session) = sessions.greeted(0).await;
    let events = [event("f-1", 1), event("f-2", 2), event("f-1", 3)];
    for event in &events {
        instance.send(event);
        instance.expect_ack(event.offset).await;
    }
    let stored = sessions.hub.mirror();
    assert_eq!(stored, [row(&events[0]), row(&events[2]), row(&events[1])]);

    for event in &events {
        instance.send(event);
        instance.expect_ack(3).await;
    }
    assert_eq!(sessions.hub.mirror_len(), 3);
    assert_eq!(sessions.hub.mirror(), stored);
    assert_eq!(sessions.hub.cursor(), 3);

    // The reason an instance resends at all: a write that failed was never
    // acked, so it comes back and sends from the cursor it is given.
    let unacked = event("f-1", 4);
    sessions.hub.fail_next_call(StoreError::TimeTooLarge(NOW));
    instance.send(&unacked);
    instance.expect_close(1011).await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::StoreFailed);
    assert_eq!(sessions.hub.cursor(), 3);
    assert_eq!(sessions.hub.mirror(), stored);
    assert!(!sessions.hub.instance().present);

    let (mut instance, session) = sessions.greeted(3).await;
    instance.send(&events[2]);
    instance.expect_ack(3).await;
    assert_eq!(sessions.hub.mirror_len(), 3);
    instance.send(&unacked);
    instance.expect_ack(4).await;
    assert_eq!(sessions.hub.mirror_len(), 4);
    assert_eq!(sessions.hub.cursor(), 4);

    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
}

#[tokio::test]
async fn sparse_offsets_advance_the_cursor_without_stalling() {
    let sessions = Sessions::new();
    let (mut instance, session) = sessions.greeted(0).await;

    instance.send(&event("f-1", 0));
    instance.expect_ack(0).await;
    assert_eq!(stored_offsets(&sessions.hub), [0]);

    for offset in [3, 7, 20] {
        instance.send(&event("f-1", offset));
        instance.expect_ack(offset).await;
        assert_eq!(sessions.hub.cursor(), offset);
    }

    instance.send(&event("f-1", 5));
    instance.expect_ack(20).await;
    assert_eq!(stored_offsets(&sessions.hub), [0, 3, 5, 7, 20]);
    assert_eq!(sessions.hub.cursor(), 20);

    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
}

#[tokio::test]
async fn a_mirror_row_holds_the_parsed_event_and_a_frame_with_an_unknown_field_is_refused() {
    const STORED_AT: u64 = 4812;
    // Keys out of the type's order, whitespace a serialiser would not write
    // and two optional ids left out: none of it may reach the row.
    let frame = r#"{ "payload": { "ticket_id": "t-3" }, "occurred_at": 1790213898,
        "kind": "step_failed", "offset": 4812, "feature_id": "f-1791043410984", "v": 1 }"#;
    let parsed: RunEvent = serde_json::from_str(frame).expect("the frame is a run event");
    assert_eq!(parsed.offset, STORED_AT);
    assert_ne!(row(&parsed).event, frame);

    let sessions = Sessions::new();
    let (mut instance, session) = sessions.greeted(0).await;
    instance.send_text(frame);
    instance.expect_ack(STORED_AT).await;
    assert_eq!(sessions.hub.mirror(), [row(&parsed)]);

    let of_another_feature = RunEvent {
        feature_id: "f-2".to_owned(),
        ..parsed.clone()
    };
    let told_differently = RunEvent {
        kind: RunEventKind::StepCompleted,
        ..parsed.clone()
    };
    for event in [&of_another_feature, &told_differently] {
        instance.send(event);
        instance.expect_ack(STORED_AT).await;
    }
    let stored = sessions.hub.mirror();
    assert_eq!(stored, [row(&parsed), row(&of_another_feature)]);
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);

    // Each at an offset the mirror does not hold, so that a frame which got
    // as far as the store would leave a row behind.
    let mut with_a_cost = fixture("run_event.json");
    with_a_cost["offset"] = json!(STORED_AT + 1);
    with_a_cost["cost_usd"] = json!(1.25);
    let mut with_a_summary = fixture("run_event.json");
    with_a_summary["offset"] = json!(STORED_AT + 2);
    with_a_summary["payload"]["summary"] = json!("approved");
    let past_bigint = i64::MAX as u64 + 1;
    let mut beyond_the_key = fixture("run_event.json");
    beyond_the_key["offset"] = json!(past_bigint);
    let refused = [
        (
            "a field the event lacks",
            with_a_cost,
            FrameError::Unrecognised,
        ),
        (
            "a field the payload lacks",
            with_a_summary,
            FrameError::Unrecognised,
        ),
        (
            "an offset past BIGINT",
            beyond_the_key,
            FrameError::OffsetTooLarge(past_bigint),
        ),
    ];
    for (what, frame, error) in refused {
        let (mut instance, session) = sessions.greeted(STORED_AT).await;
        instance.send(&frame);
        instance.expect_close(1008).await;
        instance.finish().await;

        assert_eq!(
            ended(session).await,
            Ended::Refused(Violation::Frame(error)),
            "{what}"
        );
        assert_eq!(sessions.hub.mirror(), stored, "{what}");
        assert_eq!(sessions.hub.cursor(), STORED_AT, "{what}");
    }
}

/// Each of these would fail at the store, where a failure is answered
/// `1011` and the instance resends the same frame for ever; the frame is
/// refused before it gets there.
#[tokio::test]
async fn a_frame_the_store_could_never_take_is_refused_and_stores_nothing() {
    const STORED_AT: u64 = 7;
    let snapshot: Snapshot = serde_json::from_value(fixture("snapshot.json")).expect("a snapshot");
    let at_the_bound = event(&"f".repeat(MAX_FEATURE_ID_BYTES), STORED_AT);

    let sessions = Sessions::new();
    let (mut instance, session) = sessions.greeted(0).await;
    instance.send(&snapshot);
    instance.send(&at_the_bound);
    instance.expect_ack(STORED_AT).await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(sessions.hub.mirror(), [row(&at_the_bound)]);
    assert_eq!(sessions.hub.snapshot(), held(&snapshot, NOW));
    let departed = sessions.hub.instance();
    assert_eq!((departed.present, departed.last_seen), (false, Some(NOW)));

    let mut with_a_nul_in_a_project = snapshot.clone();
    with_a_nul_in_a_project.projects[0].name = "demeteo\u{0}".to_owned();
    let with_a_nul_in_a_payload = RunEvent {
        payload: RunEventPayload {
            step_id: Some("s-\u{0}".to_owned()),
            ..RunEventPayload::default()
        },
        ..event("f-1", STORED_AT + 2)
    };
    // Each at an offset the mirror does not hold, so that a frame which got
    // as far as the store would be acked and leave a row behind.
    let refused: [(&str, Value, FrameError); 4] = [
        (
            "U+0000 in the feature id",
            json!(event("f-\u{0}", STORED_AT + 1)),
            FrameError::Nul,
        ),
        (
            "U+0000 in a payload id",
            json!(with_a_nul_in_a_payload),
            FrameError::Nul,
        ),
        (
            "a feature id one byte over the bound",
            json!(event(&"f".repeat(MAX_FEATURE_ID_BYTES + 1), STORED_AT + 3)),
            FrameError::FeatureIdTooLong(MAX_FEATURE_ID_BYTES + 1),
        ),
        (
            "U+0000 in a snapshot's project",
            json!(with_a_nul_in_a_project),
            FrameError::Nul,
        ),
    ];
    for (what, frame, error) in refused {
        let (mut instance, session) = sessions.greeted(STORED_AT).await;
        instance.send(&frame);
        instance.expect_close(1008).await;
        instance.finish().await;

        assert_eq!(
            ended(session).await,
            Ended::Refused(Violation::Frame(error)),
            "{what}"
        );
        assert_eq!(sessions.hub.instance(), departed, "{what}");
        assert_eq!(sessions.hub.cursor(), STORED_AT, "{what}");
        assert_eq!(sessions.hub.mirror(), [row(&at_the_bound)], "{what}");
        assert_eq!(sessions.hub.snapshot(), held(&snapshot, NOW), "{what}");
    }

    let (mut instance, session) = sessions.connect();
    instance.send(&Hello {
        install_id: "inst-\u{0}".to_owned(),
        ..hello()
    });
    instance.expect_close(1008).await;
    instance.finish().await;
    assert_eq!(
        ended(session).await,
        Ended::Refused(Violation::Frame(FrameError::Nul))
    );
    assert_eq!(sessions.hub.instance(), departed);
    assert_eq!(sessions.hub.cursor(), STORED_AT);
    assert_eq!(sessions.hub.mirror(), [row(&at_the_bound)]);
    assert_eq!(sessions.hub.snapshot(), held(&snapshot, NOW));
}

fn held(snapshot: &Snapshot, updated_at: u64) -> Option<SnapshotRow> {
    Some(SnapshotRow {
        body: serde_json::to_string(snapshot).expect("a snapshot serialises"),
        updated_at,
    })
}

#[tokio::test]
async fn a_snapshot_replaces_the_previous_one() {
    let text = protocol_fixtures()
        .remove("snapshot.json")
        .expect("the snapshot fixture");
    let first: Snapshot = as_written("snapshot.json", &text);
    let mut second = first.clone();
    second.projects.truncate(1);
    second.projects[0].name = "renamed".to_owned();
    second.workflows.clear();
    assert_ne!(first, second);
    // The fixture is laid out over many lines, which no row may keep.
    assert_ne!(held(&first, NOW).expect("a row").body, text);

    let sessions = Sessions::new();
    let (mut instance, session) = sessions.greeted(0).await;
    assert_eq!(sessions.hub.snapshot(), None);

    // A snapshot is answered with nothing, so the ack of an event sent after
    // it is what says the session is done with it: frames are taken in order.
    instance.send_text(&text);
    instance.send(&event("f-1", 1));
    instance.expect_ack(1).await;
    assert_eq!(sessions.hub.snapshot(), held(&first, NOW));

    sessions.clock_reads(NOW + 40);
    instance.send(&second);
    instance.send(&event("f-1", 2));
    instance.expect_ack(2).await;
    assert_eq!(sessions.hub.snapshot(), held(&second, NOW + 40));

    sessions.hub.fail_next_call(StoreError::TimeTooLarge(NOW));
    instance.send(&first);
    instance.expect_close(1011).await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::StoreFailed);
    assert_eq!(sessions.hub.snapshot(), held(&second, NOW + 40));
    assert_eq!(sessions.hub.mirror_len(), 2);

    let unannounced = Sessions::new();
    let (mut instance, session) = unannounced.connect();
    instance.send(&first);
    instance.expect_close(1008).await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::Refused(Violation::BeforeHello));
    assert_eq!(unannounced.hub.snapshot(), None);
    assert_eq!(unannounced.hub.instance(), never_connected());
}

/// The store double's half of displacement, driven by hand: the session that
/// is displaced arrives with a later ticket, and the condition it leans on
/// is the double's to apply from the start.
#[tokio::test]
async fn the_memory_hub_keeps_an_instance_present_when_a_stale_connection_departs() {
    let sessions = Sessions::new();
    let store = sessions.hub.store();
    let earlier = registered(&sessions.connections, sessions.instance).connection;
    let later = registered(&sessions.connections, sessions.instance).connection;
    let hello = hello();
    let first = store.greeted(&hello, earlier, NOW).await;
    assert_eq!(first.expect("a first hello"), 0);
    let second = store.greeted(&hello, later, NOW + 1).await;
    assert_eq!(second.expect("a second hello"), 0);
    let present = sessions.hub.instance();
    assert_eq!((present.present, present.last_seen), (true, Some(NOW + 1)));

    store
        .departed(earlier, NOW + 2)
        .await
        .expect("a departure that changes nothing");
    assert_eq!(sessions.hub.instance(), present);

    sessions.hub.fail_next_call(StoreError::TimeTooLarge(NOW));
    assert!(store.departed(later, NOW + 3).await.is_err());
    assert_eq!(sessions.hub.instance(), present);

    store.departed(later, NOW + 4).await.expect("a departure");
    assert_eq!(
        sessions.hub.instance(),
        InstanceView {
            present: false,
            last_seen: Some(NOW + 4),
            ..present
        }
    );
}

/// The earlier session is held back until its successor has said `hello`,
/// which is the order a departure has to survive: one that named the
/// instance and not the connection would mark the successor absent.
#[tokio::test]
async fn a_displaced_connection_does_not_mark_its_successor_absent() {
    let sessions = Sessions::new();
    let (mut earlier, displaced) = sessions.session();
    let mut displaced = Box::pin(displaced);
    tokio::select! {
        ended = &mut displaced => panic!("the earlier session ended before it was displaced: {ended:?}"),
        () = async {
            earlier.send(&hello());
            earlier.expect_ack(0).await;
        } => {}
    }
    assert_eq!(sessions.hub.instance().last_seen, Some(NOW));

    sessions.clock_reads(NOW + 10);
    let (later, session) = sessions.greeted(0).await;
    let present = sessions.hub.instance();
    assert_eq!((present.present, present.last_seen), (true, Some(NOW + 10)));

    sessions.clock_reads(NOW + 20);
    assert_eq!(
        ended(tokio::spawn(displaced)).await,
        Ended::Asked(Closing::Displaced)
    );
    earlier.expect_close(4002).await;
    earlier.finish().await;
    assert_eq!(sessions.hub.instance(), present);

    sessions.clock_reads(NOW + 30);
    later.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(
        sessions.hub.instance(),
        InstanceView {
            present: false,
            last_seen: Some(NOW + 30),
            ..present
        }
    );
}

/// The first half is the control: the same limits and a peer that answers,
/// kept past the deadline. Without it a session that gave up on every peer
/// after `pong_deadline` would pass.
///
/// The control runs in real time, and a stall of the runner longer than the
/// gap between a pong and the deadline ends it as `Silent`. That gap is
/// `pong_deadline` less `ping_interval`, and it is held well over a second.
#[tokio::test]
async fn a_peer_that_stops_answering_pings_is_marked_absent() {
    const DEADLINE: Duration = Duration::from_millis(1500);
    let sessions = Sessions::new().with(Limits {
        ping_interval: Duration::from_millis(100),
        pong_deadline: DEADLINE,
    });
    let (instance, session) = sessions.greeted(0).await;

    tokio::time::sleep(DEADLINE * 2).await;
    assert!(!session.is_finished());
    assert!(sessions.hub.instance().present);

    sessions.clock_reads(NOW + 120);
    instance.go_silent();

    assert_eq!(ended_within(session, DEADLINE * 2).await, Ended::Silent);
    let gone = sessions.hub.instance();
    assert_eq!((gone.present, gone.last_seen), (false, Some(NOW + 120)));
    instance.finish().await;
}

/// No ping is sent here, so nothing but the token's own expiry wakes the
/// session: its wait is bounded by the second that was left when it last read
/// the clock, and that second is spent in real time.
#[tokio::test]
async fn a_session_is_closed_when_the_token_it_was_admitted_with_expires() {
    let sessions = Sessions::new();
    let (mut instance, session) = sessions.greeted(0).await;

    // The expiry is looked at between two frames, so the second ack says the
    // session read the clock a second short of it and went on.
    sessions.clock_reads(TOKEN_EXPIRES_AT - 1);
    instance.send(&event("f-1", 1));
    instance.expect_ack(1).await;
    instance.send(&event("f-1", 2));
    instance.expect_ack(2).await;
    assert!(sessions.hub.instance().present);

    sessions.clock_reads(TOKEN_EXPIRES_AT);
    let mut instance = instance.waiting_at_most(Duration::from_secs(5));
    instance.expect_close(4001).await;
    instance.finish().await;

    assert_eq!(ended(session).await, Ended::TokenExpired);
    let gone = sessions.hub.instance();
    assert_eq!(
        (gone.present, gone.last_seen),
        (false, Some(TOKEN_EXPIRES_AT))
    );
    assert_eq!(sessions.hub.mirror_len(), 2);
}

#[tokio::test]
async fn closing_every_connection_ends_each_session_and_marks_it_absent() {
    let connections = Arc::new(Connections::default());
    let [one, other, unannounced] = [(); 3].map(|()| Sessions::seated_in(&connections));
    let (mut first, first_session) = one.greeted(0).await;
    let (mut second, second_session) = other.greeted(0).await;
    let (mut silent, silent_session) = unannounced.connect();
    assert!(one.hub.instance().present);
    assert!(other.hub.instance().present);
    one.clock_reads(NOW + 5);
    other.clock_reads(NOW + 9);

    connections.close_all();

    for instance in [&mut first, &mut second, &mut silent] {
        instance.expect_close(1001).await;
    }
    for session in [first_session, second_session, silent_session] {
        assert_eq!(ended(session).await, Ended::Asked(Closing::ShuttingDown));
    }
    for (sessions, left_at) in [(&one, NOW + 5), (&other, NOW + 9)] {
        let gone = sessions.hub.instance();
        assert_eq!((gone.present, gone.last_seen), (false, Some(left_at)));
    }
    assert_eq!(unannounced.hub.instance(), never_connected());
    first.finish().await;
    second.finish().await;
    silent.finish().await;
}

/// The limits of a session whose writes are given up on after `deadline`,
/// and which pings no more often than [`UNHURRIED`] does.
fn writes_given_up_after(deadline: Duration) -> Limits {
    Limits {
        pong_deadline: deadline,
        ..UNHURRIED
    }
}

/// The event is stored and its ack is what stalls. The wait beside it is the
/// second of the token that was left when the ack was begun, spent in real
/// time; the close that follows stalls too, since the peer reads that no
/// more than the ack, and is waited for until the deadline.
#[tokio::test]
async fn a_peer_that_stops_reading_does_not_hold_a_session_past_its_token() {
    const DEADLINE: Duration = Duration::from_secs(2);
    let sessions = Sessions::new().with(writes_given_up_after(DEADLINE));
    let (instance, session) = sessions.greeted(0).await;

    sessions.clock_reads(TOKEN_EXPIRES_AT - 1);
    instance.stop_reading();
    instance.send(&event("f-1", 1));
    tokio::time::sleep(IMPATIENT).await;
    assert_eq!(sessions.hub.mirror_len(), 1);
    sessions.clock_reads(TOKEN_EXPIRES_AT);

    assert_eq!(
        ended_within(session, DEADLINE * 2).await,
        Ended::TokenExpired
    );
    let gone = sessions.hub.instance();
    assert_eq!(
        (gone.present, gone.last_seen),
        (false, Some(TOKEN_EXPIRES_AT))
    );
    instance.finish().await;
}

/// Each peer here is one whose frames the deadline of the read would go on
/// hearing for as long as it liked. What it does not read is what counts.
#[tokio::test]
async fn a_peer_that_stops_reading_is_given_up_on_at_the_deadline() {
    const DEADLINE: Duration = Duration::from_millis(500);
    let pinged_often = Limits {
        ping_interval: Duration::from_millis(10),
        pong_deadline: DEADLINE,
    };
    let stalled = [
        ("an ack", writes_given_up_after(DEADLINE), true),
        ("a ping", pinged_often, false),
    ];
    for (what, limits, sends_an_event) in stalled {
        let sessions = Sessions::new().with(limits);
        let (instance, session) = sessions.greeted(0).await;
        sessions.clock_reads(NOW + 30);

        instance.stop_reading();
        if sends_an_event {
            instance.send(&event("f-1", 1));
        }

        assert_eq!(ended(session).await, Ended::Silent, "{what}");
        let gone = sessions.hub.instance();
        assert_eq!(
            (gone.present, gone.last_seen),
            (false, Some(NOW + 30)),
            "{what}"
        );
        instance.finish().await;
    }
}

#[tokio::test]
async fn a_stalled_send_does_not_outlive_a_request_to_end() {
    let connections = Arc::new(Connections::default());
    let sessions =
        Sessions::seated_in(&connections).with(writes_given_up_after(Duration::from_secs(1)));
    let (instance, session) = sessions.greeted(0).await;
    sessions.clock_reads(NOW + 5);
    instance.stop_reading();
    instance.send(&event("f-1", 1));
    tokio::time::sleep(IMPATIENT).await;
    assert_eq!(sessions.hub.mirror_len(), 1);

    connections.close_all();

    assert_eq!(ended(session).await, Ended::Asked(Closing::ShuttingDown));
    let gone = sessions.hub.instance();
    assert_eq!((gone.present, gone.last_seen), (false, Some(NOW + 5)));
    instance.finish().await;
}

#[tokio::test]
async fn a_failed_ack_or_ping_ends_the_session_and_marks_it_absent() {
    let pinged_often = Limits {
        ping_interval: Duration::from_millis(10),
        ..UNHURRIED
    };
    let failed = [("an ack", UNHURRIED, true), ("a ping", pinged_often, false)];
    for (what, limits, sends_an_event) in failed {
        let sessions = Sessions::new().with(limits);
        let (instance, session) = sessions.greeted(0).await;
        sessions.clock_reads(NOW + 15);

        instance.refuse_sends();
        if sends_an_event {
            instance.send(&event("f-1", 1));
        }

        assert_eq!(ended(session).await, Ended::PeerGone, "{what}");
        let gone = sessions.hub.instance();
        assert_eq!(
            (gone.present, gone.last_seen),
            (false, Some(NOW + 15)),
            "{what}"
        );
        instance.finish().await;
    }
}

/// A Hub whose pool never connects. Must be made inside a Tokio runtime,
/// as `lazy_state` requires.
fn unconnected_state() -> AppState {
    let rp = RelyingParty {
        rp_id: "hub.example".to_owned(),
        origin: "https://hub.example".to_owned(),
    };
    lazy_state("postgres://hub@db.invalid/hub", rp).expect("a parsable database URL")
}

/// `startup::serve_until_stopped` over the Hub's real router on a loopback
/// port, stopped as soon as it is serving, and how long it took to return.
async fn stopped(state: &AppState, drain: Duration) -> Duration {
    let web_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-web-dir");
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("a free loopback port");
    let (stop, stop_asked) = oneshot::channel::<()>();
    let serving = startup::serve_until_stopped(
        listener,
        routes::router(state.clone(), &web_dir),
        state.clone(),
        async {
            stop_asked.await.ok();
        },
        drain,
    );
    let began = Instant::now();
    stop.send(()).expect("a stop not yet dropped");
    tokio::time::timeout(Duration::from_secs(10), serving)
        .await
        .expect("the stop to return")
        .expect("a stop that serves without an I/O error");
    began.elapsed()
}

/// Long enough that a stop which did not wait for it returns well inside it.
const SLOW_DEPARTURE: Duration = Duration::from_millis(150);

/// Each session is spawned and its handle dropped at once, as axum spawns
/// one: nothing but the stop is left to wait for it.
#[tokio::test]
async fn a_graceful_stop_returns_only_after_every_session_has_left() {
    let state = unconnected_state();
    let [one, other, unannounced] = [(); 3].map(|()| Sessions::seated_at(&state));
    let (mut first, _) = one.greeted(0).await;
    let (mut second, _) = other.greeted(0).await;
    let (mut silent, _) = unannounced.connect();
    one.clock_reads(NOW + 5);
    other.clock_reads(NOW + 9);
    for sessions in [&one, &other] {
        assert!(sessions.hub.instance().present);
        sessions.hub.departing_slowly(SLOW_DEPARTURE);
    }

    stopped(&state, Duration::from_secs(SHUTDOWN_DRAIN_SECS)).await;

    for (sessions, left_at) in [(&one, NOW + 5), (&other, NOW + 9)] {
        let gone = sessions.hub.instance();
        assert_eq!((gone.present, gone.last_seen), (false, Some(left_at)));
    }
    assert_eq!(unannounced.hub.instance(), never_connected());
    for instance in [&mut first, &mut second, &mut silent] {
        instance.expect_close(1001).await;
    }
    first.finish().await;
    second.finish().await;
    silent.finish().await;
}

#[tokio::test]
async fn a_graceful_stop_gives_up_on_a_session_that_outlasts_the_bound() {
    const BOUND: Duration = Duration::from_millis(100);
    const DEPARTURE: Duration = Duration::from_secs(1);
    let state = unconnected_state();
    let sessions = Sessions::seated_at(&state);
    let (instance, _) = sessions.greeted(0).await;
    sessions.hub.departing_slowly(DEPARTURE);

    let took = stopped(&state, BOUND).await;

    assert!(took >= BOUND, "returned after {took:?}");
    assert!(took < DEPARTURE, "returned after {took:?}");
    assert!(sessions.hub.instance().present);
    let mut instance = instance.waiting_at_most(DEPARTURE * 2);
    instance.expect_close(1001).await;
    instance.finish().await;
}

#[tokio::test]
async fn a_registry_is_drained_only_when_every_registration_is_gone() {
    let connections = Arc::new(Connections::default());
    let drained = |connections: &Arc<Connections>| {
        let connections = Arc::clone(connections);
        tokio::spawn(async move { connections.drained().await })
    };
    tokio::time::timeout(IMPATIENT, drained(&connections))
        .await
        .expect("an empty registry to be drained at once")
        .expect("a drain not to panic");

    let instance = InstanceId::generate();
    let displaced = registered(&connections, instance);
    let successor = registered(&connections, instance);
    let mut waiting = drained(&connections);
    drop(successor);
    assert!(
        tokio::time::timeout(IMPATIENT, &mut waiting).await.is_err(),
        "drained while a displaced registration was alive"
    );
    assert_eq!(connections.alive(), 1);

    drop(displaced);
    tokio::time::timeout(IMPATIENT, waiting)
        .await
        .expect("drained once the last registration was dropped")
        .expect("a drain not to panic");
}

/// The name the request-signing key goes by in every key service below.
const SIGNING_KEY: &str = "demeteo-hub-signing";

/// The six kinds: the fixture a payload of each is read from, the tag it
/// travels under, and the scope `docs/HUB.md` §7 files it under. Spelled
/// here and not asked of the Hub, which is what is being checked.
const KINDS: [(&str, &str, Option<Scope>); 6] = [
    (
        "signed_request_start_feature.json",
        "start_feature",
        Some(Scope::Spend),
    ),
    (
        "signed_request_cancel_feature.json",
        "cancel_feature",
        Some(Scope::Spend),
    ),
    (
        "signed_request_set_step_assignment.json",
        "set_step_assignment",
        Some(Scope::Spend),
    ),
    (
        "signed_request_gate_decision.json",
        "gate_decision",
        Some(Scope::Gates),
    ),
    (
        "signed_request_passkey_endorsement.json",
        "passkey_endorsement",
        None,
    ),
    (
        "signed_request_passkey_revocation.json",
        "passkey_revocation",
        None,
    ),
];

/// The payload of a fixture, and nothing else of it: its id, its times and
/// its signature are a made-up Hub's, and an enqueue makes its own.
fn payload_of(name: &str) -> RequestPayload {
    serde_json::from_value(fixture(name)["payload"].clone())
        .unwrap_or_else(|e| panic!("the payload of {name}: {e}"))
}

impl Sessions {
    /// A connection that says `hello` with `scopes` and leaves, so that they
    /// are what the Hub was last told.
    async fn reports(&self, scopes: &[Scope]) {
        let (mut instance, session) = self.connect();
        instance.send(&Hello {
            scopes: scopes.to_vec(),
            ..hello()
        });
        instance.expect_ack(0).await;
        instance.finish().await;
        ended(session).await;
        assert_eq!(self.hub.instance().scopes, scopes);
    }

    /// An enqueue for this instance at what the clock reads, signed by `keys`.
    async fn enqueue(
        &self,
        keys: &MemoryKeys,
        payload: RequestPayload,
    ) -> Result<Enqueued, EnqueueError> {
        requests::enqueued(
            &self.hub.store(),
            keys,
            &SigningKey::new(SIGNING_KEY),
            &self.connections,
            payload,
            self.now.load(Ordering::SeqCst),
        )
        .await
    }

    fn queued_kinds(&self) -> Vec<String> {
        let requests = self.hub.requests();
        requests.into_iter().map(|request| request.kind).collect()
    }
}

#[tokio::test]
async fn enqueueing_a_kind_whose_scope_is_not_granted_is_refused_and_queues_nothing() {
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);
    let scope_sets: [&[Scope]; 8] = [
        &[],
        &[Scope::Read],
        &[Scope::Spend],
        &[Scope::Gates],
        &[Scope::Read, Scope::Spend],
        &[Scope::Read, Scope::Gates],
        &[Scope::Gates, Scope::Spend],
        &[Scope::Read, Scope::Spend, Scope::Gates],
    ];

    for scopes in scope_sets {
        // Pairing approved all three; the gate has to go by the `hello`.
        let sessions = Sessions::new();
        sessions.reports(scopes).await;
        let connection = registered(&sessions.connections, sessions.instance);
        let missing = |needs: &Option<Scope>| needs.filter(|scope| !scopes.contains(scope));

        for (name, _, needs) in KINDS {
            let Some(scope) = missing(&needs) else {
                continue;
            };
            let refused = sessions.enqueue(&keys, payload_of(name)).await;
            let Err(error @ EnqueueError::Refused(EnqueueRefusal::ScopeNotGranted(named))) =
                refused
            else {
                panic!("{name} with {scopes:?}: {refused:?}");
            };
            assert_eq!(named, scope, "{name} with {scopes:?}");
            assert_eq!(
                answered(ApiError::from(error)).await,
                (409, json!({"error": "scope_not_granted"})),
                "{name} with {scopes:?}"
            );
        }
        assert_eq!(sessions.hub.requests(), [], "{scopes:?}");
        assert!(!woken(&connection).await, "{scopes:?}");

        let mut expected = Vec::new();
        for (name, kind, needs) in KINDS {
            if missing(&needs).is_some() {
                continue;
            }
            let queued = sessions.enqueue(&keys, payload_of(name)).await;
            assert!(queued.is_ok(), "{name} with {scopes:?}: {queued:?}");
            expected.push(kind);
            assert_eq!(sessions.queued_kinds(), expected, "{scopes:?}");
            assert!(woken(&connection).await, "{name} with {scopes:?}");
        }
        if scopes.is_empty() {
            assert_eq!(expected, ["passkey_endorsement", "passkey_revocation"]);
        }
    }
}

#[tokio::test]
async fn an_enqueued_request_is_stored_queued_and_signed_by_the_hub_key_over_its_signing_bytes() {
    let sessions = Sessions::new();
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);
    let connection = registered(&sessions.connections, sessions.instance);
    let payload = payload_of("signed_request_start_feature.json");

    let queued = sessions
        .enqueue(&keys, payload.clone())
        .await
        .expect("a kind whose scope is on");

    let [row] = <[RequestView; 1]>::try_from(sessions.hub.requests()).expect("one row");
    assert_eq!(
        queued,
        Enqueued {
            id: row.id,
            expires_at: NOW + 600
        }
    );
    assert_eq!(
        (row.status, row.deliveries, row.outcome),
        (RequestStatus::Queued, 0, None)
    );
    assert_eq!(
        (row.kind.as_str(), row.expires_at),
        ("start_feature", NOW + 600)
    );
    let envelope = &row.envelope;
    assert_eq!(
        *envelope,
        SignedRequest {
            v: 1,
            request_id: base64url_encode(&row.id),
            instance_id: sessions.instance.0.to_string(),
            issued_at: NOW,
            expires_at: NOW + 600,
            payload,
            signature: envelope.signature.clone(),
        }
    );
    assert_eq!(envelope.expires_at, envelope.issued_at + 600);
    assert!(woken(&connection).await);

    let public_key = keys
        .verifying_key(SIGNING_KEY)
        .await
        .expect("the configured key has a public half");
    let signed = envelope.signing_bytes().expect("signing bytes");
    let signature = base64url_decode(&envelope.signature).expect("unpadded base64url");
    assert_eq!(signature.len(), 64);
    assert!(verifies_ed25519(&public_key, &signed, &signature));
    assert!(!verifies_ed25519(
        &public_key,
        &Sha256::digest(&signed),
        &signature
    ));
    let of_another_hub = MemoryKeys::with_keys([SIGNING_KEY])
        .verifying_key(SIGNING_KEY)
        .await
        .expect("a key of the same name");
    assert!(!verifies_ed25519(&of_another_hub, &signed, &signature));
    let mut good_for_longer = envelope.clone();
    good_for_longer.expires_at += 1;
    let altered = good_for_longer.signing_bytes().expect("signing bytes");
    assert!(!verifies_ed25519(&public_key, &altered, &signature));

    sessions.clock_reads(NOW + 5);
    let second = sessions
        .enqueue(&keys, payload_of("signed_request_cancel_feature.json"))
        .await
        .expect("a second request");

    assert_ne!(second.id, queued.id);
    let [first, last] = <[RequestView; 2]>::try_from(sessions.hub.requests()).expect("two rows");
    assert_eq!(first, row);
    assert_eq!(
        (last.id, last.kind.as_str(), last.envelope.issued_at),
        (second.id, "cancel_feature", NOW + 5)
    );
}

#[tokio::test]
async fn a_sealed_key_service_refuses_the_enqueue_and_queues_nothing() {
    let sessions = Sessions::new();
    let connection = registered(&sessions.connections, sessions.instance);
    let sealed = MemoryKeys::sealed();

    for (name, _, _) in KINDS {
        let refused = sessions.enqueue(&sealed, payload_of(name)).await;
        let Err(error @ EnqueueError::KeyService(KeyServiceError::Sealed)) = refused else {
            panic!("{name}: {refused:?}");
        };
        assert_eq!(
            answered(ApiError::from(error)).await,
            (503, json!({"error": "key_service_unavailable"})),
            "{name}"
        );
    }
    assert_eq!(sessions.hub.requests(), []);
    assert!(!woken(&connection).await);

    // Retryable: the same request beside a key service that answers.
    let unsealed = MemoryKeys::with_keys([SIGNING_KEY]);
    let retried = sessions.enqueue(&unsealed, payload_of(KINDS[0].0)).await;
    assert!(retried.is_ok(), "{retried:?}");
    assert_eq!(sessions.queued_kinds(), [KINDS[0].1]);
    assert!(woken(&connection).await);
}

impl Sessions {
    /// A request of the kind `fixture` holds, queued at what the clock
    /// reads, and the row it was stored as.
    async fn queues(&self, keys: &MemoryKeys, fixture: &str) -> RequestView {
        let queued = self
            .enqueue(keys, payload_of(fixture))
            .await
            .unwrap_or_else(|e| panic!("{fixture} was not queued: {e}"));
        let rows = self.hub.requests();
        let row = rows.last().expect("the row of the request just queued");
        assert_eq!(row.id, queued.id);
        row.clone()
    }

    /// What became of every request of the instance, oldest first: its
    /// status, how often it was sent, and when it last was.
    fn deliveries(&self) -> Vec<(RequestStatus, u32, Option<u64>)> {
        let requests = self.hub.requests();
        requests
            .into_iter()
            .map(|request| (request.status, request.deliveries, request.last_sent_at))
            .collect()
    }
}

#[tokio::test]
async fn a_request_enqueued_while_offline_is_delivered_on_the_next_connection() {
    let sessions = Sessions::new();
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);
    let first = sessions
        .queues(&keys, "signed_request_start_feature.json")
        .await;
    sessions.clock_reads(NOW + 5);
    let second = sessions
        .queues(&keys, "signed_request_gate_decision.json")
        .await;
    assert_ne!(first.envelope.request_id, second.envelope.request_id);
    assert_eq!(sessions.deliveries(), [(RequestStatus::Queued, 0, None); 2]);

    // `greeted` reads the ack before anything else, so a request that went
    // out ahead of it fails there.
    sessions.clock_reads(NOW + 30);
    let (mut instance, session) = sessions.greeted(0).await;
    assert_eq!(instance.expect_request().await, first.envelope);
    assert_eq!(instance.expect_request().await, second.envelope);
    instance.finish().await;

    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(
        sessions.deliveries(),
        [(RequestStatus::Queued, 1, Some(NOW + 30)); 2]
    );
}

#[tokio::test]
async fn a_request_enqueued_while_connected_is_delivered_without_a_reconnect() {
    let sessions = Sessions::new();
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);
    let (mut instance, session) = sessions.connect();

    // Connected and not yet greeted: the wake-up waits for the `hello`, and
    // the ack still comes first.
    let early = sessions
        .queues(&keys, "signed_request_passkey_revocation.json")
        .await;
    instance.send(&hello());
    instance.expect_ack(0).await;
    assert_eq!(instance.expect_request().await, early.envelope);

    // Each later wake-up reads the whole queue, the requests already sent
    // on this connection among them, and `finish` refuses a second copy.
    sessions.clock_reads(NOW + 40);
    let later = sessions
        .queues(&keys, "signed_request_gate_decision.json")
        .await;
    assert_eq!(instance.expect_request().await, later.envelope);
    sessions.clock_reads(NOW + 80);
    let last = sessions
        .queues(&keys, "signed_request_passkey_endorsement.json")
        .await;
    assert_eq!(instance.expect_request().await, last.envelope);

    assert!(!session.is_finished());
    assert!(sessions.hub.instance().present);
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(
        sessions.deliveries(),
        [
            (RequestStatus::Queued, 1, Some(NOW)),
            (RequestStatus::Queued, 1, Some(NOW + 40)),
            (RequestStatus::Queued, 1, Some(NOW + 80)),
        ]
    );
}

#[tokio::test]
async fn a_request_past_its_expiry_is_marked_expired_and_never_delivered() {
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);

    let in_time = Sessions::new();
    let live = in_time
        .queues(&keys, "signed_request_cancel_feature.json")
        .await;
    assert_eq!(live.expires_at, NOW + 600);
    in_time.clock_reads(NOW + 599);
    let (mut instance, session) = in_time.greeted(0).await;
    assert_eq!(instance.expect_request().await, live.envelope);
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(
        in_time.deliveries(),
        [(RequestStatus::Queued, 1, Some(NOW + 599))]
    );

    let long_past = Sessions::new();
    long_past
        .queues(&keys, "signed_request_cancel_feature.json")
        .await;
    long_past.clock_reads(NOW + 601);
    let (instance, session) = long_past.greeted(0).await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(long_past.deliveries(), [(RequestStatus::Expired, 0, None)]);

    let late = Sessions::new();
    let overdue = late
        .queues(&keys, "signed_request_cancel_feature.json")
        .await;
    late.clock_reads(NOW + 1);
    let a_second_younger = late
        .queues(&keys, "signed_request_start_feature.json")
        .await;
    assert_eq!(
        (overdue.expires_at, a_second_younger.expires_at),
        (NOW + 600, NOW + 601)
    );

    late.clock_reads(NOW + 600);
    let (mut instance, session) = late.greeted(0).await;
    assert_eq!(instance.expect_request().await, a_second_younger.envelope);
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(
        late.deliveries(),
        [
            (RequestStatus::Expired, 0, None),
            (RequestStatus::Queued, 1, Some(NOW + 600)),
        ]
    );

    // A request that was sent runs out like one that never was, and neither
    // is sent by any later connection.
    late.clock_reads(NOW + 601);
    let (instance, session) = late.greeted(0).await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(
        late.deliveries(),
        [
            (RequestStatus::Expired, 0, None),
            (RequestStatus::Expired, 1, Some(NOW + 600)),
        ]
    );
    assert_eq!(late.hub.requests()[1].envelope, a_second_younger.envelope);

    // The mark is what holds, and not the clock that led to it: one that is
    // set back to before either expiry brings neither request out again.
    late.clock_reads(NOW + 2);
    let (instance, session) = late.greeted(0).await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(
        late.deliveries(),
        [
            (RequestStatus::Expired, 0, None),
            (RequestStatus::Expired, 1, Some(NOW + 600)),
        ]
    );
}

/// An envelope that does not parse is one row's fact and not a failed read:
/// a session that closed `1011` on it would close on every reconnect, since
/// the row is never deleted, and nothing queued behind it would go out.
#[tokio::test]
async fn an_unreadable_queued_request_is_skipped_and_the_ones_behind_it_are_delivered() {
    let sessions = Sessions::new();
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);
    let spoiled = sessions
        .queues(&keys, "signed_request_cancel_feature.json")
        .await;
    let behind = sessions
        .queues(&keys, "signed_request_gate_decision.json")
        .await;
    sessions.hub.spoil_envelope(spoiled.id);

    sessions.clock_reads(NOW + 30);
    let (mut instance, session) = sessions.greeted(0).await;
    assert_eq!(instance.expect_request().await, behind.envelope);

    // A wake-up reads the queue again, the unreadable row first.
    sessions.clock_reads(NOW + 40);
    let later = sessions
        .queues(&keys, "signed_request_passkey_revocation.json")
        .await;
    assert_eq!(instance.expect_request().await, later.envelope);

    assert!(!session.is_finished());
    assert!(sessions.hub.instance().present);
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(
        sessions.deliveries(),
        [
            (RequestStatus::Queued, 0, None),
            (RequestStatus::Queued, 1, Some(NOW + 30)),
            (RequestStatus::Queued, 1, Some(NOW + 40)),
        ]
    );
}

#[tokio::test]
async fn an_unreadable_queued_request_is_marked_expired_once_it_is_overdue() {
    let sessions = Sessions::new();
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);
    let spoiled = sessions
        .queues(&keys, "signed_request_cancel_feature.json")
        .await;
    assert_eq!(spoiled.expires_at, NOW + 600);
    sessions.hub.spoil_envelope(spoiled.id);

    sessions.clock_reads(NOW + 599);
    let (instance, session) = sessions.greeted(0).await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(sessions.deliveries(), [(RequestStatus::Queued, 0, None)]);

    sessions.clock_reads(NOW + 600);
    let (instance, session) = sessions.greeted(0).await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(sessions.deliveries(), [(RequestStatus::Expired, 0, None)]);
}

#[tokio::test]
async fn an_unanswered_request_is_redelivered_unchanged_on_reconnect() {
    let sessions = Sessions::new();
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);
    let queued = sessions
        .queues(&keys, "signed_request_start_feature.json")
        .await;

    sessions.clock_reads(NOW + 10);
    let (mut instance, session) = sessions.greeted(0).await;
    let first = instance.expect_request().await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(
        sessions.deliveries(),
        [(RequestStatus::Queued, 1, Some(NOW + 10))]
    );

    sessions.clock_reads(NOW + 20);
    let (mut instance, session) = sessions.greeted(0).await;
    let again = instance.expect_request().await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);

    assert_eq!(again.request_id, first.request_id);
    assert_eq!(again.request_id, base64url_encode(&queued.id));
    assert_eq!(again.signature, first.signature);
    assert!(!again.signature.is_empty());
    assert_eq!(again, first);
    assert_eq!(again.issued_at, NOW);
    let [row] = <[RequestView; 1]>::try_from(sessions.hub.requests()).expect("one row");
    assert_eq!(
        (row.status, row.deliveries, row.last_sent_at),
        (RequestStatus::Queued, 2, Some(NOW + 20))
    );
    assert_eq!(row.envelope, queued.envelope);
}

/// The send fails on a connection that was greeted, and the wake-up of the
/// enqueue is what makes it: no reconnect stands between the queue and it.
#[tokio::test]
async fn a_request_whose_send_failed_is_not_counted_and_is_sent_again() {
    let sessions = Sessions::new();
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);
    let (instance, session) = sessions.greeted(0).await;
    instance.refuse_sends();

    sessions.clock_reads(NOW + 10);
    let queued = sessions
        .queues(&keys, "signed_request_gate_decision.json")
        .await;

    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(sessions.deliveries(), [(RequestStatus::Queued, 0, None)]);
    assert!(!sessions.hub.instance().present);
    instance.finish().await;

    sessions.clock_reads(NOW + 20);
    let (mut instance, session) = sessions.greeted(0).await;
    let delivered = instance.expect_request().await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);

    assert_eq!(delivered.request_id, base64url_encode(&queued.id));
    assert_eq!(delivered.signature, queued.envelope.signature);
    assert_eq!(delivered, queued.envelope);
    assert_eq!(
        sessions.deliveries(),
        [(RequestStatus::Queued, 1, Some(NOW + 20))]
    );
}

/// The store of a hub, and a failure that waits for the next reading of the
/// queue and meets the call after it. A reading begins with the read, and the
/// call that marks a request follows it with no frame between, so
/// [`MemoryHub::fail_next_call`] alone reaches the read and never the mark.
struct FailingAfterRead {
    hub: MemoryHub,
    store: MemoryStore,
    then: Mutex<Option<StoreError>>,
}

impl FailingAfterRead {
    fn new(hub: &MemoryHub) -> Self {
        Self {
            hub: hub.clone(),
            store: hub.store(),
            then: Mutex::new(Some(StoreError::TimeTooLarge(NOW))),
        }
    }
}

#[async_trait]
impl SocketStore for FailingAfterRead {
    fn instance(&self) -> InstanceId {
        self.store.instance()
    }

    async fn greeted(
        &self,
        hello: &Hello,
        connection: ConnectionId,
        now: u64,
    ) -> Result<u64, StoreError> {
        self.store.greeted(hello, connection, now).await
    }

    async fn ingested(
        &self,
        event: &RunEvent,
        decide: fn(u64, u64) -> u64,
    ) -> Result<u64, StoreError> {
        self.store.ingested(event, decide).await
    }

    async fn snapshot_replaced(&self, snapshot: &Snapshot, now: u64) -> Result<(), StoreError> {
        self.store.snapshot_replaced(snapshot, now).await
    }

    async fn granted_scopes(&self) -> Result<Vec<Scope>, StoreError> {
        self.store.granted_scopes().await
    }

    async fn request_inserted(&self, request: NewRequest) -> Result<(), StoreError> {
        self.store.request_inserted(request).await
    }

    async fn queued_requests(&self) -> Result<Vec<QueuedRequest>, StoreError> {
        let queued = self.store.queued_requests().await?;
        let then = self.then.lock().expect("an unpoisoned failure").take();
        if let Some(error) = then {
            self.hub.fail_next_call(error);
        }
        Ok(queued)
    }

    async fn request_sent(&self, id: RequestId, now: u64) -> Result<(), StoreError> {
        self.store.request_sent(id, now).await
    }

    async fn request_expired(&self, id: RequestId) -> Result<(), StoreError> {
        self.store.request_expired(id).await
    }

    async fn outcome_recorded(
        &self,
        id: RequestId,
        decide: &(dyn Fn(RequestState) -> OutcomeChange + Sync),
        now: u64,
    ) -> Result<bool, StoreError> {
        self.store.outcome_recorded(id, decide, now).await
    }

    async fn departed(&self, connection: ConnectionId, now: u64) -> Result<(), StoreError> {
        self.store.departed(connection, now).await
    }
}

impl Sessions {
    /// As [`greeted`](Self::greeted), writing through a [`FailingAfterRead`].
    async fn greeted_failing_after_read(&self) -> (FakeInstance, JoinHandle<Ended>) {
        let (mut instance, session) = self.session_over(FailingAfterRead::new(&self.hub));
        let session = tokio::spawn(session);
        instance.send(&hello());
        instance.expect_ack(0).await;
        (instance, session)
    }
}

/// Each case leaves the rows as the next connection has to find them: the
/// request queued and uncounted, and sent again if it is still in time.
#[tokio::test]
async fn a_queue_the_store_cannot_read_or_mark_closes_the_socket() {
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);

    // The read. Queued through a registry the session is not in, so nothing
    // reads the queue until the test wakes it.
    let read = Sessions::new();
    let (mut instance, session) = read.greeted(0).await;
    let unread = requests::enqueued(
        &read.hub.store(),
        &keys,
        &SigningKey::new(SIGNING_KEY),
        &Connections::default(),
        payload_of("signed_request_gate_decision.json"),
        NOW,
    )
    .await
    .expect("queued");
    read.hub.fail_next_call(StoreError::TimeTooLarge(NOW));
    read.connections.wake(read.instance);
    instance.expect_close(1011).await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::StoreFailed);
    assert_eq!(read.deliveries(), [(RequestStatus::Queued, 0, None)]);
    assert!(!read.hub.instance().present);

    read.clock_reads(NOW + 10);
    let (mut instance, session) = read.greeted(0).await;
    let delivered = instance.expect_request().await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(delivered.request_id, base64url_encode(&unread.id));
    assert_eq!(
        read.deliveries(),
        [(RequestStatus::Queued, 1, Some(NOW + 10))]
    );

    // The count. The frame went out before it failed, so the peer holds a
    // request the row does not say was sent.
    let count = Sessions::new();
    let queued = count
        .queues(&keys, "signed_request_gate_decision.json")
        .await;
    count.clock_reads(NOW + 10);
    let (mut instance, session) = count.greeted_failing_after_read().await;
    let sent = instance.expect_request().await;
    assert_eq!(sent, queued.envelope);
    instance.expect_close(1011).await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::StoreFailed);
    assert_eq!(count.deliveries(), [(RequestStatus::Queued, 0, None)]);
    assert!(!count.hub.instance().present);

    count.clock_reads(NOW + 20);
    let (mut instance, session) = count.greeted(0).await;
    let again = instance.expect_request().await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(again.request_id, sent.request_id);
    assert_eq!(again.signature, sent.signature);
    assert_eq!(again, sent);
    assert_eq!(
        count.deliveries(),
        [(RequestStatus::Queued, 1, Some(NOW + 20))]
    );

    // The mark of an expiry. Nothing is sent, before the failure or after.
    let mark = Sessions::new();
    let overdue = mark
        .queues(&keys, "signed_request_cancel_feature.json")
        .await;
    mark.clock_reads(overdue.expires_at);
    let (mut instance, session) = mark.greeted_failing_after_read().await;
    instance.expect_close(1011).await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::StoreFailed);
    assert_eq!(mark.deliveries(), [(RequestStatus::Queued, 0, None)]);
    assert!(!mark.hub.instance().present);

    let (instance, session) = mark.greeted(0).await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(mark.deliveries(), [(RequestStatus::Expired, 0, None)]);
}

#[tokio::test]
async fn every_delivered_request_verifies_under_the_hub_public_key() {
    let sessions = Sessions::new();
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);
    let public_key = keys
        .verifying_key(SIGNING_KEY)
        .await
        .expect("the configured key has a public half");
    let of_another_hub = MemoryKeys::with_keys([SIGNING_KEY])
        .verifying_key(SIGNING_KEY)
        .await
        .expect("a key of the same name");
    for (name, _, _) in KINDS {
        sessions.queues(&keys, name).await;
    }

    // Sent later than it was signed, so an envelope made again at the send
    // would not be the one the signature is over.
    sessions.clock_reads(NOW + 45);
    let (mut instance, session) = sessions.greeted(0).await;
    for (name, kind, _) in KINDS {
        let delivered = instance.expect_request().await;
        assert_eq!(delivered.instance_id, sessions.instance.0.to_string());
        assert_eq!(delivered.payload, payload_of(name));
        assert_eq!(
            serde_json::to_value(&delivered.payload).unwrap()["kind"],
            kind
        );

        let signed = delivered.signing_bytes().expect("signing bytes");
        let signature = base64url_decode(&delivered.signature).expect("unpadded base64url");
        assert!(verifies_ed25519(&public_key, &signed, &signature), "{name}");
        assert!(
            !verifies_ed25519(&of_another_hub, &signed, &signature),
            "{name}"
        );

        let request_id = SignedRequest {
            request_id: base64url_encode(&[7; 16]),
            ..delivered.clone()
        };
        let instance_id = SignedRequest {
            instance_id: InstanceId::generate().0.to_string(),
            ..delivered.clone()
        };
        let issued_at = SignedRequest {
            issued_at: delivered.issued_at + 1,
            ..delivered.clone()
        };
        let expires_at = SignedRequest {
            expires_at: delivered.expires_at + 1,
            ..delivered.clone()
        };
        let other = KINDS.iter().find(|(other, _, _)| *other != name);
        let payload = SignedRequest {
            payload: payload_of(other.expect("more kinds than one").0),
            ..delivered.clone()
        };
        for (field, changed) in [
            ("request_id", request_id),
            ("instance_id", instance_id),
            ("issued_at", issued_at),
            ("expires_at", expires_at),
            ("payload", payload),
        ] {
            let altered = changed.signing_bytes().expect("signing bytes");
            assert_ne!(altered, signed, "{name} with another {field}");
            assert!(
                !verifies_ed25519(&public_key, &altered, &signature),
                "{name} with another {field}"
            );
        }
    }
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(
        sessions.deliveries(),
        [(RequestStatus::Queued, 1, Some(NOW + 45)); 6]
    );
}

/// The instance's answer to a request it was sent, under the id that request
/// arrived with.
fn result_for(request: &SignedRequest, outcome: RequestOutcome) -> RequestResult {
    RequestResult {
        v: 1,
        request_id: request.request_id.clone(),
        outcome,
    }
}

impl Sessions {
    /// What is recorded of every request of the instance, oldest first: its
    /// status, its outcome, and when that was recorded.
    fn outcomes(&self) -> Vec<(RequestStatus, Option<RequestOutcome>, Option<u64>)> {
        let requests = self.hub.requests();
        requests
            .into_iter()
            .map(|request| (request.status, request.outcome, request.answered_at))
            .collect()
    }
}

/// A result is answered with nothing. Frames are taken in order, so the ack
/// of an event sent behind one is what says the session is done with it.
async fn dealt_with(instance: &mut FakeInstance, offset: u64) {
    instance.send(&event("f-1", offset));
    instance.expect_ack(offset).await;
}

#[tokio::test]
async fn an_accepted_result_is_recorded_against_its_request() {
    let sessions = Sessions::new();
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);
    let answered = sessions
        .queues(&keys, "signed_request_start_feature.json")
        .await;
    let unanswered = sessions
        .queues(&keys, "signed_request_gate_decision.json")
        .await;

    sessions.clock_reads(NOW + 10);
    let (mut instance, session) = sessions.greeted(0).await;
    let delivered = instance.expect_request().await;
    assert_eq!(delivered, answered.envelope);
    assert_eq!(instance.expect_request().await, unanswered.envelope);
    assert_eq!(
        sessions.outcomes(),
        [(RequestStatus::Queued, None, None); 2]
    );

    sessions.clock_reads(NOW + 25);
    instance.send(&result_for(&delivered, RequestOutcome::Accepted));
    dealt_with(&mut instance, 1).await;

    let sent = |request: RequestView| RequestView {
        deliveries: 1,
        last_sent_at: Some(NOW + 10),
        ..request
    };
    let recorded = [
        RequestView {
            status: RequestStatus::Answered,
            outcome: Some(RequestOutcome::Accepted),
            answered_at: Some(NOW + 25),
            ..sent(answered)
        },
        sent(unanswered.clone()),
    ];
    assert_eq!(sessions.hub.requests(), recorded);
    assert!(!session.is_finished());
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);

    // An answered request has left the queue: the next connection is sent
    // the other one alone. A result the store cannot record ends it `1011`
    // and leaves the row as it was.
    sessions.clock_reads(NOW + 40);
    let (mut instance, session) = sessions.greeted(1).await;
    let again = instance.expect_request().await;
    assert_eq!(again, unanswered.envelope);
    dealt_with(&mut instance, 2).await;
    let before = sessions.hub.requests();
    assert_eq!(before[0], recorded[0]);

    sessions.hub.fail_next_call(StoreError::TimeTooLarge(NOW));
    instance.send(&result_for(&again, RequestOutcome::Accepted));
    instance.expect_close(1011).await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::StoreFailed);
    assert_eq!(sessions.hub.requests(), before);
}

#[tokio::test]
async fn a_refusal_is_stored_with_its_reason() {
    let reasons = [
        RefusalReason::GateNotPending,
        RefusalReason::ScopeOff,
        RefusalReason::Expired,
        RefusalReason::Replayed,
    ];
    let sessions = Sessions::new();
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);
    for _ in reasons {
        sessions
            .queues(&keys, "signed_request_gate_decision.json")
            .await;
    }

    sessions.clock_reads(NOW + 10);
    let (mut instance, session) = sessions.greeted(0).await;
    let mut delivered = Vec::new();
    for _ in reasons {
        delivered.push(instance.expect_request().await);
    }

    // Answered last request first, so that a reason is filed under the id
    // it came with and not by its turn.
    sessions.clock_reads(NOW + 20);
    for (request, reason) in delivered.iter().zip(reasons).rev() {
        instance.send(&result_for(request, RequestOutcome::Refused { reason }));
    }
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);

    let stored = reasons.map(|reason| {
        (
            RequestStatus::Answered,
            Some(RequestOutcome::Refused { reason }),
            Some(NOW + 20),
        )
    });
    assert_eq!(sessions.outcomes(), stored);
    for (row, request) in sessions.hub.requests().iter().zip(&delivered) {
        assert_eq!(row.envelope, *request);
    }
}

#[tokio::test]
async fn the_first_outcome_of_a_request_is_kept() {
    let replayed = RequestOutcome::Refused {
        reason: RefusalReason::Replayed,
    };
    let gate_gone = RequestOutcome::Refused {
        reason: RefusalReason::GateNotPending,
    };
    let sessions = Sessions::new();
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);
    sessions
        .queues(&keys, "signed_request_start_feature.json")
        .await;
    sessions
        .queues(&keys, "signed_request_gate_decision.json")
        .await;

    // Both go out, and the connection is gone before a result for either.
    sessions.clock_reads(NOW + 10);
    let (mut instance, session) = sessions.greeted(0).await;
    let start = instance.expect_request().await;
    let decision = instance.expect_request().await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(
        sessions.outcomes(),
        [(RequestStatus::Queued, None, None); 2]
    );

    sessions.clock_reads(NOW + 20);
    let (mut instance, session) = sessions.greeted(0).await;
    assert_eq!(instance.expect_request().await, start);
    assert_eq!(instance.expect_request().await, decision);
    instance.send(&result_for(&start, RequestOutcome::Accepted));
    instance.send(&result_for(&decision, gate_gone));
    dealt_with(&mut instance, 1).await;
    let first = [
        (
            RequestStatus::Answered,
            Some(RequestOutcome::Accepted),
            Some(NOW + 20),
        ),
        (RequestStatus::Answered, Some(gate_gone), Some(NOW + 20)),
    ];
    assert_eq!(sessions.outcomes(), first);
    let recorded = sessions.hub.requests();

    // What an instance says of the second copy of a request it had already
    // spent the id of. It is not the account of what became of the request.
    sessions.clock_reads(NOW + 21);
    instance.send(&result_for(&start, replayed));
    dealt_with(&mut instance, 2).await;
    assert_eq!(sessions.hub.requests(), recorded);

    // Kept because it came first, and not because it is the better news.
    instance.send(&result_for(&decision, RequestOutcome::Accepted));
    instance.send(&result_for(&decision, replayed));
    dealt_with(&mut instance, 3).await;
    assert_eq!(sessions.hub.requests(), recorded);
    assert!(!session.is_finished());
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);

    // Nor does a later connection's result replace it.
    sessions.clock_reads(NOW + 30);
    let (instance, session) = sessions.greeted(3).await;
    instance.send(&result_for(&start, replayed));
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(sessions.hub.requests(), recorded);
    assert_eq!(sessions.outcomes(), first);
}

#[tokio::test]
async fn a_result_for_an_unknown_request_changes_nothing() {
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);
    let sessions = Sessions::new();
    let held = sessions
        .queues(&keys, "signed_request_start_feature.json")
        .await;
    let elsewhere = Sessions::new();
    let of_another_instance = elsewhere
        .queues(&keys, "signed_request_start_feature.json")
        .await;
    assert_ne!(held.id, of_another_instance.id);

    sessions.clock_reads(NOW + 10);
    let (mut instance, session) = sessions.greeted(0).await;
    let delivered = instance.expect_request().await;
    dealt_with(&mut instance, 1).await;
    let before = sessions.hub.requests();
    assert_eq!(
        sessions.deliveries(),
        [(RequestStatus::Queued, 1, Some(NOW + 10))]
    );

    let of_the_fixture = fixture("request_result_accepted.json")["request_id"].clone();
    let unknown = [
        of_another_instance.envelope.request_id.clone(),
        base64url_encode(&[7; 16]),
        base64url_encode(&held.id[..15]),
        format!("{}==", delivered.request_id),
        of_the_fixture.as_str().expect("an id").to_owned(),
        "not base64url!".to_owned(),
        String::new(),
    ];
    sessions.clock_reads(NOW + 20);
    for (offset, request_id) in (2..).zip(&unknown) {
        instance.send(&RequestResult {
            v: 1,
            request_id: request_id.clone(),
            outcome: RequestOutcome::Accepted,
        });
        dealt_with(&mut instance, offset).await;
        assert_eq!(sessions.hub.requests(), before, "{request_id:?}");
        assert!(!session.is_finished(), "{request_id:?}");
    }
    assert_eq!(stored_offsets(&sessions.hub), [1, 2, 3, 4, 5, 6, 7, 8]);
    assert!(sessions.hub.instance().present);
    assert_eq!(elsewhere.outcomes(), [(RequestStatus::Queued, None, None)]);

    // The request none of them named is still there to be answered.
    instance.send(&result_for(&delivered, RequestOutcome::Accepted));
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(
        sessions.outcomes(),
        [(
            RequestStatus::Answered,
            Some(RequestOutcome::Accepted),
            Some(NOW + 20)
        )]
    );
}

#[tokio::test]
async fn a_late_result_is_recorded_for_a_request_that_expired_after_it_was_sent_and_for_no_other() {
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);

    let sessions = Sessions::new();
    let queued = sessions
        .queues(&keys, "signed_request_cancel_feature.json")
        .await;
    assert_eq!(queued.expires_at, NOW + 600);
    sessions.clock_reads(NOW + 599);
    let (mut instance, session) = sessions.greeted(0).await;
    let delivered = instance.expect_request().await;
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);

    // Marked on the next reading and not sent again: `finish` refuses a
    // second copy.
    sessions.clock_reads(NOW + 600);
    let (mut instance, session) = sessions.greeted(0).await;
    dealt_with(&mut instance, 1).await;
    let expired = RequestView {
        status: RequestStatus::Expired,
        deliveries: 1,
        last_sent_at: Some(NOW + 599),
        ..queued
    };
    assert_eq!(sessions.hub.requests(), std::slice::from_ref(&expired));

    sessions.clock_reads(NOW + 630);
    instance.send(&result_for(&delivered, RequestOutcome::Accepted));
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(
        sessions.hub.requests(),
        [RequestView {
            status: RequestStatus::Answered,
            outcome: Some(RequestOutcome::Accepted),
            answered_at: Some(NOW + 630),
            ..expired
        }]
    );

    // A request that ran out before it was ever sent has no result to wait
    // for, and one that names it leaves it expired.
    let never_sent = Sessions::new();
    let unsent = never_sent
        .queues(&keys, "signed_request_cancel_feature.json")
        .await;
    never_sent.clock_reads(NOW + 600);
    let (mut instance, session) = never_sent.greeted(0).await;
    dealt_with(&mut instance, 1).await;
    let expired = RequestView {
        status: RequestStatus::Expired,
        ..unsent
    };
    assert_eq!(never_sent.hub.requests(), std::slice::from_ref(&expired));

    never_sent.clock_reads(NOW + 630);
    instance.send(&result_for(&expired.envelope, RequestOutcome::Accepted));
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(never_sent.hub.requests(), [expired]);
}

/// Reached here without a reconnect: the wake-up of another enqueue is the
/// next reading, and the request is one this connection already carried.
#[tokio::test]
async fn a_request_sent_and_then_overdue_is_marked_expired_on_the_next_reading() {
    let sessions = Sessions::new();
    let keys = MemoryKeys::with_keys([SIGNING_KEY]);
    let first = sessions
        .queues(&keys, "signed_request_cancel_feature.json")
        .await;
    let (mut instance, session) = sessions.greeted(0).await;
    let delivered = instance.expect_request().await;
    assert_eq!(delivered, first.envelope);

    // `finish` refuses a second copy of the first.
    sessions.clock_reads(first.expires_at);
    let second = sessions
        .queues(&keys, "signed_request_gate_decision.json")
        .await;
    assert_eq!(instance.expect_request().await, second.envelope);
    dealt_with(&mut instance, 1).await;
    assert_eq!(
        sessions.deliveries(),
        [
            (RequestStatus::Expired, 1, Some(NOW)),
            (RequestStatus::Queued, 1, Some(NOW + 600)),
        ]
    );

    sessions.clock_reads(NOW + 630);
    instance.send(&result_for(&delivered, RequestOutcome::Accepted));
    instance.finish().await;
    assert_eq!(ended(session).await, Ended::PeerGone);
    assert_eq!(
        sessions.outcomes(),
        [
            (
                RequestStatus::Answered,
                Some(RequestOutcome::Accepted),
                Some(NOW + 630)
            ),
            (RequestStatus::Queued, None, None),
        ]
    );
}

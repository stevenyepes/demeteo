// Tests attached to `src-tauri/src/commands/hub.rs` (mirrored-tests
// convention). They live here rather than in demeteo-core because
// `cd src-tauri && cargo test` runs only this package's tests.
//
// The fixtures are the Hub's spellings, written out by hand rather than
// produced by the code under test: `hub_origin` has to agree byte for byte
// with demeteo-hub's `public_origin`, and `redirect_uri` with its
// `is_loopback_redirect`, and neither crate can be called from here on
// every OS.

use super::{
    disconnect_core, pair_core, pairing, policy_get_core, policy_set_core, request_log_core,
    status_core,
};
use crate::adapters::database::SqliteAdapter;
use crate::adapters::hub::device_key::DeviceKey;
use crate::adapters::hub::loopback::CallbackListener;
use crate::adapters::hub::socket::session::SessionContext;
use crate::adapters::hub::socket::wake::WakingRunEvents;
use crate::adapters::hub::socket::{supervise, Sleeper, SocketConfig, SupervisorDeps};
use crate::adapters::hub::testing::{
    MemorySecretStore, RefreshCall, ScriptedConnection, ScriptedRefreshEndpoint,
    ScriptedSnapshotSource, ScriptedSocketDialer,
};
use crate::adapters::hub::token_client::ReqwestTokenEndpoint;
use crate::adapters::hub::HubPairing;
use crate::composition::{
    build_core_context, build_core_context_with_policy_writer, CoreConfig, ExecutionMode,
};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::hub_pairing::answer::{refresh_form_body, token_form_body};
use crate::domain::hub_pairing::callback::{callback_verdict, CallbackVerdict, RefuseReason};
use crate::domain::hub_pairing::dpop::{dpop_proof, dpop_signing_input, dpop_signing_input_for};
use crate::domain::hub_pairing::jwk::{canonical_jwk, fingerprint, jwk_value, thumbprint};
use crate::domain::hub_pairing::keyring_names::{
    hub_keyring_account, keyring_backend_name, KEYRING_SERVICE,
};
use crate::domain::hub_pairing::pkce::s256_challenge;
use crate::domain::hub_pairing::{
    access_token_hash, authorize_url, check_pairing_origin, consent_label, hub_origin, hub_status,
    pairing_state, redirect_uri, socket_dial_url, socket_url, token_error, token_refusal,
    token_url, AuthorizeRequest, HubPairingError, HubPairingState, HubPins, HubSecret, HubStatus,
    KeyringStatus, PairingAnswer, PinnedPasskey, RefreshAnswer, PAIR_TIMEOUT, SCOPES,
};
use crate::domain::hub_request::log::MAX_LOG_PAGE;
use crate::domain::hub_request::{
    Consumption, HubLocalPolicy, HubRequestLogEntry, LogKind, LogOutcome,
};
use crate::domain::hub_socket::{
    after, after_close, after_refresh, backoff, hello, hub_event, refresh_refusal, resets_backoff,
    sends_events, Reconnect, RefreshEnd, SocketEnd, StopReason, BACKOFF_CAP_SECS,
    MAX_FEATURE_ID_BYTES, PREMATURE_EXPIRY_MARGIN_SECS, SNAPSHOT_DEBOUNCE_SECS,
};
use crate::domain::ids::{FeatureId, ProjectId};
use crate::domain::models::{Feature, Project, Workflow, WorkflowVersion};
use crate::error::AppError;
use crate::ports::db::{FeatureRepository, ProjectRepository};
use crate::ports::hub::{
    BrowserLauncher, DialFailure, HubPinRepository, HubRefreshEndpoint, HubRequestRepository,
    HubSecretStore, HubSocketConnection, HubSocketDialer, HubTokenEndpoint, SocketGone,
    SocketReceived, SocketUpgrade,
};
use crate::ports::run_events::{RunEvent, RunEventsPort};
use crate::state::AppContext;
use demeteo_core::adapters::notification_noop::NoopNotificationAdapter;
use rusqlite::types::ValueRef;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[test]
fn hub_origin_normalizes_to_the_hubs_public_origin() {
    let cases = [
        ("https://Hub.Example.com/", "https://hub.example.com"),
        ("https://hub.example.com", "https://hub.example.com"),
        ("https://hub.example.com:443", "https://hub.example.com"),
        ("https://hub.example.com:443/", "https://hub.example.com"),
        (
            "https://hub.example.com:8443",
            "https://hub.example.com:8443",
        ),
        ("http://localhost:8080", "http://localhost:8080"),
        ("http://LOCALHOST", "http://localhost"),
        ("http://localhost:80/", "http://localhost"),
        ("HTTPS://hub.example.com", "https://hub.example.com"),
    ];
    for (input, expected) in cases {
        assert_eq!(
            hub_origin(input).as_deref(),
            Ok(expected),
            "input {input:?}"
        );
    }
}

#[test]
fn hub_origin_refuses_what_the_hub_would_not_serve() {
    let refused = [
        "http://hub.example.com",
        "http://127.0.0.1:8080",
        "ftp://x",
        "https://hub.example.com/sub",
        "https://hub.example.com/sub/",
        "https://hub.example.com/?q=1",
        "https://hub.example.com/#frag",
        "https://user:pass@hub.example.com",
        "not a url",
        "",
    ];
    for input in refused {
        assert!(
            matches!(hub_origin(input), Err(HubPairingError::InvalidHubUrl(_))),
            "input {input:?} gave {:?}",
            hub_origin(input)
        );
    }
}

#[test]
fn token_url_is_the_htu_under_the_origin() {
    assert_eq!(
        token_url("https://hub.example.com:8443"),
        "https://hub.example.com:8443/token"
    );
}

fn request() -> AuthorizeRequest {
    AuthorizeRequest {
        code_challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".into(),
        redirect_uri: "http://127.0.0.1:53111/cb-x_y.z~".into(),
        dpop_jkt: "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I".into(),
        scope: SCOPES.into(),
        state: "st&ate=1+2".into(),
        instance_name: "Ana's laptop".into(),
        os: "linux".into(),
        arch: "x86_64".into(),
        app_version: "1.2.3".into(),
    }
}

#[test]
fn authorize_url_carries_exactly_the_eleven_hub_parameters() {
    let url = authorize_url("https://hub.example.com", &request()).unwrap();
    let parsed = reqwest::Url::parse(&url).unwrap();
    assert_eq!(
        parsed.origin().ascii_serialization(),
        "https://hub.example.com"
    );
    assert_eq!(parsed.path(), "/authorize");

    let pairs: Vec<(String, String)> = parsed.query_pairs().into_owned().collect();
    let params: BTreeMap<&str, &str> = pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(pairs.len(), 11, "no parameter repeated: {pairs:?}");
    let expected = BTreeMap::from([
        ("response_type", "code"),
        ("code_challenge_method", "S256"),
        (
            "code_challenge",
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
        ),
        ("redirect_uri", "http://127.0.0.1:53111/cb-x_y.z~"),
        ("dpop_jkt", "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I"),
        ("scope", "read spend gates"),
        ("state", "st&ate=1+2"),
        ("instance_name", "Ana's laptop"),
        ("os", "linux"),
        ("arch", "x86_64"),
        ("app_version", "1.2.3"),
    ]);
    assert_eq!(params, expected);
    assert!(!params.contains_key("client_id"));
}

#[test]
fn redirect_uri_satisfies_the_hubs_loopback_rule() {
    assert_eq!(
        redirect_uri(53111, "cb-x_y.z~").as_deref(),
        Some("http://127.0.0.1:53111/cb-x_y.z~")
    );
    assert_eq!(
        redirect_uri(1, "a").as_deref(),
        Some("http://127.0.0.1:1/a"),
        "no leading zero on a one-digit port"
    );
    assert_eq!(
        redirect_uri(0, "cb"),
        None,
        "port 0 is spelled with a leading zero"
    );
    for path in ["", "a/b", "a b", "a%2e", "a?b", "a#b", "é"] {
        assert_eq!(redirect_uri(53111, path), None, "path {path:?}");
    }
    assert_eq!(
        redirect_uri(53111, &"a".repeat(300)),
        None,
        "over the Hub's length cap"
    );
}

#[test]
fn keyring_backend_name_mirrors_keyring_v4_store_selection() {
    assert_eq!(keyring_backend_name("macos"), "macOS Keychain");
    assert_eq!(
        keyring_backend_name("windows"),
        "Windows Credential Manager"
    );
    assert_eq!(keyring_backend_name("linux"), "Secret Service");
    assert_eq!(keyring_backend_name("freebsd"), "Secret Service");
}

#[test]
fn hub_keyring_accounts_are_distinct_per_install_and_per_secret() {
    assert_eq!(KEYRING_SERVICE, "demeteo");
    assert_eq!(
        hub_keyring_account("inst-a", HubSecret::DeviceKey),
        "hub_device_key:inst-a"
    );
    assert_eq!(
        hub_keyring_account("inst-a", HubSecret::RefreshToken),
        "hub_refresh_token:inst-a"
    );
    let accounts = [
        hub_keyring_account("inst-a", HubSecret::DeviceKey),
        hub_keyring_account("inst-a", HubSecret::RefreshToken),
        hub_keyring_account("inst-b", HubSecret::DeviceKey),
        hub_keyring_account("inst-b", HubSecret::RefreshToken),
    ];
    let distinct: std::collections::BTreeSet<&String> = accounts.iter().collect();
    assert_eq!(distinct.len(), accounts.len(), "{accounts:?}");
}

#[test]
fn memory_secret_store_refuses_every_account_but_its_installs_two() {
    let store = MemorySecretStore::new("inst-a");
    for account in [
        "hub_device_key:inst-b",
        "hub_refresh_token:inst-b",
        "hub_device_key",
        "memory_agent_llm",
        "hub_access_token:inst-a",
    ] {
        assert!(store.set(account, "secret").is_err(), "{account}");
        assert!(store.get(account).is_err(), "{account}");
        assert!(store.delete(account).is_err(), "{account}");
    }
    assert!(store.entries().is_empty());
}

#[test]
fn memory_secret_store_round_trips_and_deletes_idempotently() {
    let store = MemorySecretStore::new("inst-a");
    let key = hub_keyring_account("inst-a", HubSecret::DeviceKey);
    let refresh = hub_keyring_account("inst-a", HubSecret::RefreshToken);

    assert_eq!(store.get(&key).unwrap(), None);
    store.delete(&refresh).unwrap();

    store.set(&key, "pkcs8").unwrap();
    store.set(&refresh, "rt-1").unwrap();
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("pkcs8"));
    assert_eq!(
        store.entries(),
        BTreeMap::from([
            (key.clone(), "pkcs8".into()),
            (refresh.clone(), "rt-1".into())
        ])
    );

    store.delete(&refresh).unwrap();
    store.delete(&refresh).unwrap();
    assert_eq!(store.get(&refresh).unwrap(), None);
    assert_eq!(store.entries().len(), 1);
}

#[test]
fn an_unavailable_memory_secret_store_says_so_and_refuses_writes() {
    let store = MemorySecretStore::new("inst-a");
    let key = hub_keyring_account("inst-a", HubSecret::DeviceKey);
    assert!(store.status().available);

    store.set_available(false);
    assert!(!store.status().available);
    assert!(store.set(&key, "pkcs8").is_err());
    assert!(store.entries().is_empty());
}

#[test]
fn pairing_state_covers_every_combination() {
    use HubPairingState::{Incomplete, Paired, Unpaired};
    let table = [
        ((false, false, false), Unpaired),
        ((false, true, false), Unpaired),
        ((false, false, true), Incomplete),
        ((false, true, true), Incomplete),
        ((true, false, false), Incomplete),
        ((true, true, false), Incomplete),
        ((true, false, true), Incomplete),
        ((true, true, true), Paired),
    ];
    for ((pins, key, refresh), expected) in table {
        assert_eq!(
            pairing_state(pins, key, refresh),
            expected,
            "pins={pins} device_key={key} refresh_token={refresh}"
        );
    }
}

#[test]
fn pairing_state_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&HubPairingState::Incomplete).unwrap(),
        "\"incomplete\""
    );
}

#[test]
fn clock_skew_tells_the_user_to_check_the_clock() {
    assert!(HubPairingError::ClockSkew
        .to_string()
        .contains("system clock"));
}

#[test]
fn pair_timeout_outlasts_consent_and_code_ttls() {
    assert_eq!(PAIR_TIMEOUT.as_secs(), 420);
}

#[test]
fn callback_verdict_accepts_only_the_exact_path_and_state() {
    use CallbackVerdict::{Code, Denied, Refuse};
    use RefuseReason::{MissingCode, WrongPath, WrongState};
    let (path, state) = ("cb-x_y", "st4te");
    let table: [(&str, Option<&str>, CallbackVerdict); 14] = [
        ("/cb-x_y", Some("code=C&state=st4te"), Code("C".into())),
        (
            "/cb-x_y",
            Some("state=st4te&code=a%2Bb"),
            Code("a+b".into()),
        ),
        ("/cb-x_y", Some("code=C&state=wrong"), Refuse(WrongState)),
        ("/cb-x_y", Some("code=C&state=st4tE"), Refuse(WrongState)),
        ("/cb-x_y", Some("code=C"), Refuse(WrongState)),
        ("/cb-x_y", None, Refuse(WrongState)),
        ("/cb-x_y", Some("state=st4te"), Refuse(MissingCode)),
        ("/cb-x_y", Some("code=&state=st4te"), Refuse(MissingCode)),
        (
            "/cb-x_y",
            Some("error=server_error&state=st4te"),
            Refuse(MissingCode),
        ),
        ("/cb-x_y", Some("error=access_denied&state=st4te"), Denied),
        (
            "/cb-x_y",
            Some("error=access_denied&state=wrong"),
            Refuse(WrongState),
        ),
        ("/other", Some("code=C&state=st4te"), Refuse(WrongPath)),
        ("/cb-x_y/", Some("code=C&state=st4te"), Refuse(WrongPath)),
        ("cb-x_y", Some("code=C&state=st4te"), Refuse(WrongPath)),
    ];
    for (got_path, query, expected) in table {
        assert_eq!(
            callback_verdict(got_path, query, path, state),
            expected,
            "path {got_path:?} query {query:?}"
        );
    }
}

#[test]
fn refuse_reasons_map_to_the_listeners_status_codes() {
    assert_eq!(RefuseReason::WrongPath.http_status(), 404);
    assert_eq!(RefuseReason::WrongState.http_status(), 400);
    assert_eq!(RefuseReason::MissingCode.http_status(), 400);
}

#[test]
fn s256_challenge_matches_rfc_7636_appendix_b() {
    assert_eq!(
        s256_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

/// The key of the example proof in RFC 9449 §4.1. The thumbprint is the
/// literal `src-tauri/tests/hub_pairing.rs` checks the Hub against, computed
/// outside both crates.
const RFC_9449_X: &str = "l8tFrhx-34tV3hRICRDY9zCkDlpBhF42UQUfWVAWBFs";
const RFC_9449_Y: &str = "9VE4jf_Ok_o64zbTTlcuNJajHmt6v9TDVrU0CdvGRDA";
const RFC_9449_THUMBPRINT: &str = "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I";

/// Unpadded base64url, by hand: this package has no `base64` dependency,
/// and decoding with the crate under test would check it against itself.
fn base64url_decode(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let (mut bits, mut len, mut out) = (0u32, 0, Vec::new());
    for byte in text.bytes() {
        let value = ALPHABET.iter().position(|&a| a == byte);
        bits = (bits << 6) | value.unwrap_or_else(|| panic!("{byte} in {text:?}")) as u32;
        len += 6;
        if len >= 8 {
            len -= 8;
            out.push((bits >> len) as u8);
        }
    }
    out
}

fn rfc_9449_point() -> [u8; 65] {
    let mut point = [0u8; 65];
    point[0] = 0x04;
    point[1..33].copy_from_slice(&base64url_decode(RFC_9449_X));
    point[33..].copy_from_slice(&base64url_decode(RFC_9449_Y));
    point
}

fn segment(text: &str) -> Value {
    serde_json::from_slice(&base64url_decode(text)).unwrap()
}

#[test]
fn canonical_jwk_is_the_rfc_7638_text_of_the_point() {
    assert_eq!(
        canonical_jwk(&rfc_9449_point()),
        format!(r#"{{"crv":"P-256","kty":"EC","x":"{RFC_9449_X}","y":"{RFC_9449_Y}"}}"#)
    );
}

#[test]
fn thumbprint_and_fingerprint_of_the_rfc_9449_key_are_the_hubs() {
    let jkt = thumbprint(&rfc_9449_point());
    assert_eq!(jkt, RFC_9449_THUMBPRINT);
    assert_eq!(fingerprint(&jkt), "SHA256:0ZcO CORZ NYy- DWpq");
    assert_eq!(fingerprint("abcde"), "SHA256:abcd e");
}

#[test]
fn dpop_signing_input_carries_the_header_and_claims_the_hub_verifies() {
    let htu = "https://hub.example.com/token";
    let input = dpop_signing_input(&rfc_9449_point(), htu, 1_700_000_000, "jti-0");
    let [header, claims] = <[&str; 2]>::try_from(input.split('.').collect::<Vec<_>>()).unwrap();
    assert_eq!(
        segment(header),
        json!({"typ": "dpop+jwt", "alg": "ES256", "jwk": jwk_value(&rfc_9449_point())})
    );
    assert_eq!(
        segment(header)["jwk"],
        json!({"kty": "EC", "crv": "P-256", "x": RFC_9449_X, "y": RFC_9449_Y})
    );
    assert_eq!(
        segment(claims),
        json!({"jti": "jti-0", "htm": "POST", "htu": htu, "iat": 1_700_000_000u64})
    );
}

#[test]
fn dpop_proof_appends_the_raw_signature_as_a_third_segment() {
    let mut sig = [0u8; 64];
    sig[0] = 0xfb;
    sig[63] = 0xff;
    let proof = dpop_proof("aGVhZA.Y2xhaW1z", &sig);
    let signature = proof.strip_prefix("aGVhZA.Y2xhaW1z.").unwrap();
    assert_eq!(signature.len(), 86);
    assert_eq!(base64url_decode(signature), sig);
}

#[test]
fn a_device_key_reloaded_from_its_pkcs8_keeps_its_thumbprint() {
    let key = DeviceKey::generate().unwrap();
    let reloaded = DeviceKey::from_pkcs8(&key.to_pkcs8().unwrap()).unwrap();

    assert_eq!(reloaded.public_point(), key.public_point());
    assert_eq!(reloaded.jkt(), key.jkt());
    assert_eq!(key.jkt(), thumbprint(key.public_point()));
    assert_ne!(DeviceKey::generate().unwrap().jkt(), key.jkt());
    assert!(matches!(
        DeviceKey::from_pkcs8(b"not a key"),
        Err(HubPairingError::Storage(_))
    ));
}

const ACCESS_TOKEN: &str = "dha_access-0123456789abcdefghijklmnopqrstuv";
const REFRESH_TOKEN: &str = "dhr_refresh-0123456789abcdefghijklmnopqrstu";

/// `pairing_response`'s serialization: the flattened `RefreshResponse`
/// members first, then the pins.
fn hub_answer() -> PairingAnswer {
    hub_answer_scoped("read gates")
}

fn hub_answer_scoped(scope: &str) -> PairingAnswer {
    let body = json!({
        "access_token": ACCESS_TOKEN,
        "token_type": "DPoP",
        "expires_in": 900,
        "refresh_token": REFRESH_TOKEN,
        "scope": scope,
        "instance_id": "7f0c2a4e-1b3d-4c5e-8f90-123456789abc",
        "request_signing_key": "__________________________________________8",
        "rp_id": "hub.example",
        "passkeys": [
            {"credential_id": "Y3JlZC0x", "public_key": "cGstMQ"},
            {"credential_id": "Y3JlZC0y", "public_key": "cGstMg"}
        ]
    });
    serde_json::from_value(body).unwrap()
}

#[test]
fn a_hub_answer_splits_into_public_pins_and_the_issued_tokens() {
    let (pins, tokens) = hub_answer().split("https://hub.example.com", "jkt-0", 1_700_000_000);
    assert_eq!(
        pins,
        HubPins {
            hub_origin: "https://hub.example.com".into(),
            instance_id: "7f0c2a4e-1b3d-4c5e-8f90-123456789abc".into(),
            request_signing_key: "__________________________________________8".into(),
            rp_id: "hub.example".into(),
            scopes: vec!["read".into(), "gates".into()],
            device_jkt: "jkt-0".into(),
            paired_at: 1_700_000_000,
            passkeys: vec![
                PinnedPasskey {
                    credential_id: "Y3JlZC0x".into(),
                    public_key: "cGstMQ".into(),
                },
                PinnedPasskey {
                    credential_id: "Y3JlZC0y".into(),
                    public_key: "cGstMg".into(),
                },
            ],
        }
    );
    let pins_text = format!("{pins:?}{}", serde_json::to_string(&pins).unwrap());
    assert!(!pins_text.contains(ACCESS_TOKEN) && !pins_text.contains(REFRESH_TOKEN));
    assert_eq!(tokens.access.expose(), ACCESS_TOKEN);
    assert_eq!(tokens.refresh.expose(), REFRESH_TOKEN);
}

#[test]
fn a_hub_answer_missing_a_member_is_refused() {
    let mut body = json!({
        "access_token": ACCESS_TOKEN,
        "token_type": "DPoP",
        "expires_in": 900,
        "refresh_token": REFRESH_TOKEN,
        "scope": "read",
        "instance_id": "i",
        "request_signing_key": "k",
        "rp_id": "hub.example",
        "passkeys": []
    });
    assert!(serde_json::from_value::<PairingAnswer>(body.clone()).is_ok());
    body.as_object_mut().unwrap().remove("request_signing_key");
    assert!(serde_json::from_value::<PairingAnswer>(body).is_err());
}

#[test]
fn issued_tokens_debug_prints_no_token_text() {
    let (_, tokens) = hub_answer().split("https://hub.example.com", "jkt-0", 0);
    let debug = format!("{tokens:?} {tokens:#?}");
    assert!(debug.contains("<redacted>"), "{debug}");
    for secret in [ACCESS_TOKEN, REFRESH_TOKEN, "dha_", "dhr_", "0123456789"] {
        assert!(!debug.contains(secret), "{debug}");
    }
}

#[test]
fn token_form_body_percent_encodes_and_parses_back_to_the_four_fields() {
    let redirect = "http://127.0.0.1:49152/callback";
    let verifier = "Ab9-._~xyz";
    let code = "c0de+/= &?";
    let body = token_form_body(code, verifier, redirect);
    assert_eq!(
        body,
        "grant_type=authorization_code&code=c0de%2B%2F%3D%20%26%3F\
         &code_verifier=Ab9-._~xyz\
         &redirect_uri=http%3A%2F%2F127.0.0.1%3A49152%2Fcallback"
    );
    let parsed: Vec<(String, String)> = reqwest::Url::parse(&format!("http://x/?{body}"))
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let expected = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", verifier),
        ("redirect_uri", redirect),
    ]
    .map(|(k, v)| (k.to_owned(), v.to_owned()));
    assert_eq!(parsed, expected);
}

fn pin_db(tag: &str) -> (demeteo_core::test_dir::TestDir, SqliteAdapter) {
    let dir = demeteo_core::test_dir::TestDir::new(tag);
    let conn = rusqlite::Connection::open(dir.path().join("demeteo.db")).unwrap();
    (dir, SqliteAdapter::new(conn).unwrap())
}

fn pins(origin: &str, passkeys: &[(&str, &str)]) -> HubPins {
    HubPins {
        hub_origin: origin.to_owned(),
        instance_id: format!("inst-{origin}"),
        request_signing_key: "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo".to_owned(),
        rp_id: "hub.example.com".to_owned(),
        scopes: vec!["read".to_owned(), "spend".to_owned(), "gates".to_owned()],
        device_jkt: "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I".to_owned(),
        paired_at: 1_760_000_000,
        passkeys: passkeys
            .iter()
            .map(|(id, key)| PinnedPasskey {
                credential_id: (*id).to_owned(),
                public_key: (*key).to_owned(),
            })
            .collect(),
    }
}

fn row_count(db: &SqliteAdapter, table: &str) -> i64 {
    db.conn
        .lock()
        .unwrap()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

/// Written out from spec §2.1, not read from the migration: an added column
/// — the shape a stored token would take — has to fail here.
#[test]
fn v64_creates_exactly_the_public_hub_columns() {
    let (_dir, db) = pin_db("hub_v64_columns");
    let expected: [(&str, &[&str]); 3] = [
        ("hub_install", &["id", "install_id", "created_at"]),
        (
            "hub_pairing",
            &[
                "id",
                "hub_origin",
                "instance_id",
                "request_signing_key",
                "rp_id",
                "scopes",
                "device_jkt",
                "paired_at",
            ],
        ),
        (
            "hub_pinned_passkeys",
            &["credential_id", "public_key", "pinned_at", "sign_count"],
        ),
    ];
    let conn = db.conn.lock().unwrap();
    for (table, columns) in expected {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let actual: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(actual, columns, "{table}");
    }
}

#[test]
fn hub_pins_round_trip_with_their_passkeys() {
    let (_dir, db) = pin_db("hub_pins_round_trip");
    assert_eq!(db.load().unwrap(), None);
    let saved = pins(
        "https://hub.example.com",
        &[("cred-b", "pk-b"), ("cred-a", "pk-a")],
    );
    db.save(&saved).unwrap();
    assert_eq!(db.load().unwrap(), Some(saved));
}

#[test]
fn a_second_save_replaces_the_pairing_and_every_passkey() {
    let (_dir, db) = pin_db("hub_pins_replace");
    db.save(&pins(
        "https://hub.example.com",
        &[("cred-a", "pk-a"), ("cred-b", "pk-b")],
    ))
    .unwrap();
    let second = pins("http://localhost:8080", &[("cred-c", "pk-c")]);
    db.save(&second).unwrap();
    assert_eq!(db.load().unwrap(), Some(second));
    assert_eq!(row_count(&db, "hub_pairing"), 1);
    assert_eq!(row_count(&db, "hub_pinned_passkeys"), 1);
}

#[test]
fn clear_drops_the_pairing_but_keeps_the_install_id() {
    let (_dir, db) = pin_db("hub_pins_clear");
    let install_id = db.install_id().unwrap();
    assert_eq!(install_id.len(), 22, "{install_id}");
    assert!(install_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    db.save(&pins("https://hub.example.com", &[("cred-a", "pk-a")]))
        .unwrap();

    db.clear().unwrap();

    assert_eq!(row_count(&db, "hub_pairing"), 0);
    assert_eq!(row_count(&db, "hub_pinned_passkeys"), 0);
    assert_eq!(db.load().unwrap(), None);
    assert_eq!(db.install_id().unwrap(), install_id);
    assert_eq!(row_count(&db, "hub_install"), 1);
}

#[test]
fn install_ids_differ_between_databases() {
    let (_a, first) = pin_db("hub_install_a");
    let (_b, second) = pin_db("hub_install_b");
    assert_ne!(first.install_id().unwrap(), second.install_id().unwrap());
}

const STATE: &str = "the-state";

/// The listener waiting on its own task, with the URI the Hub would redirect
/// to. The timeout is long enough that only a test asking for it sees it.
fn waiting_listener() -> (
    String,
    tokio::task::JoinHandle<Result<String, HubPairingError>>,
) {
    let (listener, redirect) = CallbackListener::bind().expect("a loopback bind");
    let wait = tokio::spawn(listener.wait(STATE, Duration::from_secs(10)));
    (redirect, wait)
}

async fn get_status(url: &str) -> u16 {
    reqwest::get(url)
        .await
        .expect("the listener answers")
        .status()
        .as_u16()
}

#[tokio::test]
async fn the_callback_listener_binds_an_ephemeral_loopback_port() {
    let (listener, redirect) = CallbackListener::bind().expect("a loopback bind");
    let addr = listener.local_addr();

    assert_eq!(addr.ip(), Ipv4Addr::LOCALHOST);
    assert_ne!(addr.port(), 0);
    let prefix = format!("http://127.0.0.1:{}/", addr.port());
    let path = redirect.strip_prefix(&prefix).expect("the bound port");
    assert_eq!(redirect_uri(addr.port(), path), Some(redirect.clone()));
}

#[tokio::test]
async fn a_wrong_state_is_refused_and_the_later_correct_code_is_returned() {
    let (redirect, wait) = waiting_listener();

    let forged = get_status(&format!("{redirect}?code=FORGED&state=wrong")).await;
    let stateless = get_status(&format!("{redirect}?code=FORGED")).await;
    let codeless = get_status(&format!("{redirect}?state={STATE}")).await;
    assert_eq!((forged, stateless, codeless), (400, 400, 400));
    assert!(!wait.is_finished());

    assert_eq!(
        get_status(&format!("{redirect}?code=REAL&state={STATE}")).await,
        200
    );
    assert_eq!(wait.await.unwrap(), Ok("REAL".to_owned()));
}

#[tokio::test]
async fn another_path_is_not_found_and_does_not_end_the_wait() {
    let (redirect, wait) = waiting_listener();
    let origin = redirect.rsplit_once('/').expect("a path").0;

    assert_eq!(
        get_status(&format!("{origin}/elsewhere?code=C&state={STATE}")).await,
        404
    );
    assert_eq!(
        get_status(&format!("{redirect}/?code=C&state={STATE}")).await,
        404
    );
    assert!(!wait.is_finished());

    get_status(&format!("{redirect}?code=C&state={STATE}")).await;
    assert_eq!(wait.await.unwrap(), Ok("C".to_owned()));
}

#[tokio::test]
async fn access_denied_with_the_right_state_ends_the_wait_denied() {
    let (redirect, wait) = waiting_listener();

    assert_eq!(
        get_status(&format!("{redirect}?error=access_denied&state={STATE}")).await,
        200
    );
    assert_eq!(wait.await.unwrap(), Err(HubPairingError::Denied));
}

#[tokio::test]
async fn no_callback_ends_the_wait_with_a_timeout() {
    let (listener, _redirect) = CallbackListener::bind().expect("a loopback bind");

    let result = listener.wait(STATE, Duration::from_millis(200)).await;

    assert_eq!(result, Err(HubPairingError::CallbackTimeout));
}

#[test]
fn token_errors_map_invalid_dpop_proof_to_clock_skew_and_keep_other_codes() {
    assert_eq!(
        token_error("invalid_dpop_proof"),
        HubPairingError::ClockSkew
    );
    for code in ["invalid_grant", "pairing_expired", "too_many_requests"] {
        assert_eq!(
            token_error(code),
            HubPairingError::HubRefused { error: code.into() }
        );
    }
}

#[test]
fn a_token_refusal_without_a_hub_error_code_is_malformed() {
    assert_eq!(
        token_refusal(br#"{"error":"invalid_grant"}"#),
        HubPairingError::HubRefused {
            error: "invalid_grant".into()
        }
    );
    for body in [
        &b"<html>upstream</html>"[..],
        b"",
        b"{}",
        br#"{"error":42}"#,
        br#"{"error":""}"#,
        br#"{"error":"Bearer dhr_leaked token"}"#,
    ] {
        assert!(
            matches!(token_refusal(body), HubPairingError::MalformedAnswer(_)),
            "body {:?}",
            String::from_utf8_lossy(body)
        );
    }
}

const TOKEN_DPOP: &str = "eyJ0eXAiOiJkcG9wK2p3dCJ9.eyJodG0iOiJQT1NUIn0.c2ln";

fn token_request_body() -> String {
    token_form_body("C0DE", "verifier-0", "http://127.0.0.1:49152/callback")
}

async fn token_stub(
    status: u16,
    content_type: &'static str,
    body: &'static str,
) -> (String, tokio::task::JoinHandle<Result<(), String>>) {
    token_stub_expecting(token_request_body(), status, content_type, body).await
}

/// One scripted `/token` answer on `127.0.0.1:0` to a POST of
/// `expected_body`. The task returns why the request was not the one
/// scripted, and answers such a request with `418` rather than the script,
/// so an unscripted request can never pass.
async fn token_stub_expecting(
    expected_body: String,
    status: u16,
    content_type: &'static str,
    body: &'static str,
) -> (String, tokio::task::JoinHandle<Result<(), String>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("a loopback bind");
    let url = format!("http://{}/token", listener.local_addr().unwrap());
    let served = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut request = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            if let Some(at) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                break at;
            }
            let n = stream.read(&mut chunk).await.map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("the request ended inside its head".into());
            }
            request.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&request[..head_end]).into_owned();
        let mut lines = head.split("\r\n");
        let request_line = lines.next().unwrap_or_default().to_owned();
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
            .collect();
        let values = |name: &str| -> Vec<&str> {
            headers
                .iter()
                .filter(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
                .collect()
        };
        let length: usize = values("content-length")
            .first()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut received = request[head_end + 4..].to_vec();
        while received.len() < length {
            let n = stream.read(&mut chunk).await.map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            received.extend_from_slice(&chunk[..n]);
        }

        let mut unscripted = Vec::new();
        if request_line != "POST /token HTTP/1.1" {
            unscripted.push(format!("request line {request_line:?}"));
        }
        if values("content-type") != ["application/x-www-form-urlencoded"] {
            unscripted.push(format!("content-type {:?}", values("content-type")));
        }
        if values("dpop") != [TOKEN_DPOP] {
            unscripted.push(format!("dpop {:?}", values("dpop")));
        }
        if received != expected_body.as_bytes() {
            unscripted.push(format!("body {:?}", String::from_utf8_lossy(&received)));
        }
        let (status, body) = if unscripted.is_empty() {
            (status, body)
        } else {
            (418, "")
        };
        let response = format!(
            "HTTP/1.1 {status} Scripted\r\ncontent-type: {content_type}\r\n\
             content-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let _ = stream.shutdown().await;
        if unscripted.is_empty() {
            Ok(())
        } else {
            Err(unscripted.join("; "))
        }
    });
    (url, served)
}

async fn exchange_with(url: &str) -> Result<PairingAnswer, HubPairingError> {
    ReqwestTokenEndpoint::new()
        .expect("an HTTP client")
        .exchange(url, token_request_body(), TOKEN_DPOP.to_owned())
        .await
}

async fn refusal_from(
    status: u16,
    content_type: &'static str,
    body: &'static str,
) -> HubPairingError {
    let (url, served) = token_stub(status, content_type, body).await;
    let result = exchange_with(&url).await;
    served.await.unwrap().expect("the scripted request");
    result.err().expect("a refusal")
}

#[tokio::test]
async fn token_exchange_posts_the_form_with_one_dpop_header_and_reads_the_answer() {
    const ANSWER: &str = r#"{"access_token":"dha_a","token_type":"DPoP","expires_in":900,
        "refresh_token":"dhr_r","scope":"read gates","instance_id":"inst-1",
        "request_signing_key":"rsk","rp_id":"hub.example",
        "passkeys":[{"credential_id":"Y3JlZC0x","public_key":"cGstMQ"}]}"#;
    let (url, served) = token_stub(200, "application/json", ANSWER).await;

    let result = exchange_with(&url).await;

    served.await.unwrap().expect("the scripted request");
    let Ok(answer) = result else {
        panic!("{:?}", result.err());
    };
    let (pins, tokens) = answer.split("https://hub.example", "jkt", 0);
    assert_eq!(
        (pins.instance_id.as_str(), pins.rp_id.as_str()),
        ("inst-1", "hub.example")
    );
    assert_eq!(tokens.refresh.expose(), "dhr_r");
}

#[tokio::test]
async fn token_exchange_refused_for_its_proof_is_clock_skew() {
    let refusal = refusal_from(400, "application/json", r#"{"error":"invalid_dpop_proof"}"#).await;
    assert_eq!(refusal, HubPairingError::ClockSkew);
}

#[tokio::test]
async fn token_exchange_refused_for_its_grant_is_hub_refused() {
    let refusal = refusal_from(400, "application/json", r#"{"error":"invalid_grant"}"#).await;
    assert_eq!(
        refusal,
        HubPairingError::HubRefused {
            error: "invalid_grant".into()
        }
    );
}

#[tokio::test]
async fn a_non_json_token_failure_is_malformed_without_its_body() {
    const PAGE: &str = "<html>proxy-page dhr_leaked-refresh</html>";
    let refusal = refusal_from(500, "text/html", PAGE).await;
    assert!(matches!(refusal, HubPairingError::MalformedAnswer(_)));
    let text = refusal.to_string();
    for fragment in ["proxy-page", "dhr_leaked", "html"] {
        assert!(!text.contains(fragment), "{text}");
    }
}

#[tokio::test]
async fn an_unreachable_token_endpoint_is_transport_without_the_url() {
    let closed = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let url = format!("http://{}/token?probe=q", closed.local_addr().unwrap());
    drop(closed);

    let refusal = exchange_with(&url)
        .await
        .err()
        .expect("a transport failure");

    assert!(matches!(refusal, HubPairingError::Transport(_)));
    let text = refusal.to_string();
    for fragment in ["127.0.0.1", "probe", "C0DE", "verifier-0", TOKEN_DPOP] {
        assert!(!text.contains(fragment), "{text}");
    }
}

const HUB_URL: &str = "https://Hub.Example.com/";
const ORIGIN: &str = "https://hub.example.com";
const CODE: &str = "C-authcode-7b1e5d0c9a3f4e2b8d6c";

type Recorded = Arc<Mutex<Option<String>>>;

fn query_param(url: &str, name: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()?
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// Records the `/authorize` URL and, when `deliver`, plays the Hub's
/// redirect to the loopback listener with [`CODE`] and the URL's own state.
struct HubRedirect {
    authorize: Recorded,
    deliver: bool,
}

impl BrowserLauncher for HubRedirect {
    fn open(&self, url: &str) -> Result<(), String> {
        *self.authorize.lock().unwrap() = Some(url.to_owned());
        if !self.deliver {
            return Ok(());
        }
        let redirect = query_param(url, "redirect_uri").ok_or("no redirect_uri")?;
        let state = query_param(url, "state").ok_or("no state")?;
        tokio::spawn(
            async move { reqwest::get(format!("{redirect}?code={CODE}&state={state}")).await },
        );
        Ok(())
    }
}

struct NeverOpened;

impl BrowserLauncher for NeverOpened {
    fn open(&self, url: &str) -> Result<(), String> {
        panic!("the browser was opened on {url}");
    }
}

enum Reply {
    Answer,
    /// One answer per exchange, the nth granting the nth `scope`; an
    /// exchange past the last is refused.
    Scoped(&'static [&'static str]),
    Refuse(&'static str),
}

/// `/token` as the Hub judges it, for the parts this desktop controls: it
/// answers only the code the redirect carried, a verifier whose S256 is the
/// authorize URL's challenge, that URL's `redirect_uri`, and a proof for
/// `<origin>/token` under the key named by `dpop_jkt`. Anything else is a
/// `Transport` error naming what was wrong.
struct ScriptedToken {
    authorize: Recorded,
    reply: Reply,
    verifiers: Mutex<Vec<String>>,
}

impl ScriptedToken {
    fn check(&self, token_url: &str, form_body: &str, dpop: &str) -> Result<String, String> {
        let authorize = self
            .authorize
            .lock()
            .unwrap()
            .clone()
            .ok_or("no /authorize")?;
        let param = |name| query_param(&authorize, name).ok_or(format!("authorize lacks {name}"));
        let form = |name| {
            query_param(&format!("http://x/?{form_body}"), name).ok_or(format!("form lacks {name}"))
        };
        if token_url != format!("{ORIGIN}/token") {
            return Err(format!("token url {token_url}"));
        }
        if form("grant_type")? != "authorization_code" || form("code")? != CODE {
            return Err("grant".into());
        }
        if form("redirect_uri")? != param("redirect_uri")? {
            return Err("redirect_uri".into());
        }
        let verifier = form("code_verifier")?;
        if s256_challenge(&verifier) != param("code_challenge")? {
            return Err("code_verifier".into());
        }
        let parts: Vec<&str> = dpop.split('.').collect();
        let [header, claims, _] = parts[..] else {
            return Err("proof shape".into());
        };
        let (header, claims) = (segment(header), segment(claims));
        if claims["htu"] != json!(token_url) || claims["htm"] != json!("POST") {
            return Err("proof htu/htm".into());
        }
        let mut point = [4u8; 65];
        point[1..33].copy_from_slice(&base64url_decode(header["jwk"]["x"].as_str().unwrap()));
        point[33..].copy_from_slice(&base64url_decode(header["jwk"]["y"].as_str().unwrap()));
        if thumbprint(&point) != param("dpop_jkt")? {
            return Err("proof key".into());
        }
        Ok(verifier)
    }
}

#[async_trait::async_trait]
impl HubTokenEndpoint for ScriptedToken {
    async fn exchange(
        &self,
        token_url: &str,
        form_body: String,
        dpop: String,
    ) -> Result<PairingAnswer, HubPairingError> {
        let verifier = self
            .check(token_url, &form_body, &dpop)
            .map_err(|why| HubPairingError::Transport(format!("token double: {why}")))?;
        let exchanged = {
            let mut verifiers = self.verifiers.lock().unwrap();
            verifiers.push(verifier);
            verifiers.len()
        };
        match self.reply {
            Reply::Answer => Ok(hub_answer()),
            Reply::Scoped(scopes) => scopes
                .get(exchanged - 1)
                .map(|scope| hub_answer_scoped(scope))
                .ok_or_else(|| HubPairingError::Transport("token double: unscripted".into())),
            Reply::Refuse(code) => Err(token_error(code)),
        }
    }
}

/// The pins, with `save` failing as a full disk would.
struct SaveFails(Arc<SqliteAdapter>);

impl HubPinRepository for SaveFails {
    fn install_id(&self) -> Result<String, AppError> {
        self.0.install_id()
    }
    fn load(&self) -> Result<Option<HubPins>, AppError> {
        self.0.load()
    }
    fn save(&self, _: &HubPins) -> Result<(), AppError> {
        Err(AppError::internal("database or disk is full"))
    }
    fn clear(&self) -> Result<(), AppError> {
        self.0.clear()
    }
}

struct Rig {
    dir: demeteo_core::test_dir::TestDir,
    db: Arc<SqliteAdapter>,
    secrets: Arc<MemorySecretStore>,
    token: Arc<ScriptedToken>,
    authorize: Recorded,
    hub: Arc<HubPairing>,
}

impl Rig {
    fn new(tag: &str, reply: Reply) -> Self {
        Self::with(tag, reply, |db| db)
    }

    fn with(
        tag: &str,
        reply: Reply,
        pins: impl FnOnce(Arc<SqliteAdapter>) -> Arc<dyn HubPinRepository>,
    ) -> Self {
        let (dir, db) = pin_db(tag);
        let db = Arc::new(db);
        let secrets = Arc::new(MemorySecretStore::new(&db.install_id().unwrap()));
        let authorize = Recorded::default();
        let token = Arc::new(ScriptedToken {
            authorize: authorize.clone(),
            reply,
            verifiers: Mutex::default(),
        });
        let hub = Arc::new(HubPairing::new(
            pins(db.clone()),
            secrets.clone(),
            token.clone(),
        ));
        Self {
            dir,
            db,
            secrets,
            token,
            authorize,
            hub,
        }
    }

    fn redirect(&self) -> HubRedirect {
        HubRedirect {
            authorize: self.authorize.clone(),
            deliver: true,
        }
    }

    async fn pair(&self, hub_url: &str) -> Result<HubStatus, HubPairingError> {
        self.hub
            .pair(hub_url, &self.redirect(), Duration::from_secs(10))
            .await
    }

    fn account(&self, secret: HubSecret) -> String {
        hub_keyring_account(&self.db.install_id().unwrap(), secret)
    }

    fn db_path(&self) -> PathBuf {
        self.dir.path().join("demeteo.db")
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Where any needle appears: the database file, its `-wal` and `-shm`, and
/// every TEXT or BLOB cell of every table `sqlite_master` names.
fn found_in_db(db: &SqliteAdapter, path: &Path, needles: &[(&str, Vec<u8>)]) -> Vec<String> {
    let mut found = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        let mut file = path.as_os_str().to_owned();
        file.push(suffix);
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        for (name, needle) in needles {
            if contains(&bytes, needle) {
                found.push(format!("{name} in demeteo.db{suffix}"));
            }
        }
    }
    let conn = db.conn.lock().unwrap();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for table in tables {
        let mut stmt = conn.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
        let columns = stmt.column_count();
        let mut rows = stmt.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for column in 0..columns {
                let cell = match row.get_ref(column).unwrap() {
                    ValueRef::Text(bytes) | ValueRef::Blob(bytes) => bytes,
                    _ => continue,
                };
                for (name, needle) in needles {
                    if contains(cell, needle) {
                        found.push(format!("{name} in a cell of {table}"));
                    }
                }
            }
        }
    }
    found
}

#[tokio::test]
async fn a_pairing_keeps_every_secret_in_the_keyring_and_pins_the_hubs_answer() {
    let rig = Rig::new("hub_pair_no_secrets", Reply::Answer);

    let status = rig.pair(HUB_URL).await.expect("a pairing");

    let entries = rig.secrets.entries();
    let device_account = rig.account(HubSecret::DeviceKey);
    let refresh_account = rig.account(HubSecret::RefreshToken);
    assert_eq!(
        entries.keys().collect::<Vec<_>>(),
        [&device_account, &refresh_account]
    );
    assert_eq!(entries[&refresh_account], REFRESH_TOKEN);
    let stored_key = entries[&device_account].clone();
    let der = base64url_decode(&stored_key);

    let pins = rig.db.load().unwrap().expect("pins");

    let verifiers = rig.token.verifiers.lock().unwrap().clone();
    let [verifier] = &verifiers[..] else {
        panic!("one /token exchange, got {}", verifiers.len());
    };
    let secrets = [
        ("refresh token", REFRESH_TOKEN.as_bytes().to_vec()),
        ("access token", ACCESS_TOKEN.as_bytes().to_vec()),
        ("authorization code", CODE.as_bytes().to_vec()),
        ("code_verifier", verifier.as_bytes().to_vec()),
        ("PKCS#8 DER", der),
        ("PKCS#8 base64url", stored_key.into_bytes()),
    ];
    let leaks = found_in_db(&rig.db, &rig.db_path(), &secrets);
    assert!(leaks.is_empty(), "{leaks:?}");

    let public = [("instance_id", pins.instance_id.as_bytes().to_vec())];
    let seen = found_in_db(&rig.db, &rig.db_path(), &public);
    assert!(
        seen.contains(&"instance_id in a cell of hub_pairing".to_owned())
            && seen
                .iter()
                .any(|s| s.starts_with("instance_id in demeteo.db")),
        "the scan must see a value that is stored: {seen:?}"
    );

    let (expected, _) = hub_answer().split(ORIGIN, &pins.device_jkt, pins.paired_at);
    assert_eq!(pins, expected);
    assert_eq!(
        DeviceKey::from_pkcs8(&secrets[4].1).unwrap().jkt(),
        pins.device_jkt,
        "the pinned jkt is the stored key's"
    );

    assert_eq!(status.state, HubPairingState::Paired);
    let text = format!("{status:?}{}", serde_json::to_string(&status).unwrap());
    for (_, secret) in &secrets[..4] {
        assert!(
            !text.contains(std::str::from_utf8(secret).unwrap()),
            "{text}"
        );
    }
}

#[tokio::test]
async fn status_after_a_pairing_names_the_hub_and_the_device_fingerprint() {
    let rig = Rig::new("hub_pair_status", Reply::Answer);
    let paired = rig.pair(HUB_URL).await.expect("a pairing");

    let status = rig.hub.status().unwrap();

    assert_eq!(status, paired);
    let jkt = rig.db.load().unwrap().unwrap().device_jkt;
    assert_eq!(status.state, HubPairingState::Paired);
    assert_eq!(status.hub_url.as_deref(), Some(ORIGIN));
    assert_eq!(
        status.instance_id.as_deref(),
        Some("7f0c2a4e-1b3d-4c5e-8f90-123456789abc")
    );
    let shown = status.fingerprint.expect("a fingerprint");
    assert_eq!(
        shown,
        format!(
            "SHA256:{} {} {} {}",
            &jkt[..4],
            &jkt[4..8],
            &jkt[8..12],
            &jkt[12..16]
        )
    );
    assert_eq!(status.rp_id.as_deref(), Some("hub.example"));
    assert_eq!(status.scopes, ["read", "gates"]);
    assert!(status.paired_at.is_some());
    assert!(!status.keyring_backend.is_empty());
    assert!(status.keyring_available);
}

#[tokio::test]
async fn status_before_any_pairing_is_unpaired() {
    let rig = Rig::new("hub_status_unpaired", Reply::Answer);
    let status = rig.hub.status().unwrap();
    assert_eq!(status.state, HubPairingState::Unpaired);
    assert_eq!((status.hub_url, status.fingerprint), (None, None));
}

#[tokio::test]
async fn the_authorize_url_carries_this_desktops_labels() {
    let rig = Rig::new("hub_pair_labels", Reply::Answer);
    rig.pair(HUB_URL).await.expect("a pairing");
    let url = rig.authorize.lock().unwrap().clone().unwrap();

    assert!(url.starts_with(&format!("{ORIGIN}/authorize?")), "{url}");
    assert_eq!(query_param(&url, "scope").as_deref(), Some(SCOPES));
    assert_eq!(
        query_param(&url, "os").as_deref(),
        Some(std::env::consts::OS)
    );
    assert_eq!(
        query_param(&url, "arch").as_deref(),
        Some(std::env::consts::ARCH)
    );
    assert_eq!(query_param(&url, "app_version").as_deref(), Some("unknown"));
    let name = query_param(&url, "instance_name").unwrap();
    assert!((1..=64).contains(&name.chars().count()), "{name}");
}

#[tokio::test]
async fn a_second_pairing_while_one_waits_is_refused() {
    let rig = Rig::new("hub_pair_in_flight", Reply::Answer);
    let held = Arc::new(HubRedirect {
        authorize: rig.authorize.clone(),
        deliver: false,
    });
    let first = {
        let (hub, held) = (rig.hub.clone(), held.clone());
        tokio::spawn(async move { hub.pair(HUB_URL, &*held, Duration::from_secs(10)).await })
    };
    let mut opened = None;
    for _ in 0..500 {
        opened = rig.authorize.lock().unwrap().clone();
        if opened.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let opened = opened.expect("the first pairing opened the browser");

    let second = rig
        .hub
        .pair(HUB_URL, &NeverOpened, Duration::from_secs(10))
        .await;
    assert_eq!(second, Err(HubPairingError::PairingInProgress));

    let redirect = query_param(&opened, "redirect_uri").unwrap();
    let state = query_param(&opened, "state").unwrap();
    assert_eq!(
        get_status(&format!("{redirect}?code={CODE}&state={state}")).await,
        200
    );
    let first = first.await.unwrap().expect("the first pairing completes");
    assert_eq!(first.state, HubPairingState::Paired);
}

#[tokio::test]
async fn an_unavailable_keyring_refuses_before_writing_anything() {
    let (_dir, db) = pin_db("hub_pair_no_keyring");
    let db = Arc::new(db);
    let secrets = Arc::new(MemorySecretStore::new("unused"));
    secrets.set_available(false);
    let token = Arc::new(ScriptedToken {
        authorize: Recorded::default(),
        reply: Reply::Answer,
        verifiers: Mutex::default(),
    });
    let hub = HubPairing::new(db.clone(), secrets.clone(), token.clone());

    let result = hub
        .pair(HUB_URL, &NeverOpened, Duration::from_secs(10))
        .await;

    assert_eq!(result, Err(HubPairingError::KeyringUnavailable));
    for table in ["hub_install", "hub_pairing", "hub_pinned_passkeys"] {
        assert_eq!(row_count(&db, table), 0, "{table}");
    }
    assert!(secrets.entries().is_empty());
    assert!(token.verifiers.lock().unwrap().is_empty());
}

async fn refused_pairing(tag: &str, code: &'static str) -> (HubPairingError, Rig) {
    let rig = Rig::new(tag, Reply::Refuse(code));
    let refusal = rig.pair(HUB_URL).await.expect_err("a refusal");
    (refusal, rig)
}

fn assert_left_unpaired(rig: &Rig) {
    let entries = rig.secrets.entries();
    assert_eq!(
        entries.keys().collect::<Vec<_>>(),
        [&rig.account(HubSecret::DeviceKey)],
        "only the device key, kept for the next attempt"
    );
    assert_eq!(rig.db.load().unwrap(), None);
    assert_eq!(rig.hub.status().unwrap().state, HubPairingState::Unpaired);
}

#[tokio::test]
async fn a_proof_refused_at_token_is_clock_skew_and_stores_no_token() {
    let (refusal, rig) = refused_pairing("hub_pair_clock_skew", "invalid_dpop_proof").await;
    assert_eq!(refusal, HubPairingError::ClockSkew);
    assert_left_unpaired(&rig);
}

#[tokio::test]
async fn a_grant_refused_at_token_is_hub_refused_and_stores_no_token() {
    let (refusal, rig) = refused_pairing("hub_pair_invalid_grant", "invalid_grant").await;
    assert_eq!(
        refusal,
        HubPairingError::HubRefused {
            error: "invalid_grant".into()
        }
    );
    assert_left_unpaired(&rig);
}

#[tokio::test]
async fn pins_that_fail_to_save_take_the_refresh_token_with_them() {
    let rig = Rig::with("hub_pair_save_fails", Reply::Answer, |db| {
        Arc::new(SaveFails(db))
    });

    let refusal = rig.pair(HUB_URL).await.expect_err("a storage failure");

    assert!(
        matches!(refusal, HubPairingError::Storage(_)),
        "{refusal:?}"
    );
    assert!(!refusal.to_string().contains(REFRESH_TOKEN));
    assert_left_unpaired(&rig);
}

#[tokio::test]
async fn re_pairing_with_the_same_hub_keeps_the_device_key() {
    let rig = Rig::new("hub_repair_same_origin", Reply::Answer);
    rig.pair(HUB_URL).await.expect("a pairing");
    let first_jkt = rig.db.load().unwrap().unwrap().device_jkt;
    let first_key = rig.secrets.entries()[&rig.account(HubSecret::DeviceKey)].clone();

    let again = rig
        .pair("https://hub.example.com:443")
        .await
        .expect("a second pairing");

    assert_eq!(again.state, HubPairingState::Paired);
    assert_eq!(rig.db.load().unwrap().unwrap().device_jkt, first_jkt);
    assert_eq!(
        rig.secrets.entries()[&rig.account(HubSecret::DeviceKey)],
        first_key
    );
    assert_eq!(rig.token.verifiers.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn pairing_with_another_hub_while_paired_is_refused() {
    let rig = Rig::new("hub_pair_elsewhere", Reply::Answer);
    rig.pair(HUB_URL).await.expect("a pairing");
    let pins = rig.db.load().unwrap();

    let refusal = rig
        .hub
        .pair(
            "https://other.example.com",
            &NeverOpened,
            Duration::from_secs(10),
        )
        .await;

    assert_eq!(
        refusal,
        Err(HubPairingError::AlreadyPairedElsewhere {
            origin: ORIGIN.into()
        })
    );
    assert_eq!(rig.db.load().unwrap(), pins);
    assert_eq!(rig.hub.status().unwrap().state, HubPairingState::Paired);
}

/// A row `disconnect` must not touch, through the repositories a run writes.
fn a_running_feature(db: &SqliteAdapter) -> Feature {
    let project_id = ProjectId::from("p-hub".to_owned());
    ProjectRepository::add(
        db,
        Project {
            id: project_id.clone(),
            name: "hub".to_owned(),
            compute_type: "local".to_owned(),
            remote_host: None,
            status: "idle".to_owned(),
            nodes: 1,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        },
    )
    .unwrap();
    let feature = Feature {
        effort: None,
        id: FeatureId::from("f-hub".to_owned()),
        project_id,
        workflow_id: None,
        workflow_version_id: None,
        title: "Keep running".to_owned(),
        description: "through a Hub disconnect".to_owned(),
        status: "running".to_owned(),
        total_cost: 0.25,
        tokens: 42,
        duration: "3s".to_owned(),
        created_at: 1000,
        agent_kind: None,
        model: None,
        mr_url: None,
        mr_state: Some("none".to_owned()),
        pr_title: None,
        pr_body: None,
        commit_artifacts: None,
        loop_iterations: None,
        max_budget_usd: None,
        step_overrides: Vec::new(),
        attachments: Vec::new(),
        harness_baseline: None,
        origin: FeatureOrigin::DefaultBranch,
        diff_base_branch: None,
        resolved_branch: None,
        launched_by: Default::default(),
    };
    FeatureRepository::add(db, feature.clone()).unwrap();
    feature
}

#[tokio::test]
async fn disconnect_forgets_the_hub_twice_over_and_leaves_runs_alone() {
    let rig = Rig::new("hub_disconnect", Reply::Answer);
    rig.pair(HUB_URL).await.expect("a pairing");
    let feature = a_running_feature(&rig.db);
    assert_eq!(rig.secrets.entries().len(), 2);
    assert_eq!(row_count(&rig.db, "hub_pinned_passkeys"), 2);

    let status = rig.hub.disconnect().await.expect("a disconnect");

    assert!(
        rig.secrets.entries().is_empty(),
        "{:?}",
        rig.secrets.entries().keys()
    );
    for table in ["hub_pairing", "hub_pinned_passkeys"] {
        assert_eq!(row_count(&rig.db, table), 0, "{table}");
    }
    assert_eq!(status, rig.hub.status().unwrap());
    assert_eq!(status.state, HubPairingState::Unpaired);
    assert_eq!(
        (status.hub_url, status.instance_id, status.fingerprint),
        (None, None, None)
    );

    let again = rig.hub.disconnect().await.expect("a second disconnect");
    assert_eq!(again.state, HubPairingState::Unpaired);

    let kept = FeatureRepository::get(&*rig.db, &feature.id)
        .unwrap()
        .expect("the feature row");
    assert_eq!(
        serde_json::to_value(kept).unwrap(),
        serde_json::to_value(feature).unwrap()
    );
}

#[test]
fn consent_labels_satisfy_the_hubs_label_rule() {
    assert_eq!(consent_label("  Ana's laptop\n", 64, "fb"), "Ana's laptop");
    assert_eq!(consent_label("a\u{202e}b\u{200b}c", 64, "fb"), "abc");
    assert_eq!(consent_label("abcdef", 4, "fb"), "abcd");
    assert_eq!(consent_label("abc def", 4, "fb"), "abc");
    assert_eq!(consent_label("", 32, "unknown"), "unknown");
    assert_eq!(consent_label(" \t\u{feff} ", 32, "unknown"), "unknown");
    assert_eq!(consent_label("東京東京東京", 3, "fb"), "東京東");
}

#[test]
fn check_pairing_origin_allows_only_the_paired_hub() {
    assert_eq!(check_pairing_origin(None, ORIGIN), Ok(()));
    assert_eq!(check_pairing_origin(Some(ORIGIN), ORIGIN), Ok(()));
    assert_eq!(
        check_pairing_origin(Some(ORIGIN), "https://other.example.com"),
        Err(HubPairingError::AlreadyPairedElsewhere {
            origin: ORIGIN.into()
        })
    );
}

#[test]
fn hub_status_without_a_keyring_cannot_read_as_paired() {
    let pins = pins(ORIGIN, &[]);
    let gone = KeyringStatus {
        backend: "Secret Service",
        available: false,
    };
    let status = hub_status(Some(&pins), gone, true, true);
    assert_eq!(status.state, HubPairingState::Incomplete);
    assert_eq!(status.hub_url.as_deref(), Some(ORIGIN));
    assert_eq!(status.keyring_backend, "Secret Service");
    assert!(!status.keyring_available);
    let none = hub_status(None, gone, false, false);
    assert_eq!(none.state, HubPairingState::Unpaired);
    assert_eq!(none.scopes, Vec::<String>::new());
}

fn core_context(tag: &str, execution_mode: ExecutionMode) -> AppContext {
    let dir = demeteo_core::test_dir::scratch(&format!("demeteo-hub-cmd-{tag}"));
    build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    )
}

#[tokio::test]
async fn a_local_only_context_has_no_hub_and_the_commands_say_so() {
    let ctx = core_context("local-only", ExecutionMode::LocalOnly);
    assert!(ctx.hub.is_none());
    assert_eq!(
        pairing(&ctx).err().as_deref(),
        Some("Hub pairing is not available in this mode")
    );
}

#[tokio::test]
async fn a_router_context_can_pair() {
    let ctx = core_context("router", ExecutionMode::Router);
    assert!(ctx.hub.is_some());
    assert!(pairing(&ctx).is_ok());
}

/// Bound by the time `build_core_context` returns, which is before the
/// desktop starts the socket.
#[tokio::test]
async fn a_router_context_binds_the_request_handler_its_socket_hands_requests_to() {
    let ctx = core_context("router-requests", ExecutionMode::Router);
    let hub = ctx.hub.as_ref().expect("a Router context can pair");
    assert!(hub.requests_bound());
}

/// Settings reads and writes the policy, and reads the log, without a
/// pairing: what was set before pairing is what the first request is judged by.
#[tokio::test]
async fn the_policy_and_log_cores_round_trip_through_one_database() {
    let dir = demeteo_core::test_dir::scratch("demeteo-hub-cmd-policy");
    let (ctx, writer) = build_core_context_with_policy_writer(
        CoreConfig {
            app_data_dir: dir.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
    assert_eq!(
        policy_get_core(ctx.hub_policy.as_ref()),
        Ok(HubLocalPolicy::default())
    );
    let policy = HubLocalPolicy {
        spend_enabled: true,
        gates_enabled: false,
        run_ceiling_cents: Some(500),
        daily_ceiling_cents: None,
    };
    policy_set_core(writer.as_ref(), &policy).expect("save the policy");
    assert_eq!(policy_get_core(ctx.hub_policy.as_ref()), Ok(policy));

    for (request_id, at) in [("r-old", 1), ("r-new", 2)] {
        ctx.hub_requests
            .append_log(&HubRequestLogEntry {
                request_id: request_id.into(),
                kind: LogKind::CancelFeature,
                scope: None,
                outcome: LogOutcome::Deferred,
                reason: None,
                feature_id: None,
                received_at_ms: at,
            })
            .expect("append a row");
    }
    let log = request_log_core(ctx.hub_requests.as_ref(), 1).expect("read the log");
    assert_eq!(
        log.iter()
            .map(|e| e.request_id.as_str())
            .collect::<Vec<_>>(),
        ["r-new"]
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Records the `limit` each `list_log` is given; every other call errors.
#[derive(Default)]
struct LimitSpy(Mutex<Vec<usize>>);

impl HubRequestRepository for LimitSpy {
    fn try_consume(&self, _: &str, _: u64, _: u64) -> Result<Consumption, AppError> {
        Err(AppError::internal("unexpected try_consume"))
    }
    fn prune_consumed(&self, _: u64) -> Result<(), AppError> {
        Err(AppError::internal("unexpected prune_consumed"))
    }
    fn append_log(&self, _: &HubRequestLogEntry) -> Result<(), AppError> {
        Err(AppError::internal("unexpected append_log"))
    }
    fn list_log(&self, limit: usize) -> Result<Vec<HubRequestLogEntry>, AppError> {
        self.0.lock().expect("lock").push(limit);
        Ok(Vec::new())
    }
}

#[test]
fn the_log_core_never_asks_for_more_than_one_page() {
    let spy = LimitSpy::default();
    for limit in [usize::MAX, MAX_LOG_PAGE + 1, MAX_LOG_PAGE, 20] {
        request_log_core(&spy, limit).expect("read the log");
    }
    assert_eq!(
        *spy.0.lock().expect("lock"),
        [MAX_LOG_PAGE, MAX_LOG_PAGE, MAX_LOG_PAGE, 20]
    );
}

/// `RunEventRecorder` appends through `ctx.run_events`, and the socket waits
/// on the wake its pairing was given: the two have to be one decorator.
#[tokio::test]
async fn an_append_through_the_context_wakes_the_socket_the_pairing_holds() {
    let ctx = core_context("router-wake", ExecutionMode::Router);
    let wake = ctx
        .hub
        .as_ref()
        .and_then(|hub| hub.socket_wake())
        .expect("a Router context attaches the socket");
    assert!(
        tokio::time::timeout(Duration::from_millis(50), wake.notified())
            .await
            .is_err(),
        "nothing was appended yet"
    );
    ctx.run_events
        .append("f-1", "feature_status", None, 1)
        .unwrap();
    assert!(woken(&wake).await);
}

#[tokio::test]
async fn a_router_context_hands_the_socket_a_snapshot_wake() {
    let ctx = core_context("router-snapshot-wake", ExecutionMode::Router);
    assert!(ctx
        .hub
        .as_ref()
        .and_then(|hub| hub.snapshot_wake())
        .is_some());
}

fn snapshot_wake(ctx: &AppContext) -> Arc<tokio::sync::Notify> {
    ctx.hub
        .as_ref()
        .and_then(|hub| hub.snapshot_wake())
        .expect("a Router context attaches the socket")
}

async fn not_woken(wake: &tokio::sync::Notify) -> bool {
    tokio::time::timeout(Duration::from_millis(50), wake.notified())
        .await
        .is_err()
}

fn composed_project(id: &str) -> Project {
    Project {
        id: ProjectId::from(id.to_owned()),
        name: "hub".to_owned(),
        compute_type: "local".to_owned(),
        remote_host: None,
        status: "idle".to_owned(),
        nodes: 1,
        spend: 0.0,
        tokens: 0,
        created_at: 1000,
    }
}

#[tokio::test]
async fn a_version_saved_through_the_context_wakes_the_snapshot_the_socket_holds() {
    let ctx = core_context("router-snapshot-version", ExecutionMode::Router);
    let wake = snapshot_wake(&ctx);
    assert!(not_woken(&wake).await, "nothing was written yet");
    ctx.workflows
        .create(Workflow {
            id: "wf-1".into(),
            name: "Ship".to_owned(),
            description: String::new(),
            is_starter: false,
            created_at: 0,
            updated_at: 0,
            schedule: None,
        })
        .unwrap();
    assert!(woken(&wake).await, "creating the workflow wakes it too");

    ctx.workflows
        .save_version(WorkflowVersion {
            id: "wf-1-v1".into(),
            workflow_id: "wf-1".into(),
            version: 1,
            steps_json: "[]".to_owned(),
            definition_json: None,
            note: None,
            created_at: 0,
        })
        .unwrap();
    assert!(woken(&wake).await);
}

#[tokio::test]
async fn a_project_written_through_the_context_wakes_the_snapshot_the_socket_holds() {
    let ctx = core_context("router-snapshot-project", ExecutionMode::Router);
    let wake = snapshot_wake(&ctx);
    ctx.projects.add(composed_project("p-1")).unwrap();
    assert!(woken(&wake).await, "adding the project wakes it too");

    let mut renamed = composed_project("p-1");
    renamed.name = "renamed".to_owned();
    ctx.projects.update(renamed).unwrap();
    assert!(woken(&wake).await);
}

#[tokio::test]
async fn a_project_status_change_through_the_context_leaves_the_snapshot_asleep() {
    let ctx = core_context("router-snapshot-status", ExecutionMode::Router);
    let wake = snapshot_wake(&ctx);
    ctx.projects.add(composed_project("p-1")).unwrap();
    assert!(woken(&wake).await, "adding the project wakes it");

    ctx.projects
        .update_status(&ProjectId::from("p-1".to_owned()), "archived")
        .unwrap();
    assert!(not_woken(&wake).await);
}

#[tokio::test]
async fn the_command_cores_answer_what_the_service_answers() {
    let rig = Rig::new("hub_command_cores", Reply::Answer);
    assert_eq!(status_core(&rig.hub), Ok(rig.hub.status().unwrap()));

    let paired = pair_core(&rig.hub, HUB_URL, &rig.redirect())
        .await
        .expect("a pairing");
    assert_eq!(paired.state, HubPairingState::Paired);
    assert_eq!(status_core(&rig.hub), Ok(paired));

    let disconnected = disconnect_core(&rig.hub).await.expect("a disconnect");
    assert_eq!(disconnected.state, HubPairingState::Unpaired);
    assert_eq!(Ok(disconnected), rig.hub.status());
}

#[tokio::test]
async fn a_command_core_error_is_the_services_error_text() {
    let rig = Rig::new("hub_command_core_error", Reply::Answer);
    rig.pair(HUB_URL).await.expect("a pairing");
    let elsewhere = pair_core(&rig.hub, "https://other.example.com", &NeverOpened).await;
    assert_eq!(
        elsewhere,
        Err(HubPairingError::AlreadyPairedElsewhere {
            origin: ORIGIN.into()
        }
        .to_string())
    );
}

#[test]
fn socket_url_is_the_https_htu_the_hub_verifies() {
    assert_eq!(
        socket_url("https://hub.example"),
        "https://hub.example/socket"
    );
    assert_eq!(
        socket_url("http://localhost:8080"),
        "http://localhost:8080/socket"
    );
}

#[test]
fn socket_dial_url_is_the_websocket_spelling_of_the_origin() {
    assert_eq!(
        socket_dial_url("https://hub.example"),
        Ok("wss://hub.example/socket".into())
    );
    assert_eq!(
        socket_dial_url("http://localhost:8080"),
        Ok("ws://localhost:8080/socket".into())
    );
    for origin in ["wss://hub.example", "ftp://hub.example", "hub.example", ""] {
        assert!(
            matches!(
                socket_dial_url(origin),
                Err(HubPairingError::InvalidHubUrl(_))
            ),
            "{origin:?}"
        );
    }
}

/// The access token and `ath` of RFC 9449 §7.1's example.
#[test]
fn access_token_hash_matches_rfc_9449() {
    assert_eq!(
        access_token_hash("Kz~8mXK1EalYznwH-LC-1fBAo.4Ljp~zsPE_NeO.gxU"),
        "fUHyO2r2Z3DZ53EsNrWBb0xWXoaNy59IiKCAqksmQEo"
    );
}

/// The claims segment is pinned as text, not parsed, because the bytes are
/// what the key signs: a reordered or re-spelled claim set would still parse
/// equal.
#[test]
fn a_post_proof_without_ath_is_the_token_signing_input_unchanged() {
    let htu = "https://hub.example.com/token";
    let point = rfc_9449_point();
    let input = dpop_signing_input_for(&point, "POST", htu, 1_700_000_000, "jti-0", None);
    assert_eq!(
        input,
        dpop_signing_input(&point, htu, 1_700_000_000, "jti-0")
    );
    assert_eq!(
        input.split('.').nth(1),
        Some("eyJodG0iOiJQT1NUIiwiaHR1IjoiaHR0cHM6Ly9odWIuZXhhbXBsZS5jb20vdG9rZW4iLCJpYXQiOjE3MDAwMDAwMDAsImp0aSI6Imp0aS0wIn0")
    );
}

#[test]
fn a_socket_proof_names_its_method_and_its_access_token() {
    let htu = socket_url("https://hub.example");
    let token = "Kz~8mXK1EalYznwH-LC-1fBAo.4Ljp~zsPE_NeO.gxU";
    let ath = access_token_hash(token);
    let input = dpop_signing_input_for(&rfc_9449_point(), "GET", &htu, 7, "jti-1", Some(&ath));
    assert_eq!(
        segment(input.split('.').nth(1).unwrap()),
        json!({"jti": "jti-1", "htm": "GET", "htu": htu, "iat": 7u64, "ath": ath})
    );

    let key = DeviceKey::generate().unwrap();
    let claims = |proof: String| segment(proof.split('.').nth(1).unwrap());
    assert_eq!(
        claims(
            key.dpop_proof_for("GET", &htu, 7, "jti-1", Some(token))
                .unwrap()
        ),
        json!({"jti": "jti-1", "htm": "GET", "htu": htu, "iat": 7u64, "ath": ath})
    );
    assert_eq!(
        claims(
            key.dpop_proof_for("POST", "https://hub.example/token", 7, "jti-1", None)
                .unwrap()
        ),
        claims(
            key.dpop_proof("https://hub.example/token", 7, "jti-1")
                .unwrap()
        )
    );
}

fn run_row(kind: &str, payload: Option<&str>) -> RunEvent {
    RunEvent {
        offset: 42,
        run_id: "f-0123456789abcdef".into(),
        kind: kind.into(),
        payload_json: payload.map(str::to_owned),
        created_at: 1_700_000_123_999,
    }
}

/// The wire payload: every id the protocol can carry, `ticket_id` included,
/// so a projection that set one it should not is caught.
fn wire_payload(step_id: Option<&str>, step_execution_id: Option<&str>) -> Value {
    json!({"step_id": step_id, "step_execution_id": step_execution_id, "ticket_id": null})
}

/// The projection serialised as the socket sends it: comparing the wire form
/// rather than the struct is what shows no row field rides along.
fn projected(row: &RunEvent) -> Option<Value> {
    hub_event(row).map(|event| serde_json::to_value(event).expect("serialisable"))
}

/// Every row of the spec's §2.3 projection table. The payloads are the shapes
/// `run_event_record` writes, extra fields included, so a projection that
/// forwarded any of them would not equal the expected wire value.
#[test]
fn every_allowlisted_run_event_projects_to_its_protocol_kind() {
    let progress = |status: &str| {
        json!({
            "step_id": "s-implement", "status": status, "cost_usd": 0.5, "tokens": 900,
            "wall_clock_secs": 12, "cache_read_input_tokens": 1, "cache_creation_input_tokens": 2,
        })
        .to_string()
    };
    let gate = json!({
        "step_execution_id": "se-7", "decision": "approve", "feedback": "/home/me/secret.txt"
    })
    .to_string();
    let none = wire_payload(None, None);
    let step = wire_payload(Some("s-implement"), None);
    let execution = wire_payload(None, Some("se-7"));
    let cases = [
        (
            "feature_status",
            json!({"status": "running"}).to_string(),
            "feature_started",
            &none,
        ),
        (
            "feature_status",
            json!({"status": "completed"}).to_string(),
            "feature_completed",
            &none,
        ),
        (
            "feature_status",
            json!({"status": "failed"}).to_string(),
            "feature_failed",
            &none,
        ),
        (
            "feature_status",
            json!({"status": "cancelled"}).to_string(),
            "feature_cancelled",
            &none,
        ),
        ("step_progress", progress("running"), "step_started", &step),
        (
            "step_progress",
            progress("completed"),
            "step_completed",
            &step,
        ),
        ("step_progress", progress("failed"), "step_failed", &step),
        (
            "gate_required",
            json!({"step_execution_id": "se-7"}).to_string(),
            "gate_pending",
            &execution,
        ),
        ("gate_decided", gate.clone(), "gate_decided", &execution),
        ("gate_auto_approved", gate, "gate_decided", &execution),
    ];
    for (kind, payload, wire_kind, wire_payload) in cases {
        assert_eq!(
            projected(&run_row(kind, Some(&payload))),
            Some(json!({
                "v": 1,
                "feature_id": "f-0123456789abcdef",
                "offset": 42,
                "kind": wire_kind,
                "occurred_at": 1_700_000_123u64,
                "payload": wire_payload,
            })),
            "{kind} {payload}"
        );
    }
}

#[test]
fn a_run_event_off_the_allowlist_never_crosses() {
    let step_status = |status: &str| json!({"step_id": "s-1", "status": status}).to_string();
    let feature_status = |status: &str| json!({"status": status}).to_string();
    let any =
        json!({"step_id": "s-1", "step_execution_id": "se-1", "status": "running"}).to_string();
    let cases: Vec<(&str, Option<String>)> = vec![
        ("agent_spawned", Some(any.clone())),
        ("retry_decision", Some(any.clone())),
        ("retry_exhausted", Some(any.clone())),
        ("env_not_ready", Some(any.clone())),
        ("agent_commits_folded", Some(any.clone())),
        ("gate_held_by_review", Some(any.clone())),
        ("bootstrap_progress", Some(any.clone())),
        ("critic_rework_requested", Some(any.clone())),
        ("step_output", Some(any.clone())),
        ("not_a_kind_anyone_writes", Some(any)),
        ("step_progress", Some(step_status("pending"))),
        ("step_progress", Some(step_status("verifying"))),
        ("step_progress", Some(step_status("awaiting_gate"))),
        ("step_progress", Some(step_status("interrupted"))),
        ("step_progress", Some(step_status("Running"))),
        ("feature_status", Some(feature_status("awaiting_gate"))),
        ("feature_status", Some(feature_status("paused"))),
        ("feature_status", None),
        ("feature_status", Some("not json".into())),
        ("feature_status", Some(json!({"status": 1}).to_string())),
        ("feature_status", Some("{}".into())),
        ("step_progress", None),
        ("step_progress", Some("{".into())),
        (
            "step_progress",
            Some(json!({"status": "running"}).to_string()),
        ),
        (
            "step_progress",
            Some(json!({"step_id": null, "status": "running"}).to_string()),
        ),
        ("gate_required", None),
        ("gate_required", Some("[]".into())),
        ("gate_required", Some("{}".into())),
        ("gate_decided", None),
        ("gate_decided", Some("se-1".into())),
        ("gate_decided", Some(json!({"step_id": "s-1"}).to_string())),
        ("gate_auto_approved", None),
        (
            "gate_auto_approved",
            Some(json!({"step_execution_id": 7}).to_string()),
        ),
        (
            "step_progress",
            Some(json!({"step_id": "s\u{0}1", "status": "running"}).to_string()),
        ),
        (
            "gate_required",
            Some(json!({"step_execution_id": "se\u{0}1"}).to_string()),
        ),
        (
            "gate_decided",
            Some(json!({"step_execution_id": "\u{0}"}).to_string()),
        ),
        (
            "gate_auto_approved",
            Some(json!({"step_execution_id": "se-1\u{0}"}).to_string()),
        ),
    ];
    for (kind, payload) in cases {
        assert_eq!(
            projected(&run_row(kind, payload.as_deref())),
            None,
            "{kind} {payload:?}"
        );
    }
}

#[test]
fn a_run_event_the_hub_would_refuse_is_dropped_before_it_is_sent() {
    let running = json!({"status": "running"}).to_string();
    let row = |edit: &dyn Fn(&mut RunEvent)| {
        let mut row = run_row("feature_status", Some(&running));
        edit(&mut row);
        projected(&row)
    };
    assert!(row(&|_| {}).is_some());

    assert_eq!(MAX_FEATURE_ID_BYTES, 256);
    let at_bound = row(&|r| r.run_id = "f".repeat(MAX_FEATURE_ID_BYTES));
    assert_eq!(
        at_bound.map(|e| e["feature_id"].clone()),
        Some(json!("f".repeat(256)))
    );
    assert_eq!(
        row(&|r| r.run_id = "f".repeat(MAX_FEATURE_ID_BYTES + 1)),
        None
    );
    assert_eq!(
        row(&|r| r.run_id = "é".repeat(129)),
        None,
        "258 bytes in 129 chars"
    );
    assert_eq!(row(&|r| r.run_id = "f-01\u{0}23".into()), None);

    assert_eq!(row(&|r| r.offset = 0), None);
    assert_eq!(row(&|r| r.offset = -1), None);
    assert_eq!(row(&|r| r.offset = i64::MIN), None);
    assert_eq!(row(&|r| r.created_at = -1), None);
    assert_eq!(row(&|r| r.created_at = i64::MIN), None);
}

#[test]
fn a_projected_event_keeps_the_rows_offset_and_its_time_in_whole_seconds() {
    let running = json!({"status": "running"}).to_string();
    for (offset, created_at, occurred_at) in [
        (1, 0, 0),
        (2, 999, 0),
        (3, 1_000, 1),
        (77, 1_700_000_000_999, 1_700_000_000),
        (i64::MAX, i64::MAX, i64::MAX as u64 / 1000),
    ] {
        let mut row = run_row("feature_status", Some(&running));
        row.offset = offset;
        row.created_at = created_at;
        let event = hub_event(&row).expect("allowlisted row");
        assert_eq!(event.offset, offset as u64);
        assert_eq!(event.occurred_at, occurred_at);
    }
}

#[test]
fn hello_announces_only_the_pinned_scopes_the_protocol_knows() {
    let pins = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    let sent = hello(
        "install-1",
        "1.3.0",
        "linux",
        "x86_64",
        &pins(&["read", "logs", "gates", "Read", "", "configure", "spend"]),
    );
    assert_eq!(
        serde_json::to_value(sent).expect("serialisable"),
        json!({
            "v": 1,
            "install_id": "install-1",
            "app_version": "1.3.0",
            "os": "linux",
            "arch": "x86_64",
            "scopes": ["read", "gates", "spend"],
        })
    );
    assert!(hello("i", "v", "o", "a", &pins(&["logs"]))
        .scopes
        .is_empty());
    assert!(hello("i", "v", "o", "a", &[]).scopes.is_empty());
}

#[test]
fn events_are_sent_only_when_read_is_pinned() {
    let pins = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    assert!(sends_events(&pins(&["read"])));
    assert!(sends_events(&pins(&["spend", "read", "gates"])));
    assert!(!sends_events(&pins(&["spend", "gates"])));
    assert!(!sends_events(&pins(&["Read", "read ", "logs"])));
    assert!(!sends_events(&[]));
}

#[test]
fn a_socket_end_decides_the_reconnect_by_the_ac4_table() {
    use Reconnect::{AfterBackoff, RefreshNow, Stop};
    let refused = |status, fresh_token| SocketEnd::Refused {
        status,
        fresh_token,
    };
    for (end, verdict) in [
        (SocketEnd::Closed(Some(1001)), AfterBackoff),
        (SocketEnd::Closed(Some(1011)), AfterBackoff),
        (SocketEnd::Closed(None), AfterBackoff),
        (SocketEnd::Silent, AfterBackoff),
        (SocketEnd::Closed(Some(4001)), RefreshNow),
        (SocketEnd::Closed(Some(4002)), Stop(StopReason::Displaced)),
        (
            SocketEnd::Closed(Some(1008)),
            Stop(StopReason::ProtocolViolation),
        ),
        (SocketEnd::Closed(Some(1000)), AfterBackoff),
        (SocketEnd::Closed(Some(1006)), AfterBackoff),
        (SocketEnd::Closed(Some(4000)), AfterBackoff),
        (SocketEnd::Closed(Some(4003)), AfterBackoff),
        (refused(401, false), RefreshNow),
        (refused(401, true), Stop(StopReason::Unauthorized)),
        (refused(503, false), AfterBackoff),
        (refused(503, true), AfterBackoff),
        (refused(400, false), Stop(StopReason::Refused)),
        (refused(400, true), Stop(StopReason::Refused)),
        (refused(500, false), AfterBackoff),
        (refused(500, true), AfterBackoff),
        (SocketEnd::Unreachable, AfterBackoff),
        (refused(502, false), AfterBackoff),
        (refused(429, false), AfterBackoff),
        (refused(403, false), Stop(StopReason::Refused)),
        (refused(404, true), Stop(StopReason::Refused)),
        (refused(302, false), Stop(StopReason::Refused)),
        (refused(200, false), Stop(StopReason::Refused)),
    ] {
        assert_eq!(after(end), verdict, "{end:?}");
        for fresh_token in [false, true] {
            for secs_left in [None, Some(0), Some(PREMATURE_EXPIRY_MARGIN_SECS)] {
                assert_eq!(
                    after_close(end, fresh_token, secs_left),
                    verdict,
                    "{end:?} fresh={fresh_token} left={secs_left:?}"
                );
            }
        }
    }
}

/// A `4001` is premature only on a fresh token the local clock gives more
/// than the margin to; every other `4001` is the AC4 row.
#[test]
fn a_4001_on_a_fresh_token_with_life_left_backs_off_before_refreshing() {
    use Reconnect::{AfterBackoff, RefreshAfterBackoff, RefreshNow};
    let expired = SocketEnd::Closed(Some(4001));
    let margin = Some(PREMATURE_EXPIRY_MARGIN_SECS);
    let beyond = Some(PREMATURE_EXPIRY_MARGIN_SECS + 1);
    for (end, fresh_token, secs_left, verdict) in [
        (expired, true, beyond, RefreshAfterBackoff),
        (expired, true, Some(900), RefreshAfterBackoff),
        (expired, true, Some(u64::MAX), RefreshAfterBackoff),
        (expired, true, margin, RefreshNow),
        (expired, true, Some(0), RefreshNow),
        (expired, true, None, RefreshNow),
        (expired, false, Some(900), RefreshNow),
        (expired, false, None, RefreshNow),
        (SocketEnd::Closed(Some(1011)), true, Some(900), AfterBackoff),
        (SocketEnd::Silent, true, Some(900), AfterBackoff),
    ] {
        assert_eq!(
            after_close(end, fresh_token, secs_left),
            verdict,
            "{end:?} fresh={fresh_token} left={secs_left:?}"
        );
    }
}

#[test]
fn only_an_acked_session_not_judged_premature_resets_the_backoff() {
    use Reconnect::{AfterBackoff, RefreshAfterBackoff, RefreshNow, Stop};
    for verdict in [AfterBackoff, RefreshNow, Stop(StopReason::Displaced)] {
        assert!(resets_backoff(true, verdict), "{verdict:?}");
        assert!(!resets_backoff(false, verdict), "{verdict:?}");
    }
    assert!(!resets_backoff(true, RefreshAfterBackoff));
    assert!(!resets_backoff(false, RefreshAfterBackoff));
}

#[test]
fn a_failed_refresh_backs_off_only_when_nothing_can_have_rotated() {
    use Reconnect::{AfterBackoff, Stop};
    let hub = |error: &str| {
        RefreshEnd::Refused(HubPairingError::HubRefused {
            error: error.into(),
        })
    };
    let read = |body: Value| RefreshEnd::Refused(token_refusal(body.to_string().as_bytes()));
    for (end, verdict) in [
        (RefreshEnd::Unsent, AfterBackoff),
        (RefreshEnd::Unavailable, AfterBackoff),
        (RefreshEnd::Unanswered, Stop(StopReason::RefreshUncertain)),
        (hub("invalid_grant"), Stop(StopReason::NeedsRepair)),
        (hub("invalid_dpop_proof"), Stop(StopReason::Refused)),
        (hub("too_many_requests"), AfterBackoff),
        (hub("invalid_request"), Stop(StopReason::Refused)),
        (hub("internal"), Stop(StopReason::Refused)),
        (
            RefreshEnd::Refused(HubPairingError::ClockSkew),
            Stop(StopReason::ClockSkew),
        ),
        (
            read(json!({"error": "invalid_grant"})),
            Stop(StopReason::NeedsRepair),
        ),
        (
            read(json!({"error": "invalid_dpop_proof"})),
            Stop(StopReason::ClockSkew),
        ),
        (read(json!({"error": "too_many_requests"})), AfterBackoff),
        (
            read(json!({"detail": "bad gateway"})),
            Stop(StopReason::Refused),
        ),
        (
            RefreshEnd::Refused(HubPairingError::Transport("reset".into())),
            Stop(StopReason::Refused),
        ),
    ] {
        assert_eq!(after_refresh(end.clone()), verdict, "{end:?}");
    }
}

const HTML_BODY: &[u8] = b"<html><body><h1>Service Unavailable</h1></body></html>";

/// AC5.4 "or with 503, is retried after backoff": a `503` the Hub did not
/// spell was never forwarded and backs off; any other bare `5xx` may have
/// been, so it stops. A Hub-coded body reads as `token_refusal` reads it.
#[test]
fn a_refused_refresh_is_judged_by_its_hub_code_then_by_its_status() {
    use Reconnect::{AfterBackoff, Stop};
    let coded = |error: &str| json!({ "error": error }).to_string().into_bytes();
    for (status, body, verdict) in [
        (503, HTML_BODY.to_vec(), AfterBackoff),
        (503, Vec::new(), AfterBackoff),
        (503, coded("too_many_requests"), AfterBackoff),
        (503, coded("internal"), Stop(StopReason::Refused)),
        (500, HTML_BODY.to_vec(), Stop(StopReason::RefreshUncertain)),
        (502, HTML_BODY.to_vec(), Stop(StopReason::RefreshUncertain)),
        (504, HTML_BODY.to_vec(), Stop(StopReason::RefreshUncertain)),
        (500, coded("internal"), Stop(StopReason::Refused)),
        (400, coded("invalid_grant"), Stop(StopReason::NeedsRepair)),
        (
            400,
            coded("invalid_dpop_proof"),
            Stop(StopReason::ClockSkew),
        ),
        (400, HTML_BODY.to_vec(), Stop(StopReason::Refused)),
        (429, Vec::new(), Stop(StopReason::Refused)),
    ] {
        let end = refresh_refusal(status, &body);
        assert_eq!(
            after_refresh(end.clone()),
            verdict,
            "{status} {}: {end:?}",
            String::from_utf8_lossy(&body)
        );
        if body.starts_with(b"{") {
            assert_eq!(end, RefreshEnd::Refused(token_refusal(&body)), "{status}");
        }
    }
}

#[test]
fn backoff_doubles_to_the_cap_scaled_by_a_clamped_jitter() {
    for attempt in 0..=10u32 {
        let secs = (1u64 << attempt).min(300);
        for (jitter, permille) in [(0.5, 500), (0.75, 750), (1.0, 1000)] {
            assert_eq!(
                backoff(attempt, jitter),
                Duration::from_millis(secs * permille),
                "attempt {attempt}, jitter {jitter}"
            );
        }
        assert_eq!(backoff(attempt, 0.1), Duration::from_millis(secs * 500));
        assert_eq!(backoff(attempt, 2.0), Duration::from_secs(secs));
    }
    assert_eq!(BACKOFF_CAP_SECS, 300);
    assert_eq!(backoff(9, 1.0), Duration::from_secs(300));
    for attempt in [63, 64, 65, u32::MAX] {
        assert_eq!(backoff(attempt, 1.0), Duration::from_secs(300), "{attempt}");
    }
    assert_eq!(backoff(3, f64::NAN), Duration::from_secs(8));
    assert_eq!(backoff(3, f64::INFINITY), Duration::from_secs(8));
    assert_eq!(backoff(3, f64::NEG_INFINITY), Duration::from_secs(4));
}

fn waking_db(
    tag: &str,
) -> (
    demeteo_core::test_dir::TestDir,
    Arc<SqliteAdapter>,
    WakingRunEvents,
) {
    let (dir, db) = pin_db(tag);
    let db = Arc::new(db);
    let events = WakingRunEvents::new(db.clone());
    (dir, db, events)
}

async fn woken(wake: &tokio::sync::Notify) -> bool {
    tokio::time::timeout(Duration::from_secs(5), wake.notified())
        .await
        .is_ok()
}

#[tokio::test]
async fn an_append_through_the_decorator_wakes_a_pending_waiter() {
    let (_dir, _db, events) = waking_db("hub-wake-pending");
    let wake = events.wake();
    let waiter = tokio::spawn(async move { woken(&wake).await });
    tokio::task::yield_now().await;
    events
        .append("f-1", "feature_status", Some(r#"{"status":"running"}"#), 1)
        .unwrap();
    assert!(waiter.await.unwrap());
}

#[tokio::test]
async fn an_append_with_nobody_waiting_leaves_one_wake_for_the_next_wait() {
    let (_dir, _db, events) = waking_db("hub-wake-permit");
    events.append("f-1", "feature_status", None, 1).unwrap();
    events.append("f-1", "feature_status", None, 2).unwrap();
    let wake = events.wake();
    assert!(woken(&wake).await);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), wake.notified())
            .await
            .is_err(),
        "two appends before the wait collapse into one wake"
    );
}

#[tokio::test]
async fn a_failed_append_does_not_wake() {
    let (_dir, db, events) = waking_db("hub-wake-failed");
    db.conn
        .lock()
        .unwrap()
        .execute_batch("DROP TABLE run_events")
        .unwrap();
    assert!(events.append("f-1", "feature_status", None, 1).is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.wake().notified())
            .await
            .is_err()
    );
}

#[test]
fn the_decorator_reads_what_the_wrapped_store_holds() {
    let (_dir, db, events) = waking_db("hub-wake-reads");
    let first = db.append("f-1", "feature_status", None, 1).unwrap();
    let second = events
        .append("f-2", "step_progress", Some("{}"), 2)
        .unwrap();
    let third = db.append("f-1", "gate_required", None, 3).unwrap();
    let offsets = |rows: Vec<RunEvent>| rows.iter().map(|e| e.offset).collect::<Vec<_>>();
    assert_eq!(
        offsets(events.list_after(0, 10).unwrap()),
        [first, second, third]
    );
    assert_eq!(offsets(events.list_after(first, 1).unwrap()), [second]);
    assert_eq!(
        offsets(events.list_since("f-1", 0).unwrap()),
        [first, third]
    );
    assert_eq!(offsets(events.list_since("f-1", first).unwrap()), [third]);
    assert_eq!(
        offsets(db.list_after(0, 10).unwrap()),
        offsets(events.list_after(0, 10).unwrap())
    );
}

fn form_pairs(body: &str) -> Vec<(String, String)> {
    reqwest::Url::parse(&format!("http://x/?{body}"))
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

#[test]
fn refresh_form_body_parses_back_to_the_refresh_grant_and_its_token() {
    let token = "dhr_r0+/= &?~";
    let body = refresh_form_body(token);
    assert_eq!(
        body,
        "grant_type=refresh_token&refresh_token=dhr_r0%2B%2F%3D%20%26%3F~"
    );
    let expected = [("grant_type", "refresh_token"), ("refresh_token", token)]
        .map(|(k, v)| (k.to_owned(), v.to_owned()));
    assert_eq!(form_pairs(&body), expected);
}

/// `refresh_response`'s serialization in `crates/demeteo-hub/src/pairing/grant.rs`.
fn hub_refresh_body() -> Value {
    json!({
        "access_token": ACCESS_TOKEN,
        "token_type": "DPoP",
        "expires_in": 900,
        "refresh_token": REFRESH_TOKEN,
    })
}

#[test]
fn a_refresh_answer_parses_the_hubs_shape_into_the_rotated_pair() {
    let answer: RefreshAnswer = serde_json::from_value(hub_refresh_body()).unwrap();
    assert_eq!(answer.expires_in(), 900);
    let tokens = answer.into_tokens();
    assert_eq!(tokens.access.expose(), ACCESS_TOKEN);
    assert_eq!(tokens.refresh.expose(), REFRESH_TOKEN);

    for member in ["access_token", "token_type", "expires_in", "refresh_token"] {
        let mut body = hub_refresh_body();
        body.as_object_mut().unwrap().remove(member);
        assert!(
            serde_json::from_value::<RefreshAnswer>(body).is_err(),
            "{member}"
        );
    }
}

async fn refresh_with(url: &str, token: &str) -> Result<RefreshAnswer, RefreshEnd> {
    ReqwestTokenEndpoint::new()
        .expect("an HTTP client")
        .refresh(url, refresh_form_body(token), TOKEN_DPOP.to_owned())
        .await
}

#[tokio::test]
async fn a_refresh_that_never_connects_is_unsent() {
    let closed = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let url = format!("http://{}/token", closed.local_addr().unwrap());
    drop(closed);

    let end = refresh_with(&url, REFRESH_TOKEN).await.err();

    assert_eq!(end, Some(RefreshEnd::Unsent));
}

#[tokio::test]
async fn a_refresh_read_and_closed_without_an_answer_is_unanswered() {
    use tokio::io::AsyncReadExt;

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("a loopback bind");
    let url = format!("http://{}/token", listener.local_addr().unwrap());
    let read = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut request = Vec::new();
        let mut chunk = [0u8; 4096];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut chunk).await.map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("the request ended inside its head".to_owned());
            }
            request.extend_from_slice(&chunk[..n]);
        }
        Ok::<_, String>(())
    });

    let end = refresh_with(&url, REFRESH_TOKEN).await.err();

    read.await
        .unwrap()
        .expect("the request reached the listener");
    assert_eq!(end, Some(RefreshEnd::Unanswered));
}

/// A refresh of `REFRESH_TOKEN` answered by [`token_stub_expecting`].
async fn refresh_answered(status: u16, body: &'static str) -> Result<RefreshAnswer, RefreshEnd> {
    let (url, served) = token_stub_expecting(
        refresh_form_body(REFRESH_TOKEN),
        status,
        "application/json",
        body,
    )
    .await;
    let result = refresh_with(&url, REFRESH_TOKEN).await;
    served.await.unwrap().expect("the scripted request");
    result
}

#[tokio::test]
async fn a_refresh_posts_the_grant_with_one_dpop_header_and_reads_the_rotated_pair() {
    const ROTATED: &str = r#"{"access_token":"dha_next","token_type":"DPoP",
        "expires_in":900,"refresh_token":"dhr_next"}"#;
    let result = refresh_answered(200, ROTATED).await;
    let Ok(answer) = result else {
        panic!("{:?}", result.err());
    };
    let tokens = answer.into_tokens();
    assert_eq!(
        (tokens.access.expose(), tokens.refresh.expose()),
        ("dha_next", "dhr_next")
    );
}

#[tokio::test]
async fn a_refresh_refused_by_the_hub_is_refused_as_token_refusal_reads_it() {
    let end = refresh_answered(400, r#"{"error":"invalid_grant"}"#)
        .await
        .err();
    assert_eq!(
        end,
        Some(RefreshEnd::Refused(HubPairingError::HubRefused {
            error: "invalid_grant".into()
        }))
    );
    let Err(busy) = refresh_answered(503, r#"{"error":"too_many_requests"}"#).await else {
        panic!("a 503 read as an answer");
    };
    assert_eq!(after_refresh(busy), Reconnect::AfterBackoff);
    let html = std::str::from_utf8(HTML_BODY).expect("an ASCII body");
    for (status, end) in [
        (503, RefreshEnd::Unavailable),
        (502, RefreshEnd::Unanswered),
    ] {
        assert_eq!(refresh_answered(status, html).await.err(), Some(end));
    }
}

#[tokio::test]
async fn a_refresh_answered_200_without_a_readable_pair_is_unanswered() {
    let end = refresh_answered(200, r#"{"access_token":"dha_only"}"#)
        .await
        .err();
    assert_eq!(end, Some(RefreshEnd::Unanswered));
}

#[tokio::test]
async fn the_scripted_refresh_endpoint_answers_only_its_script_and_records_every_call() {
    let endpoint = ScriptedRefreshEndpoint::new();
    endpoint.answer("dha_1", "dhr_1", 900);
    endpoint.fail(RefreshEnd::Unsent);

    let Ok(first) = endpoint.refresh("u1", "b1".into(), "p1".into()).await else {
        panic!("the scripted answer was not given");
    };
    let second = endpoint.refresh("u2", "b2".into(), "p2".into()).await.err();
    assert_eq!(endpoint.unscripted(), 0);
    let third = endpoint.refresh("u3", "b3".into(), "p3".into()).await.err();

    assert_eq!(first.into_tokens().refresh.expose(), "dhr_1");
    assert_eq!(second, Some(RefreshEnd::Unsent));
    assert_eq!(third, Some(RefreshEnd::Unanswered));
    assert_eq!(endpoint.unscripted(), 1);
    let call = |n: u8| RefreshCall {
        token_url: format!("u{n}"),
        form_body: format!("b{n}"),
        dpop: format!("p{n}"),
    };
    assert_eq!(endpoint.calls(), [call(1), call(2), call(3)]);
}

#[tokio::test]
async fn a_held_scripted_refresh_waits_for_release() {
    let endpoint = Arc::new(ScriptedRefreshEndpoint::new());
    endpoint.answer("dha_1", "dhr_1", 900);
    endpoint.hold();
    let call = tokio::spawn({
        let endpoint = Arc::clone(&endpoint);
        async move { endpoint.refresh("u", "b".into(), "p".into()).await.is_ok() }
    });

    endpoint.called_times(1).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!call.is_finished());
    endpoint.release();

    assert!(call.await.unwrap());
    assert_eq!(endpoint.unscripted(), 0);
}

fn presented_refresh(call: &RefreshCall) -> String {
    let pairs = form_pairs(&call.form_body);
    match &pairs[..] {
        [(grant, kind), (name, token)] if grant == "grant_type" && name == "refresh_token" => {
            assert_eq!(kind, "refresh_token");
            token.clone()
        }
        _ => panic!("not a refresh grant: {pairs:?}"),
    }
}

/// A desktop paired with [`ORIGIN`] holding `refresh`, whose refreshes reach
/// a [`ScriptedRefreshEndpoint`] and whose code exchanges fail.
struct RefreshRig {
    dir: demeteo_core::test_dir::TestDir,
    db: Arc<SqliteAdapter>,
    secrets: Arc<MemorySecretStore>,
    endpoint: Arc<ScriptedRefreshEndpoint>,
    key: DeviceKey,
    hub: Arc<HubPairing>,
}

impl RefreshRig {
    fn new(tag: &str, refresh: &str) -> Self {
        let (dir, db) = pin_db(tag);
        let db = Arc::new(db);
        let key = DeviceKey::generate().unwrap();
        let secrets = Arc::new(
            MemorySecretStore::paired(db.as_ref(), &pins(ORIGIN, &[]), &key, refresh).unwrap(),
        );
        let endpoint = Arc::new(ScriptedRefreshEndpoint::new());
        let token = Arc::new(ScriptedToken {
            authorize: Recorded::default(),
            reply: Reply::Answer,
            verifiers: Mutex::default(),
        });
        let hub = HubPairing::new(db.clone(), secrets.clone(), token)
            .with_refresh_endpoint(endpoint.clone());
        Self {
            dir,
            db,
            secrets,
            endpoint,
            key,
            hub: Arc::new(hub),
        }
    }

    fn stored_refresh(&self) -> Option<String> {
        let account = hub_keyring_account(&self.db.install_id().unwrap(), HubSecret::RefreshToken);
        self.secrets.entries().get(&account).cloned()
    }
}

/// The first refresh is parked inside the endpoint before the second is
/// first polled, so the second can only miss the endpoint by waiting.
#[tokio::test]
async fn concurrent_refreshes_reach_the_hub_in_turn_and_present_each_token_once() {
    let rig = RefreshRig::new("hub_refresh_serial", "dhr_r0");
    rig.endpoint.answer("dha_a1", "dhr_r1", 900);
    rig.endpoint.answer("dha_a2", "dhr_r2", 900);
    rig.endpoint.hold();

    let (first, second, ()) =
        tokio::join!(rig.hub.refresh_access(), rig.hub.refresh_access(), async {
            rig.endpoint.called_times(1).await;
            tokio::task::yield_now().await;
            rig.endpoint.release();
        });

    let presented: Vec<String> = rig.endpoint.calls().iter().map(presented_refresh).collect();
    assert_eq!(presented, ["dhr_r0", "dhr_r1"]);
    assert_eq!(rig.endpoint.unscripted(), 0);
    assert_eq!(rig.stored_refresh().as_deref(), Some("dhr_r2"));
    let mut issued = [first, second].map(|access| access.unwrap().expose().to_owned());
    issued.sort();
    assert_eq!(issued, ["dha_a1", "dha_a2"]);
    assert_eq!(rig.hub.access_token().unwrap().expose(), "dha_a2");
}

#[tokio::test]
async fn a_refreshed_access_token_is_held_in_memory_and_never_stored() {
    let rig = RefreshRig::new("hub_refresh_memory", "dhr_r0");
    rig.endpoint.answer("dha_a1", "dhr_r1", 900);

    let access = rig.hub.refresh_access().await.expect("a refresh");

    assert_eq!(access.expose(), "dha_a1");
    assert_eq!(rig.hub.access_token().unwrap().expose(), "dha_a1");
    let written = rig.secrets.written();
    assert!(written.iter().any(|v| v == "dhr_r1"), "{written:?}");
    assert!(!written.iter().any(|v| v == "dha_a1"));
    let leaks = found_in_db(
        &rig.db,
        &rig.dir.path().join("demeteo.db"),
        &[("access token", b"dha_a1".to_vec())],
    );
    assert!(leaks.is_empty(), "{leaks:?}");
}

#[tokio::test]
async fn a_pairing_holds_its_access_token_in_memory_and_never_stores_it() {
    let rig = Rig::new("hub_pair_access_memory", Reply::Answer);

    rig.pair(HUB_URL).await.expect("a pairing");

    assert_eq!(rig.hub.access_token().unwrap().expose(), ACCESS_TOKEN);
    assert!(!rig.secrets.written().iter().any(|v| v == ACCESS_TOKEN));
    assert!(rig.hub.disconnect().await.is_ok());
    assert!(rig.hub.access_token().is_none());
}

#[tokio::test]
async fn a_refresh_without_an_endpoint_a_pairing_or_a_token_sends_nothing() {
    let unconfigured = RefreshRig::new("hub_refresh_no_endpoint", "dhr_r0");
    let bare = HubPairing::new(
        unconfigured.db.clone(),
        unconfigured.secrets.clone(),
        Arc::new(ScriptedToken {
            authorize: Recorded::default(),
            reply: Reply::Answer,
            verifiers: Mutex::default(),
        }),
    );
    let no_token = RefreshRig::new("hub_refresh_no_token", "dhr_r0");
    no_token
        .secrets
        .delete(&hub_keyring_account(
            &no_token.db.install_id().unwrap(),
            HubSecret::RefreshToken,
        ))
        .unwrap();
    let no_pins = RefreshRig::new("hub_refresh_no_pins", "dhr_r0");
    no_pins.db.clear().unwrap();

    for (case, end) in [
        ("no endpoint", bare.refresh_access().await.err()),
        ("no token", no_token.hub.refresh_access().await.err()),
        ("no pins", no_pins.hub.refresh_access().await.err()),
    ] {
        assert!(
            matches!(end, Some(RefreshEnd::Refused(HubPairingError::Storage(_)))),
            "{case}: {end:?}"
        );
    }
    assert!(no_token.endpoint.calls().is_empty());
    assert!(no_pins.endpoint.calls().is_empty());
}

/// The rotation has happened at the Hub, so the access token issued with a
/// refresh token that could not be kept is not handed out either.
#[tokio::test]
async fn a_rotation_the_keyring_cannot_keep_yields_no_access_token() {
    let rig = RefreshRig::new("hub_refresh_unkept", "dhr_r0");
    rig.endpoint.answer("dha_a1", "dhr_r1", 900);
    rig.endpoint.hold();

    let (end, ()) = tokio::join!(rig.hub.refresh_access(), async {
        rig.endpoint.called_times(1).await;
        rig.secrets.set_available(false);
        rig.endpoint.release();
    });

    assert!(
        matches!(end, Err(RefreshEnd::Refused(HubPairingError::Storage(_)))),
        "{:?}",
        end.map(|_| "an access token")
    );
    assert!(rig.hub.access_token().is_none());
    assert_eq!(rig.endpoint.calls().len(), 1);
}

fn socket_upgrade(access: &str) -> SocketUpgrade {
    let answer: RefreshAnswer = serde_json::from_value(json!({
        "access_token": access,
        "token_type": "DPoP",
        "expires_in": 900,
        "refresh_token": "dhr_unused",
    }))
    .unwrap();
    SocketUpgrade {
        url: "wss://hub.example.com/socket".to_owned(),
        access_token: answer.into_tokens().access,
        dpop: "proof".to_owned(),
    }
}

#[test]
fn a_socket_upgrade_never_prints_its_access_token() {
    let printed = format!("{:?}", socket_upgrade("dha_secret"));
    assert!(!printed.contains("dha_secret"), "{printed}");
}

/// An unscripted dial is refused with a status the supervisor stops on, so
/// a test that scripts too few dials fails on its count, not its timeout.
#[tokio::test]
async fn an_unscripted_dial_is_refused_and_counted() {
    let dialer = ScriptedSocketDialer::new();
    dialer.refuse(DialFailure::Refused {
        status: 401,
        error: Some("invalid_token".to_owned()),
    });

    let first = dialer.dial(socket_upgrade("dha_1")).await.err();
    let second = dialer.dial(socket_upgrade("dha_2")).await.err();

    assert_eq!(
        first,
        Some(DialFailure::Refused {
            status: 401,
            error: Some("invalid_token".to_owned()),
        })
    );
    assert_eq!(
        second,
        Some(DialFailure::Refused {
            status: ScriptedSocketDialer::UNSCRIPTED_STATUS,
            error: Some(ScriptedSocketDialer::UNSCRIPTED.to_owned()),
        })
    );
    for fresh_token in [false, true] {
        assert_eq!(
            after(SocketEnd::Refused {
                status: ScriptedSocketDialer::UNSCRIPTED_STATUS,
                fresh_token,
            }),
            Reconnect::Stop(StopReason::Refused)
        );
    }
    assert_eq!(dialer.unscripted(), 1);
    let tokens: Vec<_> = dialer
        .upgrades()
        .iter()
        .map(|u| u.access_token.expose().to_owned())
        .collect();
    assert_eq!(tokens, ["dha_1", "dha_2"]);
}

#[tokio::test]
async fn a_scripted_connection_replays_then_closes_and_refuses_sends() {
    let dialer = ScriptedSocketDialer::new();
    let connection =
        ScriptedConnection::new([SocketReceived::Text("{}".to_owned()), SocketReceived::Alive]);
    let log = connection.log();
    dialer.connect(connection);
    let dialed_at = Arc::new(Mutex::new(Vec::new()));
    let seen = dialed_at.clone();
    dialer.on_dial(move |u| seen.lock().unwrap().push(u.url.clone()));

    let mut socket = dialer.dial(socket_upgrade("dha_1")).await.unwrap();
    assert_eq!(socket.send_text("hello".to_owned()).await, Ok(()));
    let mut received = Vec::new();
    for _ in 0..4 {
        received.push(socket.received().await);
    }
    let late = socket.send_text("late".to_owned()).await;
    socket.close(1000).await;

    assert_eq!(
        received,
        [
            SocketReceived::Text("{}".to_owned()),
            SocketReceived::Alive,
            SocketReceived::Closed(None),
            SocketReceived::Closed(None),
        ]
    );
    assert_eq!(late, Err(SocketGone));
    assert_eq!(log.unscripted(), 2, "both reads past the script");
    assert_eq!(log.sent(), ["hello"]);
    assert_eq!(log.closed_with(), Some(1000));
    assert_eq!(*dialed_at.lock().unwrap(), ["wss://hub.example.com/socket"]);
    assert_eq!(dialer.unscripted(), 0);
}

/// Records each wait with the number of `/token` calls made before it, so a
/// test can tell a backoff before a refresh from one after it.
struct CountingSleeper {
    endpoint: Arc<ScriptedRefreshEndpoint>,
    waits: Mutex<Vec<(Duration, usize)>>,
}

#[async_trait::async_trait]
impl Sleeper for CountingSleeper {
    async fn sleep(&self, duration: Duration) {
        let calls = self.endpoint.calls().len();
        self.waits.lock().unwrap().push((duration, calls));
        tokio::task::yield_now().await;
    }
}

impl RefreshRig {
    /// Supervises on its own task until it stops of its own accord, and
    /// returns why with every wait it asked for. A supervisor that never
    /// stops fails the timeout rather than hanging the suite.
    async fn supervise(
        &self,
        dialer: &Arc<ScriptedSocketDialer>,
    ) -> (StopReason, Vec<(Duration, usize)>) {
        let run_events = Arc::new(WakingRunEvents::new(self.db.clone()));
        let sleeper = Arc::new(CountingSleeper {
            endpoint: self.endpoint.clone(),
            waits: Mutex::default(),
        });
        let deps = SupervisorDeps {
            tokens: self.hub.clone(),
            origin: ORIGIN.to_owned(),
            key: DeviceKey::from_pkcs8(&self.key.to_pkcs8().unwrap()).unwrap(),
            dialer: dialer.clone(),
            session: SessionContext::new(
                self.db.install_id().unwrap(),
                "0.0.0-test".to_owned(),
                vec!["read".to_owned()],
                run_events.clone(),
                run_events.wake(),
                Arc::new(ScriptedSnapshotSource::default()),
                Arc::new(tokio::sync::Notify::new()),
            ),
            sleeper: sleeper.clone(),
            jitter: Box::new(|| 1.0),
        };
        let (_stop, stopped) = tokio::sync::watch::channel(false);
        let reason = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::spawn(supervise(deps, stopped)),
        )
        .await
        .expect("the supervisor stops")
        .expect("the supervisor did not panic");
        assert_eq!(dialer.unscripted(), 0, "an unscripted dial");
        assert_eq!(self.endpoint.unscripted(), 0, "an unscripted refresh");
        let waits = sleeper.waits.lock().unwrap().clone();
        (reason, waits)
    }
}

fn closing_with(code: u16) -> ScriptedConnection {
    ScriptedConnection::new([SocketReceived::Closed(Some(code))])
}

fn dialed_tokens(dialer: &ScriptedSocketDialer) -> Vec<String> {
    dialer
        .upgrades()
        .iter()
        .map(|upgrade| upgrade.access_token.expose().to_owned())
        .collect()
}

/// A refusal as the real endpoint hands it over: read by `token_refusal`.
fn refresh_refused(error: &str) -> RefreshEnd {
    RefreshEnd::Refused(token_refusal(
        json!({ "error": error }).to_string().as_bytes(),
    ))
}

/// AC5.4: a refresh the Hub may have rotated, or refused for good, is never
/// sent again — the supervisor stops on it, before any dial.
#[tokio::test]
async fn a_refresh_that_may_have_rotated_or_was_refused_stops_with_no_further_token_call() {
    for (tag, end, reason) in [
        (
            "unanswered",
            RefreshEnd::Unanswered,
            StopReason::RefreshUncertain,
        ),
        (
            "invalid_grant",
            refresh_refused("invalid_grant"),
            StopReason::NeedsRepair,
        ),
        (
            "invalid_dpop_proof",
            refresh_refused("invalid_dpop_proof"),
            StopReason::ClockSkew,
        ),
        (
            "invalid_dpop_proof_unread",
            RefreshEnd::Refused(HubPairingError::HubRefused {
                error: "invalid_dpop_proof".to_owned(),
            }),
            StopReason::Refused,
        ),
        (
            "bare_502",
            refresh_refusal(502, HTML_BODY),
            StopReason::RefreshUncertain,
        ),
    ] {
        let rig = RefreshRig::new(&format!("hub_supervise_stops_{tag}"), "dhr_r0");
        rig.endpoint.fail(end);
        let dialer = Arc::new(ScriptedSocketDialer::new());

        let (stopped, waits) = rig.supervise(&dialer).await;

        assert_eq!(stopped, reason, "{tag}");
        assert_eq!(rig.endpoint.calls().len(), 1, "{tag}");
        assert!(dialer.upgrades().is_empty(), "{tag}");
        assert!(waits.is_empty(), "{tag}: {waits:?}");
        assert_eq!(rig.stored_refresh().as_deref(), Some("dhr_r0"), "{tag}");
    }
}

/// AC5.4: a refresh that never left, or that the Hub turned away before
/// reading its token, is waited out and sent again with the same token.
#[tokio::test]
async fn a_refresh_nothing_can_have_rotated_backs_off_then_refreshes_again() {
    for (tag, end) in [
        ("unsent", RefreshEnd::Unsent),
        ("busy_503", refresh_refused("too_many_requests")),
        ("bare_503", refresh_refusal(503, HTML_BODY)),
    ] {
        let rig = RefreshRig::new(&format!("hub_supervise_retries_{tag}"), "dhr_r0");
        rig.endpoint.fail(end);
        rig.endpoint.answer("dha_a1", "dhr_r1", 900);
        let dialer = Arc::new(ScriptedSocketDialer::new());
        dialer.connect(closing_with(4002));

        let (stopped, waits) = rig.supervise(&dialer).await;

        assert_eq!(stopped, StopReason::Displaced, "{tag}");
        assert_eq!(
            waits,
            [(backoff(0, 1.0), 1)],
            "{tag}: one wait, between the calls"
        );
        let presented: Vec<String> = rig.endpoint.calls().iter().map(presented_refresh).collect();
        assert_eq!(presented, ["dhr_r0", "dhr_r0"], "{tag}");
        assert_eq!(dialed_tokens(&dialer), ["dha_a1"], "{tag}");
    }
}

/// AC4: a `401` to a token held from before this attempt is that token
/// expiring, and is refreshed once and redialled with no backoff.
#[tokio::test]
async fn an_upgrade_401_to_a_held_token_refreshes_once_and_redials() {
    let rig = RefreshRig::new("hub_supervise_401_held", "dhr_r0");
    rig.endpoint.answer("dha_a1", "dhr_r1", 900);
    rig.endpoint.answer("dha_a2", "dhr_r2", 900);
    rig.hub.refresh_access().await.expect("the held token");
    let dialer = Arc::new(ScriptedSocketDialer::new());
    dialer.refuse(DialFailure::Refused {
        status: 401,
        error: Some("invalid_token".to_owned()),
    });
    dialer.connect(closing_with(4002));

    let (stopped, waits) = rig.supervise(&dialer).await;

    assert_eq!(stopped, StopReason::Displaced);
    let presented: Vec<String> = rig.endpoint.calls().iter().map(presented_refresh).collect();
    assert_eq!(
        presented,
        ["dhr_r0", "dhr_r1"],
        "one refresh while supervised"
    );
    assert_eq!(dialed_tokens(&dialer), ["dha_a1", "dha_a2"]);
    assert!(waits.is_empty(), "{waits:?}");
}

/// AC4: a `401` to a token issued for this very attempt would meet a `401`
/// again, so it stops instead of refreshing.
#[tokio::test]
async fn an_upgrade_401_to_a_freshly_refreshed_token_stops_unauthorized() {
    let rig = RefreshRig::new("hub_supervise_401_fresh", "dhr_r0");
    rig.endpoint.answer("dha_a1", "dhr_r1", 900);
    let dialer = Arc::new(ScriptedSocketDialer::new());
    dialer.refuse(DialFailure::Refused {
        status: 401,
        error: Some("invalid_token".to_owned()),
    });

    let (stopped, waits) = rig.supervise(&dialer).await;

    assert_eq!(stopped, StopReason::Unauthorized);
    assert_eq!(rig.endpoint.calls().len(), 1);
    assert_eq!(dialed_tokens(&dialer), ["dha_a1"]);
    assert!(waits.is_empty(), "{waits:?}");
}

/// AC4: a busy Hub is waited out with the token already held.
#[tokio::test]
async fn an_upgrade_503_backs_off_without_refreshing() {
    let rig = RefreshRig::new("hub_supervise_503", "dhr_r0");
    rig.endpoint.answer("dha_a1", "dhr_r1", 900);
    rig.hub.refresh_access().await.expect("the held token");
    let dialer = Arc::new(ScriptedSocketDialer::new());
    dialer.refuse(DialFailure::Refused {
        status: 503,
        error: Some("too_many_requests".to_owned()),
    });
    dialer.connect(closing_with(4002));

    let (stopped, waits) = rig.supervise(&dialer).await;

    assert_eq!(stopped, StopReason::Displaced);
    assert_eq!(waits, [(backoff(0, 1.0), 1)]);
    assert_eq!(
        rig.endpoint.calls().len(),
        1,
        "only the refresh before supervising"
    );
    assert_eq!(dialed_tokens(&dialer), ["dha_a1", "dha_a1"]);
}

/// A `4001` on a just-issued token the local clock gives most of its life
/// to is not that token expiring. Each one waits longer than the last, the
/// ack of the session it ended notwithstanding, and is followed by exactly
/// one refresh.
#[tokio::test]
async fn a_premature_4001_backs_off_longer_each_time_with_one_refresh_per_wait() {
    let rig = RefreshRig::new("hub_supervise_premature_4001", "dhr_r0");
    for n in 1..=4 {
        rig.endpoint
            .answer(&format!("dha_a{n}"), &format!("dhr_r{n}"), 900);
    }
    let dialer = Arc::new(ScriptedSocketDialer::new());
    for _ in 0..3 {
        dialer.connect(ScriptedConnection::new([
            first_ack(),
            SocketReceived::Closed(Some(4001)),
        ]));
    }
    dialer.connect(closing_with(4002));

    let (stopped, waits) = rig.supervise(&dialer).await;

    assert_eq!(stopped, StopReason::Displaced);
    assert_eq!(
        waits,
        [
            (backoff(0, 1.0), 1),
            (backoff(1, 1.0), 2),
            (backoff(2, 1.0), 3)
        ],
        "a wait between every two /token calls, each longer"
    );
    assert_eq!(rig.endpoint.calls().len(), 4);
    assert_eq!(
        dialed_tokens(&dialer),
        ["dha_a1", "dha_a2", "dha_a3", "dha_a4"]
    );
}

/// Each end the Hub never acked waits longer than the last; a session it
/// acked starts the count again, so the wait after it is the first one.
#[tokio::test]
async fn backoff_grows_until_a_session_is_acked_and_starts_again_after_it() {
    let rig = RefreshRig::new("hub_supervise_backoff_resets", "dhr_r0");
    rig.endpoint.answer("dha_a1", "dhr_r1", 900);
    let dialer = Arc::new(ScriptedSocketDialer::new());
    let unreachable = || DialFailure::Unreachable("wss://hub.example.com/socket".to_owned());
    dialer.refuse(unreachable());
    dialer.refuse(unreachable());
    dialer.connect(ScriptedConnection::new([
        first_ack(),
        SocketReceived::Closed(None),
    ]));
    dialer.refuse(unreachable());
    dialer.connect(closing_with(4002));

    let (stopped, waits) = rig.supervise(&dialer).await;

    assert_eq!(stopped, StopReason::Displaced);
    assert_eq!(
        waits,
        [
            (backoff(0, 1.0), 1),
            (backoff(1, 1.0), 1),
            (backoff(0, 1.0), 1),
            (backoff(1, 1.0), 1)
        ]
    );
    assert_eq!(dialed_tokens(&dialer), ["dha_a1"; 5]);
}

/// Records each wait, then never wakes: a socket that backs off stays in
/// that backoff until it is stopped, however late the test stops it.
struct ParkedSleeper(tokio::sync::watch::Sender<Vec<Duration>>);

impl ParkedSleeper {
    fn new() -> Arc<Self> {
        Arc::new(Self(tokio::sync::watch::Sender::new(Vec::new())))
    }

    /// Resolves, with the waits so far, once a socket has parked.
    async fn parked(&self) -> Vec<Duration> {
        let mut waits = self.0.subscribe();
        waits
            .wait_for(|waits| !waits.is_empty())
            .await
            .map(|parked| parked.clone())
            .expect("the sender lives as long as self")
    }
}

#[async_trait::async_trait]
impl Sleeper for ParkedSleeper {
    async fn sleep(&self, duration: Duration) {
        self.0.send_modify(|waits| waits.push(duration));
        std::future::pending::<()>().await;
    }
}

fn socket_config(
    db: &Arc<SqliteAdapter>,
    dialer: &Arc<ScriptedSocketDialer>,
    sleeper: &Arc<ParkedSleeper>,
) -> SocketConfig {
    let run_events = Arc::new(WakingRunEvents::new(db.clone()));
    SocketConfig {
        dialer: dialer.clone(),
        wake: run_events.wake(),
        run_events,
        snapshots: Arc::new(ScriptedSnapshotSource::default()),
        snapshot_wake: Arc::new(tokio::sync::Notify::new()),
        snapshot_debounce: Duration::from_secs(SNAPSHOT_DEBOUNCE_SECS),
        runtime: tokio::runtime::Handle::current(),
        sleeper: sleeper.clone(),
        jitter: || 1.0,
    }
}

impl Rig {
    /// A [`Rig`] whose pairing starts a socket on `dialer`, waiting on
    /// `sleeper`.
    fn socketed(
        tag: &str,
        reply: Reply,
        dialer: &Arc<ScriptedSocketDialer>,
        sleeper: &Arc<ParkedSleeper>,
    ) -> Self {
        let mut rig = Self::new(tag, reply);
        let hub = HubPairing::new(rig.db.clone(), rig.secrets.clone(), rig.token.clone())
            .with_socket(socket_config(&rig.db, dialer, sleeper));
        rig.hub = Arc::new(hub);
        rig
    }
}

impl RefreshRig {
    /// This rig's pairing, with a socket on `dialer`. A socket task holds a
    /// clone of `dialer` while it runs, so its strong count tells a test
    /// whether one is running.
    fn socketed(&self, dialer: &Arc<ScriptedSocketDialer>) -> Arc<HubPairing> {
        let token = Arc::new(ScriptedToken {
            authorize: Recorded::default(),
            reply: Reply::Answer,
            verifiers: Mutex::default(),
        });
        let hub = HubPairing::new(self.db.clone(), self.secrets.clone(), token)
            .with_refresh_endpoint(self.endpoint.clone())
            .with_socket(socket_config(&self.db, dialer, &ParkedSleeper::new()));
        Arc::new(hub)
    }
}

async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

/// AC6.1: a desktop with nothing to authenticate a socket with spawns none.
#[tokio::test]
async fn a_socket_without_pins_or_a_refresh_token_dials_nothing() {
    let no_pins = RefreshRig::new("hub_socket_no_pins", "dhr_r0");
    no_pins.db.clear().unwrap();
    let no_token = RefreshRig::new("hub_socket_no_token", "dhr_r0");
    no_token
        .secrets
        .delete(&hub_keyring_account(
            &no_token.db.install_id().unwrap(),
            HubSecret::RefreshToken,
        ))
        .unwrap();

    for (case, rig) in [("no pins", no_pins), ("no token", no_token)] {
        let dialer = Arc::new(ScriptedSocketDialer::new());
        let hub = rig.socketed(&dialer);
        let idle = Arc::strong_count(&dialer);

        hub.start_socket();
        assert_eq!(Arc::strong_count(&dialer), idle, "{case}: a socket task");
        settle().await;

        assert!(dialer.upgrades().is_empty(), "{case}");
        assert!(rig.endpoint.calls().is_empty(), "{case}");
    }
}

/// AC6.2: the socket dials with the access token the pairing was issued,
/// and starting it again while it runs starts no second one. Its dial is
/// refused, so it is parked in a backoff — nothing that contends for the
/// refresh lock, and nothing that ends on its own — when `disconnect` has
/// to wait for it to end.
#[tokio::test]
async fn a_pairing_starts_one_socket_and_disconnect_waits_for_it_to_end() {
    let dialer = Arc::new(ScriptedSocketDialer::new());
    dialer.refuse(DialFailure::Unreachable("connection refused".to_owned()));
    let sleeper = ParkedSleeper::new();
    let rig = Rig::socketed("hub_pair_starts_socket", Reply::Answer, &dialer, &sleeper);
    let idle = Arc::strong_count(&dialer);

    rig.pair(HUB_URL).await.expect("a pairing");
    assert_eq!(Arc::strong_count(&dialer), idle + 1, "a socket task");
    rig.hub.start_socket();
    assert_eq!(Arc::strong_count(&dialer), idle + 1, "one socket task");

    let waits = tokio::time::timeout(Duration::from_secs(5), sleeper.parked())
        .await
        .expect("the socket backs off");
    rig.hub.disconnect().await.expect("a disconnect");

    assert_eq!(
        Arc::strong_count(&dialer),
        idle,
        "the socket task ended first"
    );
    assert_eq!(waits, [backoff(0, 1.0)]);
    assert_eq!(dialed_tokens(&dialer), [ACCESS_TOKEN]);
    assert_eq!(dialer.unscripted(), 0);
}

/// AC6.3: the socket's own refresh, parked in the endpoint, is dropped by
/// the stop signal; `disconnect` neither waits on the endpoint nor leaves
/// that refresh a keyring write to make once it is released.
#[tokio::test]
async fn disconnect_abandons_the_sockets_parked_refresh_and_leaves_no_secret() {
    let rig = RefreshRig::new("hub_disconnect_socket_refresh", "dhr_r0");
    rig.endpoint.answer("dha_a1", "dhr_r1", 900);
    rig.endpoint.hold();
    let dialer = Arc::new(ScriptedSocketDialer::new());
    let hub = rig.socketed(&dialer);
    let idle = Arc::strong_count(&dialer);
    hub.start_socket();
    tokio::time::timeout(Duration::from_secs(5), rig.endpoint.called_times(1))
        .await
        .expect("the socket refreshes");
    assert_eq!(Arc::strong_count(&dialer), idle + 1);

    let status = tokio::time::timeout(Duration::from_secs(5), hub.disconnect())
        .await
        .expect("disconnect does not wait on the endpoint")
        .expect("a disconnect");

    assert_eq!(
        Arc::strong_count(&dialer),
        idle,
        "the socket task ended first"
    );
    assert!(
        rig.secrets.entries().is_empty(),
        "{:?}",
        rig.secrets.entries()
    );
    rig.endpoint.release();
    settle().await;
    assert!(
        rig.secrets.entries().is_empty(),
        "{:?}",
        rig.secrets.entries()
    );
    assert!(!rig.secrets.written().iter().any(|v| v == "dhr_r1"));
    assert_eq!(status.state, HubPairingState::Unpaired);
    assert!(dialer.upgrades().is_empty());
}

/// AC6.3: a refresh another caller has parked in the endpoint holds the
/// refresh lock, so `disconnect` deletes only after its rotation is stored,
/// and the socket queued behind it never reaches the endpoint.
#[tokio::test]
async fn disconnect_deletes_only_after_a_refresh_in_the_endpoint_has_stored_its_rotation() {
    let rig = RefreshRig::new("hub_disconnect_mid_refresh", "dhr_r0");
    rig.endpoint.answer("dha_a1", "dhr_r1", 900);
    rig.endpoint.hold();
    let dialer = Arc::new(ScriptedSocketDialer::new());
    let hub = rig.socketed(&dialer);
    let idle = Arc::strong_count(&dialer);
    let refreshing = tokio::spawn({
        let hub = hub.clone();
        async move { hub.refresh_access().await.map(|_| ()) }
    });
    tokio::time::timeout(Duration::from_secs(5), rig.endpoint.called_times(1))
        .await
        .expect("the refresh reaches the endpoint");
    hub.start_socket();
    settle().await;
    assert_eq!(Arc::strong_count(&dialer), idle + 1);

    let (status, ()) = tokio::join!(hub.disconnect(), async {
        settle().await;
        rig.endpoint.release();
    });

    assert_eq!(
        Arc::strong_count(&dialer),
        idle,
        "the socket task ended first"
    );
    assert!(
        rig.secrets.entries().is_empty(),
        "{:?}",
        rig.secrets.entries()
    );
    let refreshed = refreshing.await.expect("the refresh did not panic");
    assert!(refreshed.is_ok(), "{:?}", refreshed.err());
    assert!(rig.secrets.written().iter().any(|v| v == "dhr_r1"));
    settle().await;
    assert!(
        rig.secrets.entries().is_empty(),
        "{:?}",
        rig.secrets.entries()
    );
    assert!(hub.access_token().is_none());
    assert_eq!(
        status.expect("a disconnect").state,
        HubPairingState::Unpaired
    );
    assert_eq!(rig.endpoint.calls().len(), 1, "the socket never refreshed");
    assert!(dialer.upgrades().is_empty());
}

/// What an [`OpenConnection`] was sent, how often it was read, and the code
/// it was closed with.
#[derive(Clone, Default)]
struct Traffic {
    sent: Vec<String>,
    reads: usize,
    closed_with: Option<u16>,
}

/// Answers what the test feeds it, in order, and is otherwise open and
/// silent: unlike a [`ScriptedConnection`], whose `Closed` is always ready,
/// a session on it gets as far as sending events, and runs until stopped.
struct OpenConnection {
    inbound: tokio::sync::mpsc::UnboundedReceiver<SocketReceived>,
    traffic: tokio::sync::watch::Sender<Traffic>,
}

impl OpenConnection {
    fn new() -> (
        Self,
        tokio::sync::mpsc::UnboundedSender<SocketReceived>,
        tokio::sync::watch::Receiver<Traffic>,
    ) {
        let (feed, inbound) = tokio::sync::mpsc::unbounded_channel();
        let traffic = tokio::sync::watch::Sender::new(Traffic::default());
        let seen = traffic.subscribe();
        (Self { inbound, traffic }, feed, seen)
    }
}

#[async_trait::async_trait]
impl HubSocketConnection for OpenConnection {
    async fn received(&mut self) -> SocketReceived {
        self.traffic.send_modify(|t| t.reads += 1);
        match self.inbound.recv().await {
            Some(frame) => frame,
            None => std::future::pending().await,
        }
    }

    async fn send_text(&mut self, text: String) -> Result<(), SocketGone> {
        self.traffic.send_modify(|t| t.sent.push(text));
        Ok(())
    }

    async fn close(self: Box<Self>, code: u16) {
        self.traffic.send_modify(|t| t.closed_with = Some(code));
    }
}

async fn traffic_until(
    traffic: &mut tokio::sync::watch::Receiver<Traffic>,
    what: &str,
    until: impl FnMut(&Traffic) -> bool,
) -> Traffic {
    tokio::time::timeout(Duration::from_secs(5), traffic.wait_for(until))
        .await
        .unwrap_or_else(|_| panic!("timed out waiting: {what}"))
        .map(|seen| seen.clone())
        .unwrap_or_else(|_| panic!("the connection went away: {what}"))
}

fn first_ack() -> SocketReceived {
    SocketReceived::Text(r#"{"v":1,"offset":0}"#.to_owned())
}

/// `frame` read as the type `built` returns. The protocol's `Hello` and
/// `RunEvent` are reachable from this package only that way, through
/// [`hello`] and [`hub_event`].
fn read_as<T: serde::de::DeserializeOwned>(_built: impl FnOnce() -> T, frame: &str) -> T {
    serde_json::from_str(frame).unwrap_or_else(|e| panic!("{frame}: {e}"))
}

fn announced_scopes(frame: &str) -> Value {
    let hello = read_as(|| hello("", "", "", "", &[]), frame);
    serde_json::to_value(hello.scopes).expect("serialisable")
}

/// Re-pairing is how scopes narrow (docs/HUB.md §4), so the socket running
/// when it happens is replaced: the old one has ended by the time `pair`
/// returns, and the next Hello announces only what the new pairing granted.
/// Without `read`, the row the first socket relayed is not sent again.
#[tokio::test]
async fn a_re_pairing_replaces_the_running_socket_with_one_on_its_scopes() {
    let dialer = Arc::new(ScriptedSocketDialer::new());
    let (first, first_feed, mut first_traffic) = OpenConnection::new();
    let (second, second_feed, mut second_traffic) = OpenConnection::new();
    dialer.connect(first);
    dialer.connect(second);
    first_feed.send(first_ack()).unwrap();
    second_feed.send(first_ack()).unwrap();
    let rig = Rig::socketed(
        "hub_repair_replaces_socket",
        Reply::Scoped(&["read gates", "gates"]),
        &dialer,
        &ParkedSleeper::new(),
    );
    let row = rig
        .db
        .append(
            "f-0123456789abcdef",
            "feature_status",
            Some(r#"{"status":"running"}"#),
            1_700_000_000_000,
        )
        .unwrap();
    let idle = Arc::strong_count(&dialer);

    rig.pair(HUB_URL).await.expect("a pairing");
    let relayed = traffic_until(&mut first_traffic, "the run event", |t| t.sent.len() >= 2).await;
    assert_eq!(announced_scopes(&relayed.sent[0]), json!(["read", "gates"]));
    let event = read_as(
        || hub_event(&run_row("feature_status", Some(r#"{"status":"running"}"#))).unwrap(),
        &relayed.sent[1],
    );
    assert_eq!(i64::try_from(event.offset).unwrap(), row);

    rig.pair(HUB_URL).await.expect("a second pairing");
    assert_eq!(
        first_traffic.borrow().closed_with,
        Some(1001),
        "the first socket ended before pair returned"
    );
    assert_eq!(Arc::strong_count(&dialer), idle + 1, "one socket task");
    let past_ack = traffic_until(&mut second_traffic, "the ack", |t| t.reads >= 2).await;
    assert_eq!(
        past_ack.sent.len(),
        1,
        "only the Hello: {:?}",
        past_ack.sent
    );
    assert_eq!(announced_scopes(&past_ack.sent[0]), json!(["gates"]));

    rig.hub.disconnect().await.expect("a disconnect");
    assert_eq!(Arc::strong_count(&dialer), idle);
    assert_eq!(second_traffic.borrow().sent.len(), 1);
    assert_eq!(dialed_tokens(&dialer), [ACCESS_TOKEN, ACCESS_TOKEN]);
    assert_eq!(dialer.unscripted(), 0);
}

/// `pair` awaits the old socket while holding the refresh lock. A refresh
/// that socket queued on the lock behind the pairing is dropped by the stop
/// signal, so the pairing neither deadlocks on it nor lets it reach the
/// endpoint with the refresh token the pairing just replaced.
#[tokio::test]
async fn a_re_pairing_drops_the_sockets_refresh_queued_behind_it() {
    let dialer = Arc::new(ScriptedSocketDialer::new());
    let (first, first_feed, mut first_traffic) = OpenConnection::new();
    let (second, _second_feed, mut second_traffic) = OpenConnection::new();
    dialer.connect(first);
    dialer.connect(second);
    let endpoint = Arc::new(ScriptedRefreshEndpoint::new());
    endpoint.answer("dha_other", "dhr_other", 900);
    endpoint.hold();
    let mut rig = Rig::new(
        "hub_repair_queued_refresh",
        Reply::Scoped(&["read gates", "gates"]),
    );
    let hub = HubPairing::new(rig.db.clone(), rig.secrets.clone(), rig.token.clone())
        .with_refresh_endpoint(endpoint.clone())
        .with_socket(socket_config(&rig.db, &dialer, &ParkedSleeper::new()));
    rig.hub = Arc::new(hub);
    let idle = Arc::strong_count(&dialer);
    rig.pair(HUB_URL).await.expect("a pairing");
    traffic_until(&mut first_traffic, "the Hello", |t| !t.sent.is_empty()).await;

    let other = tokio::spawn({
        let hub = rig.hub.clone();
        async move { hub.refresh_access().await.map(|_| ()) }
    });
    tokio::time::timeout(Duration::from_secs(5), endpoint.called_times(1))
        .await
        .expect("the other refresh holds the lock in the endpoint");
    let re_pairing = tokio::spawn({
        let hub = rig.hub.clone();
        let redirect = rig.redirect();
        async move { hub.pair(HUB_URL, &redirect, Duration::from_secs(10)).await }
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while rig.token.verifiers.lock().unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the re-pairing exchanged its code and queued on the lock");
    first_feed.send(SocketReceived::Closed(Some(4001))).unwrap();
    settle().await;
    endpoint.release();

    let status = tokio::time::timeout(Duration::from_secs(5), re_pairing)
        .await
        .expect("the re-pairing does not wait on the socket's refresh")
        .expect("the re-pairing did not panic")
        .expect("a second pairing");

    assert_eq!(status.state, HubPairingState::Paired);
    assert!(other.await.unwrap().is_ok());
    assert_eq!(endpoint.calls().len(), 1, "the socket never refreshed");
    assert_eq!(endpoint.unscripted(), 0);
    assert_eq!(
        rig.secrets.entries()[&rig.account(HubSecret::RefreshToken)],
        REFRESH_TOKEN
    );
    assert_eq!(Arc::strong_count(&dialer), idle + 1, "one socket task");
    let hello = traffic_until(&mut second_traffic, "the Hello", |t| !t.sent.is_empty()).await;
    assert_eq!(announced_scopes(&hello.sent[0]), json!(["gates"]));
    assert_eq!(dialed_tokens(&dialer), [ACCESS_TOKEN, ACCESS_TOKEN]);
    rig.hub.disconnect().await.expect("a disconnect");
    assert_eq!(dialer.unscripted(), 0);
}

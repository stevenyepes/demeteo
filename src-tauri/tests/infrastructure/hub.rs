// Tests attached to `src-tauri/src/commands/hub.rs` (mirrored-tests
// convention). They live here rather than in demeteo-core because
// `cd src-tauri && cargo test` runs only this package's tests.
//
// The fixtures are the Hub's spellings, written out by hand rather than
// produced by the code under test: `hub_origin` has to agree byte for byte
// with demeteo-hub's `public_origin`, and `redirect_uri` with its
// `is_loopback_redirect`, and neither crate can be called from here on
// every OS.

use super::{disconnect_core, pair_core, pairing, status_core};
use crate::adapters::database::SqliteAdapter;
use crate::adapters::hub::device_key::DeviceKey;
use crate::adapters::hub::loopback::CallbackListener;
use crate::adapters::hub::testing::MemorySecretStore;
use crate::adapters::hub::token_client::ReqwestTokenEndpoint;
use crate::adapters::hub::HubPairing;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::hub_pairing::answer::token_form_body;
use crate::domain::hub_pairing::callback::{callback_verdict, CallbackVerdict, RefuseReason};
use crate::domain::hub_pairing::dpop::{dpop_proof, dpop_signing_input};
use crate::domain::hub_pairing::jwk::{canonical_jwk, fingerprint, jwk_value, thumbprint};
use crate::domain::hub_pairing::keyring_names::{
    hub_keyring_account, keyring_backend_name, KEYRING_SERVICE,
};
use crate::domain::hub_pairing::pkce::s256_challenge;
use crate::domain::hub_pairing::{
    authorize_url, check_pairing_origin, consent_label, hub_origin, hub_status, pairing_state,
    redirect_uri, token_error, token_refusal, token_url, AuthorizeRequest, HubPairingError,
    HubPairingState, HubPins, HubSecret, HubStatus, KeyringStatus, PairingAnswer, PinnedPasskey,
    PAIR_TIMEOUT, SCOPES,
};
use crate::domain::ids::{FeatureId, ProjectId};
use crate::domain::models::{Feature, Project};
use crate::error::AppError;
use crate::ports::db::{FeatureRepository, ProjectRepository};
use crate::ports::hub::{BrowserLauncher, HubPinRepository, HubSecretStore, HubTokenEndpoint};
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
    let body = json!({
        "access_token": ACCESS_TOKEN,
        "token_type": "DPoP",
        "expires_in": 900,
        "refresh_token": REFRESH_TOKEN,
        "scope": "read gates",
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
            &["credential_id", "public_key", "pinned_at"],
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

/// One scripted `/token` answer on `127.0.0.1:0`. The task returns why the
/// request was not the one scripted, and answers such a request with `418`
/// rather than the script, so an unscripted request can never pass.
async fn token_stub(
    status: u16,
    content_type: &'static str,
    body: &'static str,
) -> (String, tokio::task::JoinHandle<Result<(), String>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("a loopback bind");
    let url = format!("http://{}/token", listener.local_addr().unwrap());
    let expected_body = token_request_body();
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
        self.verifiers.lock().unwrap().push(verifier);
        match self.reply {
            Reply::Answer => Ok(hub_answer()),
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

    let status = rig.hub.disconnect().expect("a disconnect");

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

    let again = rig.hub.disconnect().expect("a second disconnect");
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

#[tokio::test]
async fn the_command_cores_answer_what_the_service_answers() {
    let rig = Rig::new("hub_command_cores", Reply::Answer);
    assert_eq!(status_core(&rig.hub), Ok(rig.hub.status().unwrap()));

    let paired = pair_core(&rig.hub, HUB_URL, &rig.redirect())
        .await
        .expect("a pairing");
    assert_eq!(paired.state, HubPairingState::Paired);
    assert_eq!(status_core(&rig.hub), Ok(paired));

    let disconnected = disconnect_core(&rig.hub).expect("a disconnect");
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

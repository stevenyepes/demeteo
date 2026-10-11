use crate::ports::{DataKey, KeyService, KeyServiceError};
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use reqwest::header::HeaderValue;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

const TOKEN_HEADER: &str = "X-Vault-Token";
const STATUS_SEALED: u16 = 503;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_BAD_REQUEST: u16 = 400;
const KEY_NOT_FOUND_MARKER: &str = "key not found";

#[derive(Clone, Copy)]
enum Op {
    Encrypt,
    Decrypt,
    DataKey,
}

impl Op {
    fn path(self) -> &'static str {
        match self {
            Op::Encrypt => "encrypt",
            Op::Decrypt => "decrypt",
            Op::DataKey => "datakey/plaintext",
        }
    }
}

#[derive(Serialize)]
struct PlaintextBody {
    plaintext: String,
}

#[derive(Serialize)]
struct CiphertextBody<'a> {
    ciphertext: &'a str,
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Deserialize)]
struct EncryptData {
    ciphertext: String,
}

#[derive(Deserialize)]
struct DecryptData {
    plaintext: String,
}

#[derive(Deserialize)]
struct DataKeyData {
    plaintext: String,
    ciphertext: String,
}

#[derive(Deserialize)]
struct ErrorBody {
    errors: Vec<String>,
}

/// Transit client. Address, mount and token are constructor arguments (read
/// from `Config` by the composition root); the key name travels per call.
pub struct OpenBaoKeys {
    http: reqwest::Client,
    addr: String,
    mount: String,
    token: HeaderValue,
}

impl fmt::Debug for OpenBaoKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenBaoKeys")
            .field("addr", &self.addr)
            .field("mount", &self.mount)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl OpenBaoKeys {
    pub fn new(
        http: reqwest::Client,
        addr: &str,
        mount: &str,
        token: &str,
    ) -> Result<Self, KeyServiceError> {
        let mut token = HeaderValue::from_str(token)
            .map_err(|_| KeyServiceError::Transport("token is not a valid header value".into()))?;
        token.set_sensitive(true);
        Ok(Self {
            http,
            addr: addr.to_owned(),
            mount: check_mount(mount)?,
            token,
        })
    }

    async fn call(
        &self,
        op: Op,
        key: &str,
        body: &impl Serialize,
    ) -> Result<Vec<u8>, KeyServiceError> {
        check_key_name(key)?;
        let response = self
            .http
            .post(endpoint_url(&self.addr, &self.mount, op, key))
            .header(TOKEN_HEADER, self.token.clone())
            .json(body)
            .send()
            .await
            .map_err(|e| KeyServiceError::Transport(e.without_url().to_string()))?;
        let status = response.status().as_u16();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| KeyServiceError::Transport(e.without_url().to_string()))?;
        if (200..300).contains(&status) {
            Ok(bytes.to_vec())
        } else {
            Err(classify_failure(status, &bytes, key))
        }
    }
}

#[async_trait]
impl KeyService for OpenBaoKeys {
    async fn encrypt(&self, key: &str, plaintext: &[u8]) -> Result<String, KeyServiceError> {
        let body = self
            .call(Op::Encrypt, key, &plaintext_body(plaintext))
            .await?;
        parse_encrypt(&body)
    }

    async fn decrypt(&self, key: &str, ciphertext: &str) -> Result<Vec<u8>, KeyServiceError> {
        let body = self
            .call(Op::Decrypt, key, &CiphertextBody { ciphertext })
            .await?;
        parse_decrypt(&body)
    }

    async fn data_key(&self, key: &str) -> Result<DataKey, KeyServiceError> {
        let body = self.call(Op::DataKey, key, &serde_json::json!({})).await?;
        parse_data_key(&body)
    }
}

/// The client `OpenBaoKeys` should be built over. Without a total timeout a
/// black-holed OpenBao hangs the startup probe forever instead of letting it
/// report `Transport`.
pub fn http_client(connect: Duration, total: Duration) -> Result<reqwest::Client, KeyServiceError> {
    reqwest::Client::builder()
        .connect_timeout(connect)
        .timeout(total)
        .build()
        .map_err(|e| KeyServiceError::Transport(e.to_string()))
}

/// Key names and mounts are interpolated into the request path, so a `/` or
/// `..` in either would address another OpenBao route under the Hub's token.
/// They are refused rather than percent-encoded: OpenBao stays the authority
/// on what a key name may be, and a bad name fails loudly instead of naming
/// some other key.
fn is_path_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn check_key_name(key: &str) -> Result<(), KeyServiceError> {
    if is_path_segment(key) {
        Ok(())
    } else {
        Err(KeyServiceError::InvalidKeyName(key.to_owned()))
    }
}

fn check_mount(mount: &str) -> Result<String, KeyServiceError> {
    let trimmed = mount.trim_matches('/');
    if trimmed.split('/').all(is_path_segment) {
        Ok(trimmed.to_owned())
    } else {
        Err(KeyServiceError::InvalidMount(mount.to_owned()))
    }
}

fn endpoint_url(addr: &str, mount: &str, op: Op, key: &str) -> String {
    format!(
        "{}/v1/{}/{}/{}",
        addr.trim_end_matches('/'),
        mount.trim_matches('/'),
        op.path(),
        key
    )
}

fn plaintext_body(plaintext: &[u8]) -> PlaintextBody {
    PlaintextBody {
        plaintext: STANDARD.encode(plaintext),
    }
}

fn parse_envelope<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, KeyServiceError> {
    serde_json::from_slice::<Envelope<T>>(body)
        .map(|e| e.data)
        .map_err(|e| KeyServiceError::Malformed(e.to_string()))
}

fn decode_plaintext(encoded: &str) -> Result<Vec<u8>, KeyServiceError> {
    STANDARD
        .decode(encoded)
        .map_err(|e| KeyServiceError::Malformed(e.to_string()))
}

fn parse_encrypt(body: &[u8]) -> Result<String, KeyServiceError> {
    Ok(parse_envelope::<EncryptData>(body)?.ciphertext)
}

fn parse_decrypt(body: &[u8]) -> Result<Vec<u8>, KeyServiceError> {
    decode_plaintext(&parse_envelope::<DecryptData>(body)?.plaintext)
}

fn parse_data_key(body: &[u8]) -> Result<DataKey, KeyServiceError> {
    let data = parse_envelope::<DataKeyData>(body)?;
    Ok(DataKey {
        plaintext: decode_plaintext(&data.plaintext)?,
        wrapped: data.ciphertext,
    })
}

/// OpenBao reports a missing Transit key as 400 (decrypt, datakey) or 404
/// depending on the route, so either is `UnknownKey` only when the error text
/// says the key was not found. A bare 404 also means an unmounted path — a
/// wrong `DEMETEO_HUB_TRANSIT_MOUNT` — and must stay `Upstream` so the operator
/// is not sent looking for a key. The response body is OpenBao's own error
/// text; the request body (which carries plaintext) is never part of an error.
fn classify_failure(status: u16, body: &[u8], key: &str) -> KeyServiceError {
    let errors = serde_json::from_slice::<ErrorBody>(body)
        .map(|b| b.errors.join("; "))
        .unwrap_or_default();
    match status {
        STATUS_SEALED => KeyServiceError::Sealed,
        STATUS_NOT_FOUND | STATUS_BAD_REQUEST if errors.contains(KEY_NOT_FOUND_MARKER) => {
            KeyServiceError::UnknownKey(key.to_owned())
        }
        _ => KeyServiceError::Upstream {
            status,
            detail: errors,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_cover_the_three_transit_routes() {
        let url = |op| endpoint_url("http://bao:8200/", "transit", op, "hub");
        assert_eq!(url(Op::Encrypt), "http://bao:8200/v1/transit/encrypt/hub");
        assert_eq!(url(Op::Decrypt), "http://bao:8200/v1/transit/decrypt/hub");
        assert_eq!(
            url(Op::DataKey),
            "http://bao:8200/v1/transit/datakey/plaintext/hub"
        );
    }

    #[test]
    fn mount_slashes_are_normalised() {
        assert_eq!(
            endpoint_url("http://bao:8200", "/kv/transit/", Op::Encrypt, "k"),
            "http://bao:8200/v1/kv/transit/encrypt/k"
        );
    }

    #[test]
    fn key_names_outside_the_transit_alphabet_are_refused() {
        for key in ["../sys/seal", "a/b", "", "k.v", "k?x", "k%2F"] {
            assert!(
                matches!(check_key_name(key), Err(KeyServiceError::InvalidKeyName(name)) if name == key),
                "{key:?}"
            );
        }
        assert!(check_key_name("demeteo-hub").is_ok());
        assert!(check_key_name("Tenant_42").is_ok());
    }

    #[tokio::test]
    async fn a_refused_key_name_never_reaches_the_network() {
        let keys = OpenBaoKeys::new(
            reqwest::Client::new(),
            "http://unresolvable.invalid",
            "transit",
            "s.token",
        )
        .unwrap();
        assert!(matches!(
            keys.encrypt("../sys/seal", b"x").await,
            Err(KeyServiceError::InvalidKeyName(_))
        ));
        assert!(matches!(
            keys.decrypt("a/b", "vault:v1:x").await,
            Err(KeyServiceError::InvalidKeyName(_))
        ));
        assert!(matches!(
            keys.data_key("").await,
            Err(KeyServiceError::InvalidKeyName(_))
        ));
    }

    #[test]
    fn mounts_with_empty_dot_or_foreign_segments_are_refused() {
        for mount in [
            "",
            "/",
            "..",
            "transit/..",
            "../sys",
            "kv/./transit",
            "kv//transit",
            "a b",
            "a?b",
        ] {
            assert!(
                matches!(
                    OpenBaoKeys::new(reqwest::Client::new(), "http://bao", mount, "s.token"),
                    Err(KeyServiceError::InvalidMount(_))
                ),
                "{mount:?}"
            );
        }
    }

    #[test]
    fn the_default_mount_and_key_are_accepted() {
        let keys =
            OpenBaoKeys::new(reqwest::Client::new(), "http://bao", "transit", "s.token").unwrap();
        assert_eq!(keys.mount, "transit");
        assert!(check_key_name("demeteo-hub").is_ok());
        let nested = OpenBaoKeys::new(
            reqwest::Client::new(),
            "http://bao",
            "/kv/transit/",
            "s.token",
        )
        .unwrap();
        assert_eq!(nested.mount, "kv/transit");
    }

    #[test]
    fn encrypt_body_is_base64_of_arbitrary_bytes() {
        let json = serde_json::to_value(plaintext_body(&[0xff, 0x00, 0x80])).unwrap();
        assert_eq!(json, serde_json::json!({ "plaintext": "/wCA" }));
    }

    #[test]
    fn parses_encrypt_decrypt_and_data_key_responses() {
        let ct = br#"{"data":{"ciphertext":"vault:v1:abc"}}"#;
        assert_eq!(parse_encrypt(ct).unwrap(), "vault:v1:abc");

        let pt = br#"{"data":{"plaintext":"/wCA"}}"#;
        assert_eq!(parse_decrypt(pt).unwrap(), vec![0xff, 0x00, 0x80]);

        let dk = br#"{"data":{"plaintext":"AQID","ciphertext":"vault:v1:xyz"}}"#;
        let key = parse_data_key(dk).unwrap();
        assert_eq!(key.plaintext, vec![1, 2, 3]);
        assert_eq!(key.wrapped, "vault:v1:xyz");
    }

    #[test]
    fn malformed_responses_are_typed_errors() {
        assert!(matches!(
            parse_encrypt(b"not json"),
            Err(KeyServiceError::Malformed(_))
        ));
        assert!(matches!(
            parse_decrypt(br#"{"data":{"plaintext":"***"}}"#),
            Err(KeyServiceError::Malformed(_))
        ));
        assert!(matches!(
            parse_data_key(br#"{"data":{"plaintext":"AQID"}}"#),
            Err(KeyServiceError::Malformed(_))
        ));
    }

    #[test]
    fn http_503_is_sealed() {
        let body = br#"{"errors":["Vault is sealed"]}"#;
        assert!(matches!(
            classify_failure(503, body, "hub"),
            KeyServiceError::Sealed
        ));
        assert!(matches!(
            classify_failure(503, b"", "hub"),
            KeyServiceError::Sealed
        ));
    }

    #[test]
    fn missing_key_is_unknown_key() {
        let body = br#"{"errors":["encryption key not found"]}"#;
        assert!(matches!(
            classify_failure(400, body, "nope"),
            KeyServiceError::UnknownKey(name) if name == "nope"
        ));
        assert!(matches!(
            classify_failure(404, body, "nope"),
            KeyServiceError::UnknownKey(name) if name == "nope"
        ));
    }

    #[test]
    fn a_404_that_does_not_name_a_missing_key_is_upstream() {
        let wrong_mount = serde_json::to_vec(&serde_json::json!({
            "errors": ["no handler for route \"bogus/encrypt/demeteo-hub\". route entry not found."]
        }))
        .unwrap();
        match classify_failure(404, &wrong_mount, "demeteo-hub") {
            KeyServiceError::Upstream { status, detail } => {
                assert_eq!(status, 404);
                assert!(detail.contains("no handler for route"), "{detail}");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            classify_failure(404, b"", "demeteo-hub"),
            KeyServiceError::Upstream { status: 404, .. }
        ));
    }

    #[test]
    fn other_statuses_are_upstream_errors_with_the_status() {
        let body = br#"{"errors":["permission denied"]}"#;
        match classify_failure(403, body, "hub") {
            KeyServiceError::Upstream { status, detail } => {
                assert_eq!(status, 403);
                assert_eq!(detail, "permission denied");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            classify_failure(400, br#"{"errors":["bad ciphertext"]}"#, "hub"),
            KeyServiceError::Upstream { status: 400, .. }
        ));
        assert!(matches!(
            classify_failure(500, b"<html>", "hub"),
            KeyServiceError::Upstream { status: 500, .. }
        ));
    }

    #[tokio::test]
    async fn a_silent_server_is_a_transport_error_not_a_hang() {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let silent = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        let short = Duration::from_millis(200);
        let keys = OpenBaoKeys::new(
            http_client(short, short).unwrap(),
            &format!("http://{addr}"),
            "transit",
            "s.token",
        )
        .unwrap();

        let outcome = tokio::time::timeout(Duration::from_secs(10), keys.encrypt("k", b"x"))
            .await
            .expect("the client timeout must fire before the test's own bound");
        silent.abort();
        assert!(
            matches!(outcome, Err(KeyServiceError::Transport(_))),
            "{outcome:?}"
        );
    }

    #[test]
    fn debug_never_prints_the_token() {
        let keys =
            OpenBaoKeys::new(reqwest::Client::new(), "http://bao", "transit", "s.sekret").unwrap();
        let shown = format!("{keys:?}");
        assert!(!shown.contains("sekret"), "{shown}");
        assert!(keys.token.is_sensitive());
    }

    #[test]
    fn a_token_that_cannot_be_a_header_is_rejected_without_echoing_it() {
        let err = OpenBaoKeys::new(
            reqwest::Client::new(),
            "http://bao",
            "transit",
            "bad\ntoken",
        )
        .unwrap_err();
        assert!(!err.to_string().contains("bad"));
    }
}

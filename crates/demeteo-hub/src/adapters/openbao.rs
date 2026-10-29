use crate::ports::{DataKey, KeyService, KeyServiceError};
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use reqwest::header::HeaderValue;
use reqwest::{Method, RequestBuilder};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

const TOKEN_HEADER: &str = "X-Vault-Token";
const STATUS_SEALED: u16 = 503;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_BAD_REQUEST: u16 = 400;
const KEY_NOT_FOUND_MARKER: &str = "key not found";
const SIGNATURE_PREFIX: &str = "vault:v";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Encrypt,
    Decrypt,
    DataKey,
    ReadKey,
    Sign,
}

impl Op {
    fn path(self) -> &'static str {
        match self {
            Op::Encrypt => "encrypt",
            Op::Decrypt => "decrypt",
            Op::DataKey => "datakey/plaintext",
            Op::ReadKey => "keys",
            Op::Sign => "sign",
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

#[derive(Serialize)]
struct InputBody {
    input: String,
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
struct KeyData {
    latest_version: u64,
    keys: BTreeMap<String, KeyVersion>,
}

#[derive(Deserialize)]
struct KeyVersion {
    public_key: String,
}

#[derive(Deserialize)]
struct SignData {
    signature: String,
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

    fn request(
        &self,
        method: Method,
        op: Op,
        key: &str,
    ) -> Result<RequestBuilder, KeyServiceError> {
        check_key_name(key)?;
        Ok(self
            .http
            .request(method, endpoint_url(&self.addr, &self.mount, op, key))
            .header(TOKEN_HEADER, self.token.clone()))
    }

    async fn call(
        &self,
        op: Op,
        key: &str,
        body: &impl Serialize,
    ) -> Result<Vec<u8>, KeyServiceError> {
        let request = self.request(Method::POST, op, key)?.json(body);
        answer(request, op, key).await
    }

    async fn read(&self, op: Op, key: &str) -> Result<Vec<u8>, KeyServiceError> {
        answer(self.request(Method::GET, op, key)?, op, key).await
    }
}

async fn answer(request: RequestBuilder, op: Op, key: &str) -> Result<Vec<u8>, KeyServiceError> {
    let response = request
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
        Err(classify_failure(op, status, &bytes, key))
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

    async fn verifying_key(&self, key: &str) -> Result<[u8; 32], KeyServiceError> {
        parse_verifying_key(&self.read(Op::ReadKey, key).await?)
    }

    async fn sign(&self, key: &str, message: &[u8]) -> Result<[u8; 64], KeyServiceError> {
        let body = self.call(Op::Sign, key, &input_body(message)).await?;
        parse_signature(&body)
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

/// What [`is_key_name`] accepts, worded for an operator.
pub const KEY_NAME_ALPHABET: &str = "ASCII letters, digits, `_` and `-`";

/// Whether this adapter will address `name`. `Config` asks at startup, so a
/// name every call here would refuse stops the boot instead of failing the
/// first request that needs the key.
pub fn is_key_name(name: &str) -> bool {
    is_path_segment(name)
}

fn check_key_name(key: &str) -> Result<(), KeyServiceError> {
    if is_key_name(key) {
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

fn input_body(message: &[u8]) -> InputBody {
    InputBody {
        input: STANDARD.encode(message),
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

fn decode_fixed<const N: usize>(encoded: &str, what: &str) -> Result<[u8; N], KeyServiceError> {
    let bytes = decode_plaintext(encoded)?;
    bytes.as_slice().try_into().map_err(|_| {
        KeyServiceError::Malformed(format!("{what} is {} bytes, not {N}", bytes.len()))
    })
}

/// Transit keeps every version of a key and signs with the latest unless told
/// otherwise, so the latest is the one whose public half is read. For an
/// Ed25519 key that half is standard base64 of the raw 32 bytes; the other
/// asymmetric types answer PEM, and a symmetric key has no `public_key` at
/// all, so a key of the wrong type is `Malformed` here rather than a key the
/// Hub would go on to advertise.
fn parse_verifying_key(body: &[u8]) -> Result<[u8; 32], KeyServiceError> {
    let data = parse_envelope::<KeyData>(body)?;
    let latest = data
        .keys
        .get(&data.latest_version.to_string())
        .ok_or_else(|| {
            KeyServiceError::Malformed(format!(
                "latest version {} is not among the key's versions",
                data.latest_version
            ))
        })?;
    decode_fixed(&latest.public_key, "public key")
}

/// `vault:v<version>:<standard base64>`. The version is checked for shape and
/// then dropped: the port has no way to ask for a version, so a signature is
/// always under the latest.
fn parse_signature(body: &[u8]) -> Result<[u8; 64], KeyServiceError> {
    let signature = parse_envelope::<SignData>(body)?.signature;
    let (version, encoded) = signature
        .strip_prefix(SIGNATURE_PREFIX)
        .and_then(|rest| rest.split_once(':'))
        .ok_or_else(|| KeyServiceError::Malformed("signature has no version prefix".into()))?;
    if version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit()) {
        return Err(KeyServiceError::Malformed(
            "signature has no key version".into(),
        ));
    }
    decode_fixed(encoded, "signature")
}

/// OpenBao reports a missing Transit key as 400 (decrypt, datakey, sign) or
/// 404 depending on the route, so either is `UnknownKey` only when the error
/// text says the key was not found. A bare 404 also means an unmounted path —
/// a wrong `DEMETEO_HUB_TRANSIT_MOUNT` — and must stay `Upstream` so the
/// operator is not sent looking for a key.
///
/// Reading a key is the one route that says nothing: a missing key is a 404
/// whose `errors` list is present and empty, where an unmounted path names the
/// route it could not find. So only that exact body, and only on that route,
/// is `UnknownKey`; an unreadable body stays `Upstream`.
///
/// The response body is OpenBao's own error text; the request body (which
/// carries plaintext) is never part of an error.
fn classify_failure(op: Op, status: u16, body: &[u8], key: &str) -> KeyServiceError {
    let reported = serde_json::from_slice::<ErrorBody>(body)
        .map(|b| b.errors)
        .ok();
    let silent = reported.as_deref().is_some_and(<[String]>::is_empty);
    let errors = reported.unwrap_or_default().join("; ");
    match status {
        STATUS_SEALED => KeyServiceError::Sealed,
        STATUS_NOT_FOUND | STATUS_BAD_REQUEST if errors.contains(KEY_NOT_FOUND_MARKER) => {
            KeyServiceError::UnknownKey(key.to_owned())
        }
        STATUS_NOT_FOUND if op == Op::ReadKey && silent => {
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

    const PUBLIC_KEY: [u8; 32] = [7; 32];
    const SIGNATURE: [u8; 64] = [9; 64];

    fn key_response(latest_version: u64, version: &str, public_key: &[u8]) -> Vec<u8> {
        let body = serde_json::json!({"data": {
            "type": "ed25519",
            "latest_version": latest_version,
            "keys": {version: {
                "name": "ed25519",
                "public_key": STANDARD.encode(public_key),
                "creation_time": "2026-10-13T09:00:00Z",
            }},
        }});
        serde_json::to_vec(&body).unwrap()
    }

    fn sign_response(signature: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"data": {
            "signature": signature,
            "key_version": 1,
        }}))
        .unwrap()
    }

    fn is_malformed<T>(result: Result<T, KeyServiceError>) -> bool {
        matches!(result, Err(KeyServiceError::Malformed(_)))
    }

    #[test]
    fn urls_cover_the_five_transit_routes() {
        let url = |op| endpoint_url("http://bao:8200/", "transit", op, "hub");
        assert_eq!(url(Op::Encrypt), "http://bao:8200/v1/transit/encrypt/hub");
        assert_eq!(url(Op::Decrypt), "http://bao:8200/v1/transit/decrypt/hub");
        assert_eq!(
            url(Op::DataKey),
            "http://bao:8200/v1/transit/datakey/plaintext/hub"
        );
        assert_eq!(url(Op::ReadKey), "http://bao:8200/v1/transit/keys/hub");
        assert_eq!(url(Op::Sign), "http://bao:8200/v1/transit/sign/hub");
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
        assert!(matches!(
            keys.verifying_key("../sys/seal").await,
            Err(KeyServiceError::InvalidKeyName(_))
        ));
        assert!(matches!(
            keys.sign("a/b", b"x").await,
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
    fn sign_body_is_base64_of_arbitrary_bytes() {
        let json = serde_json::to_value(input_body(&[0xff, 0x00, 0x80])).unwrap();
        assert_eq!(json, serde_json::json!({ "input": "/wCA" }));
    }

    #[test]
    fn parses_the_latest_version_of_a_key_into_its_32_bytes() {
        assert_eq!(
            parse_verifying_key(&key_response(1, "1", &PUBLIC_KEY)).unwrap(),
            PUBLIC_KEY
        );

        let rotated = serde_json::to_vec(&serde_json::json!({"data": {
            "latest_version": 2,
            "keys": {
                "1": {"public_key": STANDARD.encode([1u8; 32])},
                "2": {"public_key": STANDARD.encode(PUBLIC_KEY)},
            },
        }}))
        .unwrap();
        assert_eq!(parse_verifying_key(&rotated).unwrap(), PUBLIC_KEY);
    }

    #[test]
    fn a_public_key_of_the_wrong_length_is_malformed() {
        for len in [0, 31, 33, 64] {
            assert!(
                is_malformed(parse_verifying_key(&key_response(1, "1", &vec![7; len]))),
                "{len}"
            );
        }
    }

    #[test]
    fn a_key_whose_latest_version_is_missing_is_malformed() {
        assert!(is_malformed(parse_verifying_key(&key_response(
            2,
            "1",
            &PUBLIC_KEY
        ))));
        assert!(is_malformed(parse_verifying_key(
            br#"{"data":{"keys":{"1":{"public_key":"BwcH"}}}}"#
        )));
    }

    #[test]
    fn a_key_that_is_not_ed25519_is_malformed() {
        let symmetric =
            br#"{"data":{"type":"aes256-gcm96","latest_version":1,"keys":{"1":1442851412}}}"#;
        assert!(is_malformed(parse_verifying_key(symmetric)));

        let pem = serde_json::to_vec(&serde_json::json!({"data": {
            "latest_version": 1,
            "keys": {"1": {"public_key": "-----BEGIN PUBLIC KEY-----\nMFkw\n-----END PUBLIC KEY-----"}},
        }}))
        .unwrap();
        assert!(is_malformed(parse_verifying_key(&pem)));
    }

    #[test]
    fn parses_a_signature_into_its_64_bytes() {
        for version in ["1", "12"] {
            let text = format!("vault:v{version}:{}", STANDARD.encode(SIGNATURE));
            assert_eq!(
                parse_signature(&sign_response(&text)).unwrap(),
                SIGNATURE,
                "{version}"
            );
        }
    }

    #[test]
    fn a_signature_of_the_wrong_length_is_malformed() {
        for len in [0, 63, 65, 32] {
            let text = format!("vault:v1:{}", STANDARD.encode(vec![9; len]));
            assert!(
                is_malformed(parse_signature(&sign_response(&text))),
                "{len}"
            );
        }
    }

    #[test]
    fn a_signature_without_its_prefix_or_version_is_malformed() {
        let encoded = STANDARD.encode(SIGNATURE);
        for text in [
            encoded.clone(),
            format!("v1:{encoded}"),
            format!("vault:{encoded}"),
            format!("vault:v:{encoded}"),
            format!("vault:vx:{encoded}"),
            format!("vault:v1{encoded}"),
            "vault:v1:***".to_owned(),
        ] {
            assert!(
                is_malformed(parse_signature(&sign_response(&text))),
                "{text}"
            );
        }
        assert!(is_malformed(parse_signature(br#"{"data":{}}"#)));
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
            classify_failure(Op::Encrypt, 503, body, "hub"),
            KeyServiceError::Sealed
        ));
        assert!(matches!(
            classify_failure(Op::Encrypt, 503, b"", "hub"),
            KeyServiceError::Sealed
        ));
    }

    #[test]
    fn a_sealed_service_is_sealed_on_the_key_read_and_sign_routes() {
        let body = br#"{"errors":["Vault is sealed"]}"#;
        for op in [Op::ReadKey, Op::Sign] {
            assert!(matches!(
                classify_failure(op, 503, body, "hub"),
                KeyServiceError::Sealed
            ));
        }
    }

    #[test]
    fn a_missing_signing_key_is_unknown_key() {
        let named = br#"{"errors":["signing key not found"]}"#;
        assert!(matches!(
            classify_failure(Op::Sign, 400, named, "nope"),
            KeyServiceError::UnknownKey(name) if name == "nope"
        ));

        let silent = br#"{"errors":[]}"#;
        assert!(matches!(
            classify_failure(Op::ReadKey, 404, silent, "nope"),
            KeyServiceError::UnknownKey(name) if name == "nope"
        ));
    }

    #[test]
    fn a_silent_404_is_a_missing_key_only_when_reading_one() {
        let silent = br#"{"errors":[]}"#;
        for op in [Op::Encrypt, Op::Decrypt, Op::DataKey, Op::Sign] {
            assert!(matches!(
                classify_failure(op, 404, silent, "hub"),
                KeyServiceError::Upstream { status: 404, .. }
            ));
        }
        assert!(matches!(
            classify_failure(Op::ReadKey, 400, silent, "hub"),
            KeyServiceError::Upstream { status: 400, .. }
        ));
    }

    #[test]
    fn reading_a_key_under_a_wrong_mount_is_upstream() {
        let wrong_mount = serde_json::to_vec(&serde_json::json!({
            "errors": ["no handler for route \"bogus/keys/demeteo-hub-signing\". route entry not found."]
        }))
        .unwrap();
        assert!(matches!(
            classify_failure(Op::ReadKey, 404, &wrong_mount, "demeteo-hub-signing"),
            KeyServiceError::Upstream { status: 404, .. }
        ));
        assert!(matches!(
            classify_failure(Op::ReadKey, 404, b"", "demeteo-hub-signing"),
            KeyServiceError::Upstream { status: 404, .. }
        ));
        assert!(matches!(
            classify_failure(
                Op::ReadKey,
                403,
                br#"{"errors":["permission denied"]}"#,
                "hub"
            ),
            KeyServiceError::Upstream { status: 403, .. }
        ));
    }

    #[test]
    fn missing_key_is_unknown_key() {
        let body = br#"{"errors":["encryption key not found"]}"#;
        assert!(matches!(
            classify_failure(Op::Encrypt, 400, body, "nope"),
            KeyServiceError::UnknownKey(name) if name == "nope"
        ));
        assert!(matches!(
            classify_failure(Op::Encrypt, 404, body, "nope"),
            KeyServiceError::UnknownKey(name) if name == "nope"
        ));
    }

    #[test]
    fn a_404_that_does_not_name_a_missing_key_is_upstream() {
        let wrong_mount = serde_json::to_vec(&serde_json::json!({
            "errors": ["no handler for route \"bogus/encrypt/demeteo-hub\". route entry not found."]
        }))
        .unwrap();
        match classify_failure(Op::Encrypt, 404, &wrong_mount, "demeteo-hub") {
            KeyServiceError::Upstream { status, detail } => {
                assert_eq!(status, 404);
                assert!(detail.contains("no handler for route"), "{detail}");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            classify_failure(Op::Encrypt, 404, b"", "demeteo-hub"),
            KeyServiceError::Upstream { status: 404, .. }
        ));
    }

    #[test]
    fn other_statuses_are_upstream_errors_with_the_status() {
        let body = br#"{"errors":["permission denied"]}"#;
        match classify_failure(Op::Encrypt, 403, body, "hub") {
            KeyServiceError::Upstream { status, detail } => {
                assert_eq!(status, 403);
                assert_eq!(detail, "permission denied");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            classify_failure(Op::Encrypt, 400, br#"{"errors":["bad ciphertext"]}"#, "hub"),
            KeyServiceError::Upstream { status: 400, .. }
        ));
        assert!(matches!(
            classify_failure(Op::Encrypt, 500, b"<html>", "hub"),
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

    /// Answers one request with `body` and hands back what the client sent.
    async fn answering_once(body: Vec<u8>) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let served = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut seen = Vec::new();
            let mut chunk = [0u8; 1024];
            let request = loop {
                let n = socket.read(&mut chunk).await.unwrap();
                seen.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&seen).into_owned();
                let complete = text.split_once("\r\n\r\n").is_some_and(|(head, sent)| {
                    let declared = head
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    sent.len() >= declared
                });
                if complete || n == 0 {
                    break text;
                }
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
            request
        });
        (format!("http://{addr}"), served)
    }

    #[tokio::test]
    async fn the_public_key_is_read_with_a_get_that_carries_no_body() {
        let (addr, served) = answering_once(key_response(1, "1", &PUBLIC_KEY)).await;
        let keys = OpenBaoKeys::new(reqwest::Client::new(), &addr, "transit", "s.token").unwrap();

        assert_eq!(keys.verifying_key("hub").await.unwrap(), PUBLIC_KEY);

        let request = served.await.unwrap();
        let (head, sent) = request.split_once("\r\n\r\n").unwrap();
        assert!(
            head.starts_with("GET /v1/transit/keys/hub HTTP/1.1\r\n"),
            "{head}"
        );
        assert!(head.to_ascii_lowercase().contains("x-vault-token: s.token"));
        assert_eq!(sent, "");
    }

    #[tokio::test]
    async fn a_signature_is_asked_for_with_a_post_of_the_base64_input() {
        let text = format!("vault:v1:{}", STANDARD.encode(SIGNATURE));
        let (addr, served) = answering_once(sign_response(&text)).await;
        let keys = OpenBaoKeys::new(reqwest::Client::new(), &addr, "transit", "s.token").unwrap();

        assert_eq!(
            keys.sign("hub", &[0xff, 0x00, 0x80]).await.unwrap(),
            SIGNATURE
        );

        let request = served.await.unwrap();
        let (head, sent) = request.split_once("\r\n\r\n").unwrap();
        assert!(
            head.starts_with("POST /v1/transit/sign/hub HTTP/1.1\r\n"),
            "{head}"
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(sent).unwrap(),
            serde_json::json!({ "input": "/wCA" })
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

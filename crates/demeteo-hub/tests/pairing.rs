//! A desktop pairing with the Hub, over the real router and real rows: the
//! browser's `GET /authorize`, the consent routes under a session, and
//! `POST /token` with proofs made by a software device key.
//!
//! `src-tauri/tests/hub_pairing.rs` holds every pairing rule to its verdict
//! without a database. What is left for this file is what only Postgres can
//! show: that the rows a grant wrote are the ones the next request is judged
//! on, that two requests racing for one family take turns, and that nothing a
//! client was sent can be read back out of a table.
//!
//! Every test here is ignored unless `DEMETEO_HUB_TEST_DATABASE_URL` names a
//! database (`deploy/hub/README.md` has the service), and an ignored test has
//! shown nothing.

mod support;

use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::attachments::AttachmentSettings;
use demeteo_hub::auth::challenge::{base64url_decode, base64url_encode};
use demeteo_hub::auth::policy::Status;
use demeteo_hub::auth::session::{SessionSecret, COOKIE_NAME, CSRF_HEADER};
use demeteo_hub::auth::testing::{serve, SoftAuthenticator};
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::pairing::grant::{access_verdict, AccessRefusal, TokenSecret};
use demeteo_hub::pairing::jwk::fingerprint;
use demeteo_hub::pairing::testing::SoftDeviceKey;
use demeteo_hub::pairing::{ACCESS_TOKEN_TTL_SECS, CONSENT_TTL_SECS, REFRESH_TOKEN_TTL_SECS};
use demeteo_hub::ports::KeyService;
use demeteo_hub::routes::{self, AppState};
use demeteo_hub::store::instances::{list_instances, Instance};
use demeteo_hub::store::passkeys::{insert_passkey, NewPasskey};
use demeteo_hub::store::pool::create_user;
use demeteo_hub::store::sessions::{insert_session, NewSession};
use demeteo_hub::store::tokens::{
    find_token_by_hash, insert_token, NewToken, TokenHash, TokenKind, TokenRecord,
};
use demeteo_hub::store::{Db, InstanceId, UserId};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::Arc;
use support::in_fresh_schema;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const HUB_ORIGIN: &str = "https://hub.example";
const RP_ID: &str = "hub.example";
const SIGNING_KEY: &str = "demeteo-hub-signing";

/// RFC 7636 Appendix B.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

const REDIRECT: &str = "http://127.0.0.1:49152/callback";
const STATE: &str = "xyz-123";
const OS: &str = "linux";
const ARCH: &str = "x86_64";
const APP_VERSION: &str = "1.4.0";

fn now() -> u64 {
    u64::try_from(OffsetDateTime::now_utc().unix_timestamp()).unwrap()
}

fn at(unix_seconds: u64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(i64::try_from(unix_seconds).unwrap()).unwrap()
}

fn unix(time: OffsetDateTime) -> u64 {
    u64::try_from(time.unix_timestamp()).unwrap()
}

/// A served Hub as its one tenant holds it: a session of the tenant's
/// trusted passkey, and the three values a pairing response must hand over.
struct Hub {
    base: String,
    db: Db,
    user: UserId,
    session: SessionSecret,
    /// The `request_signing_key`, `rp_id` and `passkeys` members, computed
    /// from the key service and the passkey this test enrolled.
    pinned: Value,
}

/// The session is written to the store directly: how a passkey sign-in comes
/// by one is `tests/passkeys.rs`'s to show.
async fn hub_over(db: Db) -> Hub {
    let user = create_user(&db).await.unwrap();
    let passkey = SoftAuthenticator::new();
    let credential_id = base64url_decode(&passkey.credential_id()).unwrap();
    let enrolled = NewPasskey {
        credential_id: credential_id.clone(),
        public_key: passkey.public_key_cose(),
        label: "trusted".to_owned(),
        sign_count: 0,
        credential: "{}".to_owned(),
    };
    insert_passkey(user, &db, &enrolled, Status::Trusted)
        .await
        .unwrap();
    let session = SessionSecret::generate().unwrap();
    let opened = NewSession {
        hash: session.hash(),
        credential_id,
        expires_at: OffsetDateTime::now_utc() + Duration::hours(1),
    };
    insert_session(user, &db, &opened).await.unwrap();

    let keys = Arc::new(MemoryKeys::with_keys([SIGNING_KEY]));
    let signing_key = keys.verifying_key(SIGNING_KEY).await.unwrap();
    let pinned = json!({
        "request_signing_key": base64url_encode(&signing_key),
        "rp_id": RP_ID,
        "passkeys": [{
            "credential_id": passkey.credential_id(),
            "public_key": base64url_encode(&passkey.public_key_cose()),
        }],
    });

    let rp = RelyingParty {
        rp_id: RP_ID.to_owned(),
        origin: HUB_ORIGIN.to_owned(),
    };
    let state = AppState::new(
        db.clone(),
        rp,
        keys,
        SIGNING_KEY.to_owned(),
        SIGNING_KEY.to_owned(),
        AttachmentSettings::default(),
    )
    .unwrap();
    let web_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-web-dir");
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    serve(listener, routes::router(state, &web_dir));
    Hub {
        base,
        db,
        user,
        session,
        pinned,
    }
}

/// What one desktop brings to a pairing: its key, the name it gives itself,
/// the scopes it asks for and the ones its user leaves checked.
#[derive(Clone, Copy)]
struct Desktop<'a> {
    key: &'a SoftDeviceKey,
    name: &'a str,
    asks: &'a str,
    grants: &'a [&'a str],
}

impl<'a> Desktop<'a> {
    fn named(name: &'a str, key: &'a SoftDeviceKey) -> Self {
        Self {
            key,
            name,
            asks: "read gates",
            grants: &["read", "gates"],
        }
    }
}

/// `name=value` pairs joined as a query or a form body, every byte of a
/// value outside RFC 3986's unreserved set written `%XX`. This crate's
/// `reqwest` has neither feature that would encode one.
fn form_encoded(pairs: &[(&str, &str)]) -> String {
    let escaped = |value: &str| -> String {
        value
            .bytes()
            .map(|byte| match byte {
                b'-' | b'.' | b'_' | b'~' => char::from(byte).to_string(),
                byte if byte.is_ascii_alphanumeric() => char::from(byte).to_string(),
                byte => format!("%{byte:02X}"),
            })
            .collect()
    };
    let pairs: Vec<String> = pairs
        .iter()
        .map(|(name, value)| format!("{name}={}", escaped(value)))
        .collect();
    pairs.join("&")
}

async fn answered(response: reqwest::Response) -> (u16, Value) {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap();
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

/// `GET /authorize` as the desktop sends a browser to it, and the segment of
/// the approval page the Hub sent that browser on to. Redirects are not
/// followed: the `303` is the answer.
async fn authorize(hub: &Hub, desktop: Desktop<'_>) -> String {
    let query = form_encoded(&[
        ("response_type", "code"),
        ("code_challenge_method", "S256"),
        ("code_challenge", CHALLENGE),
        ("redirect_uri", REDIRECT),
        ("dpop_jkt", &desktop.key.thumbprint()),
        ("scope", desktop.asks),
        ("state", STATE),
        ("instance_name", desktop.name),
        ("os", OS),
        ("arch", ARCH),
        ("app_version", APP_VERSION),
    ]);
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client
        .get(format!("{}/authorize?{query}", hub.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 303);
    let location = response.headers()["location"].to_str().unwrap();
    location
        .strip_prefix("/#/pair/")
        .unwrap_or_else(|| panic!("`{location}` is not the approval page"))
        .to_owned()
}

/// A `POST` from the Hub's own page under the tenant's session: the
/// configured origin, the cookie, and the CSRF token derived from it.
async fn post_in_session(hub: &Hub, path: &str, body: &Value) -> (u16, Value) {
    let cookie = format!("{COOKIE_NAME}={}", hub.session.cookie_value());
    let response = reqwest::Client::new()
        .post(format!("{}{path}", hub.base))
        .header("origin", HUB_ORIGIN)
        .header("cookie", cookie)
        .header(CSRF_HEADER, hub.session.csrf_token())
        .json(body)
        .send()
        .await
        .unwrap();
    answered(response).await
}

fn text_of(body: &Value, member: &str) -> String {
    body[member]
        .as_str()
        .unwrap_or_else(|| panic!("no `{member}` in {body}"))
        .to_owned()
}

/// The authorization code in the redirect an approval answered with.
fn code_in(approved: &Value) -> String {
    let redirect = text_of(approved, "redirect");
    let code = redirect
        .strip_prefix(&format!("{REDIRECT}?code="))
        .and_then(|rest| rest.strip_suffix(&format!("&state={STATE}")))
        .unwrap_or_else(|| panic!("`{redirect}` carries no code for {REDIRECT}"));
    assert_eq!(code.len(), 43, "the unpadded base64url of 32 bytes");
    code.to_owned()
}

/// The consent half of a pairing, each answer required to be a `200`: the
/// code the desktop's loopback listener would be handed.
async fn approved_code(hub: &Hub, desktop: Desktop<'_>) -> String {
    let segment = authorize(hub, desktop).await;
    let started = post_in_session(hub, "/v1/pairing/start", &json!({ "request": segment })).await;
    assert_eq!(started.0, 200, "{started:?}");
    let approval = json!({
        "pairing_id": text_of(&started.1, "pairing_id"),
        "scopes": desktop.grants,
    });
    let approved = post_in_session(hub, "/v1/pairing/approve", &approval).await;
    assert_eq!(approved.0, 200, "{approved:?}");
    code_in(&approved.1)
}

/// One `POST /token` under a fresh proof by `key`. The proof names the
/// configured origin, which is not the loopback address the Hub is reached
/// at, and is dated by the clock the Hub itself reads.
async fn post_token(hub: &Hub, key: &SoftDeviceKey, form: &[(&str, &str)]) -> (u16, Value) {
    let htu = format!("{HUB_ORIGIN}/token");
    let proof = key.proof("POST", &htu, now(), &Uuid::new_v4().to_string());
    let response = reqwest::Client::new()
        .post(format!("{}/token", hub.base))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("dpop", proof)
        .body(form_encoded(form))
        .send()
        .await
        .unwrap();
    answered(response).await
}

async fn redeem(hub: &Hub, key: &SoftDeviceKey, code: &str) -> (u16, Value) {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", VERIFIER),
        ("redirect_uri", REDIRECT),
    ];
    post_token(hub, key, &form).await
}

async fn refresh(hub: &Hub, key: &SoftDeviceKey, refresh_token: &str) -> (u16, Value) {
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];
    post_token(hub, key, &form).await
}

/// The two tokens of one `/token` answer, as the client was sent them.
struct Issued {
    access: String,
    refresh: String,
}

fn issued(answer: &(u16, Value)) -> Issued {
    assert_eq!(answer.0, 200, "{answer:?}");
    Issued {
        access: text_of(&answer.1, "access_token"),
        refresh: text_of(&answer.1, "refresh_token"),
    }
}

/// The whole body of a refresh that issued `pair`.
fn refresh_body(pair: &Issued) -> Value {
    json!({
        "access_token": pair.access,
        "token_type": "DPoP",
        "expires_in": ACCESS_TOKEN_TTL_SECS,
        "refresh_token": pair.refresh,
    })
}

/// The whole body of a pairing that issued `pair` for `scope` to `instance`.
fn pairing_body(hub: &Hub, pair: &Issued, scope: &str, instance: InstanceId) -> Value {
    let mut body = refresh_body(pair);
    body["scope"] = json!(scope);
    body["instance_id"] = json!(instance.0.to_string());
    for member in ["request_signing_key", "rp_id", "passkeys"] {
        body[member] = hub.pinned[member].clone();
    }
    body
}

/// A whole pairing, from the browser's first request to the first token pair.
async fn pair(hub: &Hub, desktop: Desktop<'_>) -> Issued {
    let code = approved_code(hub, desktop).await;
    issued(&redeem(hub, desktop.key, &code).await)
}

fn invalid_grant() -> (u16, Value) {
    (400, json!({ "error": "invalid_grant" }))
}

fn raw(secret: &str) -> Vec<u8> {
    let bytes = base64url_decode(secret).unwrap();
    assert_eq!(bytes.len(), 32, "{secret}");
    bytes
}

/// The row a token the client was sent is stored as. The digest is taken
/// here, of the bytes the text encodes, and not by `TokenSecret::hash`: a
/// lookup through the Hub's own rule would find the row whatever that rule
/// stored.
async fn row_of(db: &Db, token: &str) -> TokenRecord {
    let hash = TokenHash::digest(&raw(token));
    let found = find_token_by_hash(db, &hash).await.unwrap();
    found.unwrap_or_else(|| panic!("no row holds the digest of {token}"))
}

/// How many rows the family has, and how many of them are not retired.
async fn rows_and_live(db: &Db, family_id: Uuid) -> (i64, i64) {
    sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE retired_at IS NULL) \
         FROM tokens WHERE family_id = $1",
    )
    .bind(family_id)
    .fetch_one(db)
    .await
    .unwrap()
}

async fn instances(hub: &Hub) -> Vec<Instance> {
    list_instances(hub.user, &hub.db).await.unwrap()
}

/// The `device_keys` rows as `(instance_id, public_jwk, jkt)`.
async fn device_keys(db: &Db) -> Vec<(InstanceId, Value, String)> {
    let rows: Vec<(InstanceId, String, String)> = sqlx::query_as(
        "SELECT instance_id, public_jwk::text, jkt FROM device_keys ORDER BY created_at, id",
    )
    .fetch_all(db)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(instance, jwk, jkt)| (instance, serde_json::from_str(&jwk).unwrap(), jkt))
        .collect()
}

/// Every step is the request a browser or a desktop sends, and each answer
/// is compared whole. The members nobody can predict, a token or an id, are
/// taken from the answer and then looked for in the rows.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_desktop_pairs_through_authorize_consent_and_token_and_then_refreshes() {
    in_fresh_schema(|db| async move {
        let hub = hub_over(db).await;
        let key = SoftDeviceKey::new();
        let jkt = key.thumbprint();
        let desktop = Desktop {
            key: &key,
            name: "Workstation",
            asks: "read gates spend",
            grants: &["read", "gates"],
        };

        let segment = authorize(&hub, desktop).await;

        let before = now();
        let started =
            post_in_session(&hub, "/v1/pairing/start", &json!({ "request": segment })).await;
        let after = now();
        let pairing_id = text_of(&started.1, "pairing_id");
        let expires_at = started.1["expires_at"].as_u64().unwrap();
        let prompt = json!({
            "pairing_id": pairing_id,
            "fingerprint": fingerprint(&jkt),
            "instance_name": "Workstation",
            "os": OS,
            "arch": ARCH,
            "app_version": APP_VERSION,
            "scopes": ["read", "gates", "spend"],
            "expires_at": expires_at,
        });
        assert_eq!(started, (200, prompt));
        assert!((before + CONSENT_TTL_SECS..=after + CONSENT_TTL_SECS).contains(&expires_at));

        let approval = json!({ "pairing_id": pairing_id, "scopes": desktop.grants });
        let approved = post_in_session(&hub, "/v1/pairing/approve", &approval).await;
        let code = code_in(&approved.1);
        let redirect = format!("{REDIRECT}?code={code}&state={STATE}");
        assert_eq!(approved, (200, json!({ "redirect": redirect })));
        assert_eq!(instances(&hub).await.len(), 0, "an approval writes no row");

        let before = now();
        let paired = redeem(&hub, &key, &code).await;
        let after = now();
        let first = issued(&paired);
        let [instance] = instances(&hub).await.try_into().unwrap();
        let body = pairing_body(&hub, &first, "read gates", instance.id);
        assert_eq!(paired, (200, body));
        assert_eq!(
            (
                instance.name.as_str(),
                instance.os.as_str(),
                instance.arch.as_str(),
                instance.version.as_str(),
                instance.granted_scopes,
            ),
            (
                "Workstation",
                OS,
                ARCH,
                APP_VERSION,
                vec!["read".to_owned(), "gates".to_owned()]
            )
        );
        assert_eq!(
            device_keys(&hub.db).await,
            [(instance.id, key.jwk(), jkt.clone())]
        );

        let access = row_of(&hub.db, &first.access).await;
        let refresh_row = row_of(&hub.db, &first.refresh).await;
        let family = access.family_id;
        for (row, kind, ttl) in [
            (&access, TokenKind::Access, ACCESS_TOKEN_TTL_SECS),
            (&refresh_row, TokenKind::Refresh, REFRESH_TOKEN_TTL_SECS),
        ] {
            assert_eq!(
                (
                    row.user_id,
                    row.kind,
                    row.family_id,
                    &row.jkt,
                    row.retired_at
                ),
                (hub.user, kind, family, &jkt, None)
            );
            let expires = unix(row.expires_at);
            assert!((before + ttl..=after + ttl).contains(&expires), "{kind:?}");
        }
        let expires = unix(access.expires_at);
        assert_eq!(access_verdict(&access, &jkt, expires - 1), Ok(()));
        assert_eq!(
            access_verdict(&access, &jkt, expires),
            Err(AccessRefusal::Expired)
        );

        let refreshed = refresh(&hub, &key, &first.refresh).await;
        let second = issued(&refreshed);
        assert_eq!(refreshed, (200, refresh_body(&second)));
        assert_ne!(second.access, first.access);
        assert_ne!(second.refresh, first.refresh);

        assert!(row_of(&hub.db, &first.refresh).await.retired_at.is_some());
        for (token, kind) in [
            (&first.access, TokenKind::Access),
            (&second.access, TokenKind::Access),
            (&second.refresh, TokenKind::Refresh),
        ] {
            let row = row_of(&hub.db, token).await;
            assert_eq!(
                (row.kind, row.family_id, &row.jkt, row.retired_at),
                (kind, family, &jkt, None)
            );
        }
        assert_eq!(rows_and_live(&hub.db, family).await, (4, 3));
    })
    .await;
}

/// The bystander is the control: its family is rotated twice before the
/// reuse, which is what the reused family's newest token would have done, and
/// once more after it, which is what a revocation that reached past its own
/// family would have stopped.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_refresh_token_presented_twice_retires_its_whole_family_and_no_other() {
    in_fresh_schema(|db| async move {
        let hub = hub_over(db).await;
        let (key, other_key) = (SoftDeviceKey::new(), SoftDeviceKey::new());
        let first = pair(&hub, Desktop::named("Workstation", &key)).await;
        let other_first = pair(&hub, Desktop::named("Laptop", &other_key)).await;

        let other_second = issued(&refresh(&hub, &other_key, &other_first.refresh).await);
        let other_third = issued(&refresh(&hub, &other_key, &other_second.refresh).await);
        let second = issued(&refresh(&hub, &key, &first.refresh).await);
        let newest_access = row_of(&hub.db, &second.access).await;
        let jkt = key.thumbprint();
        assert_eq!(access_verdict(&newest_access, &jkt, now()), Ok(()));

        let reused = refresh(&hub, &key, &first.refresh).await;
        assert_eq!(reused, invalid_grant());
        let newest = refresh(&hub, &key, &second.refresh).await;
        assert_eq!(newest, invalid_grant());

        for token in [
            &first.access,
            &first.refresh,
            &second.access,
            &second.refresh,
        ] {
            assert!(row_of(&hub.db, token).await.retired_at.is_some());
        }
        let family = newest_access.family_id;
        assert_eq!(rows_and_live(&hub.db, family).await, (4, 0));
        let newest_access = row_of(&hub.db, &second.access).await;
        assert_eq!(
            access_verdict(&newest_access, &jkt, now()),
            Err(AccessRefusal::Retired)
        );

        let other_fourth = refresh(&hub, &other_key, &other_third.refresh).await;
        assert_eq!(other_fourth.0, 200, "{other_fourth:?}");
        let other_family = row_of(&hub.db, &other_first.access).await.family_id;
        assert_ne!(other_family, family);
        assert_eq!(rows_and_live(&hub.db, other_family).await, (8, 5));
    })
    .await;
}

/// One of the two rotates and the other finds the token retired, so the
/// loser's verdict is a reuse and takes the winner's pair with it: one
/// successor pair is written, and none of the family outlives the race. Two
/// successor pairs would be two lines of descent from one token; a successor
/// left alive would be the loser deciding on rows read before the winner
/// committed.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn two_concurrent_refreshes_of_one_token_leave_exactly_one_successor_and_no_live_row() {
    in_fresh_schema(|db| async move {
        let hub = hub_over(db).await;
        for round in 0..8 {
            let key = SoftDeviceKey::new();
            let first = pair(&hub, Desktop::named("Workstation", &key)).await;
            let family = row_of(&hub.db, &first.refresh).await.family_id;

            let (one, other) = tokio::join!(
                refresh(&hub, &key, &first.refresh),
                refresh(&hub, &key, &first.refresh),
            );

            let (won, lost) = match (one.0, other.0) {
                (200, 200) => panic!("round {round}: both refreshes were granted"),
                (200, _) => (one, other),
                _ => (other, one),
            };
            let successor = issued(&won);
            assert_eq!(won, (200, refresh_body(&successor)), "round {round}");
            assert_eq!(lost, invalid_grant(), "round {round}");
            assert_eq!(
                rows_and_live(&hub.db, family).await,
                (4, 0),
                "round {round}"
            );
            for token in [&successor.access, &successor.refresh] {
                let row = row_of(&hub.db, token).await;
                assert_eq!(row.family_id, family, "round {round}");
            }
            let later = refresh(&hub, &key, &successor.refresh).await;
            assert_eq!(later, invalid_grant(), "round {round}");
        }
    })
    .await;
}

/// Both rows are written the same way and differ in their expiry alone, so
/// the expiry is all the refusal can be for.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_access_token_inserted_already_expired_is_refused_on_the_row_its_hash_finds() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let jkt = SoftDeviceKey::new().thumbprint();
        let family_id = Uuid::new_v4();
        let now = now();
        let expiring = |expires_at: u64| NewToken {
            kind: TokenKind::Access,
            hash: TokenSecret::generate().unwrap().hash(),
            family_id,
            jkt: jkt.clone(),
            expires_at: at(expires_at),
        };
        let in_its_lifetime = expiring(now + ACCESS_TOKEN_TTL_SECS);
        let expired = expiring(now - 1);
        insert_token(user, &db, &in_its_lifetime).await.unwrap();
        insert_token(user, &db, &expired).await.unwrap();

        let fetched = find_token_by_hash(&db, &in_its_lifetime.hash).await;
        let fetched = fetched.unwrap().unwrap();
        assert_eq!(access_verdict(&fetched, &jkt, now), Ok(()));

        let fetched = find_token_by_hash(&db, &expired.hash).await;
        let fetched = fetched.unwrap().unwrap();
        assert_eq!(
            (fetched.user_id, fetched.kind, fetched.retired_at),
            (user, TokenKind::Access, None)
        );
        assert_eq!(fetched.expires_at, expired.expires_at);
        assert_eq!(
            access_verdict(&fetched, &jkt, now),
            Err(AccessRefusal::Expired)
        );
    })
    .await;
}

/// Pairing a second key first is the control: it adds an instance and leaves
/// the first key's family working, which a refresh then shows. Pairing the
/// first key again does neither.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn pairing_the_same_device_key_again_reuses_its_instance_and_retires_its_old_family() {
    in_fresh_schema(|db| async move {
        let hub = hub_over(db).await;
        let (key, other_key) = (SoftDeviceKey::new(), SoftDeviceKey::new());
        let first = pair(&hub, Desktop::named("Workstation", &key)).await;
        let [instance] = instances(&hub).await.try_into().unwrap();
        let old_family = row_of(&hub.db, &first.access).await.family_id;

        let other = pair(&hub, Desktop::named("Laptop", &other_key)).await;
        assert_eq!(instances(&hub).await.len(), 2);
        let second = issued(&refresh(&hub, &key, &first.refresh).await);
        assert_eq!(rows_and_live(&hub.db, old_family).await, (4, 3));

        let renamed = Desktop {
            key: &key,
            name: "Workstation, reinstalled",
            asks: "read gates",
            grants: &["read"],
        };
        let code = approved_code(&hub, renamed).await;
        let paired_again = redeem(&hub, &key, &code).await;

        let again = issued(&paired_again);
        let body = pairing_body(&hub, &again, "read", instance.id);
        assert_eq!(paired_again, (200, body));
        let listed = instances(&hub).await;
        assert_eq!(listed.len(), 2);
        let kept = listed.iter().find(|row| row.id == instance.id).unwrap();
        assert_eq!(
            (kept.name.as_str(), kept.granted_scopes.as_slice()),
            ("Workstation, reinstalled", &["read".to_owned()][..])
        );
        assert_eq!(kept.created_at, instance.created_at);
        let keys = device_keys(&hub.db).await;
        let of_this_key: Vec<_> = keys
            .iter()
            .filter(|row| row.2 == key.thumbprint())
            .collect();
        assert_eq!(of_this_key, [&(instance.id, key.jwk(), key.thumbprint())]);
        assert_eq!(keys.len(), 2);

        assert_eq!(rows_and_live(&hub.db, old_family).await, (4, 0));
        let new_family = row_of(&hub.db, &again.access).await.family_id;
        assert_ne!(new_family, old_family);
        assert_eq!(rows_and_live(&hub.db, new_family).await, (2, 2));

        let old = refresh(&hub, &key, &second.refresh).await;
        assert_eq!(old, invalid_grant());
        let new = refresh(&hub, &key, &again.refresh).await;
        assert_eq!(new.0, 200, "{new:?}");
        let bystander = refresh(&hub, &other_key, &other.refresh).await;
        assert_eq!(bystander.0, 200, "{bystander:?}");
    })
    .await;
}

/// One stored value, as bytes: a `BYTEA` as it is stored, anything else as
/// the text Postgres writes it as.
struct Cell {
    table: &'static str,
    column: String,
    bytes: Vec<u8>,
}

/// Every non-null value of every column of every row of `table`.
///
/// The columns are read from the catalog and not listed here, so a column a
/// later migration adds is read without this test being told of it.
async fn cells_of(db: &Db, table: &'static str) -> Vec<Cell> {
    let columns: Vec<(String, String)> = sqlx::query_as(
        "SELECT column_name::text, data_type::text FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(db)
    .await
    .unwrap();
    assert!(!columns.is_empty(), "no table `{table}` in this schema");

    let mut cells = Vec::new();
    for (column, data_type) in columns {
        let as_bytes = match data_type.as_str() {
            "bytea" => format!("\"{column}\""),
            _ => format!("convert_to(\"{column}\"::text, 'UTF8')"),
        };
        let select = format!("SELECT {as_bytes} FROM {table}");
        let stored: Vec<Option<Vec<u8>>> = sqlx::query_scalar(sqlx::AssertSqlSafe(select))
            .fetch_all(db)
            .await
            .unwrap();
        cells.extend(stored.into_iter().flatten().map(|bytes| Cell {
            table,
            column: column.clone(),
            bytes,
        }));
    }
    cells
}

/// The columns in which some value equals or contains `needle`.
fn columns_holding<'a>(cells: &'a [Cell], needle: &[u8]) -> Vec<(&'a str, &'a str)> {
    let mut columns: Vec<(&str, &str)> = cells
        .iter()
        .filter(|cell| cell.bytes.windows(needle.len()).any(|run| run == needle))
        .map(|cell| (cell.table, cell.column.as_str()))
        .collect();
    columns.dedup();
    columns
}

/// The search is first watched find what the rows do hold, in a text column
/// of each table and in the one `BYTEA` a token's digest is kept in. Only
/// then does finding nothing say anything.
///
/// Each secret is looked for as the text the client was sent and as the 32
/// bytes that text encodes: a token kept raw in a `BYTEA` is no less kept.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn no_stored_row_holds_a_token_in_plaintext() {
    in_fresh_schema(|db| async move {
        let hub = hub_over(db).await;
        let key = SoftDeviceKey::new();
        let code = approved_code(&hub, Desktop::named("Workstation", &key)).await;
        let first = issued(&redeem(&hub, &key, &code).await);
        let second = issued(&refresh(&hub, &key, &first.refresh).await);

        let mut cells = Vec::new();
        for table in ["tokens", "device_keys", "instances"] {
            cells.extend(cells_of(&hub.db, table).await);
        }

        let rows_of = |table: &str| {
            let ids = cells
                .iter()
                .filter(|cell| cell.table == table && cell.column == "id");
            ids.count()
        };
        assert_eq!(
            [
                rows_of("tokens"),
                rows_of("device_keys"),
                rows_of("instances")
            ],
            [4, 1, 1]
        );
        assert_eq!(
            columns_holding(&cells, key.thumbprint().as_bytes()),
            [("tokens", "jkt"), ("device_keys", "jkt")]
        );
        assert_eq!(
            columns_holding(&cells, b"Workstation"),
            [("instances", "name")]
        );
        let secrets = [
            ("authorization code", &code),
            ("first access token", &first.access),
            ("first refresh token", &first.refresh),
            ("second access token", &second.access),
            ("second refresh token", &second.refresh),
        ];
        for (name, secret) in &secrets[1..] {
            let digest = Sha256::digest(raw(secret));
            assert_eq!(
                columns_holding(&cells, &digest),
                [("tokens", "token_hash")],
                "the digest of the {name}"
            );
        }

        for (name, secret) in secrets {
            assert_eq!(
                columns_holding(&cells, secret.as_bytes()),
                [],
                "the {name}, as text"
            );
            assert_eq!(
                columns_holding(&cells, &raw(secret)),
                [],
                "the {name}, as bytes"
            );
        }
    })
    .await;
}

//! A tenant signed in to the Hub's real router, talking to it over HTTP as
//! the Hub's own page does.

use demeteo_hub::auth::challenge::base64url_decode;
use demeteo_hub::auth::policy::Status;
use demeteo_hub::auth::session::{SessionSecret, COOKIE_NAME, CSRF_HEADER};
use demeteo_hub::auth::testing::{serve, SoftAuthenticator};
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::routes::{self, AppState};
use demeteo_hub::store::passkeys::{insert_passkey, NewPasskey};
use demeteo_hub::store::sessions::{insert_session, NewSession};
use demeteo_hub::store::{Db, UserId};
use serde_json::Value;
use std::net::Ipv4Addr;
use std::path::Path;
use time::{Duration, OffsetDateTime};

pub fn rp() -> RelyingParty {
    RelyingParty {
        rp_id: "hub.example".to_owned(),
        origin: "https://hub.example".to_owned(),
    }
}

pub fn as_json(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

pub fn in_an_hour() -> OffsetDateTime {
    OffsetDateTime::now_utc() + Duration::hours(1)
}

pub fn raw_id(key: &SoftAuthenticator) -> Vec<u8> {
    base64url_decode(&key.credential_id()).unwrap()
}

pub async fn enrol(db: &Db, user: UserId, key: &SoftAuthenticator, status: Status) {
    enrol_as(db, user, key, status.as_str(), status).await;
}

pub async fn enrol_as(db: &Db, user: UserId, key: &SoftAuthenticator, label: &str, status: Status) {
    let new = NewPasskey {
        credential_id: raw_id(key),
        public_key: key.public_key_cose(),
        label: label.to_owned(),
        sign_count: 0,
        credential: "{}".to_owned(),
    };
    insert_passkey(user, db, &new, status).await.unwrap();
}

/// The Hub's real router over `state` on a loopback port, and the base URL it
/// answers at. The web directory does not exist: no `/v1` route reads it.
pub async fn serve_state(state: AppState) -> String {
    let web_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-web-dir");
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    serve(listener, routes::router(state, &web_dir));
    format!("http://{addr}")
}

pub async fn open_session(db: &Db, user: UserId, key: &SoftAuthenticator) -> SessionSecret {
    let secret = SessionSecret::generate().unwrap();
    let session = NewSession {
        hash: secret.hash(),
        credential_id: raw_id(key),
        expires_at: in_an_hour(),
    };
    insert_session(user, db, &session).await.unwrap();
    secret
}

/// A `POST` as the Hub's own page sends it for `session`: same origin, the
/// cookie, and the CSRF token derived from it.
pub async fn post_as(
    base: &str,
    path: &str,
    session: &SessionSecret,
    body: &Value,
) -> (u16, Value) {
    let response = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .header("origin", rp().origin)
        .header(
            "cookie",
            format!("{COOKIE_NAME}={}", session.cookie_value()),
        )
        .header(CSRF_HEADER, session.csrf_token())
        .json(body)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, as_json(&response.text().await.unwrap()))
}

pub async fn get_as(base: &str, path: &str, session: &SessionSecret) -> (u16, Value) {
    let response = reqwest::Client::new()
        .get(format!("{base}{path}"))
        .header(
            "cookie",
            format!("{COOKIE_NAME}={}", session.cookie_value()),
        )
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, as_json(&response.text().await.unwrap()))
}

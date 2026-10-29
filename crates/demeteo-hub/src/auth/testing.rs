//! A software authenticator for tests of registration and the passkey chain,
//! and a router state that needs no Postgres, for this crate and the ones that
//! reach it through the `test-support` feature.
//!
//! Nothing here names a relying party, a database or an address: every caller
//! passes its own. This file is compiled outside `#[cfg(test)]`, so
//! `tests/no_literals.rs` scans it like any other source file.
//!
//! A failure of the system random source panics, and so do creation options
//! no browser could act on and a registration the Hub's own verifier refuses.
//! A test has nothing to do with any of them but fail, and a `Result` on every
//! call would be unwrapped at every call site.

use aws_lc_rs::error::Unspecified;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
use axum::response::IntoResponse;
use axum::Router;
use demeteo_hub_protocol::WebAuthnAssertion;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::iter;
use std::sync::Arc;
use time::OffsetDateTime;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::assertion::{FLAG_USER_PRESENT, FLAG_USER_VERIFIED};
use super::chain::StoredKey;
use super::challenge::{base64url_decode, base64url_encode};
use super::cose::{self, P256Point};
use super::handlers::ApiError;
use super::policy::Status;
use super::webauthn::RelyingPartyError;
use super::RelyingParty;
use crate::adapters::memory_keys::MemoryKeys;
use crate::attachments::AttachmentSettings;
use crate::ports::KeyService;
use crate::routes::AppState;
use crate::store::passkeys::Passkey;
use crate::store::{pool, StoreError, UserId};

const CREDENTIAL_ID_LEN: u8 = 32;
const FLAG_ATTESTED_CREDENTIAL_DATA: u8 = 0x40;
const AAGUID: [u8; 16] = [0; 16];
/// A three-entry CBOR map up to the value of its last entry:
/// `{"fmt": "none", "attStmt": {}, "authData": `.
const NONE_ATTESTATION_HEAD: &[u8] = b"\xa3\x63fmt\x64none\x67attStmt\xa0\x68authData";

pub struct SoftAuthenticator {
    key_pair: EcdsaKeyPair,
    credential_id: [u8; CREDENTIAL_ID_LEN as usize],
    sign_count: u32,
}

impl SoftAuthenticator {
    /// A fresh P-256 key and credential id. It reports a signature counter of
    /// zero on every assertion, as a synced passkey does;
    /// [`with_sign_count`](Self::with_sign_count) makes it report another.
    pub fn new() -> Self {
        let mut credential_id = [0u8; CREDENTIAL_ID_LEN as usize];
        or_panic(
            aws_lc_rs::rand::fill(&mut credential_id),
            "fill a credential id",
        );
        Self {
            key_pair: or_panic(
                EcdsaKeyPair::generate(&ECDSA_P256_SHA256_ASN1_SIGNING),
                "generate a P-256 key pair",
            ),
            credential_id,
            sign_count: 0,
        }
    }

    /// The counter is fixed, not incremented: an assertion reports `sign_count`
    /// however many were made before it.
    pub fn with_sign_count(mut self, sign_count: u32) -> Self {
        self.sign_count = sign_count;
        self
    }

    pub fn credential_id(&self) -> String {
        base64url_encode(&self.credential_id)
    }

    pub fn public_key_cose(&self) -> Vec<u8> {
        let Some(point) = P256Point::from_sec1_uncompressed(self.key_pair.public_key().as_ref())
        else {
            panic!("soft authenticator: a P-256 public key is an uncompressed SEC1 point");
        };
        cose::encode(&point).to_vec()
    }

    /// This passkey as a tenant's row would hold it. The status is the
    /// caller's to say: an authenticator does not know what the Hub decided
    /// about its key.
    pub fn stored(&self, status: Status) -> StoredKey {
        StoredKey {
            credential_id: self.credential_id(),
            public_key: self.public_key_cose(),
            status,
        }
    }

    /// This passkey as the row the Hub behind `state` would hold had it
    /// registered there, for a tenant of its own: `credential` is what a real
    /// registration ceremony produces, so a sign-in assertion from this
    /// authenticator verifies against it, and `status` is all that can stand
    /// between that assertion and a session. Nothing is stored.
    pub fn enrolled(&self, state: &AppState, status: Status) -> Passkey {
        let user = UserId::generate();
        let label = status.as_str();
        let verifier = state.verifier();
        let registered = verifier
            .start_registration(user.handle(), label, &[])
            .and_then(|(options, registration)| {
                let created = self.create(&state.rp().origin, &options);
                verifier.finish_registration(&created, registration)
            });
        let registered = match registered {
            Ok(registered) => registered,
            Err(error) => panic!("soft authenticator: could not register: {error}"),
        };
        Passkey {
            user_id: user,
            credential_id: registered.credential_id,
            public_key: registered.public_key.to_vec(),
            label: label.to_owned(),
            sign_count: registered.sign_count,
            status,
            credential: registered.credential,
            created_at: OffsetDateTime::now_utc(),
        }
    }

    /// What `hub-web/src/lib/webauthn.ts` sends for a user-verified
    /// `navigator.credentials.create` over `options`, run by a browser at
    /// `origin`. The relying party is the one the options name, as it is for
    /// a browser.
    pub fn create(&self, origin: &str, options: &Value) -> Value {
        self.create_with(
            origin,
            options,
            FLAG_USER_PRESENT | FLAG_USER_VERIFIED,
            &self.public_key_cose(),
        )
    }

    /// A registration claiming whatever key and user flags it is told to, for
    /// the cases a verifier must refuse. The attestation is of the `none`
    /// format, so nothing signs the claim and any `public_key_cose` goes.
    pub fn create_with(
        &self,
        origin: &str,
        options: &Value,
        flags: u8,
        public_key_cose: &[u8],
    ) -> Value {
        let public_key = &options["publicKey"];
        let challenge = public_key["challenge"].as_str().and_then(base64url_decode);
        let (Some(rp_id), Some(challenge)) = (public_key["rp"]["id"].as_str(), challenge) else {
            panic!(
                "soft authenticator: creation options without an rp.id or a base64url challenge"
            );
        };

        let mut authenticator_data = Sha256::digest(rp_id.as_bytes()).to_vec();
        authenticator_data.push(flags | FLAG_ATTESTED_CREDENTIAL_DATA);
        authenticator_data.extend_from_slice(&self.sign_count.to_be_bytes());
        authenticator_data.extend_from_slice(&AAGUID);
        authenticator_data.extend_from_slice(&u16::from(CREDENTIAL_ID_LEN).to_be_bytes());
        authenticator_data.extend_from_slice(&self.credential_id);
        authenticator_data.extend_from_slice(public_key_cose);
        let client_data = json!({
            "type": "webauthn.create",
            "challenge": base64url_encode(&challenge),
            "origin": origin,
            "crossOrigin": false,
        });

        json!({
            "id": self.credential_id(),
            "rawId": self.credential_id(),
            "type": "public-key",
            "response": {
                "clientDataJSON": base64url_encode(client_data.to_string().as_bytes()),
                "attestationObject": base64url_encode(&none_attestation(&authenticator_data)),
            },
            "clientExtensionResults": {},
        })
    }

    /// What `hub-web/src/lib/webauthn.ts` sends for a user-verified
    /// `navigator.credentials.get` over `options`, run by a browser at
    /// `origin`. `user_handle` is the caller's to say: this authenticator
    /// keeps nothing from the ceremony that created its credential, and
    /// `None` is the `userHandle: null` of one that returned no handle.
    pub fn get(&self, origin: &str, options: &Value, user_handle: Option<&[u8]>) -> Value {
        self.get_with(
            origin,
            options,
            FLAG_USER_PRESENT | FLAG_USER_VERIFIED,
            user_handle,
        )
    }

    /// A sign-in claiming whatever user flags it is told to, for the cases a
    /// verifier must refuse.
    pub fn get_with(
        &self,
        origin: &str,
        options: &Value,
        flags: u8,
        user_handle: Option<&[u8]>,
    ) -> Value {
        let public_key = &options["publicKey"];
        let challenge = public_key["challenge"].as_str().and_then(base64url_decode);
        let (Some(rp_id), Some(challenge)) = (public_key["rpId"].as_str(), challenge) else {
            panic!("soft authenticator: request options without an rpId or a base64url challenge");
        };
        let assertion = self.assert_with(rp_id, origin, flags, &challenge);

        json!({
            "id": assertion.credential_id,
            "rawId": assertion.credential_id,
            "type": "public-key",
            "response": {
                "clientDataJSON": assertion.client_data_json,
                "authenticatorData": assertion.authenticator_data,
                "signature": assertion.signature,
                "userHandle": user_handle.map(base64url_encode),
            },
            "clientExtensionResults": {},
        })
    }

    /// What a browser returns for a user-verified `navigator.credentials.get`
    /// at `rp`.
    pub fn assert(&self, rp: &RelyingParty, challenge: &[u8; 32]) -> WebAuthnAssertion {
        self.assert_with(
            &rp.rp_id,
            &rp.origin,
            FLAG_USER_PRESENT | FLAG_USER_VERIFIED,
            challenge,
        )
    }

    /// An assertion with a valid signature over whatever it is told to claim,
    /// for the cases a verifier must refuse.
    pub fn assert_with(
        &self,
        rp_id: &str,
        origin: &str,
        flags: u8,
        challenge: &[u8],
    ) -> WebAuthnAssertion {
        let mut authenticator_data = Sha256::digest(rp_id.as_bytes()).to_vec();
        authenticator_data.push(flags);
        authenticator_data.extend_from_slice(&self.sign_count.to_be_bytes());
        let client_data = serde_json::json!({
            "type": "webauthn.get",
            "challenge": base64url_encode(challenge),
            "origin": origin,
            "crossOrigin": false,
        });
        self.sign(&authenticator_data, client_data.to_string().as_bytes())
    }

    /// Signs the two byte strings as given, well-formed or not.
    ///
    /// What is signed is assembled here and not taken from the verifier: a
    /// double sharing the verifier's spelling of it could not show that
    /// spelling wrong.
    pub fn sign(&self, authenticator_data: &[u8], client_data_json: &[u8]) -> WebAuthnAssertion {
        let signed = [authenticator_data, &Sha256::digest(client_data_json)].concat();
        let signature = or_panic(
            self.key_pair.sign(&SystemRandom::new(), &signed),
            "sign an assertion",
        );
        WebAuthnAssertion {
            credential_id: self.credential_id(),
            authenticator_data: base64url_encode(authenticator_data),
            client_data_json: base64url_encode(client_data_json),
            signature: base64url_encode(signature.as_ref()),
        }
    }
}

impl Default for SoftAuthenticator {
    fn default() -> Self {
        Self::new()
    }
}

/// An [`AppState`] over a pool that never connects, for routes that are
/// refused or answered before any query runs. `database_url` is parsed and
/// never dialled; a query against this state fails instead of returning rows.
///
/// Its key service holds no key, so a route that reaches for one is answered
/// an error under whatever name it asks; [`lazy_state_with_keys`] is for the
/// routes that need an answer.
///
/// Call it inside a Tokio runtime, as [`pool::connect_lazy`] requires.
pub fn lazy_state(database_url: &str, rp: RelyingParty) -> Result<AppState, LazyStateError> {
    let no_keys = MemoryKeys::with_keys(iter::empty::<String>());
    lazy_state_with_keys(database_url, rp, Arc::new(no_keys), "")
}

/// [`lazy_state`] over a key service of the caller's, which is asked for the
/// request-signing key by the name `signing_key`. Keep a clone of `keys` to
/// sign with the key the routes advertise.
///
/// `signing_key` is also the transit key, being the one name `keys` is known
/// to hold; attachments get the default [`AttachmentSettings`].
pub fn lazy_state_with_keys(
    database_url: &str,
    rp: RelyingParty,
    keys: Arc<dyn KeyService>,
    signing_key: &str,
) -> Result<AppState, LazyStateError> {
    let db = pool::connect_lazy(database_url)?;
    Ok(AppState::new(
        db,
        rp,
        keys,
        signing_key.to_owned(),
        signing_key.to_owned(),
        AttachmentSettings::default(),
    )?)
}

#[derive(Debug, thiserror::Error)]
pub enum LazyStateError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    RelyingParty(#[from] RelyingPartyError),
}

/// Serves `app` on a listener the caller bound, until the runtime ends. It
/// exists because a crate that depends on this one without `axum` can hold a
/// [`Router`] but cannot name `axum::serve`.
pub fn serve(listener: TcpListener, app: Router) -> JoinHandle<std::io::Result<()>> {
    tokio::spawn(async move { axum::serve(listener, app).await })
}

/// What a client is sent for `error`: the status, and the body as JSON. For a
/// crate that is handed an [`ApiError`] and, without `axum`, cannot render it.
pub async fn answered(error: ApiError) -> (u16, Value) {
    let response = error.into_response();
    let status = response.status().as_u16();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    match body {
        Some(body) => (status, body),
        None => panic!("an API error whose body is not JSON"),
    }
}

/// The CBOR byte-string header is written for the lengths authenticator data
/// has: 164 bytes around a P-256 key, and past 255 around an RSA one.
fn none_attestation(authenticator_data: &[u8]) -> Vec<u8> {
    let mut object = NONE_ATTESTATION_HEAD.to_vec();
    match u8::try_from(authenticator_data.len()) {
        Ok(len) => object.extend_from_slice(&[0x58, len]),
        Err(_) => {
            let Ok(len) = u16::try_from(authenticator_data.len()) else {
                panic!("soft authenticator: authenticator data too long for a two-byte length");
            };
            object.push(0x59);
            object.extend_from_slice(&len.to_be_bytes());
        }
    }
    object.extend_from_slice(authenticator_data);
    object
}

fn or_panic<T>(result: Result<T, Unspecified>, action: &str) -> T {
    match result {
        Ok(value) => value,
        Err(Unspecified) => panic!("soft authenticator: could not {action}"),
    }
}

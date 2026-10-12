//! Passkey registration and sign-in through `webauthn-rs`, and the only file
//! that names that crate. Nothing of it crosses this module's boundary: callers
//! see the types below, `serde_json::Value` and bytes, so a bump or a
//! replacement of the library is a change to this file alone.
//!
//! Endorsement and revocation never come here. Their challenge is chosen by
//! the Hub (`super::challenge`) and the library only verifies a challenge it
//! generated itself; [`super::assertion`] verifies those.
//!
//! # What the library's passkey API does not do
//!
//! The pinned version fixes three things a usernameless, ES256-only Hub needs
//! otherwise, with no setter for any of them. Each is corrected here, and no
//! correction accepts a registration the library would have refused:
//!
//! - **It offers RS256 next to ES256**, and accepts either back.
//!   [`super::cose`] has one byte form and it is P-256, so the options are
//!   rewritten to offer ES256 alone and a verified registration carrying any
//!   other key is refused.
//! - **It asks for a non-discoverable credential.** Sign-in has no username
//!   to look a credential up by, so the options require a resident key.
//! - **It reports a credential on the exclude list with the error it uses for
//!   an algorithm nobody offered.** The exclude list is therefore never handed
//!   to it: it is written into the options and checked against the verified
//!   credential id here, which keeps the two refusals apart.
//!
//! Sign-in has three more, and the last two are the corrections that accept
//! what the library refuses:
//!
//! - **It asks for conditional mediation and sends an empty
//!   `allowCredentials`.** Conditional mediation is an autofill suggestion
//!   under a text field; the Hub's sign-in is a button and a modal prompt. Both
//!   members are dropped from the options.
//! - **It refuses a signature counter that did not advance.** `docs/HUB.md` §8
//!   does not rely on the counter, because synced passkeys commonly report
//!   zero, and the Hub refuses no sign-in over it. The library is therefore
//!   handed the stored credential with its counter at zero, where its check
//!   passes whatever is reported, and what it would have refused is returned
//!   as [`SignedIn::counter_regressed`] for the caller to record. That is
//!   judged against the counter the caller passes, the `passkeys.sign_count`
//!   column every sign-in, endorsement and revocation by that passkey
//!   rewrites. The counter inside the stored credential is the registration's
//!   for the life of the passkey, because nothing rewrites that JSON.
//! - **It refuses a credential that became backup-eligible.** Its
//!   discoverable flow fixes `allow_backup_eligible_upgrade` to `false`, where
//!   its own non-discoverable passkey flow sets it to `true`. A passkey whose
//!   authenticator began syncing after it registered would be refused at every
//!   sign-in, while it still endorsed and revoked through
//!   [`super::assertion`]. A stored credential that is not backup-eligible is
//!   therefore handed over as eligible when the response reports the flag, and
//!   only in that direction: one stored as eligible that stops reporting it
//!   stays refused. Nothing is persisted, so this is applied at each sign-in.

use super::challenge::{base64url_encode, CHALLENGE_TTL_SECS};
use super::cose::{self, P256Point, COSE_KEY_LEN};
use super::RelyingParty;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;
use webauthn_rs::prelude::{
    COSEAlgorithm, COSEEC2Key, COSEKey, COSEKeyType, DiscoverableAuthentication, DiscoverableKey,
    ECDSACurve, Passkey, PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
    Url, WebauthnError,
};
use webauthn_rs::{Webauthn, WebauthnBuilder};

const ES256: i64 = -7;
const PUBLIC_KEY_CREDENTIAL: &str = "public-key";
const STORED_COUNTER: &str = "/cred/counter";
const STORED_BACKUP_ELIGIBLE: &str = "/cred/backup_eligible";
/// Authenticator data opens with the SHA-256 of the RP ID; its flags are the
/// byte after that.
const FLAGS_OFFSET: usize = 32;
const FLAG_BACKUP_ELIGIBLE: u8 = 0x08;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RelyingPartyError {
    #[error("the relying party origin is not a URL")]
    OriginNotAUrl,
    #[error("the relying party id is neither its origin's host nor a parent domain of it")]
    IdDoesNotCoverOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RegistrationError {
    #[error("not a registration credential")]
    Malformed,
    #[error("the response answers another challenge")]
    WrongChallenge,
    #[error("the response was made at another origin")]
    WrongOrigin,
    #[error("the authenticator did not verify the user")]
    UserNotVerified,
    #[error("the credential's key is not ES256")]
    UnsupportedAlgorithm,
    #[error("the credential is one the ceremony excluded")]
    CredentialExcluded,
    #[error("the registration was refused")]
    Refused,
    #[error("the WebAuthn library produced something this module cannot read")]
    Library,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SignInError {
    #[error("not a sign-in credential")]
    Malformed,
    #[error("the response answers another challenge")]
    WrongChallenge,
    #[error("the response was made at another origin")]
    WrongOrigin,
    #[error("the authenticator did not verify the user")]
    UserNotVerified,
    #[error("the response names a credential other than the stored one")]
    WrongCredential,
    #[error("the signature is not the stored credential's")]
    BadSignature,
    #[error("the stored credential is not one this module wrote")]
    StoredCredential,
    #[error("the sign-in was refused")]
    Refused,
    #[error("the WebAuthn library produced something this module cannot read")]
    Library,
}

/// What [`Verifier::start_registration`] hands to
/// [`Verifier::finish_registration`]. It has no serialised form — the
/// library's `danger-allow-state-serialisation` feature is off — so it can be
/// held in process memory and nowhere else.
#[derive(Debug)]
pub struct RegistrationState {
    ceremony: PasskeyRegistration,
    excluded: Vec<Vec<u8>>,
}

/// Every field is read from the registration the library verified; none is a
/// member of the JSON the browser sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registered {
    pub credential_id: Vec<u8>,
    pub public_key: [u8; COSE_KEY_LEN],
    /// The library's own credential as JSON text, which a later sign-in is
    /// verified against.
    pub credential: String,
    pub sign_count: u32,
}

/// What [`Verifier::start_sign_in`] hands to [`Verifier::finish_sign_in`],
/// held in process memory only, as [`RegistrationState`] is.
#[derive(Debug)]
pub struct SignInState {
    ceremony: DiscoverableAuthentication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedIn {
    /// As the authenticator reported it.
    pub sign_count: u32,
    /// The WebAuthn clone signal, judged against the `stored_count`
    /// [`Verifier::finish_sign_in`] was given: one of the two is non-zero and
    /// the reported one is not above the stored one. The sign-in succeeded all
    /// the same (module docs).
    pub counter_regressed: bool,
}

pub struct Verifier {
    webauthn: Webauthn,
}

impl Verifier {
    /// The options' `timeout` is the ceremony's lifetime in
    /// [`super::ceremonies`], so a browser stops waiting when the Hub stops
    /// holding the state.
    pub fn new(rp: &RelyingParty) -> Result<Self, RelyingPartyError> {
        let origin = Url::parse(&rp.origin).map_err(|_| RelyingPartyError::OriginNotAUrl)?;
        let webauthn = WebauthnBuilder::new(&rp.rp_id, &origin)
            .and_then(|builder| {
                builder
                    .timeout(Duration::from_secs(CHALLENGE_TTL_SECS))
                    .build()
            })
            .map_err(|_| RelyingPartyError::IdDoesNotCoverOrigin)?;
        Ok(Self { webauthn })
    }

    /// Returns `{"publicKey": {...}}` with every binary member as base64url,
    /// the shape `hub-web/src/lib/webauthn.ts` turns into the browser's
    /// argument. `label` is both `user.name` and `user.displayName`.
    pub fn start_registration(
        &self,
        user_handle: Uuid,
        label: &str,
        exclude_credential_ids: &[Vec<u8>],
    ) -> Result<(Value, RegistrationState), RegistrationError> {
        let (options, ceremony) = self
            .webauthn
            .start_passkey_registration(user_handle, label, label, None)
            .map_err(|_| RegistrationError::Library)?;
        let mut options = serde_json::to_value(options).map_err(|_| RegistrationError::Library)?;
        narrow_options(&mut options, exclude_credential_ids).ok_or(RegistrationError::Library)?;
        let state = RegistrationState {
            ceremony,
            excluded: exclude_credential_ids.to_vec(),
        };
        Ok((options, state))
    }

    /// `credential` is the `PublicKeyCredential.toJSON()` shape of an
    /// attestation. Taking the state by value is what makes it single-use
    /// here; [`super::ceremonies`] makes it single-use across requests.
    pub fn finish_registration(
        &self,
        credential: &Value,
        state: RegistrationState,
    ) -> Result<Registered, RegistrationError> {
        let response = RegisterPublicKeyCredential::deserialize(credential)
            .map_err(|_| RegistrationError::Malformed)?;
        let passkey = self
            .webauthn
            .finish_passkey_registration(&response, &state.ceremony)
            .map_err(refusal)?;
        let point =
            es256_point(passkey.get_public_key()).ok_or(RegistrationError::UnsupportedAlgorithm)?;
        let credential_id = passkey.cred_id().to_vec();
        if state.excluded.contains(&credential_id) {
            return Err(RegistrationError::CredentialExcluded);
        }
        let stored = serde_json::to_value(&passkey).map_err(|_| RegistrationError::Library)?;
        Ok(Registered {
            credential_id,
            public_key: cose::encode(&point),
            sign_count: stored_sign_count(&stored).ok_or(RegistrationError::Library)?,
            credential: stored.to_string(),
        })
    }

    /// Returns `{"publicKey": {...}}` for a usernameless sign-in: a fresh
    /// challenge, no `allowCredentials`, user verification required.
    pub fn start_sign_in(&self) -> Result<(Value, SignInState), SignInError> {
        let (options, ceremony) = self
            .webauthn
            .start_discoverable_authentication()
            .map_err(|_| SignInError::Library)?;
        let mut options = serde_json::to_value(options).map_err(|_| SignInError::Library)?;
        make_modal_and_usernameless(&mut options).ok_or(SignInError::Library)?;
        Ok((options, SignInState { ceremony }))
    }

    /// `credential` is the sign-in shape `hub-web/src/lib/webauthn.ts` emits
    /// and `stored_credential` the [`Registered::credential`] of the passkey
    /// [`sign_in_credential_id`] names. A response naming any other credential
    /// is refused, so a caller that looked up the wrong row signs nobody in.
    ///
    /// `stored_count` is the counter that passkey's row holds, the
    /// registration's until an assertion of that passkey is accepted, and the
    /// one [`SignedIn::counter_regressed`] is judged against. The counter inside
    /// `stored_credential` is never compared (module docs).
    pub fn finish_sign_in(
        &self,
        credential: &Value,
        state: SignInState,
        stored_credential: &str,
        stored_count: u32,
    ) -> Result<SignedIn, SignInError> {
        let response =
            PublicKeyCredential::deserialize(credential).map_err(|_| SignInError::Malformed)?;
        let mut stored: Value =
            serde_json::from_str(stored_credential).map_err(|_| SignInError::StoredCredential)?;
        *stored
            .pointer_mut(STORED_COUNTER)
            .ok_or(SignInError::StoredCredential)? = json!(0);
        // The flag is read from a response nothing has verified yet. That is
        // sound only because the library verifies the signature over this same
        // authenticator data before it answers `Ok`.
        accept_backup_eligible_upgrade(&mut stored, &response.response.authenticator_data);
        let passkey = Passkey::deserialize(stored).map_err(|_| SignInError::StoredCredential)?;
        let result = self
            .webauthn
            .finish_discoverable_authentication(
                &response,
                state.ceremony,
                &[DiscoverableKey::from(passkey)],
            )
            .map_err(sign_in_refusal)?;
        let sign_count = result.counter();
        Ok(SignedIn {
            sign_count,
            counter_regressed: counter_regressed(stored_count, sign_count),
        })
    }
}

/// The raw credential id a sign-in response names, for looking the passkey up
/// before anything is verified. That lookup is what establishes the tenant.
/// `response.userHandle` is not returned: no signature covers it, so it is
/// whatever the sender typed.
pub fn sign_in_credential_id(credential: &Value) -> Result<Vec<u8>, SignInError> {
    PublicKeyCredential::deserialize(credential)
        .map(|response| response.get_credential_id().to_vec())
        .map_err(|_| SignInError::Malformed)
}

/// `None` when the library's options are not the object this rewrites.
fn make_modal_and_usernameless(options: &mut Value) -> Option<()> {
    let options = options.as_object_mut()?;
    options.remove("mediation");
    options
        .get_mut("publicKey")?
        .as_object_mut()?
        .remove("allowCredentials");
    Some(())
}

fn sign_in_refusal(error: WebauthnError) -> SignInError {
    match error {
        WebauthnError::MismatchedChallenge => SignInError::WrongChallenge,
        WebauthnError::InvalidRPOrigin => SignInError::WrongOrigin,
        WebauthnError::UserNotVerified => SignInError::UserNotVerified,
        WebauthnError::CredentialNotFound => SignInError::WrongCredential,
        WebauthnError::AuthenticationFailure => SignInError::BadSignature,
        _ => SignInError::Refused,
    }
}

fn counter_regressed(stored: u32, reported: u32) -> bool {
    (stored > 0 || reported > 0) && reported <= stored
}

/// Raises the stored credential's backup-eligible flag when the response
/// reports it, and never lowers it. The library's own upgrade path leaves the
/// raised flag for its caller to persist, and refuses the backed-up flag until
/// that is done. Raising it here stands for that persisted state, so a passkey
/// that reports both flags signs in.
fn accept_backup_eligible_upgrade(stored: &mut Value, authenticator_data: &[u8]) {
    let reported = authenticator_data
        .get(FLAGS_OFFSET)
        .is_some_and(|flags| flags & FLAG_BACKUP_ELIGIBLE != 0);
    if let Some(Value::Bool(eligible)) = stored.pointer_mut(STORED_BACKUP_ELIGIBLE) {
        *eligible |= reported;
    }
}

/// `None` when the library's options are not the object this rewrites.
fn narrow_options(options: &mut Value, exclude_credential_ids: &[Vec<u8>]) -> Option<()> {
    let public_key = options.get_mut("publicKey")?.as_object_mut()?;
    public_key.insert(
        "pubKeyCredParams".to_owned(),
        json!([{ "type": PUBLIC_KEY_CREDENTIAL, "alg": ES256 }]),
    );
    let excluded: Vec<Value> = exclude_credential_ids
        .iter()
        .map(|id| json!({ "type": PUBLIC_KEY_CREDENTIAL, "id": base64url_encode(id) }))
        .collect();
    public_key.insert("excludeCredentials".to_owned(), Value::Array(excluded));
    let selection = public_key
        .get_mut("authenticatorSelection")?
        .as_object_mut()?;
    selection.insert("residentKey".to_owned(), json!("required"));
    selection.insert("requireResidentKey".to_owned(), json!(true));
    Some(())
}

/// The library never sees an exclude list (module docs), so its
/// `CredentialAlteredAlgFromRequest` can only mean the algorithm.
fn refusal(error: WebauthnError) -> RegistrationError {
    match error {
        WebauthnError::MismatchedChallenge => RegistrationError::WrongChallenge,
        WebauthnError::InvalidRPOrigin => RegistrationError::WrongOrigin,
        WebauthnError::UserNotVerified => RegistrationError::UserNotVerified,
        WebauthnError::CredentialAlteredAlgFromRequest
        | WebauthnError::COSEKeyInvalidAlgorithm
        | WebauthnError::COSEKeyInvalidType => RegistrationError::UnsupportedAlgorithm,
        _ => RegistrationError::Refused,
    }
}

fn es256_point(key: &COSEKey) -> Option<P256Point> {
    let COSEKey {
        type_: COSEAlgorithm::ES256,
        key:
            COSEKeyType::EC_EC2(COSEEC2Key {
                curve: ECDSACurve::SECP256R1,
                x,
                y,
            }),
    } = key
    else {
        return None;
    };
    Some(P256Point {
        x: <[u8; 32]>::try_from(x.as_slice()).ok()?,
        y: <[u8; 32]>::try_from(y.as_slice()).ok()?,
    })
}

/// The library exposes a credential's counter only behind its
/// `danger-credential-internals` feature, so it is read from the serialised
/// form that is stored anyway. An absent member is an error and never zero: a
/// renamed field would otherwise store every passkey with a counter of 0.
fn stored_sign_count(stored: &Value) -> Option<u32> {
    let counter = stored.pointer(STORED_COUNTER)?.as_u64()?;
    u32::try_from(counter).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::assertion::{FLAG_USER_PRESENT, FLAG_USER_VERIFIED};
    use crate::auth::testing::SoftAuthenticator;

    const ORIGIN: &str = "https://hub.example";
    const LABEL: &str = "Work laptop";
    const UP_AND_UV: u8 = FLAG_USER_PRESENT | FLAG_USER_VERIFIED;
    const FLAG_BACKED_UP: u8 = 0x10;

    fn rp() -> RelyingParty {
        RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: ORIGIN.to_owned(),
        }
    }

    fn verifier() -> Verifier {
        Verifier::new(&rp()).unwrap()
    }

    fn start(verifier: &Verifier) -> (Value, RegistrationState) {
        verifier
            .start_registration(Uuid::new_v4(), LABEL, &[])
            .unwrap()
    }

    fn register(verifier: &Verifier, authenticator: &SoftAuthenticator) -> Registered {
        let (options, state) = start(verifier);
        verifier
            .finish_registration(&authenticator.create(ORIGIN, &options), state)
            .unwrap()
    }

    fn sign_in(
        verifier: &Verifier,
        authenticator: &SoftAuthenticator,
        stored_credential: &str,
        stored_count: u32,
    ) -> Result<SignedIn, SignInError> {
        let (options, state) = verifier.start_sign_in().unwrap();
        verifier.finish_sign_in(
            &authenticator.get(ORIGIN, &options, None),
            state,
            stored_credential,
            stored_count,
        )
    }

    fn register_with_flags(
        verifier: &Verifier,
        authenticator: &SoftAuthenticator,
        flags: u8,
    ) -> Registered {
        let (options, state) = start(verifier);
        let created =
            authenticator.create_with(ORIGIN, &options, flags, &authenticator.public_key_cose());
        verifier.finish_registration(&created, state).unwrap()
    }

    fn sign_in_with_flags(
        verifier: &Verifier,
        authenticator: &SoftAuthenticator,
        flags: u8,
        registered: &Registered,
    ) -> Result<SignedIn, SignInError> {
        let (options, state) = verifier.start_sign_in().unwrap();
        verifier.finish_sign_in(
            &authenticator.get_with(ORIGIN, &options, flags, None),
            state,
            &registered.credential,
            registered.sign_count,
        )
    }

    /// `{1: RSA, 3: RS256, -1: n, -2: 65537}`. The library accepts RS256, so
    /// only this module's own filter stands between this key and a passkey.
    fn rs256_cose_key() -> Vec<u8> {
        let mut key = vec![
            0xa4, 0x01, 0x03, 0x03, 0x39, 0x01, 0x00, 0x20, 0x59, 0x01, 0x00,
        ];
        key.extend_from_slice(&[0xc5; 256]);
        key.extend_from_slice(&[0x21, 0x43, 0x01, 0x00, 0x01]);
        key
    }

    /// `{1: OKP, 3: EdDSA, -1: Ed25519, -2: x}`, with the curve's base point
    /// as `x`. The library was never told to accept EdDSA and refuses it.
    fn eddsa_cose_key() -> Vec<u8> {
        let mut key = vec![0xa4, 0x01, 0x01, 0x03, 0x27, 0x20, 0x06, 0x21, 0x58, 0x20];
        key.push(0x58);
        key.extend_from_slice(&[0x66; 31]);
        key
    }

    #[test]
    fn a_registration_yields_the_authenticators_key_id_and_counter() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new().with_sign_count(7);
        let (options, state) = start(&verifier);

        let registered = verifier
            .finish_registration(&authenticator.create(ORIGIN, &options), state)
            .unwrap();

        assert_eq!(
            registered.public_key.to_vec(),
            authenticator.public_key_cose()
        );
        assert_eq!(
            base64url_encode(&registered.credential_id),
            authenticator.credential_id()
        );
        assert_eq!(registered.sign_count, 7);
        let stored: Value = serde_json::from_str(&registered.credential).unwrap();
        assert!(stored.is_object(), "{stored}");
    }

    #[test]
    fn the_options_ask_for_a_discoverable_user_verified_es256_credential() {
        let user_handle = Uuid::new_v4();
        let excluded = vec![vec![1u8, 2, 3], vec![0xff; 32]];

        let (options, _) = verifier()
            .start_registration(user_handle, LABEL, &excluded)
            .unwrap();
        let public_key = &options["publicKey"];

        assert_eq!(
            public_key["pubKeyCredParams"],
            json!([{ "type": "public-key", "alg": -7 }])
        );
        assert_eq!(
            public_key["authenticatorSelection"]["residentKey"],
            "required"
        );
        assert_eq!(
            public_key["authenticatorSelection"]["requireResidentKey"],
            true
        );
        assert_eq!(
            public_key["authenticatorSelection"]["userVerification"],
            "required"
        );
        assert_eq!(public_key["rp"]["id"], "hub.example");
        assert_eq!(
            public_key["user"],
            json!({
                "id": base64url_encode(user_handle.as_bytes()),
                "name": LABEL,
                "displayName": LABEL,
            })
        );
        assert_eq!(
            public_key["excludeCredentials"],
            json!([
                { "type": "public-key", "id": "AQID" },
                { "type": "public-key", "id": base64url_encode(&[0xff; 32]) },
            ])
        );
        assert_eq!(public_key["timeout"], CHALLENGE_TTL_SECS * 1000);
    }

    #[test]
    fn a_response_to_another_ceremonys_challenge_is_refused() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new();
        let (_, state) = start(&verifier);
        let (other_options, _) = start(&verifier);

        assert_eq!(
            verifier.finish_registration(&authenticator.create(ORIGIN, &other_options), state),
            Err(RegistrationError::WrongChallenge)
        );
    }

    #[test]
    fn a_response_made_at_another_origin_is_refused() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new();

        for origin in [
            "https://evil.example",
            "https://hub.example:8443",
            "http://hub.example",
            "https://sub.hub.example",
        ] {
            let (options, state) = start(&verifier);
            assert_eq!(
                verifier.finish_registration(&authenticator.create(origin, &options), state),
                Err(RegistrationError::WrongOrigin),
                "{origin}"
            );
        }
    }

    #[test]
    fn a_key_that_is_not_es256_is_refused_as_unsupported() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new();

        for (name, key) in [("RS256", rs256_cose_key()), ("EdDSA", eddsa_cose_key())] {
            let (options, state) = start(&verifier);
            let credential = authenticator.create_with(
                ORIGIN,
                &options,
                FLAG_USER_PRESENT | FLAG_USER_VERIFIED,
                &key,
            );
            assert_eq!(
                verifier.finish_registration(&credential, state),
                Err(RegistrationError::UnsupportedAlgorithm),
                "{name}"
            );
        }
    }

    #[test]
    fn a_registration_without_user_verification_is_refused() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new();
        let (options, state) = start(&verifier);
        let credential = authenticator.create_with(
            ORIGIN,
            &options,
            FLAG_USER_PRESENT,
            &authenticator.public_key_cose(),
        );

        assert_eq!(
            verifier.finish_registration(&credential, state),
            Err(RegistrationError::UserNotVerified)
        );
    }

    #[test]
    fn a_credential_the_ceremony_excluded_is_refused() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new();
        let (first_options, first_state) = start(&verifier);
        let registered = verifier
            .finish_registration(&authenticator.create(ORIGIN, &first_options), first_state)
            .unwrap();

        let (options, state) = verifier
            .start_registration(Uuid::new_v4(), LABEL, &[registered.credential_id])
            .unwrap();

        assert_eq!(
            verifier.finish_registration(&authenticator.create(ORIGIN, &options), state),
            Err(RegistrationError::CredentialExcluded)
        );
    }

    #[test]
    fn json_that_is_not_a_registration_credential_is_malformed() {
        let verifier = verifier();
        let (_, state) = start(&verifier);

        assert_eq!(
            verifier.finish_registration(&json!({ "id": "AQID" }), state),
            Err(RegistrationError::Malformed)
        );
    }

    #[test]
    fn a_registered_passkey_signs_in_from_its_stored_credential_alone() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new().with_sign_count(7);
        let user_handle = Uuid::new_v4();
        let (options, state) = verifier
            .start_registration(user_handle, LABEL, &[])
            .unwrap();
        let registered = verifier
            .finish_registration(&authenticator.create(ORIGIN, &options), state)
            .unwrap();
        let authenticator = authenticator.with_sign_count(8);

        let (options, state) = verifier.start_sign_in().unwrap();
        let credential = authenticator.get(ORIGIN, &options, Some(user_handle.as_bytes()));

        assert_eq!(
            credential["response"]["userHandle"],
            base64url_encode(user_handle.as_bytes())
        );
        assert_eq!(
            sign_in_credential_id(&credential),
            Ok(registered.credential_id)
        );
        assert_eq!(
            verifier.finish_sign_in(&credential, state, &registered.credential, 7),
            Ok(SignedIn {
                sign_count: 8,
                counter_regressed: false,
            })
        );
    }

    #[test]
    fn the_request_options_name_no_credential_and_require_user_verification() {
        let verifier = verifier();

        let (options, _) = verifier.start_sign_in().unwrap();
        let (other_options, _) = verifier.start_sign_in().unwrap();
        let public_key = &options["publicKey"];

        assert_eq!(public_key.get("allowCredentials"), None);
        assert_eq!(options.get("mediation"), None);
        assert_eq!(public_key["userVerification"], "required");
        assert_eq!(public_key["rpId"], "hub.example");
        assert_eq!(public_key["timeout"], CHALLENGE_TTL_SECS * 1000);
        let challenge = |options: &Value| {
            options["publicKey"]["challenge"]
                .as_str()
                .and_then(crate::auth::challenge::base64url_decode)
        };
        assert_eq!(challenge(&options).map(|bytes| bytes.len()), Some(32));
        assert_ne!(challenge(&options), challenge(&other_options));
    }

    #[test]
    fn a_sign_in_signed_by_another_key_is_refused() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new();
        let registered = register(&verifier, &authenticator);
        let (options, state) = verifier.start_sign_in().unwrap();

        let mut forged = SoftAuthenticator::new().get(ORIGIN, &options, None);
        forged["id"] = json!(authenticator.credential_id());
        forged["rawId"] = json!(authenticator.credential_id());

        assert_eq!(
            verifier.finish_sign_in(&forged, state, &registered.credential, 0),
            Err(SignInError::BadSignature)
        );
    }

    #[test]
    fn a_sign_in_naming_another_credential_is_refused() {
        let verifier = verifier();
        let registered = register(&verifier, &SoftAuthenticator::new());

        assert_eq!(
            sign_in(
                &verifier,
                &SoftAuthenticator::new(),
                &registered.credential,
                0
            ),
            Err(SignInError::WrongCredential)
        );
    }

    #[test]
    fn a_sign_in_over_another_ceremonys_challenge_is_refused() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new();
        let registered = register(&verifier, &authenticator);
        let (_, state) = verifier.start_sign_in().unwrap();
        let (other_options, _) = verifier.start_sign_in().unwrap();

        assert_eq!(
            verifier.finish_sign_in(
                &authenticator.get(ORIGIN, &other_options, None),
                state,
                &registered.credential,
                0
            ),
            Err(SignInError::WrongChallenge)
        );
    }

    #[test]
    fn a_sign_in_made_at_another_origin_is_refused() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new();
        let registered = register(&verifier, &authenticator);
        let (options, state) = verifier.start_sign_in().unwrap();

        assert_eq!(
            verifier.finish_sign_in(
                &authenticator.get("https://evil.example", &options, None),
                state,
                &registered.credential,
                0
            ),
            Err(SignInError::WrongOrigin)
        );
    }

    #[test]
    fn a_sign_in_without_user_verification_is_refused() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new();
        let registered = register(&verifier, &authenticator);
        let (options, state) = verifier.start_sign_in().unwrap();
        let credential = authenticator.get_with(ORIGIN, &options, FLAG_USER_PRESENT, None);

        assert_eq!(
            verifier.finish_sign_in(&credential, state, &registered.credential, 0),
            Err(SignInError::UserNotVerified)
        );
    }

    #[test]
    fn a_zero_counter_signs_in_twice_in_a_row() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new();
        let registered = register(&verifier, &authenticator);
        assert_eq!(registered.sign_count, 0);

        for attempt in ["first", "second"] {
            assert_eq!(
                sign_in(&verifier, &authenticator, &registered.credential, 0),
                Ok(SignedIn {
                    sign_count: 0,
                    counter_regressed: false,
                }),
                "{attempt}"
            );
        }
    }

    #[test]
    fn a_counter_that_did_not_advance_is_reported_and_never_refused() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new().with_sign_count(7);
        let registered = register(&verifier, &authenticator);
        let mut authenticator = authenticator;

        for (reported, regressed) in [(8, false), (7, true), (5, true), (0, true)] {
            authenticator = authenticator.with_sign_count(reported);
            assert_eq!(
                sign_in(&verifier, &authenticator, &registered.credential, 7),
                Ok(SignedIn {
                    sign_count: reported,
                    counter_regressed: regressed,
                }),
                "{reported}"
            );
        }
    }

    #[test]
    fn the_clone_signal_is_judged_against_the_counter_passed_in_not_the_registrations() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new().with_sign_count(5);
        let registered = register(&verifier, &authenticator);
        assert_eq!(registered.sign_count, 5);
        let authenticator = authenticator.with_sign_count(51);

        for (stored_count, regressed) in [(100, true), (51, true), (50, false)] {
            assert_eq!(
                sign_in(
                    &verifier,
                    &authenticator,
                    &registered.credential,
                    stored_count
                ),
                Ok(SignedIn {
                    sign_count: 51,
                    counter_regressed: regressed,
                }),
                "{stored_count}"
            );
        }
    }

    #[test]
    fn a_passkey_that_became_backup_eligible_still_signs_in() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new();
        let registered = register_with_flags(&verifier, &authenticator, UP_AND_UV);
        let stored: Value = serde_json::from_str(&registered.credential).unwrap();
        assert_eq!(stored.pointer(STORED_BACKUP_ELIGIBLE), Some(&json!(false)));

        for (name, flags) in [
            ("eligible", UP_AND_UV | FLAG_BACKUP_ELIGIBLE),
            (
                "eligible and backed up",
                UP_AND_UV | FLAG_BACKUP_ELIGIBLE | FLAG_BACKED_UP,
            ),
        ] {
            assert_eq!(
                sign_in_with_flags(&verifier, &authenticator, flags, &registered),
                Ok(SignedIn {
                    sign_count: 0,
                    counter_regressed: false,
                }),
                "{name}"
            );
        }
    }

    #[test]
    fn a_passkey_that_stopped_being_backup_eligible_is_refused() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new();
        let registered =
            register_with_flags(&verifier, &authenticator, UP_AND_UV | FLAG_BACKUP_ELIGIBLE);
        let stored: Value = serde_json::from_str(&registered.credential).unwrap();
        assert_eq!(stored.pointer(STORED_BACKUP_ELIGIBLE), Some(&json!(true)));

        assert_eq!(
            sign_in_with_flags(&verifier, &authenticator, UP_AND_UV, &registered),
            Err(SignInError::Refused)
        );
        assert!(sign_in_with_flags(
            &verifier,
            &authenticator,
            UP_AND_UV | FLAG_BACKUP_ELIGIBLE,
            &registered
        )
        .is_ok());
    }

    #[test]
    fn json_that_is_not_a_sign_in_credential_is_malformed() {
        let verifier = verifier();
        let registered = register(&verifier, &SoftAuthenticator::new());
        let (_, state) = verifier.start_sign_in().unwrap();
        let not_a_credential = json!({ "id": "AQID" });

        assert_eq!(
            sign_in_credential_id(&not_a_credential),
            Err(SignInError::Malformed)
        );
        assert_eq!(
            verifier.finish_sign_in(&not_a_credential, state, &registered.credential, 0),
            Err(SignInError::Malformed)
        );
    }

    #[test]
    fn a_stored_credential_this_module_did_not_write_is_refused() {
        let verifier = verifier();
        let authenticator = SoftAuthenticator::new();
        let registered = register(&verifier, &authenticator);
        let mut without_counter: Value = serde_json::from_str(&registered.credential).unwrap();
        without_counter["cred"]
            .as_object_mut()
            .unwrap()
            .remove("counter");

        for stored in [
            "not json".to_owned(),
            "{}".to_owned(),
            json!({ "cred": { "counter": 0 } }).to_string(),
            without_counter.to_string(),
        ] {
            assert_eq!(
                sign_in(&verifier, &authenticator, &stored, 0),
                Err(SignInError::StoredCredential),
                "{stored}"
            );
        }
    }

    #[test]
    fn a_relying_party_the_library_cannot_serve_is_not_built() {
        let build = |rp_id: &str, origin: &str| {
            Verifier::new(&RelyingParty {
                rp_id: rp_id.to_owned(),
                origin: origin.to_owned(),
            })
            .err()
        };

        assert_eq!(build("hub.example", ORIGIN), None);
        assert_eq!(build("example", "https://hub.example"), None);
        assert_eq!(
            build("other.example", ORIGIN),
            Some(RelyingPartyError::IdDoesNotCoverOrigin)
        );
        assert_eq!(
            build("hub.example", "hub.example"),
            Some(RelyingPartyError::OriginNotAUrl)
        );
    }
}

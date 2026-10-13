//! A software device key for tests of pairing: the P-256 key an instance
//! keeps in its keyring, and the DPoP proofs it makes with it. Beside it, the
//! token rows of one pairing held in memory, and the check an instance makes
//! of a signature under the key it pinned.
//!
//! Nothing here names an origin or a path: every caller passes its own `htu`.
//! This file is compiled outside `#[cfg(test)]`, so `tests/no_literals.rs`
//! scans it like any other source file.
//!
//! It panics where [`auth::testing`](crate::auth::testing) does, for the
//! reason given there.

use aws_lc_rs::error::Unspecified;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{
    EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_FIXED_SIGNING, ED25519,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use super::grant::{FamilyChange, FreshPair};
use crate::auth::challenge::base64url_encode;
use crate::auth::cose::P256Point;
use crate::store::tokens::{TokenHash, TokenRecord};
use crate::store::UserId;

pub struct SoftDeviceKey {
    key_pair: EcdsaKeyPair,
}

impl SoftDeviceKey {
    pub fn new() -> Self {
        Self {
            key_pair: or_panic(
                EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING),
                "generate a P-256 key pair",
            ),
        }
    }

    /// The public key as a client sends it: the four required members and
    /// nothing else.
    pub fn jwk(&self) -> Value {
        let point = self.point();
        json!({
            "kty": "EC",
            "crv": "P-256",
            "x": base64url_encode(&point.x),
            "y": base64url_encode(&point.y),
        })
    }

    /// The RFC 7638 thumbprint, which is what `dpop_jkt` carries.
    ///
    /// The text it is a digest of is assembled here and not taken from
    /// `DeviceJwk`: a double sharing the verifier's spelling of it could not
    /// show that spelling wrong. The members are written in the order the
    /// thumbprint needs, so the text is the same whether `serde_json` sorts
    /// an object's members or keeps them as inserted.
    pub fn thumbprint(&self) -> String {
        let point = self.point();
        let canonical = json!({
            "crv": "P-256",
            "kty": "EC",
            "x": base64url_encode(&point.x),
            "y": base64url_encode(&point.y),
        });
        base64url_encode(&Sha256::digest(canonical.to_string().as_bytes()))
    }

    /// The JOSE header of a proof by this key.
    pub fn header(&self) -> Value {
        json!({"typ": "dpop+jwt", "alg": "ES256", "jwk": self.jwk()})
    }

    /// A proof for one `htm` request to `htu`, made at `iat` (Unix seconds).
    pub fn proof(&self, htm: &str, htu: &str, iat: u64, jti: &str) -> String {
        self.proof_with(&self.header(), &claims(htm, htu, iat, jti))
    }

    /// A compact JWS with a valid ES256 signature over whatever header and
    /// claims it is told to carry, for the cases a verifier must refuse. The
    /// signature is by this key whatever the header says of `alg` and `jwk`.
    pub fn proof_with(&self, header: &Value, claims: &Value) -> String {
        let signed = format!(
            "{}.{}",
            base64url_encode(header.to_string().as_bytes()),
            base64url_encode(claims.to_string().as_bytes()),
        );
        let signature = or_panic(
            self.key_pair.sign(&SystemRandom::new(), signed.as_bytes()),
            "sign a proof",
        );
        format!("{signed}.{}", base64url_encode(signature.as_ref()))
    }

    fn point(&self) -> P256Point {
        let Some(point) = P256Point::from_sec1_uncompressed(self.key_pair.public_key().as_ref())
        else {
            panic!("soft device key: a P-256 public key is an uncompressed SEC1 point");
        };
        point
    }
}

impl Default for SoftDeviceKey {
    fn default() -> Self {
        Self::new()
    }
}

/// The claims of a proof, for [`SoftDeviceKey::proof_with`].
pub fn claims(htm: &str, htu: &str, iat: u64, jti: &str) -> Value {
    json!({"jti": jti, "htm": htm, "htu": htu, "iat": iat})
}

fn or_panic<T>(result: Result<T, Unspecified>, action: &str) -> T {
    match result {
        Ok(value) => value,
        Err(Unspecified) => panic!("soft device key: could not {action}"),
    }
}

/// Whether `signature` is an Ed25519 signature over `message` by the key
/// whose raw public half is `public_key`: what an instance asks of a request
/// under the `request_signing_key` it pinned. A key or a signature of the
/// wrong length does not verify.
pub fn verifies_ed25519(public_key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(message, signature)
        .is_ok()
}

/// The `tokens` rows of one pairing, for folding
/// [`refresh_change`](super::grant::refresh_change) over without a database.
///
/// It stands where the store's transaction does and, like it, decides
/// nothing: [`apply`](Self::apply) writes whatever change it is handed. A
/// rule written here would pass the tests that read it and hold nowhere else.
///
/// Row ids are minted here and never leave except inside a [`TokenRecord`],
/// so a caller finds a row by the hash of the token it holds.
pub struct MemoryFamily {
    user: UserId,
    rows: Vec<TokenRecord>,
}

impl MemoryFamily {
    /// A tenant's `tokens` before anything was written to it, for rows a
    /// caller was handed and writes with [`apply`](Self::apply).
    pub fn empty() -> Self {
        Self {
            user: UserId::generate(),
            rows: Vec::new(),
        }
    }

    /// What a pairing grant leaves behind: the rows of `first`, issued at
    /// `now` (Unix seconds) to the device key whose thumbprint is `jkt`, in a
    /// family of their own. They are built by [`FreshPair::rows`], as the
    /// grant's are.
    pub fn paired(first: &FreshPair, jkt: &str, now: u64) -> Self {
        let rows = match first.rows(Uuid::new_v4(), jkt, now) {
            Ok(rows) => rows,
            Err(error) => panic!("memory family: {error}"),
        };
        let mut family = Self::empty();
        let grant = FamilyChange {
            retire: Vec::new(),
            insert: vec![rows.access, rows.refresh],
        };
        family.apply(&grant, now);
        family
    }

    pub fn rows(&self) -> &[TokenRecord] {
        &self.rows
    }

    /// The row a token is stored as, whatever its kind, expiry or retirement.
    pub fn find(&self, hash: &TokenHash) -> Option<&TokenRecord> {
        self.rows.iter().find(|row| row.hash == *hash)
    }

    /// Writes `change` as of `now` (Unix seconds): the rows it names are
    /// retired at `now`, and the rows it carries are added, live.
    pub fn apply(&mut self, change: &FamilyChange, now: u64) {
        let at = i64::try_from(now)
            .ok()
            .and_then(|now| OffsetDateTime::from_unix_timestamp(now).ok());
        let Some(at) = at else {
            panic!("memory family: {now} is not a time a row can hold");
        };
        for row in &mut self.rows {
            if change.retire.contains(&row.id) {
                row.retired_at = Some(at);
            }
        }
        for new in &change.insert {
            self.rows.push(TokenRecord {
                id: Uuid::new_v4(),
                user_id: self.user,
                kind: new.kind,
                hash: new.hash.clone(),
                family_id: new.family_id,
                jkt: new.jkt.clone(),
                expires_at: new.expires_at,
                retired_at: None,
            });
        }
    }
}

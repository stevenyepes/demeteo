//! The P-256 key this install proves possession of to the Hub. Only the
//! signature and the key's encodings are here; the JWK text, thumbprint and
//! proof layout are [`crate::domain::hub_pairing`]'s, so the bytes the Hub
//! compares are decided in one place.
//!
//! ES256 is the only algorithm `crates/demeteo-hub/src/pairing/dpop.rs`
//! accepts, so an Ed25519 key here would be refused at `/token`. The signing
//! algorithm is the `_FIXED` one because JWS wants the raw 64-byte `r ‖ s`:
//! the `_ASN1` variant signs in DER, which the Hub's verifier refuses.

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};

use crate::domain::hub_pairing::dpop::{dpop_proof, dpop_signing_input_for};
use crate::domain::hub_pairing::jwk::{fingerprint, thumbprint};
use crate::domain::hub_pairing::{access_token_hash, HubPairingError};

/// No `Debug`, `Clone` or `Serialize`: the private half leaves only through
/// [`DeviceKey::to_pkcs8`], on its way to the OS keyring.
pub struct DeviceKey {
    key_pair: EcdsaKeyPair,
    point: [u8; 65],
}

fn key_error(what: &str) -> HubPairingError {
    HubPairingError::Storage(format!("device key: {what}"))
}

impl DeviceKey {
    pub fn generate() -> Result<Self, HubPairingError> {
        let pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
                .map_err(|_| key_error("could not generate a P-256 key"))?;
        Self::from_pkcs8(pkcs8.as_ref())
    }

    pub fn from_pkcs8(pkcs8: &[u8]) -> Result<Self, HubPairingError> {
        let key_pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8)
            .map_err(|_| key_error("not a P-256 PKCS#8 private key"))?;
        let point = <[u8; 65]>::try_from(key_pair.public_key().as_ref())
            .map_err(|_| key_error("public key is not an uncompressed P-256 point"))?;
        Ok(Self { key_pair, point })
    }

    pub fn to_pkcs8(&self) -> Result<Vec<u8>, HubPairingError> {
        self.key_pair
            .to_pkcs8v1()
            .map(|document| document.as_ref().to_vec())
            .map_err(|_| key_error("could not export the private key"))
    }

    pub fn public_point(&self) -> &[u8; 65] {
        &self.point
    }

    pub fn jkt(&self) -> String {
        thumbprint(&self.point)
    }

    pub fn fingerprint(&self) -> String {
        fingerprint(&self.jkt())
    }

    /// A proof for `POST htu` at `iat` (Unix seconds). The Hub refuses a
    /// `jti` outside 1 to 128 bytes.
    pub fn dpop_proof(&self, htu: &str, iat: u64, jti: &str) -> Result<String, HubPairingError> {
        self.dpop_proof_for("POST", htu, iat, jti, None)
    }

    /// A proof for `htm htu`, bound to `access_token` when the request
    /// presents one.
    pub fn dpop_proof_for(
        &self,
        htm: &str,
        htu: &str,
        iat: u64,
        jti: &str,
        access_token: Option<&str>,
    ) -> Result<String, HubPairingError> {
        let ath = access_token.map(access_token_hash);
        let signing_input = dpop_signing_input_for(&self.point, htm, htu, iat, jti, ath.as_deref());
        let signature = self
            .key_pair
            .sign(&SystemRandom::new(), signing_input.as_bytes())
            .map_err(|_| key_error("could not sign a DPoP proof"))?;
        let raw = <[u8; 64]>::try_from(signature.as_ref())
            .map_err(|_| key_error("signature is not a 64-byte r ‖ s"))?;
        Ok(dpop_proof(&signing_input, &raw))
    }
}

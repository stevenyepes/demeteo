//! Transit's bring-your-own-key import, as pure functions over bytes: the PEM
//! GitHub hands back from a manifest conversion becomes PKCS#8 DER, and that
//! is wrapped the one way Transit accepts. A fresh AES-256 key wraps the
//! target with KWP (RFC 5649), the AES key is encrypted to Transit's own
//! wrapping key with RSA-OAEP over SHA-256, and the import's `ciphertext` is
//! the base64 of the two concatenated, RSA part first. Transit unwraps inside
//! itself, so the key never travels in a form an observer of the request
//! could use.
//!
//! [`OpenBaoKeys`](super::openbao::OpenBaoKeys) does the I/O; nothing here
//! talks to a server, so the exact bytes are reachable from a test.

use crate::ports::{KeyServiceError, RsaPrivateKey};
use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::key_wrap::{AesKek, KeyWrapPadded, AES_256};
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use aws_lc_rs::rsa::{
    KeyPair, OaepPublicEncryptingKey, PublicEncryptingKey, OAEP_SHA256_MGF1SHA256,
};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;

/// Transit's import names the key's type up front, and a GitHub App key is
/// always this size; any other is refused before it is sent.
const RSA_BITS: usize = 2048;
pub const TRANSIT_RSA_TYPE: &str = "rsa-2048";
const EPHEMERAL_KEY_LEN: usize = 32;
/// RFC 5649 pads to the next 8 bytes and prepends an 8-byte block.
const KWP_OVERHEAD: usize = 15;

const PKCS1_LABEL: &str = "RSA PRIVATE KEY";
const PKCS8_LABEL: &str = "PRIVATE KEY";
const PUBLIC_KEY_LABEL: &str = "PUBLIC KEY";

/// Why a key was refused before anything was sent. The text never carries key
/// material, only which check it failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyRefused {
    #[error("not a PEM-encoded RSA private key")]
    NotPem,
    #[error("the PEM does not parse as an RSA private key")]
    Unparseable,
    #[error("the RSA key is not 2048 bits")]
    WrongSize,
}

/// GitHub answers PKCS#1 (`RSA PRIVATE KEY`); PKCS#8 (`PRIVATE KEY`) is taken
/// too, because a key converted by hand arrives that way.
pub fn rsa_key_from_pem(pem: &str) -> Result<RsaPrivateKey, KeyRefused> {
    let (label, der) = pem_body(pem).ok_or(KeyRefused::NotPem)?;
    let pair = match label.as_str() {
        PKCS1_LABEL => KeyPair::from_der(&der),
        PKCS8_LABEL => KeyPair::from_pkcs8(&der),
        _ => return Err(KeyRefused::NotPem),
    }
    .map_err(|_| KeyRefused::Unparseable)?;
    if pair.public_modulus_len() * 8 != RSA_BITS {
        return Err(KeyRefused::WrongSize);
    }
    let pkcs8 = AsDer::<aws_lc_rs::encoding::Pkcs8V1Der>::as_der(&pair)
        .map_err(|_| KeyRefused::Unparseable)?;
    Ok(RsaPrivateKey::from_checked_pkcs8(pkcs8.as_ref().to_vec()))
}

/// The import's `ciphertext` for `key`, under the PEM `SubjectPublicKeyInfo`
/// Transit answers at `transit/wrapping_key`.
pub fn wrap_for_import(
    wrapping_key_pem: &str,
    key: &RsaPrivateKey,
) -> Result<String, KeyServiceError> {
    let ephemeral = ephemeral_key()?;
    wrap_with(wrapping_key_pem, key.pkcs8_der(), &ephemeral)
}

fn ephemeral_key() -> Result<[u8; EPHEMERAL_KEY_LEN], KeyServiceError> {
    let mut key = [0u8; EPHEMERAL_KEY_LEN];
    SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| KeyServiceError::Malformed("no randomness for the import key".into()))?;
    Ok(key)
}

fn wrap_with(
    wrapping_key_pem: &str,
    target: &[u8],
    ephemeral: &[u8; EPHEMERAL_KEY_LEN],
) -> Result<String, KeyServiceError> {
    let wrapping = wrapping_key(wrapping_key_pem)?;
    let mut sealed_key = vec![0u8; wrapping.ciphertext_size()];
    let sealed_key = wrapping
        .encrypt(&OAEP_SHA256_MGF1SHA256, ephemeral, &mut sealed_key, None)
        .map_err(|_| KeyServiceError::Malformed("RSA-OAEP over the import key failed".into()))?;

    let kek = AesKek::new(&AES_256, ephemeral)
        .map_err(|_| KeyServiceError::Malformed("the import key is not AES-256".into()))?;
    let mut wrapped = vec![0u8; target.len() + KWP_OVERHEAD];
    let wrapped = kek
        .wrap_with_padding(target, &mut wrapped)
        .map_err(|_| KeyServiceError::Malformed("key wrap of the target failed".into()))?;

    let mut ciphertext = Vec::with_capacity(sealed_key.len() + wrapped.len());
    ciphertext.extend_from_slice(sealed_key);
    ciphertext.extend_from_slice(wrapped);
    Ok(STANDARD.encode(ciphertext))
}

fn wrapping_key(pem: &str) -> Result<OaepPublicEncryptingKey, KeyServiceError> {
    let malformed =
        || KeyServiceError::Malformed("the wrapping key is not an RSA public key".into());
    let (label, der) = pem_body(pem).ok_or_else(malformed)?;
    if label != PUBLIC_KEY_LABEL {
        return Err(malformed());
    }
    PublicEncryptingKey::from_der(&der)
        .ok()
        .and_then(|key| OaepPublicEncryptingKey::new(key).ok())
        .ok_or_else(malformed)
}

/// The label and decoded body of the first PEM block in `pem`.
fn pem_body(pem: &str) -> Option<(String, Vec<u8>)> {
    let mut lines = pem.lines().map(str::trim).skip_while(|l| l.is_empty());
    let label = lines
        .next()?
        .strip_prefix("-----BEGIN ")?
        .strip_suffix("-----")?
        .to_owned();
    let end = format!("-----END {label}-----");
    let mut body = String::new();
    for line in lines {
        if line == end {
            return STANDARD.decode(body).ok().map(|der| (label, der));
        }
        body.push_str(line);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::encoding::{Pkcs8V1Der, PublicKeyX509Der};
    use aws_lc_rs::rsa::{KeySize, OaepPrivateDecryptingKey, PrivateDecryptingKey};
    use aws_lc_rs::signature::KeyPair as _;

    fn pem(label: &str, der: &[u8]) -> String {
        let body = STANDARD.encode(der);
        let lines: Vec<&str> = body
            .as_bytes()
            .chunks(64)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect();
        format!(
            "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
            lines.join("\n")
        )
    }

    fn pkcs8_of(pair: &KeyPair) -> Vec<u8> {
        AsDer::<Pkcs8V1Der>::as_der(pair).unwrap().as_ref().to_vec()
    }

    /// The `RSAPrivateKey` inside a PKCS#8 RSA document: its last element,
    /// an OCTET STRING. Test-only, because only fixtures need PKCS#1 made.
    fn pkcs1_inside(pkcs8: &[u8]) -> Vec<u8> {
        fn header(der: &[u8]) -> (usize, usize) {
            let first = der[1] as usize;
            if first < 0x80 {
                (2, first)
            } else {
                let n = first & 0x7f;
                let len = der[2..2 + n]
                    .iter()
                    .fold(0, |acc, b| (acc << 8) | *b as usize);
                (2 + n, len)
            }
        }
        let (outer, _) = header(pkcs8);
        let mut rest = &pkcs8[outer..];
        let mut last = Vec::new();
        while !rest.is_empty() {
            let (head, len) = header(rest);
            last = rest[head..head + len].to_vec();
            rest = &rest[head + len..];
        }
        last
    }

    fn generated(size: KeySize) -> KeyPair {
        KeyPair::generate(size).unwrap()
    }

    #[test]
    fn a_pkcs1_pem_becomes_the_same_key_as_pkcs8() {
        let pair = generated(KeySize::Rsa2048);
        let pkcs8 = pkcs8_of(&pair);
        let from_pkcs1 = rsa_key_from_pem(&pem(PKCS1_LABEL, &pkcs1_inside(&pkcs8))).unwrap();
        let from_pkcs8 = rsa_key_from_pem(&pem(PKCS8_LABEL, &pkcs8)).unwrap();

        let public = |key: &RsaPrivateKey| {
            KeyPair::from_pkcs8(key.pkcs8_der())
                .unwrap()
                .public_key()
                .as_ref()
                .to_vec()
        };
        assert_eq!(public(&from_pkcs1), pair.public_key().as_ref());
        assert_eq!(public(&from_pkcs8), pair.public_key().as_ref());
    }

    #[test]
    fn what_is_not_a_2048_bit_rsa_private_key_is_refused() {
        let big = pkcs8_of(&generated(KeySize::Rsa3072));
        assert_eq!(
            rsa_key_from_pem(&pem(PKCS8_LABEL, &big)).unwrap_err(),
            KeyRefused::WrongSize
        );
        assert_eq!(rsa_key_from_pem("").unwrap_err(), KeyRefused::NotPem);
        assert_eq!(
            rsa_key_from_pem(&pem("CERTIFICATE", b"x")).unwrap_err(),
            KeyRefused::NotPem
        );
        assert_eq!(
            rsa_key_from_pem("-----BEGIN RSA PRIVATE KEY-----\nAAAA\n").unwrap_err(),
            KeyRefused::NotPem,
            "an unterminated block is not PEM"
        );
        assert_eq!(
            rsa_key_from_pem(&pem(PKCS1_LABEL, b"not der")).unwrap_err(),
            KeyRefused::Unparseable
        );
    }

    #[test]
    fn the_key_never_appears_in_debug_output() {
        let pkcs8 = pkcs8_of(&generated(KeySize::Rsa2048));
        let key = rsa_key_from_pem(&pem(PKCS8_LABEL, &pkcs8)).unwrap();
        assert_eq!(format!("{key:?}"), "RsaPrivateKey(<redacted>)");
    }

    /// What Transit does on import, done here with the wrapping key's
    /// private half: the target comes back byte for byte.
    #[test]
    fn transit_unwraps_the_ciphertext_to_the_target_key() {
        let wrapping = generated(KeySize::Rsa2048);
        let wrapping_pub = AsDer::<PublicKeyX509Der>::as_der(wrapping.public_key())
            .unwrap()
            .as_ref()
            .to_vec();
        let target = pkcs8_of(&generated(KeySize::Rsa2048));
        let ephemeral = [42u8; EPHEMERAL_KEY_LEN];

        let ciphertext = STANDARD
            .decode(wrap_with(&pem(PUBLIC_KEY_LABEL, &wrapping_pub), &target, &ephemeral).unwrap())
            .unwrap();

        let private = OaepPrivateDecryptingKey::new(
            PrivateDecryptingKey::from_pkcs8(&pkcs8_of(&wrapping)).unwrap(),
        )
        .unwrap();
        let (sealed_key, wrapped) = ciphertext.split_at(private.key_size_bytes());
        let mut opened = vec![0u8; private.min_output_size()];
        let opened = private
            .decrypt(&OAEP_SHA256_MGF1SHA256, sealed_key, &mut opened, None)
            .unwrap();
        assert_eq!(opened, ephemeral);

        let mut unwrapped = vec![0u8; wrapped.len()];
        let unwrapped = AesKek::new(&AES_256, opened)
            .unwrap()
            .unwrap_with_padding(wrapped, &mut unwrapped)
            .unwrap();
        assert_eq!(unwrapped, target);
    }

    #[test]
    fn a_wrapping_key_that_is_not_an_rsa_public_key_is_malformed() {
        let target = [1u8; 16];
        for pem in [
            "",
            "-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----",
        ] {
            assert!(matches!(
                wrap_with(pem, &target, &[0; EPHEMERAL_KEY_LEN]),
                Err(KeyServiceError::Malformed(_))
            ));
        }
    }
}

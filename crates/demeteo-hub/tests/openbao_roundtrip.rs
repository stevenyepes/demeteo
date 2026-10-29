//! Round trips against a real OpenBao Transit engine. Needs the compose stack
//! initialised and unsealed as `deploy/hub/README.md` describes.

use aws_lc_rs::signature::{UnparsedPublicKey, ED25519};
use demeteo_hub::adapters::openbao::OpenBaoKeys;
use demeteo_hub::ports::{KeyService, KeyServiceError};

const ADDR_ENV: &str = "OPENBAO_ADDR";
const TOKEN_ENV: &str = "OPENBAO_TOKEN";
const MOUNT_ENV: &str = "DEMETEO_HUB_TRANSIT_MOUNT";
const KEY_ENV: &str = "DEMETEO_HUB_TRANSIT_KEY";
const SIGNING_KEY_ENV: &str = "DEMETEO_HUB_SIGNING_KEY";
const FALLBACK_MOUNT: &str = "transit";
const FALLBACK_KEY: &str = "demeteo-hub";
const FALLBACK_SIGNING_KEY: &str = "demeteo-hub-signing";
const MISSING_KEY: &str = "demeteo-hub-test-no-such-key";

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set (see deploy/hub/README.md)"))
}

fn service() -> OpenBaoKeys {
    let mount = std::env::var(MOUNT_ENV).unwrap_or_else(|_| FALLBACK_MOUNT.to_owned());
    OpenBaoKeys::new(
        reqwest::Client::new(),
        &required(ADDR_ENV),
        &mount,
        &required(TOKEN_ENV),
    )
    .expect("token is a valid header value")
}

fn keys() -> (OpenBaoKeys, String) {
    let key = std::env::var(KEY_ENV).unwrap_or_else(|_| FALLBACK_KEY.to_owned());
    (service(), key)
}

fn signing_keys() -> (OpenBaoKeys, String) {
    let key = std::env::var(SIGNING_KEY_ENV).unwrap_or_else(|_| FALLBACK_SIGNING_KEY.to_owned());
    (service(), key)
}

#[tokio::test]
#[ignore = "needs a running, unsealed OpenBao: see deploy/hub/README.md; set OPENBAO_ADDR and OPENBAO_TOKEN"]
async fn encrypt_then_decrypt_returns_the_original_bytes() {
    let (keys, key) = keys();
    let samples: [&[u8]; 4] = [b"hello", b"", &[0xff, 0xfe, 0x00, 0x80], &[0u8; 4096]];
    for plaintext in samples {
        let ciphertext = keys.encrypt(&key, plaintext).await.unwrap();
        assert!(ciphertext.starts_with("vault:"), "{ciphertext}");
        assert_eq!(keys.decrypt(&key, &ciphertext).await.unwrap(), plaintext);
    }
}

fn refused<T>(result: &Result<T, KeyServiceError>) -> bool {
    matches!(
        result,
        Err(KeyServiceError::UnknownKey(_) | KeyServiceError::Upstream { status: 403, .. })
    )
}

#[tokio::test]
#[ignore = "needs a running, unsealed OpenBao: see deploy/hub/README.md; set OPENBAO_ADDR and OPENBAO_TOKEN"]
async fn a_key_name_that_does_not_exist_errors() {
    let (keys, key) = keys();
    let ciphertext = keys.encrypt(&key, b"x").await.unwrap();

    // Decrypt and datakey never create a key; encrypt may, depending on the
    // token's policy, so it is not asserted here. The README's least-privilege
    // policy covers only the configured key's paths, so OpenBao's ACL layer
    // answers 403 before Transit ever looks the missing key up.
    let decrypted = keys.decrypt(MISSING_KEY, &ciphertext).await;
    assert!(refused(&decrypted), "{decrypted:?}");
    let data_key = keys.data_key(MISSING_KEY).await;
    assert!(refused(&data_key), "{data_key:?}");
}

#[tokio::test]
#[ignore = "needs a running, unsealed OpenBao: see deploy/hub/README.md; set OPENBAO_ADDR and OPENBAO_TOKEN"]
async fn data_key_wrapped_form_decrypts_back_to_its_plaintext() {
    let (keys, key) = keys();
    let data_key = keys.data_key(&key).await.unwrap();
    assert_eq!(data_key.plaintext.len(), 32);
    assert!(data_key.wrapped.starts_with("vault:"));
    assert_eq!(
        keys.decrypt(&key, &data_key.wrapped).await.unwrap(),
        data_key.plaintext
    );
}

#[tokio::test]
#[ignore = "needs a running, unsealed OpenBao: see deploy/hub/README.md; set OPENBAO_ADDR and OPENBAO_TOKEN"]
async fn a_signature_verifies_under_the_key_verifying_key_returned() {
    let (keys, key) = signing_keys();
    let public_key = keys.verifying_key(&key).await.unwrap();
    let verifier = UnparsedPublicKey::new(&ED25519, public_key);
    let samples: [&[u8]; 4] = [b"hello", b"", &[0xff, 0xfe, 0x00, 0x80], &[0u8; 4096]];
    for message in samples {
        let signature = keys.sign(&key, message).await.unwrap();
        verifier.verify(message, &signature).unwrap();
    }

    let signature = keys.sign(&key, b"signed").await.unwrap();
    assert!(verifier.verify(b"not signed", &signature).is_err());
}

#[tokio::test]
#[ignore = "needs a running, unsealed OpenBao: see deploy/hub/README.md; set OPENBAO_ADDR and OPENBAO_TOKEN"]
async fn a_signing_key_name_that_does_not_exist_is_refused() {
    let (keys, _) = signing_keys();
    let public_key = keys.verifying_key(MISSING_KEY).await;
    assert!(refused(&public_key), "{public_key:?}");
    let signature = keys.sign(MISSING_KEY, b"x").await;
    assert!(refused(&signature), "{signature:?}");
}

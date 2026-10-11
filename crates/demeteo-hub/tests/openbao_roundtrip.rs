//! Round trips against a real OpenBao Transit engine. Needs the compose stack
//! initialised and unsealed as `deploy/hub/README.md` describes.

use demeteo_hub::adapters::openbao::OpenBaoKeys;
use demeteo_hub::ports::{KeyService, KeyServiceError};

const ADDR_ENV: &str = "OPENBAO_ADDR";
const TOKEN_ENV: &str = "OPENBAO_TOKEN";
const MOUNT_ENV: &str = "DEMETEO_HUB_TRANSIT_MOUNT";
const KEY_ENV: &str = "DEMETEO_HUB_TRANSIT_KEY";
const FALLBACK_MOUNT: &str = "transit";
const FALLBACK_KEY: &str = "demeteo-hub";
const MISSING_KEY: &str = "demeteo-hub-test-no-such-key";

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set (see deploy/hub/README.md)"))
}

fn keys() -> (OpenBaoKeys, String) {
    let mount = std::env::var(MOUNT_ENV).unwrap_or_else(|_| FALLBACK_MOUNT.to_owned());
    let key = std::env::var(KEY_ENV).unwrap_or_else(|_| FALLBACK_KEY.to_owned());
    let service = OpenBaoKeys::new(
        reqwest::Client::new(),
        &required(ADDR_ENV),
        &mount,
        &required(TOKEN_ENV),
    )
    .expect("token is a valid header value");
    (service, key)
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

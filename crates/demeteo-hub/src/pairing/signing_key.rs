//! The Hub's request-signing key as the Hub holds it: a name in the key
//! service. Its public half is what a pairing response hands a desktop to
//! pin, and a signature under it is what the desktop checks a request by.

use crate::ports::{KeyService, KeyServiceError};
use tokio::sync::OnceCell;

/// The key is read on the first pairing that needs it and never at startup:
/// `/healthz` and sign-in owe the key service nothing, and a Hub that asked at
/// boot would not come up beside a sealed OpenBao, which
/// [`router`](crate::routes::router) promises it does.
///
/// Only an answer is kept. `Sealed` ends when an operator unseals, so a kept
/// refusal would outlive its cause and hold every pairing back until the Hub
/// was restarted. The answer is kept for the life of the process because a
/// desktop pins it at pairing and nothing carries a later key to one
/// (`docs/HUB.md` §11, item 7): a Hub that re-read it could only start
/// advertising a key its paired instances do not trust.
pub struct SigningKey {
    name: String,
    public_key: OnceCell<[u8; 32]>,
}

impl SigningKey {
    /// `name` is the key's name in the key service, not key material.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            public_key: OnceCell::new(),
        }
    }

    /// Concurrent first reads share one request to `keys`; if it fails, each
    /// of them makes its own in turn until one is answered.
    pub async fn public_key(&self, keys: &dyn KeyService) -> Result<[u8; 32], KeyServiceError> {
        self.public_key
            .get_or_try_init(|| keys.verifying_key(&self.name))
            .await
            .copied()
    }

    /// The Ed25519 signature over `message` under this key. `message` is
    /// signed as it is handed over and is not hashed first.
    ///
    /// Unlike the public half, nothing is kept: each call asks `keys`, and a
    /// refusal is the caller's to answer for.
    pub async fn sign(
        &self,
        keys: &dyn KeyService,
        message: &[u8],
    ) -> Result<[u8; 64], KeyServiceError> {
        keys.sign(&self.name, message).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::memory_keys::MemoryKeys;
    use crate::pairing::testing::verifies_ed25519;
    use crate::ports::DataKey;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, PoisonError};

    const NAME: &str = "signing";
    const PUBLIC_KEY: [u8; 32] = [7; 32];

    type Answer = Result<[u8; 32], KeyServiceError>;

    /// Answers `verifying_key` for [`NAME`] with the next scripted answer and
    /// counts being asked. Everything else is an error: another name, another
    /// method, and a read past the end of the script.
    struct Scripted {
        answers: Mutex<VecDeque<Answer>>,
        asked: AtomicUsize,
    }

    impl Scripted {
        fn new(answers: impl IntoIterator<Item = Answer>) -> Self {
            Self {
                answers: Mutex::new(answers.into_iter().collect()),
                asked: AtomicUsize::new(0),
            }
        }

        fn asked(&self) -> usize {
            self.asked.load(Ordering::Relaxed)
        }
    }

    fn unscripted(call: &str) -> KeyServiceError {
        KeyServiceError::Malformed(format!("unscripted call: {call}"))
    }

    #[async_trait]
    impl KeyService for Scripted {
        async fn encrypt(&self, _: &str, _: &[u8]) -> Result<String, KeyServiceError> {
            Err(unscripted("encrypt"))
        }

        async fn decrypt(&self, _: &str, _: &str) -> Result<Vec<u8>, KeyServiceError> {
            Err(unscripted("decrypt"))
        }

        async fn data_key(&self, _: &str) -> Result<DataKey, KeyServiceError> {
            Err(unscripted("data_key"))
        }

        async fn verifying_key(&self, key: &str) -> Answer {
            self.asked.fetch_add(1, Ordering::Relaxed);
            if key != NAME {
                return Err(KeyServiceError::UnknownKey(key.to_owned()));
            }
            self.answers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or_else(|| Err(unscripted("verifying_key")))
        }

        async fn sign(&self, _: &str, _: &[u8]) -> Result<[u8; 64], KeyServiceError> {
            Err(unscripted("sign"))
        }
    }

    #[tokio::test]
    async fn a_second_read_after_an_answer_does_not_ask_the_key_service() {
        let keys = Scripted::new([Ok(PUBLIC_KEY)]);
        let signing_key = SigningKey::new(NAME);

        let first = signing_key.public_key(&keys).await;
        let second = signing_key.public_key(&keys).await;

        assert!(matches!(first, Ok(PUBLIC_KEY)), "{first:?}");
        assert!(matches!(second, Ok(PUBLIC_KEY)), "{second:?}");
        assert_eq!(keys.asked(), 1);
    }

    #[tokio::test]
    async fn a_refused_read_is_asked_again_and_the_answer_that_follows_is_kept() {
        let keys = Scripted::new([
            Err(KeyServiceError::Sealed),
            Err(KeyServiceError::Transport("refused".to_owned())),
            Ok(PUBLIC_KEY),
        ]);
        let signing_key = SigningKey::new(NAME);

        let sealed = signing_key.public_key(&keys).await;
        let unreachable = signing_key.public_key(&keys).await;
        let answered = signing_key.public_key(&keys).await;
        let kept = signing_key.public_key(&keys).await;

        assert!(matches!(sealed, Err(KeyServiceError::Sealed)), "{sealed:?}");
        assert!(
            matches!(unreachable, Err(KeyServiceError::Transport(_))),
            "{unreachable:?}"
        );
        assert!(matches!(answered, Ok(PUBLIC_KEY)), "{answered:?}");
        assert!(matches!(kept, Ok(PUBLIC_KEY)), "{kept:?}");
        assert_eq!(keys.asked(), 3);
    }

    #[tokio::test]
    async fn the_key_is_asked_for_by_the_name_it_was_built_with() {
        let keys = Scripted::new([Ok(PUBLIC_KEY)]);

        let read = SigningKey::new("another").public_key(&keys).await;

        assert!(
            matches!(&read, Err(KeyServiceError::UnknownKey(name)) if name == "another"),
            "{read:?}"
        );
    }

    #[tokio::test]
    async fn a_signature_is_made_under_the_name_the_key_was_built_with() {
        let keys = MemoryKeys::with_keys([NAME, "another"]);
        let signing_key = SigningKey::new(NAME);
        let message = b"a request";

        let signature = signing_key.sign(&keys, message).await.unwrap();
        let public_key = signing_key.public_key(&keys).await.unwrap();
        let of_another = SigningKey::new("another").public_key(&keys).await.unwrap();
        let unknown = SigningKey::new("absent").sign(&keys, message).await;

        assert!(verifies_ed25519(&public_key, message, &signature));
        assert!(!verifies_ed25519(&of_another, message, &signature));
        assert!(
            matches!(&unknown, Err(KeyServiceError::UnknownKey(name)) if name == "absent"),
            "{unknown:?}"
        );
    }
}

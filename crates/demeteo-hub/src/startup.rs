use crate::ports::KeyServiceError;

/// What the startup probe of the key service means for the process. Nothing
/// here ends it: unseal is a manual operator step that can follow the Hub's
/// own start, and a crash-looping Hub would hide that step behind restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    Ready,
    Sealed,
    Unavailable,
}

pub fn classify_key_probe<T>(probe: &Result<T, KeyServiceError>) -> KeyStatus {
    match probe {
        Ok(_) => KeyStatus::Ready,
        Err(KeyServiceError::Sealed) => KeyStatus::Sealed,
        Err(_) => KeyStatus::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_seal_is_reported_as_sealed() {
        assert_eq!(classify_key_probe(&Ok(())), KeyStatus::Ready);
        assert_eq!(
            classify_key_probe::<()>(&Err(KeyServiceError::Sealed)),
            KeyStatus::Sealed
        );
        for err in [
            KeyServiceError::UnknownKey("k".into()),
            KeyServiceError::InvalidKeyName("a/b".into()),
            KeyServiceError::Transport("down".into()),
            KeyServiceError::Upstream {
                status: 403,
                detail: String::new(),
            },
        ] {
            assert_eq!(classify_key_probe::<()>(&Err(err)), KeyStatus::Unavailable);
        }
    }
}

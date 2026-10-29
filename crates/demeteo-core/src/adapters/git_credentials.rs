use crate::ports::git_credentials::GitCredentialSource;

/// The desktop's credential source: the provider PAT in the OS keyring, read
/// through the in-process credential cache.
///
/// The repository a lookup runs in does not change the answer, because the
/// keyring is keyed by provider alone.
#[derive(Clone, Copy, Default)]
pub struct KeyringCredentialSource;

impl GitCredentialSource for KeyringCredentialSource {
    fn pat_for(&self, provider_id: &str, _repo_dir: &str) -> Result<String, String> {
        crate::adapters::mr_publisher::resolve_pat_for(provider_id)
    }
}

#[cfg(test)]
#[path = "../../tests/infrastructure/git_credentials.rs"]
mod tests;

/// Where the personal access token for a git remote comes from.
///
/// A port because who holds the token differs by process, not by transport: the
/// desktop reads the OS keyring, while a runner holds each run's token in
/// memory and nowhere else. Callers ask by provider and by the repository the
/// git invocation runs in, and a source answers with whichever it can resolve.
///
/// `Err` means "no credential", never a failure to surface: the fetch or push
/// then runs unauthenticated and git says why in its own words.
pub trait GitCredentialSource: Send + Sync {
    fn pat_for(&self, provider_id: &str, repo_dir: &str) -> Result<String, String>;
}

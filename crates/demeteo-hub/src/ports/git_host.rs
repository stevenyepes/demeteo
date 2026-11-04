use super::KeyServiceError;
use async_trait::async_trait;
use std::fmt;

/// A GitHub repository as a minter addresses it: the host its Project clones
/// from and the two halves of `owner/repo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubRepository {
    pub host: String,
    pub owner: String,
    pub name: String,
}

/// A minted installation token. It is handed to one runner and dropped: never
/// stored, logged or reused (decision 71), so `Debug` omits it and the text
/// is reachable only through [`secret`](Self::secret).
pub struct InstallationToken {
    token: String,
    /// Unix seconds, GitHub's own expiry.
    pub expires_at: u64,
}

impl InstallationToken {
    pub fn new(token: String, expires_at: u64) -> Self {
        Self { token, expires_at }
    }

    pub fn secret(&self) -> &str {
        &self.token
    }
}

impl fmt::Debug for InstallationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstallationToken")
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Why no token was minted. No variant holds a response body: GitHub's error
/// text is not the Hub's to repeat, and a status is enough to tell an
/// operator which side to look at.
#[derive(Debug, thiserror::Error)]
pub enum MintError {
    /// The repository's host is neither github.com nor the configured GitHub
    /// Enterprise Server, so there is no API to ask. Nothing a retry changes.
    #[error("no GitHub API is configured for host `{0}`")]
    UnknownHost(String),
    #[error("the App JWT could not be signed: {0}")]
    Signing(KeyServiceError),
    #[error("GitHub could not be reached: {0}")]
    Transport(String),
    /// The installation lookup answered 404: the App is not installed on the
    /// repository, or the App id names another App.
    #[error("the GitHub App is not installed on the repository")]
    NotInstalled,
    #[error("GitHub answered HTTP {status} to the {step}")]
    Refused { step: &'static str, status: u16 },
    #[error("GitHub answered the {0} with a body this build cannot read")]
    Malformed(&'static str),
}

/// Mints a short-lived token reaching one repository with `contents: write`
/// and `pull_requests: write`, and nothing else.
#[async_trait]
pub trait GitTokenMinter: Send + Sync {
    /// `now` is Unix seconds, the clock the App JWT is issued by.
    async fn mint(
        &self,
        repository: &GithubRepository,
        now: u64,
    ) -> Result<InstallationToken, MintError>;
}

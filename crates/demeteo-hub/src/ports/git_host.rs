use super::KeyServiceError;
use crate::store::UserId;
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

/// A repository one of a tenant's Apps is installed on, as the project
/// picker offers it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ReachableRepository {
    /// Lowercase: the spelling a Hub project and a start take.
    pub host: String,
    /// `owner/repo`.
    pub full_name: String,
    pub private: bool,
    pub default_branch: Option<String>,
    pub description: Option<String>,
}

/// An App whose repositories could not be listed. Its id, not GitHub's
/// error text, which is not the Hub's to repeat.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UnlistedApp {
    pub host: String,
    pub app_id: u64,
}

/// Every repository a tenant's Apps reach, beside the Apps that did not
/// answer. An App failing never hides another's repositories.
#[derive(Debug, Default)]
pub struct RepositoryListing {
    pub repositories: Vec<ReachableRepository>,
    pub unlisted: Vec<UnlistedApp>,
    /// A listing stopped at its page cap: more repositories exist.
    pub truncated: bool,
}

/// Lists what [`GitTokenMinter`] could mint for: the same Apps, judged the
/// same way, so the picker never offers a repository a run cannot reach.
#[async_trait]
pub trait RepositoryLister: Send + Sync {
    /// Err only when the tenant's own record of Apps cannot be read.
    async fn repositories(&self, user: UserId, now: u64) -> Result<RepositoryListing, MintError>;
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
    /// No App is connected or configured for the repository's host. A human
    /// can connect one while the run waits, so this is retried, not final.
    #[error("no GitHub App is connected for host `{0}`")]
    UnknownHost(String),
    /// More than one of the tenant's Apps is installed on the repository.
    /// Neither is chosen: which bot opens the pull request is the user's
    /// call, and a guess would differ from one fetch to the next.
    #[error("{0} connected GitHub Apps are installed on the repository")]
    Ambiguous(usize),
    /// The Hub's own record of connected Apps could not be read.
    #[error("the connected git hosts could not be read: {0}")]
    Hosts(String),
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
    /// `user` is the tenant whose start asked: only its connected Apps, and
    /// the one configured for the whole Hub, may mint. `now` is Unix
    /// seconds, the clock the App JWT is issued by.
    async fn mint(
        &self,
        user: UserId,
        repository: &GithubRepository,
        now: u64,
    ) -> Result<InstallationToken, MintError>;
}

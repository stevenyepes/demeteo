//! OAuth 2.1 grant validation — the pure half of MCP request authorization.
//!
//! See [`crate::domain`]: this module has no `async fn` and takes no port.
//! Every request-time check (the HTTP guard, the adapter integration tests)
//! funnels through [`validate_grant`], which is why its check order —
//! revoked, then expired, then audience, then scope — is the check order for
//! the whole feature, not just this function.
//!
//! [`GrantRecord`] omits `token_hash`: validation is never given a hash to
//! compare, only a caller-supplied token already hashed and looked up by at
//! the repository boundary. [`Scope`] is a flat set — [`Scope::Spend`] does
//! not imply [`Scope::Read`] — per AGENTS.md's "compiled `PermissionProfile`
//! is complete, never `ask`" invariant one layer up.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::ids::{ClientId, GrantId};

pub mod pkce;
pub mod registration;
pub mod tools;

/// A grant's authorized operations. Membership check only — no hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Read,
    Spend,
    Configure,
}

impl Scope {
    /// Every scope this server can grant, in the order metadata advertises them.
    pub const ALL: [Scope; 3] = [Scope::Read, Scope::Spend, Scope::Configure];

    /// The stable lowercase identifier used in the `oauth_grants.scopes`
    /// column and in RFC 8707 `scope` parameters — mirrors the serde
    /// `rename_all = "snake_case"` spelling above rather than defining a
    /// second one.
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Spend => "spend",
            Scope::Configure => "configure",
        }
    }

    /// Parse a stored/wire scope string. Returns `None` for unknown values
    /// so a stale row or a request from a newer client is rejected as an
    /// unrecognized scope rather than panicking — mirrors `EffortLevel::parse`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "read" => Some(Scope::Read),
            "spend" => Some(Scope::Spend),
            "configure" => Some(Scope::Configure),
            _ => None,
        }
    }
}

/// A validated `oauth_grants` row, minus the hash validation never needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRecord {
    pub id: GrantId,
    pub client_id: ClientId,
    pub scopes: Vec<Scope>,
    pub resource: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
}

/// A registered `oauth_clients` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthClient {
    pub id: ClientId,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub created_at: i64,
}

/// Everything that can fail while authorizing an MCP flow or request.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OAuthError {
    #[error("code_challenge is required")]
    MissingPkce,
    #[error("code_challenge_method must be S256")]
    UnsupportedPkceMethod,
    #[error("resource parameter is required")]
    MissingResource,
    #[error("resource does not match {expected}")]
    ResourceMismatch { expected: String },
    #[error("unknown scope: {0}")]
    UnknownScope(String),
    #[error("token has expired")]
    TokenExpired,
    #[error("token has been revoked")]
    TokenRevoked,
    #[error("token is not bound to this resource")]
    AudienceMismatch,
    #[error("insufficient scope: {required:?} required")]
    InsufficientScope { required: Scope },
    #[error("invalid token")]
    InvalidToken,
}

/// Every request-time check funnels through here — see the module doc for
/// why the check order is fixed.
pub fn validate_grant(
    grant: &GrantRecord,
    now: i64,
    expected_resource: &str,
    required: Scope,
) -> Result<(), OAuthError> {
    if grant.revoked_at.is_some() {
        return Err(OAuthError::TokenRevoked);
    }
    if grant.expires_at <= now {
        return Err(OAuthError::TokenExpired);
    }
    if grant.resource != expected_resource {
        return Err(OAuthError::AudienceMismatch);
    }
    if !grant.scopes.contains(&required) {
        return Err(OAuthError::InsufficientScope { required });
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/domain/oauth/validate_grant.rs"]
mod validate_grant;

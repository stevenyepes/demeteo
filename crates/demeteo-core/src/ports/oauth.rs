//! Persistence for the MCP OAuth 2.1 flow: registered clients and issued
//! grants.
//!
//! Split into two traits along the same line the domain does — clients are
//! long-lived registrations, grants are per-authorization records — so a
//! caller that only needs one doesn't depend on the other's schema.

use crate::domain::ids::{ClientId, GrantId};
use crate::domain::oauth::{GrantRecord, OAuthClient};
use crate::error::AppError;

/// Persistence for registered `oauth_clients` rows.
pub trait OAuthClientRepository: Send + Sync {
    fn register_client(&self, client: OAuthClient) -> Result<(), AppError>;
    fn get_client(&self, id: &ClientId) -> Result<Option<OAuthClient>, AppError>;
}

/// Persistence for issued `oauth_grants` rows.
///
/// Grants are looked up by `token_hash`, never by the raw token: the token
/// itself is never stored, so [`GrantRecord`] (see `domain::oauth`) omits it.
pub trait OAuthGrantRepository: Send + Sync {
    fn insert_grant(&self, grant: GrantRecord, token_hash: &str) -> Result<(), AppError>;
    fn find_grant_by_token_hash(&self, token_hash: &str) -> Result<Option<GrantRecord>, AppError>;
    /// Every non-revoked, non-expired grant, paired with its client — the
    /// shape the Settings grants list and the `list_mcp_grants` Tauri command
    /// need to render `client_name` without a second round trip per row.
    fn list_active_grants(&self) -> Result<Vec<(OAuthClient, GrantRecord)>, AppError>;
    fn revoke_grant(&self, id: &GrantId) -> Result<(), AppError>;
}

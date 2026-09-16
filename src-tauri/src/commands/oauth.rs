use crate::adapters::mcp::consent_waiter::ConsentDecision;
use crate::domain::ids::GrantId;
use crate::domain::oauth::{GrantRecord, OAuthClient, Scope};
use crate::error::AppError;
use crate::state::AppContext;
use serde::Serialize;
use tauri::State;

/// The Settings-visible shape of an active `oauth_grants` row. `revoked` is
/// `revoked_at.is_some()`, but in practice always `false` here:
/// `OAuthGrantRepository::list_active_grants` already excludes revoked (and
/// expired) rows at the query level, so a revoked grant simply drops out of
/// [`list_mcp_grants`] rather than appearing with the flag set.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct McpGrantSummary {
    pub id: String,
    pub client_name: String,
    pub scopes: Vec<Scope>,
    pub issued_at: i64,
    pub expires_at: i64,
    pub revoked: bool,
}

fn grant_summary((client, grant): (OAuthClient, GrantRecord)) -> McpGrantSummary {
    McpGrantSummary {
        id: grant.id.as_str().to_string(),
        client_name: client.client_name,
        scopes: grant.scopes,
        issued_at: grant.issued_at,
        expires_at: grant.expires_at,
        revoked: grant.revoked_at.is_some(),
    }
}

#[tauri::command]
pub fn list_mcp_grants(ctx: State<'_, AppContext>) -> Result<Vec<McpGrantSummary>, AppError> {
    Ok(ctx
        .oauth_grants
        .list_active_grants()?
        .into_iter()
        .map(grant_summary)
        .collect())
}

#[tauri::command]
pub fn revoke_mcp_grant(ctx: State<'_, AppContext>, grant_id: String) -> Result<(), AppError> {
    ctx.oauth_grants.revoke_grant(&GrantId::new(grant_id))
}

#[tauri::command]
pub fn mcp_consent_decide(
    ctx: State<'_, AppContext>,
    request_id: String,
    approve: bool,
) -> Result<(), AppError> {
    let decision = if approve {
        ConsentDecision::Approved
    } else {
        ConsentDecision::Denied
    };
    ctx.mcp_consent.deliver(&request_id, decision);
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/infrastructure/oauth.rs"]
mod tests;

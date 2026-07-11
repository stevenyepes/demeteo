use crate::domain::ids::ProviderId;
use crate::domain::models::ProviderInstance;
#[cfg(feature = "keyring")]
use crate::paths;
use crate::ports::provider_http::{CreateRepoRequest, CreatedRepo, NamespaceSummary};
use crate::state::AppContext;
#[cfg(feature = "keyring")]
use keyring::Entry;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct ProviderValidationResult {
    pub valid: bool,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub error: Option<String>,
}

pub fn sanitize_host(host: &str) -> String {
    let mut h = host.trim();
    if let Some(pos) = h.find("://") {
        h = &h[pos + 3..];
    }
    if let Some(pos) = h.find('/') {
        h = &h[..pos];
    }
    h.to_string()
}

pub async fn validate_pat(
    ctx: &AppContext,
    provider_type: String,
    host: String,
    pat: String,
) -> Result<ProviderValidationResult, String> {
    match ctx
        .provider_http
        .validate_pat(&host, &provider_type, &pat)
        .await
    {
        Ok(info) => Ok(ProviderValidationResult {
            valid: true,
            username: Some(info.username),
            avatar_url: Some(info.avatar_url),
            error: None,
        }),
        Err(e) => Ok(ProviderValidationResult {
            valid: false,
            username: None,
            avatar_url: None,
            error: Some(e.to_string()),
        }),
    }
}

pub async fn fetch_repos(ctx: &AppContext, provider_id: String) -> Result<Vec<String>, String> {
    let (provider, pat) = resolve_provider_and_pat(ctx, &provider_id)?;

    let repos = ctx
        .provider_http
        .list_repos(&provider.host, &provider.kind, &pat)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|r| r.full_name)
        .collect();

    Ok(repos)
}

/// Looks up a connected provider instance and resolves its PAT via the
/// keyring-backed credential cache (never crosses the IPC boundary).
///
/// This is the **single** place in the backend that opens the
/// `'demeteo'` keyring for a provider id; every other site that needs
/// the PAT (`fetch_repos`, `create_repo`, `list_groups`, and the
/// wizard's `Commit` arm in `commands::create_project`) routes through
/// this helper.
pub fn resolve_provider_and_pat(
    ctx: &AppContext,
    provider_id: &str,
) -> Result<(ProviderInstance, String), String> {
    let providers = ctx.app_settings.get_provider_instances()?;
    let provider_id_typed = ProviderId::from(provider_id.to_string());
    let provider = providers
        .into_iter()
        .find(|p| p.id == provider_id_typed)
        .ok_or_else(|| "Provider not found".to_string())?;

    let pat = crate::credential_cache::get_or_fetch(provider.id.as_str(), || {
        #[cfg(feature = "keyring")]
        {
            let entry = Entry::new("demeteo", provider.id.as_str()).map_err(|e| e.to_string())?;
            entry.get_password().map_err(|e| {
                tracing::warn!("Keyring error for id '{}': {}", provider.id, e);
                e.to_string()
            })
        }
        #[cfg(not(feature = "keyring"))]
        {
            Err("OS-keyring credential cache is disabled in this build".to_string())
        }
    })?;

    Ok((provider, pat))
}

/// Lists the namespaces (personal account + orgs/groups) a repo can be
/// created under for the given provider.
pub async fn list_groups(
    ctx: &AppContext,
    provider_id: String,
) -> Result<Vec<NamespaceSummary>, String> {
    let (provider, pat) = resolve_provider_and_pat(ctx, &provider_id)?;

    ctx.provider_http
        .list_namespaces(&provider.host, &provider.kind, &pat)
        .await
        .map_err(|e| e.to_string())
}

/// Creates a new repository on the given provider under `namespace_id`.
/// `auto_init` is forced on so the repo has a default branch + initial commit
/// before the wizard clones it.
pub async fn create_repo(
    ctx: &AppContext,
    provider_id: String,
    namespace_id: String,
    name: String,
    private: bool,
) -> Result<CreatedRepo, String> {
    let (provider, pat) = resolve_provider_and_pat(ctx, &provider_id)?;

    // Resolve the requested namespace id back to a full NamespaceSummary so
    // the adapter knows whether to route to a personal / org / group endpoint.
    let namespaces = ctx
        .provider_http
        .list_namespaces(&provider.host, &provider.kind, &pat)
        .await
        .map_err(|e| e.to_string())?;
    let namespace = namespaces
        .into_iter()
        .find(|n| n.id == namespace_id)
        .ok_or_else(|| "Namespace not found".to_string())?;

    let req = CreateRepoRequest {
        namespace,
        name,
        private,
        auto_init: true,
    };

    ctx.provider_http
        .create_repo(&provider.host, &provider.kind, &pat, &req)
        .await
        .map_err(|e| e.to_string())
}

pub async fn connect_instance(
    ctx: &AppContext,
    provider_type: String,
    host: String,
    pat: String,
) -> Result<ProviderInstance, String> {
    #[cfg(not(feature = "keyring"))]
    {
        let _ = (ctx, &provider_type, &host, &pat);
        Err("OS-keyring credential cache is disabled in this build".to_string())
    }

    #[cfg(feature = "keyring")]
    {
        let res = ctx
            .provider_http
            .validate_pat(&host, &provider_type, &pat)
            .await
            .map_err(|e| e.to_string())?;

        let kind = provider_type.to_lowercase();
        let sanitized_host = sanitize_host(&host);
        let h = if sanitized_host.is_empty() {
            if kind == "github" {
                "github.com".to_string()
            } else {
                "gitlab.com".to_string()
            }
        } else {
            sanitized_host
        };

        let id = ProviderId::from(format!("{}_{}", kind, h.replace('.', "_")));

        let entry = Entry::new("demeteo", id.as_str()).map_err(|e| e.to_string())?;
        // Delete before set: macOS SecItemAdd fails if the item already exists.
        // A stale entry (e.g. from a failed prior delete) would block re-connection.
        let _ = entry.delete_credential();
        entry.set_password(&pat).map_err(|e| e.to_string())?;
        crate::credential_cache::set(id.as_str(), &pat);

        let now = paths::now_ms();
        let instance = ProviderInstance {
            id: id.clone(),
            kind,
            host: h,
            username: res.username,
            avatar_url: res.avatar_url,
            created_at: now,
        };

        ctx.app_settings.add_provider_instance(instance.clone())?;
        Ok(instance)
    }
}

#[cfg(test)]
#[path = "../../tests/application/providers.rs"]
mod tests;

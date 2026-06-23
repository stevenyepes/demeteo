use crate::domain::ids::ProviderId;
use crate::domain::models::ProviderInstance;
use crate::paths;
use crate::state::AppContext;
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
    let providers = ctx.app_settings.get_provider_instances()?;
    let provider_id_typed = ProviderId::from(provider_id.clone());
    let provider = providers
        .into_iter()
        .find(|p| p.id == provider_id_typed)
        .ok_or_else(|| "Provider not found".to_string())?;

    let pat = crate::credential_cache::get_or_fetch(provider.id.as_str(), || {
        let entry = Entry::new("demeteo", provider.id.as_str()).map_err(|e| e.to_string())?;
        entry.get_password().map_err(|e| {
            tracing::warn!("Keyring error for id '{}': {}", provider.id, e);
            e.to_string()
        })
    })?;

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

pub async fn connect_instance(
    ctx: &AppContext,
    provider_type: String,
    host: String,
    pat: String,
) -> Result<ProviderInstance, String> {
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
    entry.set_password(&pat).map_err(|e| e.to_string())?;
    crate::credential_cache::invalidate(id.as_str());

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

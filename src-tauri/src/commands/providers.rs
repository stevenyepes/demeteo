use serde::{Deserialize, Serialize};
use tauri::State;
use crate::state::AppContext;
use crate::domain::ids::ProviderId;
use crate::domain::models::ProviderInstance;
use crate::paths;
use keyring::Entry;

#[derive(Serialize, Deserialize, Debug)]
pub struct ProviderValidationResult {
    pub valid: bool,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub error: Option<String>,
}

fn sanitize_host(host: &str) -> String {
    let mut h = host.trim();
    if let Some(pos) = h.find("://") {
        h = &h[pos + 3..];
    }
    if let Some(pos) = h.find('/') {
        h = &h[..pos];
    }
    h.to_string()
}

#[tauri::command]
pub async fn validate_provider_pat(
    provider_type: String,
    host: String,
    pat: String,
) -> Result<ProviderValidationResult, String> {
    let client = reqwest::Client::builder()
        .user_agent("demeteo-orchestrator")
        .build()
        .map_err(|e| e.to_string())?;

    let host = sanitize_host(&host);

    let url = if provider_type.to_lowercase() == "github" {
        let h = if host.is_empty() { "api.github.com" } else { &host };
        if h == "api.github.com" {
            format!("https://{}/user", h)
        } else {
            format!("https://{}/api/v3/user", h)
        }
    } else if provider_type.to_lowercase() == "gitlab" {
        let h = if host.is_empty() { "gitlab.com" } else { &host };
        format!("https://{}/api/v4/user", h)
    } else {
        return Err("Unsupported provider type".to_string());
    };

    let res = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", pat))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if res.status().is_success() {
        let data: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
        let username = data["login"].as_str().or_else(|| data["username"].as_str()).unwrap_or("").to_string();
        let avatar_url = data["avatar_url"].as_str().unwrap_or("").to_string();

        Ok(ProviderValidationResult {
            valid: true,
            username: Some(username),
            avatar_url: Some(avatar_url),
            error: None,
        })
    } else {
        Ok(ProviderValidationResult {
            valid: false,
            username: None,
            avatar_url: None,
            error: Some(format!("HTTP {}", res.status())),
        })
    }
}

#[tauri::command]
pub async fn fetch_provider_repos(
    ctx: State<'_, AppContext>,
    provider_id: String,
) -> Result<Vec<String>, String> {
    let providers = ctx.app_settings.get_provider_instances()?;
    let provider_id_typed = ProviderId::from(provider_id.clone());
    let provider = providers.into_iter().find(|p| p.id == provider_id_typed)
        .ok_or_else(|| "Provider not found".to_string())?;

    let pat = crate::credential_cache::get_or_fetch(provider.id.as_str(), || {
        let entry = Entry::new("demeteo", provider.id.as_str()).map_err(|e| e.to_string())?;
        entry.get_password().map_err(|e| {
            let _ = std::fs::write("/tmp/demeteo_fetch.log", format!("Keyring error for id '{}': {}\n", provider.id, e));
            e.to_string()
        })
    })?;

    let provider_type = provider.kind;
    let host = provider.host;
    let client = reqwest::Client::builder()
        .user_agent("demeteo-orchestrator")
        .build()
        .map_err(|e| e.to_string())?;

    let host = sanitize_host(&host);
    let url = if provider_type.to_lowercase() == "github" {
        let h = if host.is_empty() { "api.github.com" } else { &host };
        if h == "api.github.com" {
            format!("https://{}/user/repos?per_page=100", h)
        } else {
            format!("https://{}/api/v3/user/repos?per_page=100", h)
        }
    } else if provider_type.to_lowercase() == "gitlab" {
        let h = if host.is_empty() { "gitlab.com" } else { &host };
        format!("https://{}/api/v4/projects?membership=true&per_page=100", h)
    } else {
        return Err("Unsupported provider type".to_string());
    };

    let res = match client
        .get(&url)
        .header("Authorization", format!("Bearer {}", pat))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::write("/tmp/demeteo_fetch.log", format!("reqwest send error: {}\n", e));
            return Err(e.to_string());
        }
    };

    if res.status().is_success() {
        let text = res.text().await.map_err(|e| {
            let _ = std::fs::write("/tmp/demeteo_fetch.log", format!("text error: {}\n", e));
            e.to_string()
        })?;
        let data: Vec<serde_json::Value> = serde_json::from_str(&text).map_err(|e| {
            let _ = std::fs::write("/tmp/demeteo_fetch.log", format!("json parse error: {}, text: {}\n", e, text));
            e.to_string()
        })?;
        let mut repos = Vec::new();
        for item in data {
            if provider_type.to_lowercase() == "github" {
                if let Some(full_name) = item["full_name"].as_str() {
                    repos.push(full_name.to_string());
                }
            } else if provider_type.to_lowercase() == "gitlab" {
                if let Some(path) = item["path_with_namespace"].as_str() {
                    repos.push(path.to_string());
                }
            }
        }
        let _ = std::fs::write("/tmp/demeteo_fetch.log", format!("Returning {} repos\n", repos.len()));
        Ok(repos)
    } else {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        let _ = std::fs::write("/tmp/demeteo_fetch.log", format!("HTTP {} error: {}\n", status, body));
        Err(format!("HTTP {} - {}", status, body))
    }
}

#[tauri::command]
pub async fn connect_provider_instance(
    ctx: State<'_, AppContext>,
    provider_type: String,
    host: String,
    pat: String,
) -> Result<ProviderInstance, String> {
    let validation = validate_provider_pat(provider_type.clone(), host.clone(), pat.clone()).await?;
    
    if !validation.valid {
        return Err(validation.error.unwrap_or_else(|| "Invalid PAT".to_string()));
    }

    let kind = provider_type.to_lowercase();
    let sanitized_host = sanitize_host(&host);
    let h = if sanitized_host.is_empty() {
        if kind == "github" { "github.com".to_string() } else { "gitlab.com".to_string() }
    } else {
        sanitized_host
    };

    let id = ProviderId::from(format!("{}_{}", kind, h.replace('.', "_")));

    // Store PAT in keyring
    let entry = Entry::new("demeteo", id.as_str()).map_err(|e| e.to_string())?;
    entry.set_password(&pat).map_err(|e| e.to_string())?;
    crate::credential_cache::invalidate(id.as_str());

    let now = paths::now_ms();

    let instance = ProviderInstance {
        id: id.clone(),
        kind,
        host: h,
        username: validation.username.unwrap_or_default(),
        avatar_url: validation.avatar_url.unwrap_or_default(),
        created_at: now,
    };

    ctx.app_settings.add_provider_instance(instance.clone())?;

    Ok(instance)
}

#[tauri::command]
pub fn list_provider_instances(
    ctx: State<'_, AppContext>,
) -> Result<Vec<ProviderInstance>, String> {
    ctx.app_settings.get_provider_instances()
}

#[tauri::command]
pub async fn delete_provider_instance(
    ctx: State<'_, AppContext>,
    provider_id: String,
) -> Result<(), String> {
    let provider_id_typed = ProviderId::from(provider_id.clone());
    if let Ok(entry) = Entry::new("demeteo", &provider_id) {
        let _ = entry.delete_credential();
    }
    crate::credential_cache::invalidate(&provider_id);
    ctx.app_settings.delete_provider_instance(&provider_id_typed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_host() {
        assert_eq!(
            sanitize_host("https://gitlab.stvcloud.dev/prototype/spectacular.git"),
            "gitlab.stvcloud.dev"
        );
        assert_eq!(
            sanitize_host("http://gitlab.company.com:8080/path"),
            "gitlab.company.com:8080"
        );
        assert_eq!(
            sanitize_host("gitlab.company.com"),
            "gitlab.company.com"
        );
        assert_eq!(
            sanitize_host("   https://api.github.com   "),
            "api.github.com"
        );
    }
}

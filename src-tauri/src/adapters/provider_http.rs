use crate::error::AppError;
use crate::ports::provider_http::{ProviderHttpPort, ProviderUserInfo, RepoSummary};
use async_trait::async_trait;

pub struct ReqwestProviderHttpAdapter {
    client: reqwest::Client,
}

impl Default for ReqwestProviderHttpAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestProviderHttpAdapter {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent("demeteo-orchestrator")
            .build()
            .unwrap_or_default();
        Self { client }
    }
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

#[async_trait]
impl ProviderHttpPort for ReqwestProviderHttpAdapter {
    async fn validate_pat(
        &self,
        host: &str,
        kind: &str,
        pat: &str,
    ) -> Result<ProviderUserInfo, AppError> {
        let host = sanitize_host(host);
        let url = if kind.to_lowercase() == "github" {
            let h = if host.is_empty() {
                "api.github.com"
            } else {
                &host
            };
            if h == "api.github.com" {
                format!("https://{}/user", h)
            } else {
                format!("https://{}/api/v3/user", h)
            }
        } else if kind.to_lowercase() == "gitlab" {
            let h = if host.is_empty() { "gitlab.com" } else { &host };
            format!("https://{}/api/v4/user", h)
        } else {
            return Err(AppError::validation("Unsupported provider type"));
        };

        let res = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", pat))
            .send()
            .await
            .map_err(|e| AppError::Transport {
                message: e.to_string(),
            })?;

        if res.status().is_success() {
            let data: serde_json::Value = res.json().await.map_err(|e| AppError::Transport {
                message: e.to_string(),
            })?;
            let username = data["login"]
                .as_str()
                .or_else(|| data["username"].as_str())
                .unwrap_or("")
                .to_string();
            let avatar_url = data["avatar_url"].as_str().unwrap_or("").to_string();

            Ok(ProviderUserInfo {
                username,
                avatar_url,
            })
        } else {
            Err(AppError::Provider {
                message: format!("HTTP {}", res.status()),
            })
        }
    }

    async fn list_repos(
        &self,
        host: &str,
        kind: &str,
        pat: &str,
    ) -> Result<Vec<RepoSummary>, AppError> {
        let host = sanitize_host(host);
        let url = if kind.to_lowercase() == "github" {
            let h = if host.is_empty() {
                "api.github.com"
            } else {
                &host
            };
            if h == "api.github.com" {
                format!("https://{}/user/repos?per_page=100", h)
            } else {
                format!("https://{}/api/v3/user/repos?per_page=100", h)
            }
        } else if kind.to_lowercase() == "gitlab" {
            let h = if host.is_empty() { "gitlab.com" } else { &host };
            format!("https://{}/api/v4/projects?membership=true&per_page=100", h)
        } else {
            return Err(AppError::validation("Unsupported provider type"));
        };

        let res = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", pat))
            .send()
            .await
            .map_err(|e| AppError::Transport {
                message: e.to_string(),
            })?;

        if res.status().is_success() {
            let text = res.text().await.map_err(|e| AppError::Transport {
                message: e.to_string(),
            })?;
            let data: Vec<serde_json::Value> =
                serde_json::from_str(&text).map_err(|e| AppError::Transport {
                    message: e.to_string(),
                })?;
            let mut repos = Vec::new();
            for item in data {
                if kind.to_lowercase() == "github" {
                    if let Some(full_name) = item["full_name"].as_str() {
                        repos.push(RepoSummary {
                            full_name: full_name.to_string(),
                        });
                    }
                } else if kind.to_lowercase() == "gitlab" {
                    if let Some(path) = item["path_with_namespace"].as_str() {
                        repos.push(RepoSummary {
                            full_name: path.to_string(),
                        });
                    }
                }
            }
            Ok(repos)
        } else {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            Err(AppError::Provider {
                message: format!("HTTP {} - {}", status, body),
            })
        }
    }
}

use crate::error::AppError;
use crate::ports::provider_http::{
    CreateRepoRequest, CreatedRepo, NamespaceSummary, ProviderHttpPort, ProviderUserInfo,
    RepoSummary,
};
use async_trait::async_trait;
use serde_json::json;

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

    /// GETs a URL with the PAT and decodes a single JSON object, mapping
    /// transport failures to `Transport` and non-2xx responses to `Provider`.
    async fn get_json(&self, url: &str, pat: &str) -> Result<serde_json::Value, AppError> {
        let res = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {}", pat))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| AppError::Transport {
                message: e.to_string(),
            })?;

        if res.status().is_success() {
            res.json().await.map_err(|e| AppError::Transport {
                message: e.to_string(),
            })
        } else {
            let status = res.status().as_u16();
            let body = res.text().await.unwrap_or_default();
            Err(provider_http_error(status, &body))
        }
    }

    /// GETs a URL with the PAT and decodes a JSON array.
    async fn get_json_array(
        &self,
        url: &str,
        pat: &str,
    ) -> Result<Vec<serde_json::Value>, AppError> {
        let res = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {}", pat))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| AppError::Transport {
                message: e.to_string(),
            })?;

        if res.status().is_success() {
            res.json().await.map_err(|e| AppError::Transport {
                message: e.to_string(),
            })
        } else {
            let status = res.status().as_u16();
            let body = res.text().await.unwrap_or_default();
            Err(provider_http_error(status, &body))
        }
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

/// Resolves the API base URL (scheme + host + version prefix) for a
/// provider host/kind, reusing the same GitHub `api.github.com` / `/api/v3`
/// (enterprise) and GitLab `/api/v4` branching as the other endpoints. Never
/// hardcodes a default domain beyond the documented provider fallbacks.
fn api_base(host: &str, kind: &str) -> Result<String, AppError> {
    let host = sanitize_host(host);
    match kind.to_lowercase().as_str() {
        "github" => {
            let h = if host.is_empty() || host == "github.com" {
                "api.github.com".to_string()
            } else {
                host
            };
            if h == "api.github.com" {
                Ok(format!("https://{}", h))
            } else {
                // GitHub Enterprise Server exposes the REST API under /api/v3.
                Ok(format!("https://{}/api/v3", h))
            }
        }
        "gitlab" => {
            let h = if host.is_empty() {
                "gitlab.com".to_string()
            } else {
                host
            };
            Ok(format!("https://{}/api/v4", h))
        }
        _ => Err(AppError::validation("Unsupported provider type")),
    }
}

/// Builds the create-repo endpoint URL for a given namespace. GitHub routes
/// org repos through `/orgs/{org}/repos` and personal repos through
/// `/user/repos`; GitLab always POSTs to `/projects`.
pub fn create_repo_url(
    host: &str,
    kind: &str,
    namespace: &NamespaceSummary,
) -> Result<String, AppError> {
    let base = api_base(host, kind)?;
    match kind.to_lowercase().as_str() {
        "github" => {
            if namespace.kind == "org" {
                Ok(format!("{}/orgs/{}/repos", base, namespace.id))
            } else {
                Ok(format!("{}/user/repos", base))
            }
        }
        "gitlab" => Ok(format!("{}/projects", base)),
        _ => Err(AppError::validation("Unsupported provider type")),
    }
}

/// Builds the JSON request body for creating a repo. GitHub uses
/// `{name, private, auto_init}`; GitLab uses `{name, path, visibility,
/// initialize_with_readme, namespace_id?}` (namespace_id omitted for the
/// user's personal namespace).
pub fn create_repo_body(
    kind: &str,
    req: &CreateRepoRequest,
) -> Result<serde_json::Value, AppError> {
    match kind.to_lowercase().as_str() {
        "github" => Ok(json!({
            "name": req.name,
            "private": req.private,
            "auto_init": req.auto_init,
        })),
        "gitlab" => {
            let mut body = json!({
                "name": req.name,
                "path": req.name,
                "visibility": if req.private { "private" } else { "public" },
                "initialize_with_readme": req.auto_init,
            });
            // Personal namespace → omit namespace_id so GitLab defaults to
            // the authenticated user. Group/subgroup → send the numeric id.
            if req.namespace.kind != "personal" {
                if let Ok(id) = req.namespace.id.parse::<i64>() {
                    body["namespace_id"] = json!(id);
                }
            }
            Ok(body)
        }
        _ => Err(AppError::validation("Unsupported provider type")),
    }
}

/// Parses the provider's create-repo response into a [`CreatedRepo`].
pub fn parse_created_repo(kind: &str, data: &serde_json::Value) -> CreatedRepo {
    match kind.to_lowercase().as_str() {
        "gitlab" => CreatedRepo {
            full_name: data["path_with_namespace"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            default_branch: data["default_branch"]
                .as_str()
                .unwrap_or("main")
                .to_string(),
            clone_url: data["http_url_to_repo"].as_str().unwrap_or("").to_string(),
        },
        _ => CreatedRepo {
            full_name: data["full_name"].as_str().unwrap_or("").to_string(),
            default_branch: data["default_branch"]
                .as_str()
                .unwrap_or("main")
                .to_string(),
            clone_url: data["clone_url"].as_str().unwrap_or("").to_string(),
        },
    }
}

/// Merges the personal namespace (from `GET /user`) with the list of
/// orgs/groups into a flat list of [`NamespaceSummary`]. GitLab group ids are
/// numeric in the JSON and are surfaced as their string form.
pub fn parse_namespaces(
    kind: &str,
    user: &serde_json::Value,
    extras: &[serde_json::Value],
) -> Vec<NamespaceSummary> {
    let mut out = Vec::new();
    match kind.to_lowercase().as_str() {
        "github" => {
            if let Some(login) = user["login"].as_str() {
                out.push(NamespaceSummary {
                    id: login.to_string(),
                    name: "Personal".to_string(),
                    kind: "personal".to_string(),
                });
            }
            for org in extras {
                if let Some(login) = org["login"].as_str() {
                    out.push(NamespaceSummary {
                        id: login.to_string(),
                        name: login.to_string(),
                        kind: "org".to_string(),
                    });
                }
            }
        }
        "gitlab" => {
            // GitLab `/user` reports the user's own namespace id; fall back to
            // the user id if the field is absent.
            let personal_id = user["namespace_id"]
                .as_i64()
                .or_else(|| user["id"].as_i64())
                .map(|n| n.to_string())
                .unwrap_or_default();
            let personal_name = user["username"].as_str().unwrap_or("Personal").to_string();
            out.push(NamespaceSummary {
                id: personal_id,
                name: personal_name,
                kind: "personal".to_string(),
            });
            for group in extras {
                if let Some(id) = group["id"].as_i64() {
                    let name = group["full_path"]
                        .as_str()
                        .or_else(|| group["path"].as_str())
                        .unwrap_or("")
                        .to_string();
                    out.push(NamespaceSummary {
                        id: id.to_string(),
                        name,
                        kind: "group".to_string(),
                    });
                }
            }
        }
        _ => {}
    }
    out
}

/// Maps a non-2xx provider HTTP response to an [`AppError`]. Auth failures
/// (401/403) and duplicate/invalid-name errors (422) all surface as
/// `Provider` carrying the status + body so the wizard can render them
/// inline; transport-level failures are mapped separately at the call site.
pub fn provider_http_error(status: u16, body: &str) -> AppError {
    AppError::Provider {
        message: format!("HTTP {} - {}", status, body),
    }
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
            let h = if host.is_empty() || host == "github.com" {
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
            let h = if host.is_empty() || host == "github.com" {
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

    async fn list_namespaces(
        &self,
        host: &str,
        kind: &str,
        pat: &str,
    ) -> Result<Vec<NamespaceSummary>, AppError> {
        let base = api_base(host, kind)?;
        let user_url = format!("{}/user", base);
        let extras_url = match kind.to_lowercase().as_str() {
            "github" => format!("{}/user/orgs?per_page=100", base),
            "gitlab" => format!("{}/groups?min_access_level=30&per_page=100", base),
            _ => return Err(AppError::validation("Unsupported provider type")),
        };

        let user = self.get_json(&user_url, pat).await?;
        let extras = self.get_json_array(&extras_url, pat).await?;
        Ok(parse_namespaces(kind, &user, &extras))
    }

    async fn create_repo(
        &self,
        host: &str,
        kind: &str,
        pat: &str,
        req: &CreateRepoRequest,
    ) -> Result<CreatedRepo, AppError> {
        let url = create_repo_url(host, kind, &req.namespace)?;
        let body = create_repo_body(kind, req)?;

        let res = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", pat))
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Transport {
                message: e.to_string(),
            })?;

        if res.status().is_success() {
            let data: serde_json::Value = res.json().await.map_err(|e| AppError::Transport {
                message: e.to_string(),
            })?;
            Ok(parse_created_repo(kind, &data))
        } else {
            let status = res.status().as_u16();
            let body = res.text().await.unwrap_or_default();
            Err(provider_http_error(status, &body))
        }
    }
}

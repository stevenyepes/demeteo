//! HTTP-backed [`MrPublisher`] implementation.
//!
//! Two providers, both authenticated with the project instance's
//! PAT (resolved via `AppSettingsRepository::get_provider_instances`
//! + `Keyring`):
//!
//! - **GitHub**: `POST /repos/{owner}/{repo}/pulls` against `api.github.com`
//!   (or `<host>/api/v3` for GitHub Enterprise).
//! - **GitLab**: `POST /projects/{url-encoded-path}/merge_requests`
//!   against `<host>/api/v4`.
//!
//! The publisher is **idempotent on re-entry**: if `features.mr_url`
//! is already set, we return the existing `MrInfo` instead of
//! creating a duplicate MR. The UI can refresh `mr_state` via
//! [`MrPublisher::fetch_mr_state`].

use std::sync::Arc;

use keyring::Entry;
use serde::Deserialize;

use crate::domain::ids::FeatureId;
use crate::domain::models::{MrInfo, PublishOptions};
use crate::ports::db::{AppSettingsRepository, FeaturePatch, FeatureRepository, ProjectRepository};
use crate::ports::execution::ExecutionPort;
use crate::ports::mr_publisher::MrPublisher;

pub struct HttpMrPublisher {
    app_settings: Arc<dyn AppSettingsRepository>,
    projects: Arc<dyn ProjectRepository>,
    features: Arc<dyn FeatureRepository>,
    exec: Arc<dyn ExecutionPort>,
    /// Used by tests + dry-runs. When `Some`, skip the live HTTP
    /// call and synthesize a fake URL/state. Production wiring leaves
    /// this `None`.
    http_override: Option<Arc<dyn HttpClient>>,
}

impl HttpMrPublisher {
    pub fn new(
        app_settings: Arc<dyn AppSettingsRepository>,
        projects: Arc<dyn ProjectRepository>,
        features: Arc<dyn FeatureRepository>,
        exec: Arc<dyn ExecutionPort>,
    ) -> Self {
        Self {
            app_settings,
            projects,
            features,
            exec,
            http_override: None,
        }
    }

    /// Test-only constructor that swaps the real HTTP client for a
    /// fake (see `tests::FakeHttpClient`).
    #[cfg(test)]
    pub fn with_http_override(
        app_settings: Arc<dyn AppSettingsRepository>,
        projects: Arc<dyn ProjectRepository>,
        features: Arc<dyn FeatureRepository>,
        exec: Arc<dyn ExecutionPort>,
        http: Arc<dyn HttpClient>,
    ) -> Self {
        Self {
            app_settings,
            projects,
            features,
            exec,
            http_override: Some(http),
        }
    }
}

/// The HTTP abstraction. Lets us inject a fake for tests; in
/// production this is `ReqwestHttp`.
pub trait HttpClient: Send + Sync {
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &serde_json::Value,
    ) -> Result<HttpResponse, String>;
    fn get_json(&self, url: &str, headers: &[(String, String)]) -> Result<HttpResponse, String>;
}

/// HTTP response. Body is always captured as text so we can log it
/// when the provider returns an error.
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

impl MrPublisher for HttpMrPublisher {
    fn publish_mr(
        &self,
        project_id: &str,
        feature_id: &FeatureId,
        options: PublishOptions,
    ) -> Result<MrInfo, String> {
        // 0. Idempotency: if the feature already has an MR URL,
        //    return that. The caller can use `fetch_mr_state` to
        //    refresh the state.
        if let Ok(Some(f)) = self.features.get(feature_id) {
            if let Some(url) = f.mr_url.as_ref().filter(|s| !s.is_empty()) {
                return Ok(MrInfo {
                    url: url.clone(),
                    state: f.mr_state.unwrap_or_else(|| "open".to_string()),
                    number: extract_number_from_url(url).unwrap_or(0),
                    provider_kind: String::new(),
                    provider_host: String::new(),
                });
            }
        }

        // 1. Resolve the project + its (single) provider instance.
        let pid = crate::domain::ids::ProjectId::from(project_id.to_string());
        let project = self
            .projects
            .get_projects()?
            .into_iter()
            .find(|p| p.id == pid)
            .ok_or_else(|| format!("Project not found: {}", project_id))?;
        let repos = self.projects.get_repositories_for(&pid)?;
        let repo = repos
            .first()
            .ok_or_else(|| "Project has no repositories configured".to_string())?;

        let provider = self
            .app_settings
            .get_provider_instances()?
            .into_iter()
            .find(|p| p.host == repo.provider_id.0 || p.id.0 == repo.provider_id.0)
            .or_else(|| {
                // Fallback: take the first provider of the matching kind.
                self.app_settings.get_provider_instances().ok().and_then(|v| {
                    v.into_iter().find(|p| {
                        let repo_kind = match repo.provider_id.0.as_str() {
                            host if host.starts_with("github") => "github",
                            host if host.starts_with("gitlab") => "gitlab",
                            _ => "",
                        };
                        !repo_kind.is_empty() && p.kind == repo_kind
                    })
                })
            })
            .ok_or_else(|| {
                "No provider instance configured for this project. Connect one in Preferences → Providers."
                    .to_string()
            })?;

        let pat = resolve_pat(&provider.id.0)?;
        let repo_path = repo.repo_path.clone();
        let feature = self
            .features
            .get(feature_id)?
            .ok_or_else(|| format!("Feature not found: {}", feature_id.0))?;
        let title = options
            .title
            .clone()
            .unwrap_or_else(|| feature.title.clone());
        let settings = self
            .projects
            .get_settings(&pid)?
            .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);
        let body = options
            .body
            .clone()
            .unwrap_or_else(|| {
                settings
                    .worktree_strategy
                    .pr_template
                    .unwrap_or_else(|| {
                        format!(
                            "## Summary\n\n{}\n\n## Test plan\n\n- [ ] Tests pass locally\n- [ ] Manual smoke\n",
                            feature.title
                        )
                    })
            });

        // Resolve local target directory of the repository.
        let target_dir = crate::paths::repo_target_dir_str(
            &self.exec,
            &project.compute_type,
            project.remote_host.as_ref().map(|m| m.as_str()),
            project_id,
            &repo.repo_path,
        )?;

        let source_branch = format!(
            "{}{}",
            settings.worktree_strategy.branch_prefix,
            feature_id.as_str()
        );

        let machine_str = project
            .remote_host
            .as_ref()
            .map(|m| m.as_str())
            .unwrap_or("local");

        // Push the local feature branch to origin remote before creating MR.
        // We use `-f` to force push so subsequent publish_mr calls can update
        // the remote branch if the feature was retried/replayed.
        let push_cmd = format!(
            "git -C {} push -f origin {}",
            crate::paths::shell_escape_posix(&target_dir),
            crate::paths::shell_escape_posix(&source_branch)
        );
        self.exec
            .run_command(machine_str, &push_cmd)
            .map_err(|e| format!("Failed to push feature branch to origin: {}", e))?;

        let http: &dyn HttpClient = match self.http_override.as_ref() {
            Some(arc) => arc.as_ref(),
            None => &ReqwestHttp,
        };

        let info = match provider.kind.as_str() {
            "github" => publish_github(
                http,
                &provider.host,
                &repo_path,
                source_branch,
                settings.worktree_strategy.default_branch.as_str(),
                &title,
                &body,
                options.draft,
                &pat,
            )?,
            "gitlab" => publish_gitlab(
                http,
                &provider.host,
                &repo_path,
                source_branch,
                settings.worktree_strategy.default_branch.as_str(),
                &title,
                &body,
                options.draft,
                &pat,
            )?,
            other => return Err(format!("Unsupported provider kind: {}", other)),
        };

        // Persist the URL + state on the feature so subsequent
        // publish_mr calls are idempotent and the UI can show the
        // MR link without a second round-trip.
        let _ = self.features.update(
            feature_id,
            &FeaturePatch {
                mr_url: Some(Some(info.url.clone())),
                mr_state: Some(Some(info.state.clone())),
                ..Default::default()
            },
        );

        Ok(info)
    }

    fn fetch_mr_state(&self, project_id: &str, mr_url: &str) -> Result<String, String> {
        let _ = project_id;
        let _ = mr_url;
        // Best-effort state refresh. Without parsing the URL we
        // can't know whether this is a GitHub PR or GitLab MR; the
        // full implementation would do an HTTP GET and return the
        // provider's `state` field. The first version returns
        // "open" so the UI has something to render — the user can
        // click through to the URL for the authoritative state.
        Ok("open".to_string())
    }
}

#[allow(dead_code)]
fn feature_id_to_branch(_title: &str, fid: &FeatureId) -> String {
    fid.as_str().to_string()
}

fn extract_number_from_url(url: &str) -> Option<u64> {
    // GitHub: …/pull/123, GitLab: …/-/merge_requests/123
    let s = url.rsplit('/').next()?;
    s.parse::<u64>().ok()
}

fn resolve_pat(provider_id: &str) -> Result<String, String> {
    crate::credential_cache::get_or_fetch(provider_id, || {
        let entry =
            Entry::new("demeteo", provider_id).map_err(|e| format!("Keyring error: {}", e))?;
        entry
            .get_password()
            .map_err(|e| format!("Provider PAT not found in keyring: {}", e))
    })
}

#[allow(clippy::too_many_arguments)]
fn publish_github(
    http: &dyn HttpClient,
    host: &str,
    repo_path: &str,
    head_branch: String,
    base_branch: &str,
    title: &str,
    body: &str,
    draft: bool,
    pat: &str,
) -> Result<MrInfo, String> {
    let url = format!("https://{}/repos/{}/pulls", host, repo_path);
    let payload = serde_json::json!({
        "title": title,
        "head": head_branch,
        "base": base_branch,
        "body": body,
        "draft": draft,
    });
    let headers: Vec<(String, String)> = vec![
        ("Authorization".to_string(), format!("Bearer {}", pat)),
        (
            "Accept".to_string(),
            "application/vnd.github+json".to_string(),
        ),
        ("User-Agent".to_string(), "demeteo".to_string()),
    ];
    let resp = http.post_json(&url, &headers, &payload)?;
    if resp.status >= 300 {
        return Err(format!(
            "GitHub returned HTTP {}: {}",
            resp.status,
            truncate(&resp.body, 512)
        ));
    }
    let v: GithubPull = serde_json::from_str(&resp.body)
        .map_err(|e| format!("Failed to parse GitHub response: {}", e))?;
    Ok(MrInfo {
        url: v.html_url,
        state: v
            .state
            .unwrap_or_else(|| if draft { "draft".into() } else { "open".into() }),
        number: v.number,
        provider_kind: "github".into(),
        provider_host: host.into(),
    })
}

#[allow(clippy::too_many_arguments)]
fn publish_gitlab(
    http: &dyn HttpClient,
    host: &str,
    repo_path: &str,
    source_branch: String,
    target_branch: &str,
    title: &str,
    description: &str,
    draft: bool,
    pat: &str,
) -> Result<MrInfo, String> {
    let url = format!(
        "https://{}/api/v4/projects/{}/merge_requests",
        host,
        urlencoded(repo_path)
    );
    let payload = serde_json::json!({
        "source_branch": source_branch,
        "target_branch": target_branch,
        "title": title,
        "description": description,
        // GitLab's "draft" flag lives on the MR's WIP toggle.
        // Setting `draft: true` puts it in draft via the toggle.
        "draft": draft,
    });
    let headers: Vec<(String, String)> = vec![
        ("PRIVATE-TOKEN".to_string(), pat.to_string()),
        ("Content-Type".to_string(), "application/json".to_string()),
    ];
    let resp = http.post_json(&url, &headers, &payload)?;
    if resp.status >= 300 {
        return Err(format!(
            "GitLab returned HTTP {}: {}",
            resp.status,
            truncate(&resp.body, 512)
        ));
    }
    let v: GitlabMr = serde_json::from_str(&resp.body)
        .map_err(|e| format!("Failed to parse GitLab response: {}", e))?;
    Ok(MrInfo {
        url: v.web_url,
        state: if draft { "draft".into() } else { v.state },
        number: v.iid as u64,
        provider_kind: "gitlab".into(),
        provider_host: host.into(),
    })
}

fn urlencoded(s: &str) -> String {
    // Minimal path-segment encoder. We don't need a full URL crate
    // for `owner/repo` style inputs.
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            for b in s.bytes() {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

#[derive(Deserialize)]
struct GithubPull {
    html_url: String,
    number: u64,
    state: Option<String>,
}

#[derive(Deserialize)]
struct GitlabMr {
    web_url: String,
    iid: i64,
    state: String,
}

// ── production HTTP client (reqwest) ────────────────────────────────────────

pub struct ReqwestHttp;

impl HttpClient for ReqwestHttp {
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &serde_json::Value,
    ) -> Result<HttpResponse, String> {
        let url_str = url.to_string();
        let headers_vec = headers.to_vec();
        let body_val = body.clone();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("Failed to build local tokio runtime: {}", e))?;
            rt.block_on(async {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
                let mut req = client.post(&url_str).json(&body_val);
                for (k, v) in &headers_vec {
                    req = req.header(k.as_str(), v.as_str());
                }
                let resp = req
                    .send()
                    .await
                    .map_err(|e| format!("Git provider request failed: {}", e))?;
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                Ok(HttpResponse { status, body })
            })
        })
        .join()
        .map_err(|_| "HTTP request thread panicked".to_string())?
    }

    fn get_json(&self, url: &str, headers: &[(String, String)]) -> Result<HttpResponse, String> {
        let url_str = url.to_string();
        let headers_vec = headers.to_vec();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("Failed to build local tokio runtime: {}", e))?;
            rt.block_on(async {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
                let mut req = client.get(&url_str);
                for (k, v) in &headers_vec {
                    req = req.header(k.as_str(), v.as_str());
                }
                let resp = req
                    .send()
                    .await
                    .map_err(|e| format!("Git provider request failed: {}", e))?;
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                Ok(HttpResponse { status, body })
            })
        })
        .join()
        .map_err(|_| "HTTP request thread panicked".to_string())?
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencoded_handles_slashes() {
        assert_eq!(urlencoded("owner/repo"), "owner%2Frepo");
        assert_eq!(urlencoded("group/sub/proj"), "group%2Fsub%2Fproj");
        assert_eq!(urlencoded("plain"), "plain");
        assert_eq!(urlencoded("with space"), "with%20space");
    }

    #[test]
    fn extract_number_from_github_url() {
        assert_eq!(
            extract_number_from_url("https://api.github.com/repos/o/r/pulls/42"),
            Some(42)
        );
        assert_eq!(
            extract_number_from_url("https://gitlab.com/g/p/-/merge_requests/7"),
            Some(7)
        );
        assert_eq!(extract_number_from_url("https://example.com/"), None);
    }

    #[test]
    fn feature_id_to_branch_returns_feature_id() {
        let fid = FeatureId::from("f-12345");
        let branch = feature_id_to_branch("any title", &fid);
        assert_eq!(branch, "f-12345");
    }
}

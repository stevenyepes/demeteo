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

mod github;
mod gitlab;
mod http;
mod push;

use std::sync::Arc;

use async_trait::async_trait;
#[cfg(feature = "keyring")]
use keyring::Entry;

use crate::domain::ids::FeatureId;
use crate::domain::models::{MrInfo, PublishOptions};
use crate::ports::db::{AppSettingsRepository, FeaturePatch, FeatureRepository, ProjectRepository};
use crate::ports::execution::ExecutionPort;
use crate::ports::mr_publisher::MrPublisher;

pub use http::{HttpClient, HttpResponse, ReqwestHttp};

pub struct HttpMrPublisher {
    app_settings: Arc<dyn AppSettingsRepository>,
    projects: Arc<dyn ProjectRepository>,
    features: Arc<dyn FeatureRepository>,
    exec: Arc<dyn ExecutionPort>,
    workspace_dir: std::path::PathBuf,
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
        workspace_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            app_settings,
            projects,
            features,
            exec,
            workspace_dir,
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
            workspace_dir: std::path::PathBuf::from("/tmp"),
            http_override: Some(http),
        }
    }
}

struct MrRequest<'a> {
    host: &'a str,
    repo_path: &'a str,
    source_branch: &'a str,
    target_branch: &'a str,
    title: &'a str,
    body: &'a str,
    draft: bool,
    pat: &'a str,
}

#[async_trait]
impl MrPublisher for HttpMrPublisher {
    async fn publish_mr(
        &self,
        project_id: &str,
        feature_id: &FeatureId,
        options: PublishOptions,
    ) -> Result<MrInfo, String> {
        self.publish_mr_inner(project_id, feature_id, options, None)
            .await
    }

    async fn publish_mr_with_pat(
        &self,
        project_id: &str,
        feature_id: &FeatureId,
        options: PublishOptions,
        pat_override: Option<&str>,
    ) -> Result<MrInfo, String> {
        self.publish_mr_inner(project_id, feature_id, options, pat_override)
            .await
    }

    async fn fetch_mr_state(&self, project_id: &str, mr_url: &str) -> Result<String, String> {
        self.fetch_mr_state_impl(project_id, mr_url).await
    }
}

impl HttpMrPublisher {
    async fn publish_mr_inner(
        &self,
        project_id: &str,
        feature_id: &FeatureId,
        options: PublishOptions,
        pat_override: Option<&str>,
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

        let pat = match pat_override {
            Some(p) => p.to_string(),
            None => resolve_pat(&provider.id.0)?,
        };
        let repo_path = repo.repo_path.clone();
        let feature = self
            .features
            .get(feature_id)?
            .ok_or_else(|| format!("Feature not found: {}", feature_id.0))?;
        // Title/body resolution, most specific first:
        //   1. What the caller explicitly asked for.
        //   2. What the `finalize` step's agent authored onto the feature row
        //      after squashing the branch — the normal path for any feature on
        //      a workflow that ends in `finalize`. Resolving it *here* means
        //      every publish route (the driver's auto-publish, the headless
        //      runner, the manual Publish button) gets the agent's summary
        //      without any of them having to know it exists.
        //   3. The old mechanical defaults, for features whose workflow has no
        //      finalize step.
        let non_empty = |s: &String| !s.trim().is_empty();
        let title = options
            .title
            .clone()
            .filter(non_empty)
            .or_else(|| feature.pr_title.clone().filter(non_empty))
            .unwrap_or_else(|| feature.title.clone());
        let settings = self
            .projects
            .get_settings(&pid)?
            .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);
        let body = options
            .body
            .clone()
            .filter(non_empty)
            .or_else(|| feature.pr_body.clone().filter(non_empty))
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

        let source_branch = format!(
            "{}{}",
            settings.worktree_strategy.branch_prefix,
            feature_id.as_str()
        );

        push::push_feature_branch(
            &self.exec,
            &push::BranchPush {
                compute_type: &project.compute_type,
                remote_host: project.remote_host.as_ref().map(|m| m.as_str()),
                project_id,
                workspace_dir: &self.workspace_dir,
                repo_path: &repo_path,
                provider_kind: &provider.kind,
                provider_host: &provider.host,
                pat: &pat,
                source_branch: &source_branch,
            },
        )
        .await?;

        let http: &dyn HttpClient = match self.http_override.as_ref() {
            Some(arc) => arc.as_ref(),
            None => &ReqwestHttp,
        };

        let request = MrRequest {
            host: &provider.host,
            repo_path: &repo_path,
            source_branch: &source_branch,
            target_branch: settings.worktree_strategy.default_branch.as_str(),
            title: &title,
            body: &body,
            draft: options.draft,
            pat: &pat,
        };

        let info = match provider.kind.as_str() {
            "github" => github::publish_github(http, &request).await?,
            "gitlab" => gitlab::publish_gitlab(http, &request).await?,
            other => return Err(format!("Unsupported provider kind: {}", other)),
        };

        // Persist the URL + state on the feature so subsequent
        // publish_mr calls are idempotent and the UI can show the
        // MR link without a second round-trip. If the feature was
        // sitting in `awaiting_mr` (i.e. all steps done but MR not
        // yet published), promote it to `completed` now that the MR
        // is on the provider.
        let _ = self.features.update(
            feature_id,
            &FeaturePatch {
                mr_url: Some(Some(info.url.clone())),
                mr_state: Some(Some(info.state.clone())),
                status: Some("completed".to_string()),
                ..Default::default()
            },
        );

        Ok(info)
    }

    async fn fetch_mr_state_impl(&self, project_id: &str, mr_url: &str) -> Result<String, String> {
        if mr_url.is_empty() {
            return Ok("none".to_string());
        }

        // Resolve the project's provider to know which URL shape /
        // auth header to use. Falls back to URL-shape inference when
        // no provider is configured (offline / cancelled-installation
        // project) — `none` is returned so the UI doesn't have to
        // special-case missing config.
        let pid = crate::domain::ids::ProjectId::from(project_id.to_string());
        let repos = self.projects.get_repositories_for(&pid).ok();
        let provider = repos
            .as_ref()
            .and_then(|rs| rs.first())
            .and_then(|_r| self.app_settings.get_provider_instances().ok())
            .and_then(|list| {
                let repo_kind = match repos
                    .as_ref()
                    .and_then(|rs| rs.first())
                    .map(|r| r.provider_id.0.as_str())
                    .unwrap_or("")
                {
                    h if h.starts_with("github") => "github",
                    h if h.starts_with("gitlab") => "gitlab",
                    _ => "",
                };
                if repo_kind.is_empty() {
                    None
                } else {
                    list.into_iter().find(|p| p.kind == repo_kind)
                }
            });

        // Pick the HTTP client (test override or production reqwest).
        let http: &dyn HttpClient = match self.http_override.as_ref() {
            Some(arc) => arc.as_ref(),
            None => &ReqwestHttp,
        };

        // Resolve the PAT for auth. Without it, private repos return
        // 401/404 and the code below would silently coerce that to
        // "open", so merged MRs on private repos are never detected.
        // `resolve_pat` is best-effort: if the keyring entry is gone
        // (provider removed / PAT rotated) we still proceed without
        // auth so public-repo polling keeps working.
        let pat = provider.as_ref().and_then(|p| match resolve_pat(&p.id.0) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!(
                    "[MrPublisher] could not resolve PAT for provider {}: {}",
                    p.id.0, e
                );
                None
            }
        });

        match (&provider, &pat) {
            (Some(p), Some(token)) if p.kind == "github" => {
                github::fetch_github_pr_state(http, &p.host, mr_url, token).await
            }
            (Some(p), Some(token)) if p.kind == "gitlab" => {
                gitlab::fetch_gitlab_mr_state(http, &p.host, mr_url, token).await
            }
            (Some(p), None) if p.kind == "github" => {
                github::fetch_github_pr_state_unauth(http, &p.host, mr_url).await
            }
            (Some(p), None) if p.kind == "gitlab" => {
                gitlab::fetch_gitlab_mr_state_unauth(http, &p.host, mr_url).await
            }
            _ => Ok("open".to_string()),
        }
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
        #[cfg(feature = "keyring")]
        {
            let entry =
                Entry::new("demeteo", provider_id).map_err(|e| format!("Keyring error: {}", e))?;
            entry
                .get_password()
                .map_err(|e| format!("Provider PAT not found in keyring: {}", e))
        }
        #[cfg(not(feature = "keyring"))]
        {
            Err("OS-keyring credential cache is disabled in this build".to_string())
        }
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

#[cfg(test)]
#[path = "../../../tests/infrastructure/mr_publisher/mod.rs"]
mod tests;

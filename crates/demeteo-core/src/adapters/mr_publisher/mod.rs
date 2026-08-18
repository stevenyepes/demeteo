//! HTTP-backed [`MrPublisher`] implementation.
//!
//! Two providers, both authenticated with the project instance's
//! PAT (instance and token both resolved by the sibling `provider`
//! module):
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
mod provider;
mod push;

use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::ids::FeatureId;
use crate::domain::models::{MrInfo, ProviderInstance, PublishOptions};
use crate::domain::mr_list_error::{classify_list_response, ListResponse, ListTarget, MrListError};
use crate::domain::mr_summary::MrSummary;
use crate::ports::db::{AppSettingsRepository, FeaturePatch, FeatureRepository, ProjectRepository};
use crate::ports::execution::ExecutionPort;
use crate::ports::mr_publisher::MrPublisher;
use provider::{resolve_pat, resolve_pat_best_effort, resolve_provider, resolve_target, MrTarget};

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

/// One repository's worth of "list the open requests".
struct ListRequest<'a> {
    kind: &'a str,
    host: &'a str,
    repo_path: &'a str,
    pat: &'a str,
}

impl<'a> ListRequest<'a> {
    fn target(&self) -> ListTarget<'a> {
        ListTarget {
            kind: self.kind,
            host: self.host,
        }
    }
}

/// Which provider's payload shape one element is read with. The two arms of
/// the listing differ in exactly this and nothing else, so the choice is a
/// value the caller picks once rather than a branch repeated per element.
type MrMapper = fn(&serde_json::Value) -> Result<MrSummary, String>;

/// One page, and only one. Both providers cap `per_page` at 100, and a review
/// queue that needs a second page is not a queue anyone is working through — the
/// listing is a place to start from, not an archive.
const LIST_PAGE_SIZE: u32 = 100;

/// GET a list endpoint and hand back its elements, with every non-2xx routed
/// through [`classify_list_response`].
///
/// The `>= 300 => Ok(…)` shape two functions above this one is the thing that
/// must never be copied here: it is right for a state poll and catastrophic for
/// a listing, because the fallback value is an empty queue. That is what
/// `tests/infrastructure/mr_publisher/list.rs` exists to hold.
async fn read_list(
    http: &dyn HttpClient,
    url: &str,
    headers: &[(String, String)],
    target: ListTarget<'_>,
) -> Result<Vec<serde_json::Value>, MrListError> {
    let resp = http
        .get_json(url, headers)
        .await
        .map_err(|e| MrListError::other(target.host, e))?;

    classify_list_response(
        target,
        ListResponse {
            status: resp.status,
            body: &resp.body,
            headers: &resp.headers,
        },
    )?;

    let parsed: serde_json::Value = serde_json::from_str(&resp.body)
        .map_err(|e| MrListError::other(target.host, format!("unreadable list response: {e}")))?;

    match parsed {
        serde_json::Value::Array(items) => Ok(items),
        _ => Err(MrListError::other(
            target.host,
            "list endpoint answered with something other than an array",
        )),
    }
}

/// GET one resource and hand back its object, on the same terms as
/// [`read_list`] — including that a non-2xx never becomes a value.
async fn read_object(
    http: &dyn HttpClient,
    url: &str,
    headers: &[(String, String)],
    target: ListTarget<'_>,
) -> Result<serde_json::Value, MrListError> {
    let resp = http
        .get_json(url, headers)
        .await
        .map_err(|e| MrListError::other(target.host, e))?;

    classify_list_response(
        target,
        ListResponse {
            status: resp.status,
            body: &resp.body,
            headers: &resp.headers,
        },
    )?;

    serde_json::from_str(&resp.body)
        .map_err(|e| MrListError::other(target.host, format!("unreadable response: {e}")))
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

    async fn list_open_mrs(
        &self,
        project_id: &str,
        repository_id: Option<&str>,
    ) -> Result<Vec<MrSummary>, MrListError> {
        self.list_open_mrs_impl(project_id, repository_id).await
    }

    async fn fetch_mr_detail(
        &self,
        project_id: &str,
        mr_url: &str,
    ) -> Result<MrSummary, MrListError> {
        self.fetch_mr_detail_impl(project_id, mr_url).await
    }

    async fn post_mr_comment(
        &self,
        project_id: &str,
        mr_url: &str,
        body: &str,
    ) -> Result<String, String> {
        self.post_mr_comment_impl(project_id, mr_url, body).await
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
        let MrTarget {
            provider,
            repo_path,
        } = resolve_target(self.app_settings.as_ref(), self.projects.as_ref(), &pid)?;

        let pat = match pat_override {
            Some(p) => p.to_string(),
            None => resolve_pat(&provider.id.0)?,
        };
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

        let source_branch = feature.run_branch(&settings.worktree_strategy.branch_prefix);
        let target_branch = feature.origin.publish_target(
            options.target_branch.as_deref(),
            &settings.worktree_strategy.default_branch,
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
            target_branch: &target_branch,
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

    async fn post_mr_comment_impl(
        &self,
        project_id: &str,
        mr_url: &str,
        body: &str,
    ) -> Result<String, String> {
        if body.trim().is_empty() {
            return Err("There is nothing to post: the review report is empty.".to_string());
        }

        let pid = crate::domain::ids::ProjectId::from(project_id.to_string());
        let MrTarget { provider, .. } =
            resolve_target(self.app_settings.as_ref(), self.projects.as_ref(), &pid)?;

        // `resolve_pat_best_effort` is the state poll's, and reading a public
        // pull request unauthenticated is a real answer. Writing to one is not:
        // the provider would reject it, and degrading to `None` here would
        // report that rejection as the token's fault when the token was simply
        // never sent.
        let pat = resolve_pat(&provider.id.0)?;

        let http: &dyn HttpClient = match self.http_override.as_ref() {
            Some(arc) => arc.as_ref(),
            None => &ReqwestHttp,
        };
        let body = crate::domain::mr_comment::attributed(body);

        match provider.kind.as_str() {
            "github" => {
                github::post_github_comment(http, &provider.host, mr_url, &pat, &body).await
            }
            "gitlab" => gitlab::post_gitlab_note(http, &provider.host, mr_url, &pat, &body).await,
            other => Err(format!("Unsupported provider kind: {}", other)),
        }
    }

    async fn list_open_mrs_impl(
        &self,
        project_id: &str,
        repository_id: Option<&str>,
    ) -> Result<Vec<MrSummary>, MrListError> {
        let pid = crate::domain::ids::ProjectId::from(project_id.to_string());
        let repos = self
            .projects
            .get_repositories_for(&pid)
            .map_err(|e| MrListError::other("", e))?;

        let selected: Vec<_> = match repository_id {
            Some(id) => repos.into_iter().filter(|r| r.id.0 == id).collect(),
            None => repos,
        };

        let http: &dyn HttpClient = match self.http_override.as_ref() {
            Some(arc) => arc.as_ref(),
            None => &ReqwestHttp,
        };

        let mut summaries = Vec::new();
        for repo in &selected {
            let provider = resolve_provider(self.app_settings.as_ref(), &repo.provider_id)
                .map_err(|_| MrListError::NoProvider)?;
            summaries.extend(self.list_one_repo(http, &provider, &repo.repo_path).await?);
        }

        // Newest activity first across every repository, because the queue is
        // read top-down and a per-repository ordering would bury an hour-old
        // request under a month-old one from the repository that sorted first.
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(summaries)
    }

    async fn list_one_repo(
        &self,
        http: &dyn HttpClient,
        provider: &ProviderInstance,
        repo_path: &str,
    ) -> Result<Vec<MrSummary>, MrListError> {
        let target = ListTarget {
            kind: &provider.kind,
            host: &provider.host,
        };

        // The keyring is keyed on the provider *id*, while `resolve_provider`
        // above will match on host or fall back to any instance of the same
        // kind — so a resolved provider is no evidence that a token for it was
        // ever stored under that id. `NoProvider` would send the user to
        // connect one they already have; `Unauthorized` would name a host and a
        // status for a request that was never sent, which is how a working
        // token ends up being audited for a failure that happened locally.
        let pat = resolve_pat(&provider.id.0).map_err(|e| {
            tracing::warn!(provider = %provider.id.0, error = %e, "no PAT resolved for provider");
            MrListError::no_credential(target, e)
        })?;

        let request = ListRequest {
            kind: &provider.kind,
            host: &provider.host,
            repo_path,
            pat: &pat,
        };

        let (items, map): (Vec<serde_json::Value>, MrMapper) = match provider.kind.as_str() {
            "github" => (
                github::list_github_pulls(http, &request).await?,
                MrSummary::from_github,
            ),
            "gitlab" => (
                gitlab::list_gitlab_merge_requests(http, &request).await?,
                MrSummary::from_gitlab,
            ),
            other => {
                return Err(MrListError::other(
                    &provider.host,
                    format!("Demeteo cannot list pull requests on a {other} provider"),
                ))
            }
        };

        // An element that fails to map describes no reviewable request — it is
        // missing a number, a branch pair or a URL. Dropping it loses one row;
        // failing the listing loses every row to one malformed neighbour, and
        // the user cannot fix either from here.
        Ok(items
            .iter()
            .filter_map(|item| match map(item) {
                Ok(summary) => Some(summary),
                Err(e) => {
                    tracing::warn!(provider = %provider.kind, error = %e, "skipping unreadable merge request");
                    None
                }
            })
            .collect())
    }

    async fn fetch_mr_detail_impl(
        &self,
        project_id: &str,
        mr_url: &str,
    ) -> Result<MrSummary, MrListError> {
        let pid = crate::domain::ids::ProjectId::from(project_id.to_string());
        let provider = resolve_target(self.app_settings.as_ref(), self.projects.as_ref(), &pid)
            .map_err(|_| MrListError::NoProvider)?
            .provider;
        let target = ListTarget {
            kind: &provider.kind,
            host: &provider.host,
        };
        let pat = resolve_pat(&provider.id.0).map_err(|e| {
            tracing::warn!(provider = %provider.id.0, error = %e, "no PAT resolved for provider");
            MrListError::no_credential(target, e)
        })?;

        let http: &dyn HttpClient = match self.http_override.as_ref() {
            Some(arc) => arc.as_ref(),
            None => &ReqwestHttp,
        };

        let (payload, map): (serde_json::Value, MrMapper) = match provider.kind.as_str() {
            "github" => (
                github::fetch_github_pr_detail(http, &provider.host, mr_url, &pat).await?,
                MrSummary::from_github,
            ),
            "gitlab" => (
                gitlab::fetch_gitlab_mr_detail(http, &provider.host, mr_url, &pat).await?,
                MrSummary::from_gitlab,
            ),
            other => {
                return Err(MrListError::other(
                    &provider.host,
                    format!("Demeteo cannot read pull requests on a {other} provider"),
                ))
            }
        };

        map(&payload).map_err(|e| MrListError::other(&provider.host, e))
    }

    async fn fetch_mr_state_impl(&self, project_id: &str, mr_url: &str) -> Result<String, String> {
        if mr_url.is_empty() {
            return Ok("none".to_string());
        }

        // A project with no provider configured (offline / cancelled
        // installation) is not an error here: the match below reports
        // `open` so the UI doesn't have to special-case missing config.
        let pid = crate::domain::ids::ProjectId::from(project_id.to_string());
        let provider = resolve_target(self.app_settings.as_ref(), self.projects.as_ref(), &pid)
            .ok()
            .map(|t| t.provider);

        // Pick the HTTP client (test override or production reqwest).
        let http: &dyn HttpClient = match self.http_override.as_ref() {
            Some(arc) => arc.as_ref(),
            None => &ReqwestHttp,
        };

        // Without a PAT, private repos return 401/404 and the match
        // below coerces that to "open", so merged MRs on private repos
        // are never detected.
        let pat = provider.as_ref().and_then(resolve_pat_best_effort);

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

use serde::Deserialize;

use crate::domain::models::MrInfo;
use crate::domain::mr_list_error::MrListError;

use super::{truncate, HttpClient, MrRequest};

/// Pull the current state of a GitHub PR. The MR URL is the user-facing
/// `https://github.com/<owner>/<repo>/pull/<n>` shape; we derive the API
/// URL from it. `pat` is the provider's PAT and is required for private
/// repos — without it, GitHub returns 404 and the request is silently
/// coerced to "open", so merged PRs on private repos are never detected.
pub(super) async fn fetch_github_pr_state(
    http: &dyn HttpClient,
    host: &str,
    mr_url: &str,
    pat: &str,
) -> Result<String, String> {
    let (owner, repo, number) = parse_github_pr_url(mr_url)?;
    let url = format!(
        "https://{}/repos/{}/{}/pulls/{}",
        github_api_host(host),
        owner,
        repo,
        number
    );
    let headers: Vec<(String, String)> = vec![
        ("Authorization".to_string(), format!("Bearer {}", pat)),
        (
            "Accept".to_string(),
            "application/vnd.github+json".to_string(),
        ),
        ("User-Agent".to_string(), "demeteo".to_string()),
    ];
    fetch_github_pr_state_with_headers(http, &url, &headers).await
}

/// Same as `fetch_github_pr_state` but without auth — used as a
/// fallback when the provider's PAT is missing from the keyring so
/// public-repo polling still works.
pub(super) async fn fetch_github_pr_state_unauth(
    http: &dyn HttpClient,
    host: &str,
    mr_url: &str,
) -> Result<String, String> {
    let (owner, repo, number) = parse_github_pr_url(mr_url)?;
    let url = format!(
        "https://{}/repos/{}/{}/pulls/{}",
        github_api_host(host),
        owner,
        repo,
        number
    );
    let headers: Vec<(String, String)> = vec![
        (
            "Accept".to_string(),
            "application/vnd.github+json".to_string(),
        ),
        ("User-Agent".to_string(), "demeteo".to_string()),
    ];
    fetch_github_pr_state_with_headers(http, &url, &headers).await
}

async fn fetch_github_pr_state_with_headers(
    http: &dyn HttpClient,
    url: &str,
    headers: &[(String, String)],
) -> Result<String, String> {
    let resp = http.get_json(url, headers).await?;
    if resp.status >= 300 {
        return Ok("open".to_string());
    }
    let v: serde_json::Value = serde_json::from_str(&resp.body)
        .map_err(|e| format!("Failed to parse GitHub PR response: {}", e))?;
    if v.get("merged_at").map(|m| !m.is_null()).unwrap_or(false) {
        return Ok("merged".to_string());
    }
    Ok(v.get("state")
        .and_then(|s| s.as_str())
        .unwrap_or("open")
        .to_string())
}

/// Parse a `https://github.com/<owner>/<repo>/pull/<n>` URL.
fn parse_github_pr_url(url: &str) -> Result<(String, String, u64), String> {
    // Strip the scheme + host prefix if present so we can split on `/`.
    let trimmed = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let parts: Vec<&str> = trimmed.split('/').collect();
    // Expected: ["github.com", "<owner>", "<repo>", "pull", "<n>"]
    if parts.len() < 5 || parts[3] != "pull" {
        return Err(format!("Not a GitHub PR URL: {}", url));
    }
    let number: u64 = parts[4]
        .parse()
        .map_err(|_| format!("Invalid PR number in URL: {}", url))?;
    Ok((parts[1].to_string(), parts[2].to_string(), number))
}

/// Map a user-visible GitHub host (e.g. "github.com") to the API hostname.
/// For GitHub Enterprise the host is already the API hostname (e.g. "ghes.corp.com").
fn github_api_host(host: &str) -> &str {
    match host {
        "" | "github.com" | "api.github.com" => "api.github.com",
        other => other,
    }
}

/// Read the open pull requests of one repository.
///
/// `state=open` is the server-side filter; asking for everything and filtering
/// here would page through years of closed requests to find this week's four.
pub(super) async fn list_github_pulls(
    http: &dyn HttpClient,
    req: &super::ListRequest<'_>,
) -> Result<Vec<serde_json::Value>, MrListError> {
    let url = format!(
        "https://{}/repos/{}/pulls?state=open&sort=updated&direction=desc&per_page={}",
        github_api_host(req.host),
        req.repo_path,
        super::LIST_PAGE_SIZE
    );
    let headers: Vec<(String, String)> = vec![
        ("Authorization".to_string(), format!("Bearer {}", req.pat)),
        (
            "Accept".to_string(),
            "application/vnd.github+json".to_string(),
        ),
        ("User-Agent".to_string(), "demeteo".to_string()),
    ];
    super::read_list(http, &url, &headers, req.target()).await
}

pub(super) async fn publish_github(
    http: &dyn HttpClient,
    req: &MrRequest<'_>,
) -> Result<MrInfo, String> {
    let url = format!(
        "https://{}/repos/{}/pulls",
        github_api_host(req.host),
        req.repo_path
    );
    let payload = serde_json::json!({
        "title": req.title,
        "head": req.source_branch,
        "base": req.target_branch,
        "body": req.body,
        "draft": req.draft,
    });
    let headers: Vec<(String, String)> = vec![
        ("Authorization".to_string(), format!("Bearer {}", req.pat)),
        (
            "Accept".to_string(),
            "application/vnd.github+json".to_string(),
        ),
        ("User-Agent".to_string(), "demeteo".to_string()),
    ];
    let resp = http.post_json(&url, &headers, &payload).await?;
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
        state: v.state.unwrap_or_else(|| {
            if req.draft {
                "draft".into()
            } else {
                "open".into()
            }
        }),
        number: v.number,
        provider_kind: "github".into(),
        provider_host: req.host.into(),
    })
}

#[derive(Deserialize)]
struct GithubPull {
    html_url: String,
    number: u64,
    state: Option<String>,
}

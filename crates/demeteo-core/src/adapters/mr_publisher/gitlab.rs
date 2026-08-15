use serde::Deserialize;

use crate::domain::models::MrInfo;
use crate::domain::mr_list_error::MrListError;

use super::{truncate, urlencoded, HttpClient, MrRequest};

/// Pull the current state of a GitLab MR. Mirrors `fetch_github_pr_state`
/// for the GitLab API shape. `pat` is the provider's PAT and is required
/// for private repos — without it, GitLab returns 401/404 and the
/// request is silently coerced to "open", so merged MRs on private
/// repos are never detected.
pub(super) async fn fetch_gitlab_mr_state(
    http: &dyn HttpClient,
    host: &str,
    mr_url: &str,
    pat: &str,
) -> Result<String, String> {
    let (project_path, iid) = parse_gitlab_mr_url(mr_url)?;
    let url = format!(
        "https://{}/api/v4/projects/{}/merge_requests/{}",
        host,
        urlencoded(&project_path),
        iid
    );
    let headers: Vec<(String, String)> = vec![
        ("PRIVATE-TOKEN".to_string(), pat.to_string()),
        ("Accept".to_string(), "application/json".to_string()),
    ];
    fetch_gitlab_mr_state_with_headers(http, &url, &headers).await
}

/// Same as `fetch_gitlab_mr_state` but without auth — used as a
/// fallback when the provider's PAT is missing from the keyring so
/// public-repo polling still works.
pub(super) async fn fetch_gitlab_mr_state_unauth(
    http: &dyn HttpClient,
    host: &str,
    mr_url: &str,
) -> Result<String, String> {
    let (project_path, iid) = parse_gitlab_mr_url(mr_url)?;
    let url = format!(
        "https://{}/api/v4/projects/{}/merge_requests/{}",
        host,
        urlencoded(&project_path),
        iid
    );
    let headers: Vec<(String, String)> =
        vec![("Accept".to_string(), "application/json".to_string())];
    fetch_gitlab_mr_state_with_headers(http, &url, &headers).await
}

/// Normalize a raw GitLab `state` value to the canonical set
/// (`open`, `merged`, `closed`). GitLab uses `"opened"` where we
/// store `"open"`; `"locked"` is a short-lived transitional state
/// that is still effectively open.
fn normalize_gitlab_state(s: &str) -> &str {
    match s {
        "opened" => "open",
        "locked" => "open",
        other => other,
    }
}

async fn fetch_gitlab_mr_state_with_headers(
    http: &dyn HttpClient,
    url: &str,
    headers: &[(String, String)],
) -> Result<String, String> {
    let resp = http.get_json(url, headers).await?;
    if resp.status >= 300 {
        return Ok("open".to_string());
    }
    let v: serde_json::Value = serde_json::from_str(&resp.body)
        .map_err(|e| format!("Failed to parse GitLab MR response: {}", e))?;
    let raw = v.get("state").and_then(|s| s.as_str()).unwrap_or("opened");
    Ok(normalize_gitlab_state(raw).to_string())
}

/// Parse a `https://gitlab.com/<group>/<sub>/<project>/-/merge_requests/<iid>` URL.
fn parse_gitlab_mr_url(url: &str) -> Result<(String, u64), String> {
    let trimmed = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let marker_idx = trimmed.find("/-/merge_requests/");
    let path = match marker_idx {
        Some(i) => &trimmed[..i],
        None => return Err(format!("Not a GitLab MR URL: {}", url)),
    };
    let project_path = path.split_once('/').map(|(_, after)| after).unwrap_or(path);
    let iid_str = &trimmed[marker_idx.unwrap() + "/-/merge_requests/".len()..];
    let iid: u64 = iid_str
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .map_err(|_| format!("Invalid MR iid in URL: {}", url))?;
    Ok((project_path.to_string(), iid))
}

/// Read the open merge requests of one project.
///
/// GitLab spells open `opened`, which `normalize_gitlab_state` already knows
/// about on the read side; the query parameter takes the provider's spelling.
pub(super) async fn list_gitlab_merge_requests(
    http: &dyn HttpClient,
    req: &super::ListRequest<'_>,
) -> Result<Vec<serde_json::Value>, MrListError> {
    let url = format!(
        "https://{}/api/v4/projects/{}/merge_requests?state=opened&order_by=updated_at&sort=desc&per_page={}",
        req.host,
        urlencoded(req.repo_path),
        super::LIST_PAGE_SIZE
    );
    let headers: Vec<(String, String)> = vec![
        ("PRIVATE-TOKEN".to_string(), req.pat.to_string()),
        ("Accept".to_string(), "application/json".to_string()),
    ];
    super::read_list(http, &url, &headers, req.target()).await
}

/// Comment on a merge request, and answer with the note's URL.
///
/// A created note answers with an `id` and no URL of its own — GitLab has no
/// canonical address for a note outside the page it is on — so the address is
/// the merge request plus the `#note_<id>` anchor its own UI links to. That is
/// a construction, not a value the provider returned, and it is the reason this
/// function reads `id` rather than looking for a `web_url` that never arrives.
pub(super) async fn post_gitlab_note(
    http: &dyn HttpClient,
    host: &str,
    mr_url: &str,
    pat: &str,
    body: &str,
) -> Result<String, String> {
    let (project_path, iid) = parse_gitlab_mr_url(mr_url)?;
    let url = format!(
        "https://{}/api/v4/projects/{}/merge_requests/{}/notes",
        host,
        urlencoded(&project_path),
        iid
    );
    let headers: Vec<(String, String)> = vec![
        ("PRIVATE-TOKEN".to_string(), pat.to_string()),
        ("Content-Type".to_string(), "application/json".to_string()),
    ];
    let resp = http
        .post_json(&url, &headers, &serde_json::json!({ "body": body }))
        .await?;
    if resp.status >= 300 {
        return Err(format!(
            "GitLab returned HTTP {}: {}",
            resp.status,
            truncate(&resp.body, 512)
        ));
    }
    let v: GitlabNote = serde_json::from_str(&resp.body)
        .map_err(|e| format!("Failed to parse GitLab note response: {}", e))?;
    Ok(format!("{}#note_{}", mr_url.trim_end_matches('/'), v.id))
}

pub(super) async fn publish_gitlab(
    http: &dyn HttpClient,
    req: &MrRequest<'_>,
) -> Result<MrInfo, String> {
    let url = format!(
        "https://{}/api/v4/projects/{}/merge_requests",
        req.host,
        urlencoded(req.repo_path)
    );
    let payload = serde_json::json!({
        "source_branch": req.source_branch,
        "target_branch": req.target_branch,
        "title": req.title,
        "description": req.body,
        // GitLab's "draft" flag lives on the MR's WIP toggle.
        // Setting `draft: true` puts it in draft via the toggle.
        "draft": req.draft,
    });
    let headers: Vec<(String, String)> = vec![
        ("PRIVATE-TOKEN".to_string(), req.pat.to_string()),
        ("Content-Type".to_string(), "application/json".to_string()),
    ];
    let resp = http.post_json(&url, &headers, &payload).await?;
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
        state: if req.draft {
            "draft".into()
        } else {
            match v.state.as_str() {
                "opened" => "open".into(),
                s => s.into(),
            }
        },
        number: v.iid as u64,
        provider_kind: "gitlab".into(),
        provider_host: req.host.into(),
    })
}

#[derive(Deserialize)]
struct GitlabNote {
    id: i64,
}

#[derive(Deserialize)]
struct GitlabMr {
    web_url: String,
    iid: i64,
    state: String,
}

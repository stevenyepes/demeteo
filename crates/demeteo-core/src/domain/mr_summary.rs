//! One open pull request or merge request, in the single shape the listing and
//! the launch path both read. See [`crate::domain`].
//!
//! The two providers disagree about field names and agree about the thing that
//! matters: the head branch they report — `patch-1`, `feature/x` — names a
//! branch in the *contributor's* repository, and for most pull requests that is
//! a fork this clone has no remote for. Fetching that name from `origin` either
//! finds nothing or, worse, finds an unrelated branch that happens to share it.
//! Both providers publish the head commit under a ref in the upstream
//! repository instead — `refs/pull/<n>/head`, `refs/merge-requests/<iid>/head`
//! — so [`MrSummary::head_fetch_spec`] carries that, as a [`Refspec`], and the
//! branch name survives only as a label to render.
//!
//! ## Absent is not permitted
//!
//! The permission fields answer whether a run may push its result back onto the
//! contributor's branch. A provider omits them for reasons its payload does not
//! distinguish: an unauthenticated read, a token without the scope, an older
//! API, a fork that has since been deleted. Every one of those means *we do not
//! know*, and mapping "we do not know" onto `true` spends that uncertainty as a
//! push into someone else's repository. So each of them defaults to
//! not-permitted, and the only route to `true` is a provider that said so —
//! the same rule [`crate::domain::diff_base`] applies to a base branch nothing
//! named.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::feature_origin::Refspec;

/// An open pull request or merge request, provider-independent.
///
/// Identity — the number, the branches, the URL — is required, because a
/// payload missing any of it describes no reviewable request and a summary
/// built from the gaps would be offered as one. Everything cosmetic is
/// tolerated as absent, so a renamed display field costs the listing one blank
/// cell rather than every row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MrSummary {
    pub number: u64,
    pub title: String,
    pub author: String,
    /// What the provider calls the head branch. Display only — the module doc
    /// says why nothing fetches it.
    pub source_branch: String,
    pub target_branch: String,
    pub draft: bool,
    pub web_url: String,
    pub created_at: String,
    pub updated_at: String,
    /// `owner/name` of the repository holding the head branch, when the
    /// provider names it: GitHub reports `null` once a fork is deleted, and a
    /// GitLab merge request identifies its source project by id alone.
    pub head_repo_path: Option<String>,
    /// The ref in the upstream repository that resolves to the head commit.
    pub head_fetch_spec: Refspec,
    pub from_fork: bool,
    /// GitHub `maintainer_can_modify`; GitLab `allow_collaboration`.
    pub maintainer_can_modify: bool,
    /// GitHub `head.repo.permissions.push`. A merge request carries no
    /// equivalent, so a GitLab summary always answers `false`.
    pub head_repo_push: bool,
}

impl MrSummary {
    /// Map one element of `GET /repos/{owner}/{repo}/pulls`.
    pub fn from_github(pull: &Value) -> Result<Self, String> {
        let pull = GithubPull::deserialize(pull)
            .map_err(|e| format!("unreadable GitHub pull request: {e}"))?;
        let head_repo = pull.head.repo;
        let head_repo_path = head_repo.as_ref().map(|r| r.full_name.clone());
        let base_repo_path = pull.base.repo.map(|r| r.full_name);
        Ok(Self {
            number: pull.number,
            title: pull.title,
            author: pull.user.map(|u| u.login).unwrap_or_default(),
            source_branch: pull.head.branch,
            target_branch: pull.base.branch,
            draft: pull.draft,
            web_url: pull.html_url,
            created_at: pull.created_at,
            updated_at: pull.updated_at,
            head_fetch_spec: Refspec::try_from(format!("refs/pull/{}/head", pull.number))?,
            from_fork: match (&head_repo_path, &base_repo_path) {
                (Some(head), Some(base)) => head != base,
                // A head whose repository the payload declines to name is not
                // this one. Treating it as same-repo would send the branch
                // listing, and then a push, at `origin`.
                _ => true,
            },
            maintainer_can_modify: pull.maintainer_can_modify,
            head_repo_push: head_repo
                .and_then(|r| r.permissions)
                .map_or_else(not_permitted, |p| p.push),
            head_repo_path,
        })
    }

    /// Map one element of `GET /projects/{id}/merge_requests`.
    pub fn from_gitlab(merge_request: &Value) -> Result<Self, String> {
        let mr = GitlabMergeRequest::deserialize(merge_request)
            .map_err(|e| format!("unreadable GitLab merge request: {e}"))?;
        Ok(Self {
            number: mr.iid,
            title: mr.title,
            author: mr.author.map(|a| a.username).unwrap_or_default(),
            source_branch: mr.source_branch,
            target_branch: mr.target_branch,
            draft: mr.draft,
            web_url: mr.web_url,
            created_at: mr.created_at,
            updated_at: mr.updated_at,
            head_repo_path: None,
            head_fetch_spec: Refspec::try_from(format!("refs/merge-requests/{}/head", mr.iid))?,
            from_fork: mr.source_project_id != mr.target_project_id,
            maintainer_can_modify: mr.allow_collaboration,
            head_repo_push: not_permitted(),
        })
    }
}

const fn not_permitted() -> bool {
    false
}

#[derive(Deserialize)]
struct GithubPull {
    number: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    user: Option<GithubUser>,
    head: GithubSide,
    base: GithubSide,
    #[serde(default)]
    draft: bool,
    html_url: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default = "not_permitted")]
    maintainer_can_modify: bool,
}

#[derive(Deserialize)]
struct GithubUser {
    #[serde(default)]
    login: String,
}

#[derive(Deserialize)]
struct GithubSide {
    #[serde(rename = "ref")]
    branch: String,
    #[serde(default)]
    repo: Option<GithubRepo>,
}

#[derive(Deserialize)]
struct GithubRepo {
    full_name: String,
    #[serde(default)]
    permissions: Option<GithubPermissions>,
}

#[derive(Deserialize)]
struct GithubPermissions {
    #[serde(default = "not_permitted")]
    push: bool,
}

#[derive(Deserialize)]
struct GitlabMergeRequest {
    iid: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    author: Option<GitlabUser>,
    source_branch: String,
    target_branch: String,
    #[serde(default)]
    draft: bool,
    web_url: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
    source_project_id: u64,
    target_project_id: u64,
    #[serde(default = "not_permitted")]
    allow_collaboration: bool,
}

#[derive(Deserialize)]
struct GitlabUser {
    #[serde(default)]
    username: String,
}

#[cfg(test)]
#[path = "../../tests/domain/mr_summary.rs"]
mod tests;

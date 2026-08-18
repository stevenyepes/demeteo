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
    /// What the provider calls the head branch. Nothing fetches it — the
    /// module doc says why — and everything else renders it as a label.
    ///
    /// One consumer does not: [`crate::domain::fix_destination`] returns it as
    /// the base of a PR-create call, and only when `!from_fork &&
    /// head_repo_push`, i.e. only when the provider itself placed this branch
    /// in the upstream repository. So this is not free text to sanitize or
    /// truncate for rendering: a shortened value is a base branch that does not
    /// exist, and the module doc's warning about resolving this name against
    /// origin is exactly why that guard is narrow.
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
    /// Whether the request conflicts with its target branch, and `None` while
    /// nobody knows yet.
    ///
    /// Both providers compute mergeability asynchronously — GitHub answers
    /// `mergeable: null` and GitLab `merge_status: checking` until they are
    /// done — and neither GitHub list endpoint carries the field at all. Every
    /// one of those is *not yet decided*, which the "Absent is not permitted"
    /// rule above forbids spending as `false`: a green row on a request that
    /// will not merge is the answer a reviewer acts on and then loses an hour
    /// to.
    ///
    /// Serialized even when `None`, deliberately — the frontend renders the
    /// undecided state as its own chip, and a skipped field would arrive
    /// indistinguishable from a clean one.
    #[serde(default)]
    pub has_conflicts: Option<bool>,
    /// Lines added, lines removed and files touched, when the payload that
    /// produced this summary carried them. Absent and unknown are the same
    /// fact to a reader of a diffstat, so unlike `has_conflicts` these are
    /// skipped rather than sent as null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additions: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletions: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_files: Option<u64>,
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
            has_conflicts: pull.mergeable.map(|m| !m),
            additions: pull.additions,
            deletions: pull.deletions,
            changed_files: pull.changed_files,
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
            has_conflicts: mr
                .has_conflicts
                .or_else(|| mr.merge_status.as_deref().and_then(conflict_verdict)),
            additions: None,
            deletions: None,
            changed_files: mr.changes_count.as_deref().and_then(changed_file_count),
        })
    }
}

const fn not_permitted() -> bool {
    false
}

/// Read GitLab's `merge_status`, and only once it has settled.
///
/// `unchecked` and `checking` are the provider still working. Answering `false`
/// for either is the same lie as reading GitHub's `mergeable: null` as clean,
/// so they leave the verdict undecided and the row shows no reassurance it was
/// not given.
fn conflict_verdict(merge_status: &str) -> Option<bool> {
    match merge_status {
        "cannot_be_merged" => Some(true),
        "can_be_merged" => Some(false),
        _ => None,
    }
}

/// Read GitLab's `changes_count`, which is a string and caps itself.
///
/// Very large merge requests answer `"1000+"`. Trimming the marker keeps the
/// floor, which is what a "N files" label means anyway; a parse of the raw
/// string would drop the count entirely and the row would claim the request
/// touches nothing.
fn changed_file_count(changes_count: &str) -> Option<u64> {
    changes_count.trim().trim_end_matches('+').parse().ok()
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
    /// Present only on the single-request GET, and `null` there until GitHub
    /// has finished deciding.
    #[serde(default)]
    mergeable: Option<bool>,
    #[serde(default)]
    additions: Option<u64>,
    #[serde(default)]
    deletions: Option<u64>,
    #[serde(default)]
    changed_files: Option<u64>,
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
    #[serde(default)]
    has_conflicts: Option<bool>,
    #[serde(default)]
    merge_status: Option<String>,
    #[serde(default)]
    changes_count: Option<String>,
}

#[derive(Deserialize)]
struct GitlabUser {
    #[serde(default)]
    username: String,
}

#[cfg(test)]
#[path = "../../tests/domain/mr_summary.rs"]
mod tests;

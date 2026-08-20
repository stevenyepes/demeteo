//! Pushing the feature branch to `origin` before an MR is opened.
//!
//! The credential path this needs — and every other push in the app needs
//! too — lives in [`crate::adapters::git_push`], which also carries the
//! reasoning for why the PAT rides an inline helper rather than the URL or a
//! file on disk. What stays here is the part that is about a *merge request*:
//! resolving the target directory, re-pointing `origin` at the provider's
//! HTTPS URL, and force-pushing a branch that was squashed under an open MR.
//!
//! The `remote set-url` is the reason `git_push` exists as a shared module.
//! It writes a deliberately token-free URL, so a project that has published
//! one MR has a remote that no longer authenticates by itself — and every
//! push that did not know about the helper failed from then on.

use std::sync::Arc;

use crate::adapters::git_push::{push_request, redacted, remote_user, GitCredential};
use crate::ports::execution::{ExecutionPort, ProgramRequest};

pub(super) struct BranchPush<'a> {
    pub compute_type: &'a str,
    pub remote_host: Option<&'a str>,
    pub project_id: &'a str,
    pub workspace_dir: &'a std::path::Path,
    pub repo_path: &'a str,
    pub provider_kind: &'a str,
    pub provider_host: &'a str,
    pub pat: &'a str,
    pub source_branch: &'a str,
}

pub(super) async fn push_feature_branch(
    exec: &Arc<dyn ExecutionPort>,
    push: &BranchPush<'_>,
) -> Result<(), String> {
    // Resolve target directory of the repository.
    let target_dir = if push.compute_type.eq_ignore_ascii_case("local") {
        crate::paths::repo_target_dir_local(push.workspace_dir, push.project_id, push.repo_path)
            .to_string_lossy()
            .to_string()
    } else {
        crate::paths::repo_target_dir_str(
            exec,
            push.compute_type,
            push.remote_host,
            push.project_id,
            push.repo_path,
            None,
        )
        .await?
    };

    let machine_str = push
        .remote_host
        .unwrap_or(crate::domain::ids::LOCAL_MACHINE);

    let remote_user = remote_user(push.provider_kind);
    let remote_url = format!(
        "https://{}@{}/{}",
        remote_user, push.provider_host, push.repo_path
    );
    exec.run_program(
        machine_str,
        git_request(&target_dir, ["remote", "set-url", "origin", &remote_url]),
    )
    .await
    .map_err(|e| format!("Failed to update remote origin URL: {}", e))?;

    // `force` so a retried or replayed feature can update a branch it already
    // pushed — the one push in the app that may, and the reason
    // `push_request` takes the flag rather than assuming it.
    let credential = GitCredential {
        user: remote_user,
        pat: push.pat.to_string(),
    };
    exec.run_program(
        machine_str,
        push_request(&target_dir, push.source_branch, true, Some(&credential)),
    )
    .await
    .map_err(|e| {
        format!(
            "Failed to push feature branch to origin: {}",
            redacted(&e, push.pat)
        )
    })?;

    Ok(())
}

fn git_request<const N: usize>(repo_dir: &str, args: [&str; N]) -> ProgramRequest {
    ProgramRequest {
        executable: "git".to_string(),
        args: [
            vec!["-C".to_string(), repo_dir.to_string()],
            args.into_iter().map(str::to_string).collect(),
        ]
        .concat(),
        ..ProgramRequest::default()
    }
}

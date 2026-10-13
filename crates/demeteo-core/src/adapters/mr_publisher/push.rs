//! Pushing the feature branch to `origin` before an MR is opened.
//!
//! The credential path this needs — and every other push in the app needs
//! too — lives in [`crate::adapters::git_push`], which also carries the
//! reasoning for why the PAT rides an inline helper rather than the URL or a
//! file on disk. What stays here is the part that is about a *merge request*:
//! resolving the target directory, re-pointing `origin` at the provider's
//! HTTPS URL, and force-pushing a branch that was squashed under an open MR —
//! under a lease, never a bare `-f` ([`crate::domain::push_lease`]).
//!
//! The `remote set-url` is the reason `git_push` exists as a shared module.
//! It writes a deliberately token-free URL, so a project that has published
//! one MR has a remote that no longer authenticates by itself — and every
//! push that did not know about the helper failed from then on.
//!
//! ## Why this pushes from the clone, unlike a sync publish
//!
//! A `pre-push` hook runs in the tree the push runs in, and this path pushes
//! from the project's clone because it does not know the feature's worktree —
//! only the repository and the branch name. A sync publish does know its
//! worktree and pushes from there (see `application::sync_session::publish`).
//! The difference is deliberate and the push is unchanged; only its error is
//! read, by [`classify_push_failure`](crate::domain::git_push::classify_push_failure),
//! so a hook that stopped the push is named as one. The clone is prepared
//! first when it has a hook to satisfy — see [`prepare_push_tree`].

use std::sync::Arc;

use crate::adapters::git_push::{
    prepare_push_tree, push_failure, push_request, redacted, remote_user, GitCredential,
};
use crate::domain::git_push::{classify_push_failure, host_without_port, PushFailure};
use crate::domain::push_lease::{is_stale_lease, lease_from_tracking, tracking_query};
use crate::ports::db::AppSettingsRepository;
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
    pub prepare_command: Option<&'a str>,
}

pub(super) async fn push_feature_branch(
    exec: &Arc<dyn ExecutionPort>,
    app_settings: &dyn AppSettingsRepository,
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

    // Forced so a retried or replayed feature can update a branch it already
    // pushed — the one push in the app that may, and the reason
    // `push_request` takes the lease rather than assuming one.
    let tracking = exec
        .run_program(
            machine_str,
            git_request(&target_dir, tracking_query(push.source_branch)),
        )
        .await
        .map_err(|e| format!("Failed to read origin/{}: {e}", push.source_branch))?;
    let lease = lease_from_tracking(push.source_branch, &tracking);
    prepare_push_tree(
        exec.as_ref(),
        app_settings,
        machine_str,
        &target_dir,
        push.prepare_command,
    )
    .await?;
    let credential = GitCredential {
        user: remote_user,
        pat: push.pat.to_string(),
        host: host_without_port(push.provider_host).to_string(),
    };
    exec.run_program(
        machine_str,
        push_request(
            &target_dir,
            push.source_branch,
            Some(&lease),
            Some(&credential),
        ),
    )
    .await
    .map_err(|e| {
        let clean = redacted(&e, push.pat);
        if is_stale_lease(&clean) {
            return lease.refusal();
        }
        match classify_push_failure(&clean) {
            PushFailure::HookFailed => push_failure(&e, Some(&credential)),
            _ => format!("Failed to push feature branch to origin: {clean}"),
        }
    })?;

    Ok(())
}

fn git_request<S: Into<String>, const N: usize>(repo_dir: &str, args: [S; N]) -> ProgramRequest {
    ProgramRequest {
        executable: "git".to_string(),
        args: [
            vec!["-C".to_string(), repo_dir.to_string()],
            args.into_iter().map(Into::into).collect(),
        ]
        .concat(),
        ..ProgramRequest::default()
    }
}

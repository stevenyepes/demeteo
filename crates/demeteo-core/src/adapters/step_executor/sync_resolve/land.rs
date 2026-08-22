//! Everything after the gate: stage, confirm, commit, publish.
//!
//! A free function over the ports it actually touches rather than a stage
//! inside the turn, which is what makes the commit-message rejection, the
//! unmerged re-check and the publish assertable without a registry, an agent
//! runtime or a database behind them (AGENTS.md §3).

use std::sync::Arc;

use crate::adapters::step_executor::steps::list_unmerged::try_list_unmerged_files;
use crate::adapters::step_executor::steps::pending_commit::{self, PendingCommit};
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::sync_session::{resolution_refusal, ResolutionPublish};
use crate::paths;
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::{ask, Answer, ExecutionPort};

use super::ResolveSyncError;

/// The worktree a resolution is landed from, and the ports that can reach it.
pub(super) struct Landing<'a> {
    pub exec: &'a Arc<dyn ExecutionPort>,
    pub git_ops: &'a GitOpsHelper,
    pub app_settings: &'a Arc<dyn AppSettingsRepository>,
    pub machine_str: &'a str,
    pub resolved_cwd: &'a str,
    pub base_branch: &'a str,
    pub feature_branch: &'a str,
    /// What [`publish_policy`](crate::domain::sync_session::publish_policy)
    /// decided, already decided.
    pub publish: ResolutionPublish,
}

/// The commit the resolution landed as, and whether origin was seen to have it.
///
/// `turn_stop` is how the turn ended, which is not evidence about the tree and
/// is read only to explain one that really is unresolved
/// ([`resolution_refusal`]).
pub(super) async fn land(
    l: Landing<'_>,
    turn_stop: Option<&str>,
) -> Result<(String, bool), ResolveSyncError> {
    let Landing {
        exec,
        git_ops,
        app_settings,
        machine_str,
        resolved_cwd,
        base_branch,
        feature_branch,
        publish,
    } = l;

    // `-A`, not the conflicted paths the merge reported. The sync worktree is a
    // throwaway checkout that exists only for this resolution, and it is deleted
    // the moment the resolution lands — so a file the agent had to add, or a
    // fourth file it had to fix for the tree to build, is not "extra": staging
    // only the reported paths committed a tree that does not compile and then
    // removed the rest with the directory. `-A` still honours `.gitignore`, so a
    // resolver that ran the project's tests does not stage `node_modules` or
    // `target`.
    if let Err(e) = exec
        .run_command(
            machine_str,
            &format!("git -C {} add -A", paths::shell_escape_posix(resolved_cwd)),
        )
        .await
    {
        return Err(ResolveSyncError::Failed(format!(
            "Failed to stage conflict resolution: {}",
            e
        )));
    }

    // Staging turns Git's unmerged index entries into the resolved files;
    // this is the authoritative completion check, independent of agent kind.
    let still_unmerged = match try_list_unmerged_files(&**exec, machine_str, resolved_cwd).await {
        Ok(files) => files,
        Err(why) => {
            return Err(ResolveSyncError::Failed(format!(
                "Could not read {} back to confirm the resolution: {}",
                resolved_cwd, why
            )));
        }
    };
    if !still_unmerged.is_empty() {
        return Err(ResolveSyncError::Failed(resolution_refusal(
            turn_stop,
            "Resolver did not resolve every conflicted file.",
        )));
    }

    let message = format!("chore: resolve sync conflicts with origin/{}", base_branch);
    if let Err(rejection) = git_ops
        .validate_commit_message(
            if machine_str == crate::domain::ids::LOCAL_MACHINE {
                None
            } else {
                Some(machine_str)
            },
            resolved_cwd,
            &message,
        )
        .await
    {
        return Err(ResolveSyncError::Failed(format!(
            "The repository's commit-msg hook rejected the sync-resolution commit: {}",
            rejection.hook_output
        )));
    }

    // The hook has already accepted this exact message above, matching the
    // finalize flow's validate-then-commit split. Avoid rerunning arbitrary
    // repository hooks after the merge has been staged.
    //
    // Guarded because an agent that committed on its own leaves nothing to
    // record — see `steps::pending_commit`.
    match pending_commit::probe(&**exec, machine_str, resolved_cwd).await {
        PendingCommit::Nothing => {}
        // The one arm with data loss behind it. A skipped commit still pushes
        // (a no-op), still reads a sha back (the pre-merge one), still files
        // the session `Resolved` — and the teardown then force-removes the
        // worktree the agent's work is sitting in, unpublished.
        PendingCommit::Unreadable(why) => {
            return Err(ResolveSyncError::Failed(format!(
                "Could not tell whether the resolution still needs committing, so it was left in {}: {}",
                resolved_cwd, why
            )));
        }
        PendingCommit::Pending => {
            let commit_resolved = exec
                .run_command(
                    machine_str,
                    &format!(
                        "{} -c user.email=demeteo@local -c user.name=demeteo commit -m {}",
                        paths::git_no_hooks(resolved_cwd),
                        paths::shell_escape_posix(&message),
                    ),
                )
                .await;
            if let Err(e) = commit_resolved {
                return Err(ResolveSyncError::Failed(format!(
                    "Failed to commit resolution: {}",
                    e
                )));
            }
        }
    }

    // Read before the push rather than after it. `push` does not move `HEAD`,
    // so the two orders name the same commit — but this one is still on the
    // failing side of the publish, so an unreadable answer can be refused
    // outright instead of becoming the empty sha a `Succeeded` verdict then
    // carries as its evidence.
    let head_sha = match ask(
        &**exec,
        machine_str,
        &format!(
            "git -C {} rev-parse HEAD",
            paths::shell_escape_posix(resolved_cwd)
        ),
    )
    .await
    {
        Answer::Said(out) => out.trim().to_string(),
        Answer::Refused | Answer::Unreadable(_) => {
            return Err(ResolveSyncError::Failed(format!(
                "The resolution was committed in {} but its commit could not be read back, so it was not published.",
                resolved_cwd
            )));
        }
    };

    // The row's `pushed_at` is written from this bool, and the button's
    // `publish` refuses to write it on the strength of an exit code
    // ([`push_landed`](crate::application::sync_session::push_landed)). Two
    // paths writing one column on opposite evidence rules is how a merge origin
    // never received suppresses its own review card forever, so this one asks
    // origin too. An unconfirmed push is not a failed resolution — the commit
    // is on the branch either way — so it lands as a resolution still waiting,
    // which is the state that keeps a surface pointing at it.
    let published = if publish == ResolutionPublish::Push {
        // The credential the remote needs, read from the remote itself. A
        // resolution that is committed and unpushed is recoverable from the
        // banner; one that cannot authenticate is recoverable from nowhere
        // until the provider is reconnected, which is why the failure says
        // which of the two it is.
        let credential = crate::adapters::git_push::credential_for_repo(
            &**exec,
            app_settings.as_ref(),
            machine_str,
            resolved_cwd,
        )
        .await;
        if let Err(e) = exec
            .run_program(
                machine_str,
                crate::adapters::git_push::push_request(
                    resolved_cwd,
                    feature_branch,
                    false,
                    credential.as_ref(),
                ),
            )
            .await
        {
            return Err(ResolveSyncError::Failed(format!(
                "Resolution committed locally but push to origin/{} failed: {}. Publish it from the sync banner once the push can go through.",
                feature_branch,
                crate::adapters::git_push::push_failure(&e, credential.as_ref())
            )));
        }
        matches!(
            crate::application::sync_session::push_landed(
                &**exec,
                machine_str,
                resolved_cwd,
                feature_branch,
                &head_sha,
            )
            .await,
            Answer::Said(_)
        )
    } else {
        false
    };

    Ok((head_sha, published))
}

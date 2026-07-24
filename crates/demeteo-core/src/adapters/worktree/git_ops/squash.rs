//! Collapse a feature branch's commits into the single commit that lands
//! in the PR, and let the target repo's own `commit-msg` hook judge the
//! message before it is used.
//!
//! **Why the branch is rewritten with `commit-tree` rather than
//! `reset --soft` + `commit`.** The obvious implementation checks the
//! feature branch out, soft-resets to the merge base and re-commits. That
//! has two problems this module avoids:
//!
//! 1. *It can strand the branch.* Between the reset and a successful
//!    commit, the branch tip has already moved back to the merge base. A
//!    commit that fails (a rejecting hook, a cancelled run, a crash) leaves
//!    the feature's work reachable only from the reflog.
//! 2. *It needs a working tree*, and therefore has to care which worktree
//!    currently has the branch checked out — the same coordination problem
//!    that `merge_subtask` has to solve.
//!
//! `commit-tree` sidesteps both. The squashed commit reuses the branch
//! tip's *existing* tree verbatim — squashing changes history, never
//! content — so the new commit is built entirely from objects that already
//! exist, and the branch ref is moved exactly once, atomically, with a
//! compare-and-swap against the old tip. Nothing is touched until that
//! final move, and any worktree holding the branch stays consistent
//! because its files already match the tree the new commit points at.
//!
//! **Why the `commit-msg` hook is run by hand.** `commit-tree` is plumbing
//! and runs no hooks, but we *want* commitlint's verdict on this message:
//! it is the one commit a human will review. So the hook is invoked
//! directly, on git's own terms (repo root as cwd, message file as `$1`),
//! purely as a validator — its rejection becomes feedback for the agent
//! rather than a failed commit. `pre-commit` is deliberately *not* run:
//! the squash changes no bytes in the tree, so a content gate here would
//! be re-judging code that the pipeline's own harness steps already
//! judged, and a repo whose `pre-commit` runs the full test suite would
//! turn publishing into a second, slower CI run.

use super::GitOpsHelper;
use crate::paths;
use crate::ports::worktree_ops::{CommitMessageRejected, SquashOutcome};

/// Where the pre-squash tip is parked so the rewrite is undoable.
fn backup_ref_for(feature_branch: &str) -> String {
    format!("refs/demeteo/pre-squash/{}", feature_branch)
}

impl GitOpsHelper {
    /// Run the repo's `commit-msg` hook against `message` without committing.
    pub async fn validate_commit_message(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        message: &str,
    ) -> Result<(), CommitMessageRejected> {
        let machine_str = machine_id.unwrap_or("local");
        let safe_dir = paths::shell_escape_posix(repo_dir);

        // `--git-path` resolves through `core.hooksPath`, so this finds the
        // hook wherever husky (or a bare repo layout) actually put it.
        let hook_path = match self
            .exec
            .run_command(
                machine_str,
                &format!("git -C {} rev-parse --git-path hooks/commit-msg", safe_dir),
            )
            .await
        {
            Ok(p) => {
                let p = p.trim().to_string();
                if p.is_empty() {
                    return Ok(());
                } else if p.starts_with('/') {
                    p
                } else {
                    // `--git-path` yields a path relative to the repo root.
                    format!("{}/{}", repo_dir.trim_end_matches('/'), p)
                }
            }
            // No hooks resolvable — nothing to validate against.
            Err(_) => return Ok(()),
        };

        // Not every repo installs one, and a non-executable file is not a
        // hook as far as git is concerned.
        if self
            .exec
            .run_command(
                machine_str,
                &format!("test -x {}", paths::shell_escape_posix(&hook_path)),
            )
            .await
            .is_err()
        {
            return Ok(());
        }

        // Stage the candidate message inside the git dir so it can never
        // dirty the working tree the way a repo-root temp file would.
        let msg_path = format!("{}/.git/DEMETEO_COMMIT_MSG", repo_dir.trim_end_matches('/'));
        if self
            .exec
            .write_file_bytes(machine_str, &msg_path, message.as_bytes())
            .await
            .is_err()
        {
            // If we cannot stage the message we cannot validate it. Treat
            // that as "no opinion" rather than a rejection — a broken
            // temp-file write must not block a publish.
            return Ok(());
        }

        let safe_hook = paths::shell_escape_posix(&hook_path);
        let safe_msg = paths::shell_escape_posix(&msg_path);
        // git runs commit-msg from the repo root with the message file as
        // $1; match that exactly so a hook resolving `node_modules/.bin`
        // relative to the root behaves as it does for a real commit.
        let result = self
            .exec
            .run_command(
                machine_str,
                &format!("cd {} && {} {}", safe_dir, safe_hook, safe_msg),
            )
            .await;
        let _ = self
            .exec
            .run_command(machine_str, &format!("rm -f {}", safe_msg))
            .await;

        result.map(|_| ()).map_err(|hook_output| {
            tracing::info!(
                repo = %repo_dir,
                "validate_commit_message: repo's commit-msg hook rejected the proposed message",
            );
            CommitMessageRejected { hook_output }
        })
    }

    /// Collapse `<default_branch>..<feature_branch>` into one commit.
    pub async fn squash_feature_branch(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        default_branch: &str,
        message: &str,
    ) -> Result<SquashOutcome, String> {
        let machine_str = machine_id.unwrap_or("local");
        let safe_dir = paths::shell_escape_posix(repo_dir);
        let safe_fb = paths::shell_escape_posix(feature_branch);

        let git = |args: String| format!("git -C {} {}", safe_dir, args);

        // Prefer the pushed default branch: it is what the PR will be
        // diffed against. Fall back to the local ref for repos with no
        // origin (tests, air-gapped clones).
        let _ = self
            .exec
            .run_command(
                machine_str,
                &git(format!(
                    "fetch origin {}",
                    paths::shell_escape_posix(default_branch)
                )),
            )
            .await;
        let base = {
            let remote = format!("refs/remotes/origin/{}", default_branch);
            let candidates = [remote.as_str(), default_branch];
            let mut found = None;
            for cand in candidates {
                if let Ok(sha) = self
                    .exec
                    .run_command(
                        machine_str,
                        &git(format!(
                            "merge-base {} {}",
                            paths::shell_escape_posix(cand),
                            safe_fb
                        )),
                    )
                    .await
                {
                    let sha = sha.trim().to_string();
                    if !sha.is_empty() {
                        found = Some(sha);
                        break;
                    }
                }
            }
            found.ok_or_else(|| {
                format!(
                    "cannot squash {}: no merge base with {} (or origin/{})",
                    feature_branch, default_branch, default_branch
                )
            })?
        };

        let tip = self
            .exec
            .run_command(machine_str, &git(format!("rev-parse {}", safe_fb)))
            .await
            .map_err(|e| format!("cannot resolve {}: {}", feature_branch, e))?
            .trim()
            .to_string();

        // Nothing to do when the branch adds no commits, or adds commits
        // whose net effect on the tree is nil (e.g. a change and its
        // revert). Either way there is no PR worth opening.
        let collapsed: u32 = self
            .exec
            .run_command(
                machine_str,
                &git(format!("rev-list --count {}..{}", base, safe_fb)),
            )
            .await
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        if collapsed == 0 {
            return Ok(SquashOutcome::NothingToSquash);
        }
        let tree = self
            .exec
            .run_command(machine_str, &git(format!("rev-parse {}^{{tree}}", safe_fb)))
            .await
            .map_err(|e| format!("cannot resolve tree of {}: {}", feature_branch, e))?
            .trim()
            .to_string();
        let base_tree = self
            .exec
            .run_command(machine_str, &git(format!("rev-parse {}^{{tree}}", base)))
            .await
            .map_err(|e| format!("cannot resolve tree of {}: {}", base, e))?
            .trim()
            .to_string();
        if tree == base_tree {
            return Ok(SquashOutcome::NothingToSquash);
        }

        // The undo path. Written before the branch moves, so a failure
        // anywhere below still leaves the original history reachable by
        // name rather than only from the reflog.
        //
        // Written only once, deliberately. A finalize step that squashes and
        // then fails to publish gets retried, and that retry squashes the
        // already-squashed branch again — overwriting the backup there would
        // replace the real pre-squash history with the *first squash's*
        // single commit, quietly destroying the thing the ref exists to
        // protect. First write wins; the original history is the one worth
        // keeping.
        let backup_ref = backup_ref_for(feature_branch);
        let safe_backup = paths::shell_escape_posix(&backup_ref);
        let backup_exists = self
            .exec
            .run_command(
                machine_str,
                &git(format!("rev-parse --verify -q {}", safe_backup)),
            )
            .await
            .is_ok();
        if !backup_exists {
            self.exec
                .run_command(
                    machine_str,
                    &git(format!("update-ref {} {}", safe_backup, tip)),
                )
                .await
                .map_err(|e| format!("failed to record pre-squash backup ref: {}", e))?;
        }

        // Message via file, never argv: it is multi-line and carries
        // arbitrary text the agent wrote.
        let msg_path = format!("{}/.git/DEMETEO_SQUASH_MSG", repo_dir.trim_end_matches('/'));
        self.exec
            .write_file_bytes(machine_str, &msg_path, message.as_bytes())
            .await
            .map_err(|e| format!("failed to stage squash commit message: {}", e))?;
        let safe_msg = paths::shell_escape_posix(&msg_path);

        // Same identity the per-step commits already use (`declared.rs`),
        // so a repo with no configured user still gets a valid commit.
        let commit_res = self
            .exec
            .run_command(
                machine_str,
                &git(format!(
                    "-c user.email=demeteo@local -c user.name=demeteo \
                     commit-tree {} -p {} -F {}",
                    tree, base, safe_msg
                )),
            )
            .await;
        let _ = self
            .exec
            .run_command(machine_str, &format!("rm -f {}", safe_msg))
            .await;
        let new_sha = commit_res
            .map_err(|e| format!("failed to build squashed commit: {}", e))?
            .trim()
            .to_string();

        // Compare-and-swap: if anything moved the branch while we were
        // working, fail rather than clobber it.
        self.exec
            .run_command(
                machine_str,
                &git(format!(
                    "update-ref refs/heads/{} {} {}",
                    safe_fb, new_sha, tip
                )),
            )
            .await
            .map_err(|e| {
                format!(
                    "failed to move {} to the squashed commit (branch moved underneath us?): {}",
                    feature_branch, e
                )
            })?;

        tracing::info!(
            branch = %feature_branch,
            collapsed,
            sha = %new_sha,
            backup = %backup_ref,
            "squash_feature_branch: collapsed feature branch into one commit",
        );

        Ok(SquashOutcome::Squashed {
            sha: new_sha,
            collapsed,
            backup_ref,
        })
    }

    /// Move `feature_branch` back to the tip recorded before the squash.
    pub async fn restore_pre_squash(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or("local");
        let safe_dir = paths::shell_escape_posix(repo_dir);
        let backup_ref = backup_ref_for(feature_branch);
        let safe_backup = paths::shell_escape_posix(&backup_ref);

        let old = self
            .exec
            .run_command(
                machine_str,
                &format!("git -C {} rev-parse {}", safe_dir, safe_backup),
            )
            .await
            .map_err(|_| format!("no pre-squash backup recorded for {}", feature_branch))?
            .trim()
            .to_string();

        // Unlike the squash, this moves the branch to a *different* tree, so
        // a worktree holding it checked out has to be brought along with a
        // `reset --hard`. Only when `repo_dir` actually has the feature
        // branch checked out, though — a blind `reset --hard` there would
        // rewrite whatever *other* branch happens to be on HEAD.
        let head = self.get_head_branch(machine_id, repo_dir).await;
        let cmd = if head.as_deref() == Some(feature_branch) {
            format!(
                "git -C {} reset --hard {}",
                safe_dir,
                paths::shell_escape_posix(&old)
            )
        } else {
            format!(
                "git -C {} update-ref refs/heads/{} {}",
                safe_dir,
                paths::shell_escape_posix(feature_branch),
                paths::shell_escape_posix(&old)
            )
        };
        self.exec
            .run_command(machine_str, &cmd)
            .await
            .map(|_| ())
            .map_err(|e| format!("failed to restore {} from backup: {}", feature_branch, e))
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/squash.rs"]
mod tests;

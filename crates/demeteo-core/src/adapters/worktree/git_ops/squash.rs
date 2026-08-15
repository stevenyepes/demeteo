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
use crate::ports::execution::ProgramRequest;
use crate::ports::worktree_ops::{CommitMessageRejected, SquashOutcome};

/// Where the pre-squash tip is parked so the rewrite is undoable.
fn backup_ref_for(feature_branch: &str) -> String {
    format!("refs/demeteo/pre-squash/{}", feature_branch)
}

/// The hook `git rev-parse --git-path` named, absolute, in the spelling of the
/// machine that answered — `None` when it named nothing.
///
/// Both halves go through [`crate::paths`] rather than [`std::path::Path`],
/// whose rustdoc carries why: a Windows desktop driving a Linux machine reads
/// `/…/hooks/commit-msg` back from it, which `Path::is_absolute` calls relative
/// and `Path::join` then rewrites with a backslash. The hook that name no
/// longer reaches is silently never run, so every squashed message goes
/// unvalidated on exactly that topology.
fn hook_path_on(repo_dir: &str, reported: &str, windows_host: bool) -> Option<String> {
    let reported = reported.trim();
    if reported.is_empty() {
        return None;
    }
    if crate::paths::is_absolute_on(reported, windows_host) {
        return Some(reported.to_string());
    }
    Some(crate::paths::join_on(repo_dir, [reported], windows_host))
}

/// How a repository's `commit-msg` hook is started, which is not the same
/// question on both platforms.
///
/// Nothing here is `#[cfg(windows)]`: it is all decision and no syscall, so
/// the Windows answers are reachable from a test on a host that has no
/// Windows — AGENTS.md §3, with the extra edge `shared/win/mod.rs` records,
/// that no Windows cross-compiler runs on the development host. The module as
/// a whole is compiled away where nothing can call it.
#[cfg(any(windows, test))]
mod hook {
    use crate::ports::execution::ProgramRequest;
    use std::path::Path;

    /// What the operating system is asked to start when the hook is run.
    ///
    /// `CreateProcessW` has no `#!` handling, so handing it a shell script earns
    /// "%1 is not a valid Win32 application" — and this module reports whatever
    /// the hook invocation returned to the authoring agent *as the hook's verdict
    /// on its message*. A Windows-local run therefore rejects every message with
    /// a spawn error, which is the one shape of wrong answer the rework loop
    /// cannot act on.
    ///
    /// Git does not have that problem because it resolves the `#!` line itself
    /// (`compat/mingw.c::parse_interpreter`) against the shell it ships, and a
    /// hook Git would run must be a hook Demeteo can run: the entire reason to
    /// run it here is that its answer is the answer a real commit would get.
    ///
    /// Unix reaches none of this — `execve` honours `#!` — so [`Self::Direct`] is
    /// the only variant that platform ever produces.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Launch {
        /// Spawn the file itself; the OS can already start it.
        Direct,
        /// Hand the file to one of Git for Windows' bundled shells as a script.
        Shell(Shell),
    }

    /// Which of the pair [`crate::shared::win::posix_shell::PosixShell`] resolves
    /// a hook asked for.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Shell {
        Sh,
        Bash,
    }

    /// Decide from the hook's name and its text which of the two it is.
    ///
    /// `hook_text` is `None` when the file could not be read as UTF-8, which a
    /// compiled hook cannot be — that one is already startable and needs no
    /// shell.
    pub(super) fn launch(hook_path: &str, hook_text: Option<&str>) -> Launch {
        let extension = Path::new(&hook_path.replace('\\', "/"))
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase());
        if matches!(extension.as_deref(), Some("exe" | "com" | "bat" | "cmd")) {
            return Launch::Direct;
        }
        let Some(first_line) = hook_text.map(|text| text.lines().next().unwrap_or_default()) else {
            return Launch::Direct;
        };
        let Some(shebang) = first_line.strip_prefix("#!") else {
            // Git's own `commit-msg.sample` carries one, husky's generated hooks
            // since v9 carry none, and `sh` is what a POSIX shell falls back to
            // for a text file it is handed.
            return Launch::Shell(Shell::Sh);
        };
        match shell_from_shebang(shebang) {
            Some(shell) => Launch::Shell(shell),
            // A hook asking for node, python or perl is spawned exactly as it was
            // before, and on Windows fails exactly as loudly. Finding *those*
            // interpreters would be a second search, and
            // `shared/win/posix_shell.rs` deliberately owns the only one.
            None => Launch::Direct,
        }
    }

    /// The shell a `#!` line names, unwrapping `env`: `#!/usr/bin/env bash` is the
    /// portable spelling most generated hooks use, and its interpreter word is
    /// `env` rather than the shell.
    fn shell_from_shebang(shebang: &str) -> Option<Shell> {
        let mut words = shebang.split_whitespace();
        let mut name = base_name(words.next()?);
        if name == "env" {
            name = base_name(words.find(|word| !word.starts_with('-'))?);
        }
        match name {
            "bash" => Some(Shell::Bash),
            "sh" | "dash" | "ash" => Some(Shell::Sh),
            _ => None,
        }
    }

    fn base_name(word: &str) -> &str {
        word.rsplit(['/', '\\']).next().unwrap_or(word)
    }

    /// Rewrite a direct hook invocation into one through `interpreter`, with every
    /// path in the form a script running under Git for Windows' bash accepts.
    ///
    /// Forward slashes, and specifically **not** the `/c/Users/…` MSYS form. Both
    /// are understood by the MSYS programs a hook is built from, but the hook
    /// hands its `$1` on to things like `npx commitlint --edit "$1"`, and MSYS
    /// rewrites a POSIX-looking argument on its way to a native `.exe` while
    /// leaving `C:/Users/…` alone. Backslashes survive that trip too, but they
    /// also reach MSYS's own command-line splitter, where a `\` before a quote is
    /// an escape — and Rust quotes any argument containing a space. `cwd` is not
    /// converted: it goes to `CreateProcessW`, which never reads it as text.
    pub(super) fn through_posix_shell(
        direct: ProgramRequest,
        interpreter: &Path,
    ) -> ProgramRequest {
        let ProgramRequest {
            executable,
            args,
            cwd,
            env,
            timeout,
        } = direct;
        ProgramRequest {
            executable: interpreter.to_string_lossy().into_owned(),
            args: std::iter::once(executable)
                .chain(args)
                .map(|arg| arg.replace('\\', "/"))
                .collect(),
            cwd,
            env,
            timeout,
        }
    }
}

impl GitOpsHelper {
    /// Run the repo's `commit-msg` hook against `message` without committing.
    pub async fn validate_commit_message(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        message: &str,
    ) -> Result<(), CommitMessageRejected> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        // `--git-path` resolves through `core.hooksPath`, so this finds the
        // hook wherever husky (or a bare repo layout) actually put it.
        let hook_path = match self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["rev-parse", "--git-path", "hooks/commit-msg"]),
            )
            .await
        {
            // `--git-path` yields a path relative to the repo root.
            Ok(p) => match hook_path_on(
                repo_dir,
                &p,
                crate::paths::targets_windows_host(machine_str),
            ) {
                Some(path) => path,
                None => return Ok(()),
            },
            // No hooks resolvable — nothing to validate against.
            Err(_) => return Ok(()),
        };

        // Not every repo installs one, and a non-executable file is not a
        // hook as far as git is concerned.
        if !self
            .exec
            .is_executable(machine_str, &hook_path)
            .await
            .unwrap_or(false)
        {
            return Ok(());
        }

        // Ask Git for its admin path rather than joining `<repo>/.git`.
        // In a linked worktree `.git` is a file whose gitdir lives in the
        // primary checkout, so only Git can locate a temp message that both
        // stays outside the working tree and is writable by the host.
        let msg_path = match self
            .exec
            .run_program(
                machine_str,
                git_request(
                    repo_dir,
                    [
                        "rev-parse",
                        "--path-format=absolute",
                        "--git-path",
                        "DEMETEO_COMMIT_MSG",
                    ],
                ),
            )
            .await
        {
            Ok(path) if !path.trim().is_empty() => path.trim().to_string(),
            _ => return Ok(()),
        };
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

        let request = self
            .commit_msg_hook_request(machine_str, repo_dir, &hook_path, &msg_path)
            .await;
        let Some(request) = request else {
            let _ = self.exec.remove_file(machine_str, &msg_path).await;
            return Ok(());
        };
        let result = self.exec.run_program(machine_str, request).await;
        let _ = self.exec.remove_file(machine_str, &msg_path).await;

        result.map(|_| ()).map_err(|hook_output| {
            tracing::info!(
                repo = %repo_dir,
                "validate_commit_message: repo's commit-msg hook rejected the proposed message",
            );
            CommitMessageRejected { hook_output }
        })
    }

    /// The invocation the repo's `commit-msg` hook is run as.
    ///
    /// git runs the hook from the repo root with the message file as `$1`;
    /// that is matched exactly, so a hook resolving `node_modules/.bin`
    /// relative to the root behaves as it does for a real commit. On a
    /// Windows-*local* machine the program actually started may be the shell
    /// instead — see the `hook` module above. A remote machine is a Linux one
    /// and takes the direct path whatever the desktop is running on.
    ///
    /// `None` means the hook cannot be run here at all, which is not a verdict
    /// on the message and so is not reported as one — the same "no opinion"
    /// the missing-hook and unwritable-message paths above already take. The
    /// only way to reach it is a machine with `git.exe` and no `bash.exe`,
    /// where no user-authored script runs either and the `MissingPosixShell`
    /// preflight is what tells the user about the machine.
    async fn commit_msg_hook_request(
        &self,
        machine: &str,
        repo_dir: &str,
        hook_path: &str,
        msg_path: &str,
    ) -> Option<ProgramRequest> {
        let direct = ProgramRequest {
            executable: hook_path.to_string(),
            args: vec![msg_path.to_string()],
            cwd: Some(repo_dir.to_string()),
            ..ProgramRequest::default()
        };

        #[cfg(windows)]
        if crate::domain::ids::MachineId::from(machine).is_local() {
            let hook_text = self.exec.read_file(machine, hook_path).await.ok();
            let hook::Launch::Shell(shell) = hook::launch(hook_path, hook_text.as_deref()) else {
                return Some(direct);
            };
            let resolved = match crate::shared::win::posix_shell::posix_shell() {
                Ok(resolved) => resolved,
                Err(missing) => {
                    tracing::warn!(
                        repo = %repo_dir,
                        reason = %missing,
                        "commit_msg_hook_request: no POSIX shell to run this repo's commit-msg hook with; the message is going unvalidated",
                    );
                    return None;
                }
            };
            return Some(hook::through_posix_shell(
                direct,
                match shell {
                    hook::Shell::Bash => &resolved.bash,
                    hook::Shell::Sh => &resolved.sh,
                },
            ));
        }

        let _ = machine;
        Some(direct)
    }

    /// Collapse `<base_ref>..<feature_branch>` into one commit.
    ///
    /// `base_ref` is [`FeatureOrigin::squash_base`], where the rationale for
    /// that revision lives: whatever it names, everything between it and the
    /// branch tip becomes the one commit this produces.
    ///
    /// [`FeatureOrigin::squash_base`]: crate::domain::feature_origin::FeatureOrigin::squash_base
    pub async fn squash_feature_branch(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        base_ref: &str,
        message: &str,
    ) -> Result<SquashOutcome, String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);

        // Prefer the pushed base branch: it is what the PR will be diffed
        // against. Fall back to the local ref for repos with no origin
        // (tests, air-gapped clones). A fully-qualified `base_ref` has
        // neither: it names a ref this clone's own bootstrap fetch created,
        // which the remote has never heard of, so it is asked for as it
        // stands.
        let remote_tracking =
            (!base_ref.starts_with("refs/")).then(|| format!("refs/remotes/origin/{}", base_ref));
        if remote_tracking.is_some() {
            let _ = self
                .exec
                .run_program(
                    machine_str,
                    git_request(repo_dir, ["fetch", "origin", base_ref]),
                )
                .await;
        }
        let base = {
            let mut found = None;
            for cand in remote_tracking
                .as_deref()
                .into_iter()
                .chain(std::iter::once(base_ref))
            {
                if let Ok(sha) = self
                    .exec
                    .run_program(
                        machine_str,
                        git_request(repo_dir, ["merge-base", cand, feature_branch]),
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
                    "cannot squash {}: no merge base with {}",
                    feature_branch, base_ref
                )
            })?
        };

        let tip = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["rev-parse", feature_branch]),
            )
            .await
            .map_err(|e| format!("cannot resolve {}: {}", feature_branch, e))?
            .trim()
            .to_string();

        // Nothing to do when the branch adds no commits, or adds commits
        // whose net effect on the tree is nil (e.g. a change and its
        // revert). Either way there is no PR worth opening.
        let collapsed: u32 = self
            .exec
            .run_program(
                machine_str,
                git_request(
                    repo_dir,
                    ["rev-list", "--count", &format!("{base}..{feature_branch}")],
                ),
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
            .run_program(
                machine_str,
                git_request(
                    repo_dir,
                    ["rev-parse", &format!("{feature_branch}^{{tree}}")],
                ),
            )
            .await
            .map_err(|e| format!("cannot resolve tree of {}: {}", feature_branch, e))?
            .trim()
            .to_string();
        let base_tree = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["rev-parse", &format!("{base}^{{tree}}")]),
            )
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
        let backup_exists = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["rev-parse", "--verify", "-q", &backup_ref]),
            )
            .await
            .is_ok();
        if !backup_exists {
            self.exec
                .run_program(
                    machine_str,
                    git_request(repo_dir, ["update-ref", &backup_ref, &tip]),
                )
                .await
                .map_err(|e| format!("failed to record pre-squash backup ref: {}", e))?;
        }

        // Message via file, never argv: it is multi-line and carries
        // arbitrary text the agent wrote.
        let msg_path = self
            .exec
            .run_program(
                machine_str,
                git_request(
                    repo_dir,
                    [
                        "rev-parse",
                        "--path-format=absolute",
                        "--git-path",
                        "DEMETEO_SQUASH_MSG",
                    ],
                ),
            )
            .await
            .map_err(|e| format!("failed to resolve squash commit message path: {e}"))?
            .trim()
            .to_string();
        self.exec
            .write_file_bytes(machine_str, &msg_path, message.as_bytes())
            .await
            .map_err(|e| format!("failed to stage squash commit message: {}", e))?;

        // Same identity the per-step commits already use (`declared.rs`),
        // so a repo with no configured user still gets a valid commit.
        let commit_res = self
            .exec
            .run_program(
                machine_str,
                git_request(
                    repo_dir,
                    [
                        "-c",
                        "user.email=demeteo@local",
                        "-c",
                        "user.name=demeteo",
                        "commit-tree",
                        &tree,
                        "-p",
                        &base,
                        "-F",
                        &msg_path,
                    ],
                ),
            )
            .await;
        let _ = self.exec.remove_file(machine_str, &msg_path).await;
        let new_sha = commit_res
            .map_err(|e| format!("failed to build squashed commit: {}", e))?
            .trim()
            .to_string();

        // Compare-and-swap: if anything moved the branch while we were
        // working, fail rather than clobber it.
        self.exec
            .run_program(
                machine_str,
                git_request(
                    repo_dir,
                    [
                        "update-ref",
                        &format!("refs/heads/{feature_branch}"),
                        &new_sha,
                        &tip,
                    ],
                ),
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
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let backup_ref = backup_ref_for(feature_branch);

        let old = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["rev-parse", &backup_ref]),
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
        let args = if head.as_deref() == Some(feature_branch) {
            vec!["reset".to_string(), "--hard".to_string(), old.clone()]
        } else {
            vec![
                "update-ref".to_string(),
                format!("refs/heads/{feature_branch}"),
                old.clone(),
            ]
        };
        self.exec
            .run_program(machine_str, git_request_vec(repo_dir, args))
            .await
            .map(|_| ())
            .map_err(|e| format!("failed to restore {} from backup: {}", feature_branch, e))
    }
}

fn git_request<const N: usize>(repo_dir: &str, args: [&str; N]) -> ProgramRequest {
    git_request_vec(repo_dir, args.into_iter().map(str::to_string).collect())
}

fn git_request_vec(repo_dir: &str, args: Vec<String>) -> ProgramRequest {
    ProgramRequest {
        executable: "git".to_string(),
        args: [vec!["-C".to_string(), repo_dir.to_string()], args].concat(),
        ..ProgramRequest::default()
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/squash.rs"]
mod tests;

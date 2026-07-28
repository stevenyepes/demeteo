//! The git commands a `sequence` step issues, built in one place.
//!
//! The step reaches for git constantly — pin a commit, rewind a ref, reset a
//! worktree, ask where the anchor merges — and every one of those used to be
//! a hand-written `format!("git -C {} …")` interpolating
//! [`shell_escape_posix`](crate::paths::shell_escape_posix) by hand at each
//! operand. That is a shape where the *omission* is invisible: a missing
//! escape reads exactly like the eleven neighbours that have one, and one of
//! them was in fact missing until it was found by inspection rather than by a
//! failure. Routing every command through a builder that escapes each operand
//! itself makes that class of mistake unavailable rather than merely rare.
//!
//! **This is a plain adapter struct, not a port.** `ExecutionPort` is the one
//! behavioural contract every transport satisfies identically
//! (`AGENTS.md` §2), and nothing here may vary by transport: these are
//! command *strings*, assembled the same way whether they end up in a local
//! `sh -c`, an SSH channel, or the runner. Making it a trait would invite
//! exactly the per-transport divergence the contract exists to forbid. The
//! precedent for the shape is
//! [`GitOpsHelper`](crate::adapters::worktree::git_ops::GitOpsHelper), which
//! holds its `ExecutionPort` the same way.

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::sequence::sha::Sha;
use crate::paths::shell_escape_posix as esc;
use crate::ports::execution::ExecutionPort;

/// A borrowed `ExecutionPort` plus the machine every command runs on.
///
/// Borrowed rather than owned so a caller can build one per stage without
/// cloning the `Arc`, and so it can never outlive the run it belongs to.
pub(crate) struct SequenceGit<'a> {
    exec: &'a dyn ExecutionPort,
    machine: &'a str,
}

impl<'a> SequenceGit<'a> {
    pub(crate) fn new(exec: &'a dyn ExecutionPort, machine: &'a str) -> Self {
        Self { exec, machine }
    }

    async fn run(&self, cmd: &str) -> Result<String, String> {
        self.exec.run_command(self.machine, cmd).await
    }

    /// Resolve `rev` — a branch name, `HEAD`, anything git accepts — to the
    /// commit it names.
    ///
    /// The answer arrives trimmed, via [`Sha::from_output`]; every caller
    /// used to do that itself. An `Ok` carrying an *empty* `Sha` is still
    /// possible and still the caller's to interpret.
    pub(crate) async fn rev_parse(&self, repo: &str, rev: &str) -> Result<Sha, String> {
        self.run(&rev_parse_cmd(repo, rev))
            .await
            .map(|out| Sha::from_output(&out))
    }

    /// Does `rev` name a commit that is still reachable? `Err` is the
    /// answer "no", not a transport failure — git exits non-zero for a
    /// missing object.
    pub(crate) async fn commit_exists(&self, repo: &str, rev: &Sha) -> Result<String, String> {
        self.run(&commit_exists_cmd(repo, rev.as_str())).await
    }

    /// The best common ancestor of two commits.
    pub(crate) async fn merge_base(&self, repo: &str, a: &Sha, b: &Sha) -> Result<String, String> {
        self.run(&merge_base_cmd(repo, a.as_str(), b.as_str()))
            .await
    }

    /// Move the worktree at `repo` — index and working tree included — onto
    /// `sha`.
    pub(crate) async fn reset_hard(&self, repo: &str, sha: &Sha) -> Result<String, String> {
        self.run(&reset_hard_cmd(repo, sha.as_str())).await
    }

    /// Force `branch` to point at `sha`.
    pub(crate) async fn branch_force(
        &self,
        repo: &str,
        branch: &str,
        sha: &Sha,
    ) -> Result<String, String> {
        self.run(&branch_force_cmd(repo, branch, sha.as_str()))
            .await
    }

    /// Point `git_ref` at `sha`.
    pub(crate) async fn update_ref(
        &self,
        repo: &str,
        git_ref: &str,
        sha: &Sha,
    ) -> Result<String, String> {
        self.run(&update_ref_cmd(repo, git_ref, sha.as_str())).await
    }

    /// Delete `git_ref`.
    pub(crate) async fn delete_ref(&self, repo: &str, git_ref: &str) -> Result<String, String> {
        self.run(&delete_ref_cmd(repo, git_ref)).await
    }

    /// The paths that differ between `base` and the worktree at `repo`.
    pub(crate) async fn diff_name_only(&self, repo: &str, base: &Sha) -> Result<String, String> {
        self.run(&diff_name_only_cmd(repo, base.as_str())).await
    }
}

impl ExecutionDriver {
    /// The git surface for this step, bound to the machine the run is on.
    pub(crate) fn sequence_git<'a>(&'a self, machine: &'a str) -> SequenceGit<'a> {
        SequenceGit::new(&*self.exec, machine)
    }
}

// ── Command strings ──────────────────────────────────────────────────────────
//
// Pure and synchronous, so what goes over the wire is assertable without an
// `ExecutionPort` at all. Every interpolated operand goes through `esc`; the
// only text that may reach a command string unescaped is the literal git
// syntax written here.

fn rev_parse_cmd(repo: &str, rev: &str) -> String {
    format!("git -C {} rev-parse {}", esc(repo), esc(rev))
}

fn commit_exists_cmd(repo: &str, rev: &str) -> String {
    format!("git -C {} cat-file -e {}^{{commit}}", esc(repo), esc(rev))
}

fn merge_base_cmd(repo: &str, a: &str, b: &str) -> String {
    format!("git -C {} merge-base {} {}", esc(repo), esc(a), esc(b))
}

fn reset_hard_cmd(repo: &str, sha: &str) -> String {
    format!("git -C {} reset --hard {}", esc(repo), esc(sha))
}

fn branch_force_cmd(repo: &str, branch: &str, sha: &str) -> String {
    format!(
        "git -C {} branch -f {} {}",
        esc(repo),
        esc(branch),
        esc(sha)
    )
}

fn update_ref_cmd(repo: &str, git_ref: &str, sha: &str) -> String {
    format!(
        "git -C {} update-ref {} {}",
        esc(repo),
        esc(git_ref),
        esc(sha)
    )
}

fn delete_ref_cmd(repo: &str, git_ref: &str) -> String {
    format!("git -C {} update-ref -d {}", esc(repo), esc(git_ref))
}

fn diff_name_only_cmd(repo: &str, base: &str) -> String {
    format!("git -C {} diff --name-only {}", esc(repo), esc(base))
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/sequence/git.rs"]
mod tests;

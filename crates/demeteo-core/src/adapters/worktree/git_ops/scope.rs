//! Artifact-scope enforcement for agent steps.
//!
//! Two-layer defense for the "agent writes outside its declared artifacts"
//! class of bugs (e.g. a Research step modifying source code instead of
//! producing the research report):
//!
//! 1. **Spawn-time chmod fence** ([`apply_artifact_scope`]) — restricts the
//!    worktree so only the step's declared artifact paths are writable.
//!    The agent still has `edit: allow` + `bash: allow` in the
//!    `OPENCODE_PERMISSION` env var; the OS denies writes outside the
//!    scope.
//!
//! 2. **Post-step diff guard** ([`verify_and_revert_out_of_scope_writes`]) —
//!    after the agent returns, scan the worktree's working tree for any
//!    path that isn't in the writable set. Revert those paths via
//!    `git checkout` / `rm`, and return the list so the caller can fail
//!    the step. The failure surfaces in the next attempt's retry feedback.
//!
//! Both layers compose: chmod stops honest mistakes and most misbehavior
//! at write time; the diff guard catches anything that bypassed chmod
//! (e.g. via `chmod u+w .` shell escape) before it reaches the feature
//! branch via the merge step.
//!
//! Writable paths are derived from `StepConfig::artifacts[*].capture`:
//! - `LastWriteTo { path }` → the explicit path
//! - `ByName { .. }`, `AllWrites`, `ChangedFiles`, `Diff` → whole worktree
//!   (declaration doesn't constrain where the artifact ends up, so we
//!   allow full write). Today this means `AllWrites` (the sequence
//!   implement step's capture) opts out of scope enforcement — by design.

use std::path::{Path, PathBuf};

use crate::domain::artifact::ArtifactCapture;
use crate::domain::permission::WriteScope;

/// Sentinel writable-path meaning "the whole worktree is writable" (no
/// fence). Emitted for [`WriteScope::All`] / `Implement` steps.
pub(crate) const ALL_WRITES: &str = "__ALL_WRITES__";

/// Sentinel writable-path meaning "nothing in the worktree is writable".
/// Emitted for [`WriteScope::None`] / `ReadOnly` steps *unless* the
/// project provides extra writable paths that explicitly widen the
/// scope. The fence chmods every entry `a-w`; the diff guard reverts
/// *any* change.
pub(crate) const NONE_WRITABLE: &str = "__NONE__";

/// The conventional artifacts directory every artifact-scoped step may
/// write under, even when it declares no explicit `LastWriteTo` path.
pub(crate) const ARTIFACTS_DIR: &str = "artifacts";

/// Normalise a project-declared extra writable path. Rejects absolute
/// paths, empty entries, and any segment that would escape the worktree
/// (e.g. `..`, leading `/`). Returns the canonical repo-relative form
/// (`./foo` → `foo`). Used to prevent an attacker-controlled settings
/// payload from pivoting the fence outside the worktree.
fn normalize_extra_path(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('/') || trimmed.starts_with('\\') {
        return None;
    }
    if Path::new(trimmed).is_absolute() {
        return None;
    }
    let mut clean = PathBuf::new();
    for comp in Path::new(trimmed).components() {
        match comp {
            std::path::Component::Normal(seg) => clean.push(seg),
            std::path::Component::CurDir => {}
            // ParentDir or any prefix/root component is an escape — reject.
            std::path::Component::ParentDir
            | std::path::Component::Prefix(_)
            | std::path::Component::RootDir => return None,
        }
    }
    if clean.as_os_str().is_empty() {
        None
    } else {
        Some(clean)
    }
}

/// Build the final extra-paths list: normalise, deduplicate, preserve
/// input order. Used by [`derive_writable_paths_for_scope`] to merge
/// user-declared exceptions into the capability-derived writable set.
fn normalised_extras(extras: &[String]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for raw in extras {
        if let Some(p) = normalize_extra_path(raw) {
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// Derive writable paths from a step's *capability* write-scope, refined
/// by its declared artifacts and project-level extra writable paths.
/// This is the capability-authoritative entry point used by the agent
/// step handler — the capability decides the posture, declared
/// `LastWriteTo` paths refine where (within an artifact-scoped step) the
/// output lands, and `extra_paths` widens the fence with user-declared
/// tool side-effect directories.
///
/// - [`WriteScope::All`]  → `[__ALL_WRITES__]` (no fence). `extra_paths`
///   is ignored because the whole worktree is already writable.
/// - [`WriteScope::None`] → `[__NONE__]` (deny every write) **unless**
///   `extra_paths` is non-empty, in which case the fence widens to just
///   the extras. Even a `ReadOnly` step may opt into specific tool
///   side-effects (e.g. a coverage analyst that needs `.cache/`).
/// - [`WriteScope::ArtifactsOnly`] → `artifacts/` plus any explicit
///   `LastWriteTo` paths plus the extras. Unconstrained captures
///   (`AllWrites`/`ByName`/`Diff`/`ChangedFiles`) do **not** widen the
///   scope here: the capability is authoritative, so an artifact-scoped
///   step stays fenced to `artifacts/` + extras regardless of capture
///   shape.
pub(crate) fn derive_writable_paths_for_scope(
    scope: WriteScope,
    artifacts: Option<&Vec<crate::domain::artifact::ArtifactDecl>>,
    extra_paths: &[String],
) -> Vec<PathBuf> {
    let extras = normalised_extras(extra_paths);
    match scope {
        WriteScope::All => vec![PathBuf::from(ALL_WRITES)],
        WriteScope::None => {
            if extras.is_empty() {
                vec![PathBuf::from(NONE_WRITABLE)]
            } else {
                extras
            }
        }
        WriteScope::ArtifactsOnly => {
            let mut paths = vec![PathBuf::from(ARTIFACTS_DIR)];
            if let Some(artifacts) = artifacts {
                for decl in artifacts {
                    if let ArtifactCapture::LastWriteTo { path } = &decl.capture {
                        let p = PathBuf::from(path);
                        if !paths.contains(&p) {
                            paths.push(p);
                        }
                    }
                }
            }
            for ex in extras {
                if !paths.contains(&ex) {
                    paths.push(ex);
                }
            }
            paths
        }
    }
}

/// Derive the set of relative paths the step is allowed to write, from
/// its declared `artifacts` config plus project-level extras. Returns
/// an empty vec if the step declares no artifacts and has no extras
/// (caller decides whether to allow all or fail).
///
/// `extra_paths` widens the writable set with project-declared tool
/// side-effect directories (e.g. `target/`). Normalised and deduped.
/// Inconsequential when an unconstrained capture short-circuits to
/// `__ALL_WRITES__` — that's an explicit "whole worktree" opt-out.
pub(crate) fn derive_writable_paths(
    artifacts: Option<&Vec<crate::domain::artifact::ArtifactDecl>>,
    extra_paths: &[String],
) -> Vec<PathBuf> {
    let Some(artifacts) = artifacts else {
        // No artifacts declared — only extras remain as writable paths.
        return normalised_extras(extra_paths);
    };
    let mut paths = Vec::new();
    for decl in artifacts {
        match &decl.capture {
            ArtifactCapture::LastWriteTo { path } => {
                paths.push(PathBuf::from(path));
            }
            ArtifactCapture::ByName { .. }
            | ArtifactCapture::AllWrites
            | ArtifactCapture::ChangedFiles { .. }
            | ArtifactCapture::Diff { .. } => {
                // Unconstrained capture shape → caller must treat the
                // whole worktree as writable (e.g. `s-implement`
                // implement steps writing across the source tree).
                // Returning a sentinel that means "no scope" — the
                // apply function interprets an empty `writable_paths`
                // AND a "all_writes" present as full-write.
                return vec![PathBuf::from("__ALL_WRITES__")];
            }
            _ => {}
        }
    }
    for ex in normalised_extras(extra_paths) {
        if !paths.contains(&ex) {
            paths.push(ex);
        }
    }
    paths
}

/// True if the step's artifact declaration opts out of scope enforcement
/// (i.e. uses `AllWrites` / `ChangedFiles` / `Diff` / `ByName`).
#[cfg(test)]
pub(crate) fn step_declares_full_write(
    artifacts: Option<&Vec<crate::domain::artifact::ArtifactDecl>>,
) -> bool {
    let Some(artifacts) = artifacts else {
        return false;
    };
    artifacts.iter().any(|d| {
        matches!(
            d.capture,
            ArtifactCapture::ByName { .. }
                | ArtifactCapture::AllWrites
                | ArtifactCapture::ChangedFiles { .. }
                | ArtifactCapture::Diff { .. }
        )
    })
}

impl GitOpsHelper {
    /// Apply chmod-based scope fence. Strategy: first make the whole
    /// worktree writable (so newly-created files under a writable path
    /// inherit +w), then chmod `a-w` every top-level entry that isn't
    /// under any declared `writable_paths` path. Idempotent and safe
    /// to call multiple times.
    ///
    /// No-op when `writable_paths` is empty (caller is signaling "no
    /// scope, allow everything") or when the step declares a
    /// full-write capture (e.g. the `s-implement` sequence step).
    pub(crate) async fn apply_artifact_scope(
        &self,
        machine_id: Option<&str>,
        worktree_path: &str,
        writable_paths: &[PathBuf],
    ) -> Result<(), String> {
        let machine = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let wt = Path::new(worktree_path);

        // Full-write opt-out: do nothing. Used by `Implement` steps and
        // the `s-implement` sequence step, whose capability scope is `All`.
        if writable_paths
            .iter()
            .any(|p| p == &PathBuf::from(ALL_WRITES))
        {
            return Ok(());
        }

        // Deny-all: a `ReadOnly` step. Fall through with an *empty*
        // writable set so every top-level entry gets chmod'd `a-w`.
        let deny_all = writable_paths
            .iter()
            .any(|p| p == &PathBuf::from(NONE_WRITABLE));
        let writable_paths: &[PathBuf] = if deny_all { &[] } else { writable_paths };

        if !deny_all && writable_paths.is_empty() {
            // Nothing declared and not an explicit deny → don't chmod
            // anything. Legacy back-compat for steps without a capability
            // or artifacts; the diff guard catches any actual writes.
            return Ok(());
        }

        // 1. Make everything writable first. Cheap and idempotent.
        //    Ensures that any directory created next inherits +w for
        //    its children, regardless of the umask.
        self.exec
            .run_command(
                machine,
                &format!(
                    "chmod -R u+w {}",
                    crate::paths::shell_escape_posix(&wt.to_string_lossy())
                ),
            )
            .await
            .map_err(|e| format!("scope: chmod u+w on {} failed: {}", wt.display(), e))?;

        // 2. Ensure the parent of each writable path exists. We don't
        //    pre-create the leaf — we can't tell whether `artifacts/`
        //    is meant to be a directory the agent writes under or a
        //    file the agent creates. The agent decides at write time;
        //    we just make sure the parent dir exists and is writable.
        for w in writable_paths {
            let abs = wt.join(w);
            if let Some(parent) = abs.parent() {
                if parent > wt && parent.starts_with(wt) {
                    self.exec
                        .run_command(
                            machine,
                            &format!(
                                "mkdir -p {}",
                                crate::paths::shell_escape_posix(&parent.to_string_lossy())
                            ),
                        )
                        .await
                        .map_err(|e| {
                            format!("scope: mkdir -p {} failed: {}", parent.display(), e)
                        })?;
                }
            }
        }

        // 3. Walk the worktree's top-level entries. Every entry NOT
        //    under any writable path gets `chmod -R a-w`. The directory
        //    listing must go through the machine-aware executor —
        //    `std::fs::read_dir` only sees the host filesystem, which
        //    is wrong for remote machines where the worktree lives
        //    under the SSH target's home (e.g. for a `s-plan` step on
        //    remote machine `home`, the host would see no
        //    `/home/<user>/.demeteo/.../<worktree>` and fail with ENOENT).
        let entries = self
            .exec
            .list_dir(machine, &wt.to_string_lossy())
            .await
            .map_err(|e| format!("scope: read_dir({}) failed: {}", wt.display(), e))?;

        let mut protected: Vec<PathBuf> = Vec::new();
        for entry in entries {
            let path = PathBuf::from(&entry.path);
            let rel = path.strip_prefix(wt).unwrap_or(&path).to_path_buf();
            let is_writable = writable_paths
                .iter()
                .any(|w| rel.starts_with(w) || w.starts_with(&rel));
            if !is_writable {
                protected.push(path);
            }
        }

        // `[ -L ]` guards every one of these, and it is load-bearing rather
        // than defensive. `chmod` dereferences the path it is handed on the
        // command line — Linux has no `lchmod`, so a symlink's own mode is
        // not settable and `chmod -R a-w <wt>/node_modules` silently applies
        // to the *target* instead. That target is the feature's shared
        // dependency cache (`link_dependency_caches_cmd` symlinks every entry
        // in `paths::DEPENDENCY_CACHE_DIRS` into it), which lives outside the
        // worktree and outlives this step, so fencing one `ArtifactsOnly`
        // step would leave `npm`/`cargo`/`pip` unable to write for every
        // later step of the feature. Worse, it is one-way: step 1's
        // `chmod -R u+w` above does *not* follow symlinks met during
        // traversal, so nothing restores it and each subsequent step relinks
        // a clean worktree to the same read-only cache and fails identically.
        //
        // Skipping loses no enforcement. A symlink cannot be made read-only
        // in the first place, its target is by construction outside the
        // worktree the fence reasons about, and the post-step diff guard
        // still covers the link itself.
        for p in &protected {
            let escaped = crate::paths::shell_escape_posix(&p.to_string_lossy());
            self.exec
                .run_command(
                    machine,
                    &format!("[ -L {escaped} ] || chmod -R a-w {escaped}"),
                )
                .await
                .map_err(|e| format!("scope: chmod a-w on {} failed: {}", p.display(), e))?;
        }

        Ok(())
    }

    /// Detect any working-tree changes outside the writable set and
    /// revert them. Returns the list of paths that were reverted (empty
    /// list means the step stayed in scope).
    ///
    /// Uses `git status --porcelain` so both modified-tracked and
    /// untracked-new files are caught. Untracked files are removed;
    /// modified tracked files are `git checkout --`'d back.
    pub(crate) async fn verify_and_revert_out_of_scope_writes(
        &self,
        machine_id: Option<&str>,
        worktree_path: &str,
        writable_paths: &[PathBuf],
    ) -> Result<Vec<String>, String> {
        // Full-write opt-out: never revert.
        if writable_paths
            .iter()
            .any(|p| p == &PathBuf::from(ALL_WRITES))
        {
            return Ok(Vec::new());
        }

        // Deny-all (`ReadOnly`): treat the writable set as empty so every
        // change is out of scope and reverted.
        let deny_all = writable_paths
            .iter()
            .any(|p| p == &PathBuf::from(NONE_WRITABLE));
        let writable_paths: &[PathBuf] = if deny_all { &[] } else { writable_paths };

        let machine = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let wt = Path::new(worktree_path);

        let status = self
            .exec
            .run_command(
                machine,
                &format!(
                    "git -C {} status --porcelain",
                    crate::paths::shell_escape_posix(&wt.to_string_lossy())
                ),
            )
            .await
            .unwrap_or_default();

        if status.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut out_of_scope: Vec<String> = Vec::new();
        let mut to_revert_modified: Vec<String> = Vec::new();
        let mut to_remove_untracked: Vec<String> = Vec::new();

        for line in status.lines() {
            if line.len() < 3 {
                continue;
            }
            let xy = &line[..2];
            let path = line[3..].trim();
            // Porcelain v1: paths with spaces or non-ASCII get quoted;
            // strip quotes defensively.
            let path = path.trim_matches('"').to_string();
            if path.is_empty() || path.contains("..") {
                continue;
            }
            let rel = Path::new(&path);
            let in_scope = if deny_all {
                // ReadOnly step: nothing is in scope.
                false
            } else if writable_paths.is_empty() {
                // Legacy back-compat: no scope declared → allow.
                true
            } else {
                writable_paths
                    .iter()
                    .any(|w| rel.starts_with(w) || w.starts_with(rel))
            };
            if !in_scope {
                out_of_scope.push(path.clone());
                if xy.starts_with('?') {
                    to_remove_untracked.push(path);
                } else {
                    to_revert_modified.push(path);
                }
            }
        }

        if out_of_scope.is_empty() {
            return Ok(Vec::new());
        }

        for p in &to_revert_modified {
            let _ = self
                .exec
                .run_command(
                    machine,
                    &format!(
                        "git -C {} checkout -- {}",
                        crate::paths::shell_escape_posix(&wt.to_string_lossy()),
                        crate::paths::shell_escape_posix(p)
                    ),
                )
                .await;
        }
        for p in &to_remove_untracked {
            let _ = self
                .exec
                .run_command(
                    machine,
                    &format!(
                        "rm -f {}",
                        crate::paths::shell_escape_posix(&wt.join(p).to_string_lossy())
                    ),
                )
                .await;
        }

        Ok(out_of_scope)
    }
}

use crate::adapters::worktree::git_ops::GitOpsHelper;

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/scope.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/scope_runtime.rs"]
mod tests_runtime;

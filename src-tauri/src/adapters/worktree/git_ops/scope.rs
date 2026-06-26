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
//!   allow full write). Today this means `AllWrites` (the parallel
//!   implement step's capture) opts out of scope enforcement — by design.

use std::path::{Path, PathBuf};

use crate::domain::artifact::ArtifactCapture;

/// Derive the set of relative paths the step is allowed to write, from
/// its declared `artifacts` config. Returns an empty vec if the step
/// declares no artifacts (caller decides whether to allow all or fail).
pub(crate) fn derive_writable_paths(
    artifacts: Option<&Vec<crate::domain::artifact::ArtifactDecl>>,
) -> Vec<PathBuf> {
    let Some(artifacts) = artifacts else {
        return Vec::new();
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
                // parallel workers fanning out across the source tree).
                // Returning a sentinel that means "no scope" — the
                // apply function interprets an empty `writable_paths`
                // AND a "all_writes" present as full-write.
                return vec![PathBuf::from("__ALL_WRITES__")];
            }
            _ => {}
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
    /// full-write capture (e.g. `s-implement` parallel workers).
    pub(crate) async fn apply_artifact_scope(
        &self,
        machine_id: Option<&str>,
        worktree_path: &str,
        writable_paths: &[PathBuf],
    ) -> Result<(), String> {
        let machine = machine_id.unwrap_or("local");
        let wt = Path::new(worktree_path);

        // Full-write opt-out: do nothing. Used by `s-implement` parallel
        // workers whose artifacts capture is `AllWrites`.
        if writable_paths
            .iter()
            .any(|p| p == &PathBuf::from("__ALL_WRITES__"))
        {
            return Ok(());
        }

        if writable_paths.is_empty() {
            // Nothing declared → don't chmod anything. The step is
            // either a no-artifact step (gate) or a misconfiguration.
            // Failing here would be too strict; leave as-is and let the
            // diff guard catch any actual writes.
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
        //    under any writable path gets `chmod -R a-w`.
        let entries = std::fs::read_dir(wt)
            .map_err(|e| format!("scope: read_dir({}) failed: {}", wt.display(), e))?;

        let mut protected: Vec<PathBuf> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(wt).unwrap_or(&path).to_path_buf();
            let is_writable = writable_paths
                .iter()
                .any(|w| rel.starts_with(w) || w.starts_with(&rel));
            if !is_writable {
                protected.push(path);
            }
        }

        for p in &protected {
            self.exec
                .run_command(
                    machine,
                    &format!(
                        "chmod -R a-w {}",
                        crate::paths::shell_escape_posix(&p.to_string_lossy())
                    ),
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
            .any(|p| p == &PathBuf::from("__ALL_WRITES__"))
        {
            return Ok(Vec::new());
        }

        let machine = machine_id.unwrap_or("local");
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
            let in_scope = writable_paths.is_empty()
                || writable_paths
                    .iter()
                    .any(|w| rel.starts_with(w) || w.starts_with(rel));
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
mod tests {
    use super::*;
    use crate::domain::artifact::{ArtifactCapture, ArtifactDecl, ArtifactMode};

    fn last_write_to(name: &str, path: &str) -> ArtifactDecl {
        ArtifactDecl {
            name: name.into(),
            capture: ArtifactCapture::LastWriteTo { path: path.into() },
            mode: ArtifactMode::Full,
        }
    }

    fn all_writes(name: &str) -> ArtifactDecl {
        ArtifactDecl {
            name: name.into(),
            capture: ArtifactCapture::AllWrites,
            mode: ArtifactMode::Full,
        }
    }

    #[test]
    fn derive_returns_explicit_paths_for_last_write_to() {
        let decls = vec![last_write_to("report", "artifacts/research-report.md")];
        let paths = derive_writable_paths(Some(&decls));
        assert_eq!(paths, vec![PathBuf::from("artifacts/research-report.md")]);
    }

    #[test]
    fn derive_returns_all_writes_sentinel_for_unconstrained_capture() {
        let decls = vec![all_writes("implemented-files")];
        let paths = derive_writable_paths(Some(&decls));
        assert_eq!(paths, vec![PathBuf::from("__ALL_WRITES__")]);
        assert!(step_declares_full_write(Some(&decls)));
    }

    #[test]
    fn derive_empty_for_no_artifacts_declared() {
        let paths = derive_writable_paths(None);
        assert!(paths.is_empty());
        assert!(!step_declares_full_write(None));
    }

    #[test]
    fn derive_mixed_list_with_unconstrained_returns_all_writes_sentinel() {
        // If any capture is unconstrained, the whole worktree is
        // writable. We don't try to merge constraints.
        let decls = vec![
            last_write_to("report", "artifacts/spec.md"),
            all_writes("all"),
        ];
        let paths = derive_writable_paths(Some(&decls));
        assert_eq!(paths, vec![PathBuf::from("__ALL_WRITES__")]);
    }

    #[test]
    fn derive_handles_by_name_as_unconstrained() {
        let decls = vec![ArtifactDecl {
            name: "by-name".into(),
            capture: ArtifactCapture::ByName {
                name: "report".into(),
            },
            mode: ArtifactMode::Full,
        }];
        let paths = derive_writable_paths(Some(&decls));
        assert_eq!(paths, vec![PathBuf::from("__ALL_WRITES__")]);
    }
}

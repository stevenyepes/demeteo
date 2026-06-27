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
                // parallel workers fanning out across the source tree).
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
    /// full-write capture (e.g. `s-implement` parallel workers).
    pub(crate) async fn apply_artifact_scope(
        &self,
        machine_id: Option<&str>,
        worktree_path: &str,
        writable_paths: &[PathBuf],
    ) -> Result<(), String> {
        let machine = machine_id.unwrap_or("local");
        let wt = Path::new(worktree_path);

        // Full-write opt-out: do nothing. Used by `Implement` steps and
        // `s-implement` parallel workers whose capability scope is `All`.
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
        let paths = derive_writable_paths(Some(&decls), &no_extras());
        assert_eq!(paths, vec![PathBuf::from("artifacts/research-report.md")]);
    }

    #[test]
    fn derive_returns_all_writes_sentinel_for_unconstrained_capture() {
        let decls = vec![all_writes("implemented-files")];
        let paths = derive_writable_paths(Some(&decls), &no_extras());
        assert_eq!(paths, vec![PathBuf::from("__ALL_WRITES__")]);
        assert!(step_declares_full_write(Some(&decls)));
    }

    #[test]
    fn derive_empty_for_no_artifacts_declared() {
        let paths = derive_writable_paths(None, &no_extras());
        assert!(paths.is_empty());
        assert!(!step_declares_full_write(None));
    }

    #[test]
    fn derive_returns_extras_when_no_artifacts_declared() {
        // No artifacts but the project opted into extra writable paths.
        let paths =
            derive_writable_paths(None, &["target".to_string(), "node_modules".to_string()]);
        assert_eq!(
            paths,
            vec![PathBuf::from("target"), PathBuf::from("node_modules")]
        );
    }

    #[test]
    fn derive_mixed_list_with_unconstrained_returns_all_writes_sentinel() {
        // If any capture is unconstrained, the whole worktree is
        // writable. We don't try to merge constraints.
        let decls = vec![
            last_write_to("report", "artifacts/spec.md"),
            all_writes("all"),
        ];
        let paths = derive_writable_paths(Some(&decls), &no_extras());
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
        let paths = derive_writable_paths(Some(&decls), &no_extras());
        assert_eq!(paths, vec![PathBuf::from("__ALL_WRITES__")]);
    }

    // ── derive_writable_paths_for_scope (capability-authoritative) ───────

    fn no_extras() -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn scope_all_returns_all_writes_sentinel() {
        let paths = derive_writable_paths_for_scope(WriteScope::All, None, &no_extras());
        assert_eq!(paths, vec![PathBuf::from(ALL_WRITES)]);
    }

    #[test]
    fn scope_all_ignores_extras_because_worktree_is_already_writable() {
        // Implement capability already opens the entire worktree; extras
        // are redundant but should never introduce the NONE sentinel or
        // shadow it.
        let extras = vec!["target/".to_string()];
        let paths = derive_writable_paths_for_scope(WriteScope::All, None, &extras);
        assert_eq!(paths, vec![PathBuf::from(ALL_WRITES)]);
    }

    #[test]
    fn scope_none_returns_none_sentinel_without_extras() {
        let paths = derive_writable_paths_for_scope(WriteScope::None, None, &no_extras());
        assert_eq!(paths, vec![PathBuf::from(NONE_WRITABLE)]);
    }

    #[test]
    fn scope_none_with_extras_widens_past_deny_all() {
        // ReadOnly + extras: the user opted the step into specific tool
        // side-effects (e.g. .cache/coverage). The NONE sentinel is
        // suppressed and the extras become the writable set directly.
        let extras = vec![".cache/coverage".to_string(), "scratch/".to_string()];
        let paths = derive_writable_paths_for_scope(WriteScope::None, None, &extras);
        assert_eq!(
            paths,
            vec![PathBuf::from(".cache/coverage"), PathBuf::from("scratch"),]
        );
        assert!(!paths.iter().any(|p| p == &PathBuf::from(NONE_WRITABLE)));
    }

    #[test]
    fn scope_artifacts_defaults_to_artifacts_dir_when_no_decls() {
        let paths = derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, None, &no_extras());
        assert_eq!(paths, vec![PathBuf::from(ARTIFACTS_DIR)]);
    }

    #[test]
    fn scope_artifacts_includes_explicit_last_write_to_paths() {
        let decls = vec![last_write_to("spec", "artifacts/spec.md")];
        let paths =
            derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, Some(&decls), &no_extras());
        assert_eq!(
            paths,
            vec![
                PathBuf::from(ARTIFACTS_DIR),
                PathBuf::from("artifacts/spec.md")
            ]
        );
    }

    #[test]
    fn scope_artifacts_does_not_widen_for_unconstrained_capture() {
        // Even if an artifact-scoped step declares AllWrites, the
        // capability is authoritative: it stays fenced to artifacts/.
        let decls = vec![all_writes("everything")];
        let paths =
            derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, Some(&decls), &no_extras());
        assert_eq!(paths, vec![PathBuf::from(ARTIFACTS_DIR)]);
        assert!(!paths.contains(&PathBuf::from(ALL_WRITES)));
    }

    #[test]
    fn scope_artifacts_appends_extras_after_artifacts_dir() {
        // The canonical use case: a Verify step running `cargo test` on
        // a Rust project. The chmod fence must leave `target/` writable
        // while keeping source read-only.
        let extras = vec!["target/".to_string()];
        let paths = derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, None, &extras);
        assert_eq!(
            paths,
            vec![PathBuf::from(ARTIFACTS_DIR), PathBuf::from("target")]
        );
    }

    #[test]
    fn scope_artifacts_dedups_extras_that_overlap_artifacts_dir() {
        // If the user lists `artifacts/` again it must not be appended.
        let extras = vec!["artifacts/".to_string(), "artifacts/extra.md".to_string()];
        let paths = derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, None, &extras);
        assert_eq!(
            paths,
            vec![
                PathBuf::from(ARTIFACTS_DIR),
                PathBuf::from("artifacts/extra.md"),
            ]
        );
    }

    // ── extras normalisation (security boundary) ─────────────────────────

    #[test]
    fn extras_normalisation_strips_trailing_slashes() {
        let paths = derive_writable_paths_for_scope(
            WriteScope::ArtifactsOnly,
            None,
            &["target".to_string(), "node_modules".to_string()],
        );
        assert_eq!(
            paths,
            vec![
                PathBuf::from(ARTIFACTS_DIR),
                PathBuf::from("target"),
                PathBuf::from("node_modules"),
            ]
        );
    }

    #[test]
    fn extras_normalisation_rejects_absolute_paths() {
        // Absolute paths would escape the worktree root. The orchestrator
        // runs on Unix hosts today, where `Path::is_absolute` only
        // recognises paths with a leading `/`; a Windows drive prefix
        // would be treated as a relative path by the shell anyway.
        let extras = vec!["/etc/passwd".to_string(), "/var/log/syslog".to_string()];
        let paths = derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, None, &extras);
        assert_eq!(paths, vec![PathBuf::from(ARTIFACTS_DIR)]);
    }

    #[test]
    fn extras_normalisation_rejects_parent_dir_escape() {
        // `../foo` would land outside the worktree.
        let extras = vec![
            "../escape".to_string(),
            "ok/../../escape".to_string(),
            "safe".to_string(),
        ];
        let paths = derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, None, &extras);
        assert_eq!(
            paths,
            vec![PathBuf::from(ARTIFACTS_DIR), PathBuf::from("safe")]
        );
    }

    #[test]
    fn extras_normalisation_dedups_repeated_entries() {
        let extras = vec![
            "target".to_string(),
            "target/".to_string(),
            "./target".to_string(),
        ];
        let paths = derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, None, &extras);
        assert_eq!(
            paths,
            vec![PathBuf::from(ARTIFACTS_DIR), PathBuf::from("target")]
        );
    }

    #[test]
    fn extras_normalisation_skips_empty_entries() {
        let extras = vec!["".to_string(), "   ".to_string(), "target".to_string()];
        let paths = derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, None, &extras);
        assert_eq!(
            paths,
            vec![PathBuf::from(ARTIFACTS_DIR), PathBuf::from("target")]
        );
    }
}

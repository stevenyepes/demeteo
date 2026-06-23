//! Centralised helpers for computing the on-disk paths Demeteo uses to
//! store project state and cloned repositories.
//!
//! Two motivations drove the extraction into a single module:
//!
//! 1. **Single source of truth.** The bootstrap, workspace health check,
//!    and step executor all need the *same* target directory for a given
//!    project. A divergence here produces the classic "workspace says
//!    CLONED but the agent can't find the dir" failure where the health
//!    check probes one path and the agent `cd`s into another.
//! 2. **No `~` expansion in path construction.** Previously the codebase
//!    computed paths like `~/.demeteo/projects/<id>/repos/<name>` and
//!    relied on bash to expand `~` inside the SSH command. That works
//!    most of the time, but it ties us to the remote shell's expansion
//!    rules. If HOME is unset, the user has been renamed, or the remote
//!    is configured with a non-standard passwd entry, the agent's
//!    `cd ~/.demeteo/...` will silently land in the wrong place. We
//!    now resolve HOME once via [`ExecutionPort::resolve_home`] and use
//!    the absolute path everywhere.
//!
//! This module is also the single source of truth for small primitives
//! shared by many callers:
//!
//! * [`shell_escape_posix`] — single-quote-escape a string for safe
//!   inclusion in a POSIX shell command. Was duplicated in 5 files.
//! * [`now_ms`] / [`now_secs`] — monotonic timestamp helpers used by
//!   the database adapters, intercept payload, and command handlers.
//! * [`new_id`] — short hex ID built from the wall clock and the
//!   current thread id. Collision-resistant enough for in-app IDs.
//!
//! All public path functions take an [`ExecutionPort`] (so the remote
//! HOME can be resolved) and return absolute paths.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ports::execution::ExecutionPort;

/// The subdirectory of the user's HOME in which Demeteo stores all
/// project state. Kept under a single hidden directory so a `rm -rf`
/// from the user can't accidentally nuke it, and so a single
/// `du -sh ~/.demeteo` answers "how much disk is Demeteo using?".
pub const DEMETEO_HOME_SUBDIR: &str = ".demeteo";

/// The subdirectory under [`DEMETEO_HOME_SUBDIR`] where individual
/// projects live.
pub const PROJECTS_SUBDIR: &str = "projects";

/// The subdirectory under each project where the cloned repository
/// working trees live.
pub const REPOS_SUBDIR: &str = "repos";

/// Resolve the Demeteo project root for `project_id` on the target host.
///
/// For local projects this is `<home>/.demeteo/projects/<project_id>`.
/// For remote projects this is the remote HOME + the same suffix, with
/// the remote HOME obtained by calling
/// [`ExecutionPort::resolve_home`] so we never depend on `~` expansion
/// in the SSH command.
///
/// `compute_type` is the project's `compute_type` field
/// (`"local"` or `"remote"`); `remote_host` is `Some(<machine_id>)`
/// for remote projects and `None` for local.
pub async fn project_root(
    exec: &Arc<dyn ExecutionPort>,
    compute_type: &str,
    remote_host: Option<&str>,
    project_id: &str,
) -> Result<PathBuf, String> {
    let home = resolve_home(exec, compute_type, remote_host).await?;
    if compute_type.eq_ignore_ascii_case("local") {
        #[cfg(target_os = "macos")]
        {
            Ok(home
                .join("Library")
                .join("Application Support")
                .join("com.jsteven.demeteo")
                .join("projects")
                .join(project_id))
        }
        #[cfg(target_os = "windows")]
        {
            Ok(home
                .join("AppData")
                .join("Local")
                .join("com.jsteven.demeteo")
                .join("projects")
                .join(project_id))
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Ok(home
                .join(".local")
                .join("share")
                .join("com.jsteven.demeteo")
                .join("projects")
                .join(project_id))
        }
    } else {
        Ok(home
            .join(DEMETEO_HOME_SUBDIR)
            .join(PROJECTS_SUBDIR)
            .join(project_id))
    }
}

/// Resolve the absolute path of a cloned repository's working tree.
///
/// For a project with `id = p1781624953648` and `repo_path =
/// "prototype/spectacular"`, this returns
/// `<home>/.demeteo/projects/p1781624953648/repos/spectacular`.
///
/// The returned path is absolute and contains no `~`, so it's safe to
/// pass to `git -C`, `cd`, or SFTP calls without further shell
/// expansion.
pub async fn repo_target_dir(
    exec: &Arc<dyn ExecutionPort>,
    compute_type: &str,
    remote_host: Option<&str>,
    project_id: &str,
    repo_path: &str,
) -> Result<PathBuf, String> {
    let repo_name = repo_name_from_path(repo_path);
    Ok(project_root(exec, compute_type, remote_host, project_id)
        .await?
        .join(REPOS_SUBDIR)
        .join(repo_name))
}

/// Same as [`repo_target_dir`] but returns a `String` (the form most
/// existing callers want when building shell commands).
pub async fn repo_target_dir_str(
    exec: &Arc<dyn ExecutionPort>,
    compute_type: &str,
    remote_host: Option<&str>,
    project_id: &str,
    repo_path: &str,
) -> Result<String, String> {
    repo_target_dir(exec, compute_type, remote_host, project_id, repo_path)
        .await
        .map(|p| p.to_string_lossy().to_string())
}

/// Extract the repository name (last `/`-separated segment) from a
/// `repo_path` like `"prototype/spectacular"`.
pub fn repo_name_from_path(repo_path: &str) -> String {
    repo_path
        .split('/')
        .rfind(|s| !s.is_empty())
        .unwrap_or(repo_path)
        .to_string()
}

/// Resolve the absolute home directory for the target host.
///
/// The implementation just delegates to
/// [`ExecutionPort::resolve_home`]; the wrapper exists so the
/// `local` / `remote` discrimination lives in one place.
async fn resolve_home(
    exec: &Arc<dyn ExecutionPort>,
    compute_type: &str,
    remote_host: Option<&str>,
) -> Result<PathBuf, String> {
    let machine_id = if compute_type.eq_ignore_ascii_case("local") {
        "local"
    } else {
        remote_host.ok_or_else(|| {
            "Remote project has no `remote_host` set; cannot resolve HOME".to_string()
        })?
    };
    let home_str = exec
        .resolve_home(machine_id)
        .await
        .map_err(|e| format!("Failed to resolve HOME on '{}': {}", machine_id, e))?;
    Ok(PathBuf::from(home_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_name_from_path_handles_typical_inputs() {
        assert_eq!(repo_name_from_path("prototype/spectacular"), "spectacular");
        assert_eq!(repo_name_from_path("spectacular"), "spectacular");
        assert_eq!(repo_name_from_path("a/b/c/d"), "d");
        assert_eq!(repo_name_from_path("a/b/"), "b");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared primitives (shell escaping, time, IDs)
//
// These were previously duplicated across `commands/project.rs`,
// `commands/bootstrap.rs`, `commands/workflows.rs`, `commands/providers.rs`,
// `adapters/ssh/client.rs`, `adapters/step_executor/mod.rs`,
// `adapters/worktree/git_ops.rs`, and `domain/intercept.rs`. Each duplicate
// was a near-copy of the same algorithm; a single change to the escape
// strategy (e.g. switch to `printf %q`) used to require touching 5 files.
// The new canonical home is `crate::shared::*`; this module keeps the
// legacy function names verbatim so the migration is incremental. New
// code should prefer `crate::shared::*`.
// ─────────────────────────────────────────────────────────────────────────────

/// Single-quote-escape `s` for safe inclusion in a POSIX shell command.
///
/// * `~` and `~/...` pass through unchanged so the remote shell expands them.
/// * Strings made entirely of "safe" characters (alnum + `_-. /=:,@`) are
///   returned verbatim (the fast path; matches the previous local behaviour).
/// * Everything else is wrapped in single quotes with internal `'` escaped
///   via the standard `'\''` trick.
///
/// This is the only POSIX shell escaper in the codebase. If you find
/// yourself reaching for `format!("... {}", something)` to build a shell
/// command, route the `something` through this function.
pub fn shell_escape_posix(s: &str) -> String {
    crate::shared::shell::escape_posix(s)
}

/// Current wall-clock time in milliseconds since the UNIX epoch.
///
/// Used for `created_at` / `updated_at` columns, sidebar ordering, and
/// ad-hoc timing in workflow command handlers. The single source of truth
/// (was duplicated in 4 files).
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Current wall-clock time in seconds since the UNIX epoch. Used by
/// `domain/intercept.rs` to build the `created_at` field on the
/// `permission_requested` payload.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Generate a short, unique-enough identifier for in-app entities
/// (workflow rows, step configurations, intercepted commands).
///
/// Not cryptographically random — it's a `DefaultHasher` of the wall
/// clock and the current thread id, formatted as 16 hex digits. Good
/// enough for the "no two rows in the same table share an id" property
/// inside one process; **not** suitable for security tokens.
pub fn new_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut h);
    std::thread::current().id().hash(&mut h);
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod primitive_tests {
    use super::*;

    #[test]
    fn now_ms_is_monotonic_and_positive() {
        let a = now_ms();
        let b = now_ms();
        assert!(a > 0);
        assert!(b >= a);
    }

    #[test]
    fn new_id_is_16_hex_chars_and_unique_enough() {
        let a = new_id();
        let b = new_id();
        assert_eq!(a.len(), 16);
        assert_eq!(b.len(), 16);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

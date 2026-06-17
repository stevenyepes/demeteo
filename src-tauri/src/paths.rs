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
//! All public functions in this module take an [`ExecutionPort`] (so the
//! remote HOME can be resolved) and return absolute paths.

use std::path::PathBuf;
use std::sync::Arc;

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
pub fn project_root(
    exec: &Arc<dyn ExecutionPort>,
    compute_type: &str,
    remote_host: Option<&str>,
    project_id: &str,
) -> Result<PathBuf, String> {
    let home = resolve_home(exec, compute_type, remote_host)?;
    Ok(home
        .join(DEMETEO_HOME_SUBDIR)
        .join(PROJECTS_SUBDIR)
        .join(project_id))
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
pub fn repo_target_dir(
    exec: &Arc<dyn ExecutionPort>,
    compute_type: &str,
    remote_host: Option<&str>,
    project_id: &str,
    repo_path: &str,
) -> Result<PathBuf, String> {
    let repo_name = repo_name_from_path(repo_path);
    Ok(project_root(exec, compute_type, remote_host, project_id)?
        .join(REPOS_SUBDIR)
        .join(repo_name))
}

/// Same as [`repo_target_dir`] but returns a `String` (the form most
/// existing callers want when building shell commands).
pub fn repo_target_dir_str(
    exec: &Arc<dyn ExecutionPort>,
    compute_type: &str,
    remote_host: Option<&str>,
    project_id: &str,
    repo_path: &str,
) -> Result<String, String> {
    repo_target_dir(exec, compute_type, remote_host, project_id, repo_path)
        .map(|p| p.to_string_lossy().to_string())
}

/// Extract the repository name (last `/`-separated segment) from a
/// `repo_path` like `"prototype/spectacular"`.
pub fn repo_name_from_path(repo_path: &str) -> String {
    repo_path
        .split('/')
        .filter(|s| !s.is_empty())
        .last()
        .unwrap_or(repo_path)
        .to_string()
}

/// Resolve the absolute home directory for the target host.
///
/// The implementation just delegates to
/// [`ExecutionPort::resolve_home`]; the wrapper exists so the
/// `local` / `remote` discrimination lives in one place.
fn resolve_home(
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

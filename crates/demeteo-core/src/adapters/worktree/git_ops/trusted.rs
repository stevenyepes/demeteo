//! Local, handle-relative implementation of the trusted-worktree contract.
//!
//! This deliberately does not share the shell implementation in `worktree`.
//! A pathname checked for links and then handed to Git is still a check/use
//! race. On Unix the directory file descriptor stays open through `exec`, and
//! the child changes directory by descriptor immediately before Git starts.

use super::{git_request_vec, GitOpsHelper};
use crate::ports::worktree_ops::{
    CreateTrustedTerminalWorktreeRequest, DependencyCacheMaterialization,
    MaterializeDependencyCacheRequest, RemoveTrustedTerminalWorktreeRequest,
    TrustedTerminalWorktreeCreated, TrustedTerminalWorktreeRemoved, TrustedWorktreePort,
};
use async_trait::async_trait;

#[cfg(target_os = "macos")]
use std::ffi::CStr;
#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::path::{Component, Path, PathBuf};

impl GitOpsHelper {
    pub(super) fn trusted_target_is_local(machine: Option<&str>) -> Result<(), String> {
        match machine {
            None | Some(crate::domain::ids::LOCAL_MACHINE) => Ok(()),
            Some(_) => Err("trusted worktree operations are not available over SSH".to_string()),
        }
    }
}

#[async_trait]
impl TrustedWorktreePort for GitOpsHelper {
    async fn create_terminal_worktree(
        &self,
        request: CreateTrustedTerminalWorktreeRequest,
    ) -> Result<TrustedTerminalWorktreeCreated, String> {
        Self::trusted_target_is_local(request.target.machine_id())?;
        #[cfg(not(unix))]
        {
            let _ = request;
            return Err("trusted worktree operations are unsupported on Windows: Win32 cannot launch Git relative to a held directory handle without a pathname race".to_string());
        }
        #[cfg(unix)]
        {
            let terminal = request.terminal;
            super::worktree::validate_terminal_branch(&terminal.branch)?;
            let names = relative_name(&terminal.worktree_name)?;
            let repo_name = Path::new(request.target.repository_dir())
                .file_name()
                .ok_or_else(|| "trusted worktree: repository path has no basename".to_string())?
                .to_os_string();
            let base_ref = self
                .terminal_start_point(
                    "local",
                    request.target.repository_dir(),
                    terminal.base_branch.as_deref(),
                )
                .await?;
            let git_dir = self
                .exec
                .run_program(
                    "local",
                    git_request_vec(
                        request.target.repository_dir(),
                        vec!["rev-parse".to_string(), "--absolute-git-dir".to_string()],
                    ),
                )
                .await?
                .trim()
                .to_string();
            if git_dir.is_empty() {
                return Err("trusted worktree: Git returned an empty git directory".to_string());
            }
            let root = request.target.project_root().to_string();
            let repo = request.target.repository_dir().to_string();
            let branch = terminal.branch;
            let start = terminal.base_branch.map(|_| base_ref.clone());
            let requested_branch = branch.clone();
            let created = tokio::task::spawn_blocking(move || {
                let root_fd = open_root(Path::new(&root))?;
                let mut components = vec![
                    std::ffi::OsString::from(crate::paths::TERMINAL_WORKTREES_SUBDIR),
                    repo_name,
                ];
                components.extend(names[..names.len() - 1].iter().cloned());
                let parent = open_or_create_dirs(root_fd, &components)?;
                let leaf = names
                    .last()
                    .ok_or_else(|| "trusted worktree: empty destination".to_string())?;
                ensure_absent(&parent, leaf)?;
                run_git_in(
                    &parent,
                    &git_dir,
                    &repo,
                    &requested_branch,
                    leaf,
                    start.as_deref(),
                )?;
                let physical_parent = fd_path(&parent)?;
                Ok::<_, String>(physical_parent.join(leaf).to_string_lossy().into_owned())
            })
            .await
            .map_err(|e| format!("trusted worktree task panicked: {e}"))??;
            Ok(TrustedTerminalWorktreeCreated {
                worktree: crate::domain::models::WorktreeInfo {
                    path: created,
                    branch: Some(branch),
                    is_locked: false,
                },
                base_ref,
            })
        }
    }

    async fn remove_terminal_worktree(
        &self,
        request: RemoveTrustedTerminalWorktreeRequest,
    ) -> Result<TrustedTerminalWorktreeRemoved, String> {
        Self::trusted_target_is_local(request.target.machine_id())?;
        #[cfg(not(unix))]
        {
            let _ = request;
            return Err("trusted worktree operations are unsupported on Windows: Win32 cannot launch Git relative to a held directory handle without a pathname race".to_string());
        }
        #[cfg(unix)]
        {
            let name = relative_name(&request.worktree_name)?;
            // Removal cannot safely delegate a directory pathname to Git: even
            // with a held parent, Git re-resolves the final component. Refuse
            // rather than claim the old check/list sequence is race-free.
            let _ = (name, request.force);
            Err("trusted worktree removal is not implemented on Unix until Git registration can be retired without re-resolving the destination pathname".to_string())
        }
    }

    async fn materialize_dependency_cache(
        &self,
        request: MaterializeDependencyCacheRequest,
    ) -> Result<DependencyCacheMaterialization, String> {
        Self::trusted_target_is_local(request.target.machine_id())?;
        #[cfg(not(unix))]
        {
            let _ = request;
            return Err("trusted dependency-cache materialization is unsupported on Windows: Win32 has no handle-relative Git-compatible launch path".to_string());
        }
        #[cfg(unix)]
        {
            let root = request.target.project_root().to_string();
            let worktree = request.worktree_dir;
            let cache = request.feature_cache_dir;
            tokio::task::spawn_blocking(move || {
                materialize_cache(Path::new(&root), Path::new(&worktree), Path::new(&cache))
            })
            .await
            .map_err(|e| format!("trusted worktree task panicked: {e}"))?
        }
    }
}

#[cfg(unix)]
fn relative_name(raw: &str) -> Result<Vec<std::ffi::OsString>, String> {
    let path = Path::new(raw);
    if raw.trim().is_empty() || path.is_absolute() || raw.contains('\\') || raw.contains(':') {
        return Err(
            "trusted worktree: name must be a non-empty relative path without traversal"
                .to_string(),
        );
    }
    let parts: Vec<_> = path
        .components()
        .map(|part| match part {
            Component::Normal(value) => Ok(value.to_os_string()),
            _ => {
                Err("trusted worktree: name must be a relative path without traversal".to_string())
            }
        })
        .collect::<Result<_, _>>()?;
    if parts.is_empty() {
        Err("trusted worktree: name must be non-empty".to_string())
    } else {
        Ok(parts)
    }
}

#[cfg(unix)]
fn c_name(name: &std::ffi::OsStr) -> Result<CString, String> {
    CString::new(name.as_bytes()).map_err(|_| "trusted worktree: path contains NUL".to_string())
}

#[cfg(unix)]
fn open_root(path: &Path) -> Result<OwnedFd, String> {
    let name = c_name(path.as_os_str())?;
    // SAFETY: name is NUL-terminated and owned for the call.
    let fd = unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(format!(
            "trusted worktree: cannot open trusted root {} without following links: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: successful open transfers ownership of this fd.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

#[cfg(unix)]
fn open_or_create_dirs(
    mut current: OwnedFd,
    components: &[std::ffi::OsString],
) -> Result<OwnedFd, String> {
    for component in components {
        let name = c_name(component)?;
        // SAFETY: dirfd and name are valid. EEXIST is re-opened below with no-follow.
        if unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o755) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EEXIST) {
                return Err(format!("trusted worktree: mkdirat failed: {error}"));
            }
        }
        // SAFETY: this resolves one component from the held parent only.
        let next = unsafe {
            libc::openat(
                current.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if next < 0 {
            return Err(format!(
                "trusted worktree: refusing non-directory or symlink component: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: successful open transfers ownership.
        current = unsafe { OwnedFd::from_raw_fd(next) };
    }
    Ok(current)
}

#[cfg(unix)]
fn ensure_absent(parent: &OwnedFd, leaf: &std::ffi::OsStr) -> Result<(), String> {
    let name = c_name(leaf)?;
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: parent and name are valid; fstatat does not follow a final link.
    let result = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            &mut stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        return Err("trusted worktree: destination already exists".to_string());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ENOENT) {
        Ok(())
    } else {
        Err(format!(
            "trusted worktree: cannot inspect destination: {error}"
        ))
    }
}

#[cfg(unix)]
fn run_git_in(
    parent: &OwnedFd,
    git_dir: &str,
    repo: &str,
    branch: &str,
    leaf: &std::ffi::OsStr,
    start: Option<&str>,
) -> Result<(), String> {
    let mut command = std::process::Command::new("git");
    command.args([
        "--git-dir",
        git_dir,
        "--work-tree",
        repo,
        "worktree",
        "add",
        "-b",
        branch,
    ]);
    // `arg_os` avoids making the filesystem component UTF-8 just to hand it to Git.
    let mut destination = std::ffi::OsString::from("./");
    destination.push(leaf);
    command.args([destination]);
    if let Some(start) = start {
        command.arg(start);
    }
    let fd = parent.as_raw_fd();
    // SAFETY: `fd` is owned by `parent`, which remains alive until output()
    // returns. pre_exec only calls async-signal-safe fchdir.
    unsafe {
        command.pre_exec(move || {
            if libc::fchdir(fd) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    let output = command
        .output()
        .map_err(|e| format!("trusted worktree: could not start Git: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "trusted worktree: git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

#[cfg(unix)]
fn fd_path(fd: &OwnedFd) -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    {
        return std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd()))
            .map_err(|e| format!("trusted worktree: cannot read held directory path: {e}"));
    }
    #[cfg(target_os = "macos")]
    {
        let mut buffer = [0i8; libc::PATH_MAX as usize];
        // SAFETY: buffer is PATH_MAX bytes and fd is a live directory descriptor.
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) } == -1 {
            return Err(format!(
                "trusted worktree: cannot resolve held directory path: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: F_GETPATH writes a NUL-terminated path on success.
        return Ok(PathBuf::from(std::ffi::OsStr::from_bytes(
            unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_bytes(),
        )));
    }
    #[allow(unreachable_code)]
    Err("trusted worktree: this Unix target cannot report a descriptor path".to_string())
}

#[cfg(unix)]
fn materialize_cache(
    root: &Path,
    worktree: &Path,
    cache: &Path,
) -> Result<DependencyCacheMaterialization, String> {
    let root_fd = open_root(root)?;
    let worktree_fd = open_existing_dirs(
        root_fd.try_clone().map_err(|e| e.to_string())?,
        root,
        worktree,
    )?;
    let cache_fd = open_existing_dirs(root_fd, root, cache)?;
    let cache_path = fd_path(&cache_fd)?;
    let mut materialized = Vec::new();
    let mut absent = Vec::new();
    for &name in crate::paths::DEPENDENCY_CACHE_DIRS {
        let source = c_name(std::ffi::OsStr::new(name))?;
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: source is resolved from the held cache directory without following a final link.
        if unsafe {
            libc::fstatat(
                cache_fd.as_raw_fd(),
                source.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
                absent.push(name.to_string());
                continue;
            }
            return Err(format!(
                "trusted worktree: cannot inspect cache {name}: {}",
                std::io::Error::last_os_error()
            ));
        }
        if stat.st_mode & libc::S_IFMT != libc::S_IFDIR {
            return Err(format!(
                "trusted worktree: cache {name} is not a real directory"
            ));
        }
        ensure_absent(&worktree_fd, std::ffi::OsStr::new(name))?;
        let target = c_name(cache_path.join(name).as_os_str())?;
        // SAFETY: target and link name are valid; link name is relative to held worktree fd.
        if unsafe { libc::symlinkat(target.as_ptr(), worktree_fd.as_raw_fd(), source.as_ptr()) }
            != 0
        {
            return Err(format!(
                "trusted worktree: cannot materialize cache {name}: {}",
                std::io::Error::last_os_error()
            ));
        }
        materialized.push(name.to_string());
    }
    Ok(DependencyCacheMaterialization {
        materialized,
        absent,
    })
}

#[cfg(unix)]
fn open_existing_dirs(
    root: OwnedFd,
    root_path: &Path,
    candidate: &Path,
) -> Result<OwnedFd, String> {
    let relative = candidate.strip_prefix(root_path).map_err(|_| {
        format!(
            "trusted worktree: {} is not below trusted root {}",
            candidate.display(),
            root_path.display()
        )
    })?;
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => Ok(value.to_os_string()),
            _ => Err("trusted worktree: path below root contains traversal".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut current = root;
    for component in components {
        let name = c_name(&component)?;
        // SAFETY: this resolves exactly one component from a held parent.
        let next = unsafe {
            libc::openat(
                current.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if next < 0 {
            return Err(format!(
                "trusted worktree: refusing missing, non-directory, or symlink component: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: successful open transfers ownership.
        current = unsafe { OwnedFd::from_raw_fd(next) };
    }
    Ok(current)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn refuses_traversal_in_terminal_names() {
        assert!(relative_name("../outside").is_err());
        assert!(relative_name("nested/../outside").is_err());
        assert!(relative_name("/outside").is_err());
    }

    #[test]
    fn refuses_a_symlink_component_after_opening_root() {
        use std::os::unix::fs::symlink;

        let base =
            std::env::temp_dir().join(format!("demeteo_trusted_worktree_{}", std::process::id()));
        let outside = base.with_extension("outside");
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&base).expect("creates root");
        std::fs::create_dir_all(&outside).expect("creates outside");
        symlink(&outside, base.join("linked")).expect("creates link");
        let root = open_root(&base).expect("opens root");
        let error = open_or_create_dirs(root, &[std::ffi::OsString::from("linked")])
            .expect_err("must not traverse a symlink");
        assert!(error.contains("symlink") || error.contains("Not a directory"));
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&outside);
    }
}

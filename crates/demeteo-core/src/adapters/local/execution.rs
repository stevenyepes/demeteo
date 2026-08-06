use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::domain::models::Platform;
use crate::ports::execution::SftpEntry;
use crate::ports::execution::{ExecutionPort, InteractiveHandle, ProgramRequest, ShellOptions};
use crate::shared::fs_remove;
use crate::shared::proc::{harden_child_spawn, sanitize_child_env};

use super::invocation::{git_would_run_hook, program_path, unspawnable_arguments};
use super::process_guard::ProcessGuard;
use super::run::{local_run_command_async, local_run_program, local_run_program_blocking};

pub struct LocalSubprocessAdapter;

impl Default for LocalSubprocessAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalSubprocessAdapter {
    pub fn new() -> Self {
        Self
    }
}

struct LocalChildProcess {
    child: Arc<Mutex<std::process::Child>>,
    stdin: Arc<Mutex<Option<std::process::ChildStdin>>>,
    stdout: Arc<Mutex<Option<BufReader<std::process::ChildStdout>>>>,
    _stderr: Arc<Mutex<Option<BufReader<std::process::ChildStderr>>>>,
    guard: ProcessGuard,
}

impl LocalChildProcess {
    fn new(mut child: std::process::Child, guard: ProcessGuard) -> Self {
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().map(BufReader::new);
        let stderr = child.stderr.take().map(BufReader::new);
        Self {
            child: Arc::new(Mutex::new(child)),
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(stdout)),
            _stderr: Arc::new(Mutex::new(stderr)),
            guard,
        }
    }
}

impl InteractiveHandle for LocalChildProcess {
    fn write_line(&self, line: &str) -> std::io::Result<usize> {
        let mut stdin = self.stdin.lock().unwrap();
        let Some(ref mut stdin) = *stdin else {
            return Ok(0);
        };
        stdin.write_all(line.as_bytes())?;
        stdin.write_all(b"\n")?;
        stdin.flush()?;
        Ok(line.len() + 1)
    }

    fn try_read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut stdout = self.stdout.lock().unwrap();
        let Some(ref mut stdout) = *stdout else {
            return Ok(0);
        };
        stdout.read(buf)
    }

    fn kill(&self) -> Result<(), String> {
        self.guard.terminate();
        let mut child = self.child.lock().unwrap();
        match child.kill() {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }

    fn try_wait(&self) -> Result<Option<i32>, String> {
        let mut child = self.child.lock().unwrap();
        match child.try_wait() {
            Ok(Some(status)) => Ok(status.code()),
            Ok(None) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[async_trait]
impl ExecutionPort for LocalSubprocessAdapter {
    async fn test_connection(&self, _machine_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn run_program(
        &self,
        _machine_id: &str,
        request: ProgramRequest,
    ) -> Result<String, String> {
        local_run_program(request).await
    }

    async fn run_command_with(
        &self,
        _machine_id: &str,
        cmd: &str,
        opts: ShellOptions,
    ) -> Result<String, String> {
        // Natively async (`tokio::process`) rather than a `spawn_blocking`
        // around `Command::output()`. The blocking form could not be stopped:
        // dropping its `JoinHandle` — on a `ShellOptions::timeout`, on a
        // cancelled step — detaches the task and leaves the child running,
        // holding open a worktree the driver was about to delete. This path
        // owns the deadline and kills the process group.
        //
        // `run_command` (no override) delegates here via the trait default
        // with `ShellOptions::default()`.
        local_run_command_async(cmd, &opts).await
    }

    async fn read_file(&self, _machine_id: &str, path: &str) -> Result<String, String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || std::fs::read_to_string(&path))
            .await
            .map_err(|e| format!("blocking task panicked: {}", e))?
            .map_err(|e| format!("Failed to read file: {}", e))
    }

    async fn write_file(&self, _machine_id: &str, path: &str, content: &str) -> Result<(), String> {
        let path = path.to_string();
        let content = content.to_string();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent directories: {}", e))?;
            }
            std::fs::write(&path, &content).map_err(|e| format!("Failed to write file: {}", e))
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn write_file_bytes(
        &self,
        _machine_id: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), String> {
        let path = path.to_string();
        let content = content.to_vec();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent directories: {}", e))?;
            }
            std::fs::write(&path, &content).map_err(|e| format!("Failed to write file: {}", e))
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn create_dir_all(&self, _machine_id: &str, path: &str) -> Result<(), String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&path)
                .map_err(|e| format!("Failed to create directory '{}': {}", path, e))
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    /// The SFTP arm of this method deletes a Git checkout on any target;
    /// `std::fs::remove_dir_all` does not delete one on Windows at all. The
    /// walk in [`fs_remove`] is what closes that gap — see its module doc.
    async fn remove_dir_all(&self, _machine_id: &str, path: &str) -> Result<(), String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            fs_remove::remove_dir_all(std::path::Path::new(&path)).into_result()
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn remove_file(&self, _machine_id: &str, path: &str) -> Result<(), String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            std::fs::remove_file(&path)
                .map_err(|e| format!("Failed to remove file '{}': {}", path, e))
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn is_executable(&self, _machine_id: &str, path: &str) -> Result<bool, String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || -> Result<bool, String> {
            let metadata = std::fs::metadata(&path)
                .map_err(|e| format!("Failed to stat '{}': {}", path, e))?;
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;

                Some(metadata.permissions().mode())
            };
            #[cfg(not(unix))]
            let mode = None;
            Ok(git_would_run_hook(metadata.is_dir(), mode))
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn get_metadata(&self, _machine_id: &str, path: &str) -> Result<SftpEntry, String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || -> Result<SftpEntry, String> {
            let path_buf = std::path::Path::new(&path);
            let meta = std::fs::metadata(&path)
                .map_err(|e| format!("Failed to stat '{}': {}", path, e))?;

            let name = path_buf
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            Ok(SftpEntry {
                name,
                path: path.clone(),
                is_dir: meta.is_dir(),
                size: meta.len(),
                modified,
            })
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn list_dir(&self, _machine_id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || -> Result<Vec<SftpEntry>, String> {
            let entries = std::fs::read_dir(&path)
                .map_err(|e| format!("Failed to read directory '{}': {}", path, e))?;

            let mut list = Vec::new();
            for entry in entries {
                let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
                let path_buf = entry.path();
                let name = path_buf
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();

                if name == "." || name == ".." {
                    continue;
                }

                let meta = entry
                    .metadata()
                    .map_err(|e| format!("Failed to read metadata: {}", e))?;
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                list.push(SftpEntry {
                    name,
                    path: path_buf.to_str().unwrap_or("").to_string(),
                    is_dir: meta.is_dir(),
                    size: meta.len(),
                    modified,
                });
            }

            list.sort_by(|a, b| {
                if a.is_dir != b.is_dir {
                    b.is_dir.cmp(&a.is_dir)
                } else {
                    a.name.cmp(&b.name)
                }
            });

            Ok(list)
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn setup_worktree(
        &self,
        _machine_id: &str,
        repo_path: &str,
        branch: &str,
        sandbox_path: &str,
    ) -> Result<(), String> {
        let repo_path = repo_path.to_string();
        let branch = branch.to_string();
        let sandbox_path = sandbox_path.to_string();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let repo = std::path::PathBuf::from(&repo_path);
            std::fs::create_dir_all(repo.join(".demeteo").join("worktrees"))
                .map_err(|error| format!("Failed to create local worktree directory: {}", error))?;

            let exclude = repo.join(".git").join("info").join("exclude");
            if let Some(parent) = exclude.parent() {
                if parent.is_dir() {
                    let existing = match std::fs::read_to_string(&exclude) {
                        Ok(contents) => contents,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
                        Err(error) => {
                            return Err(format!(
                                "Failed to read Git exclude file '{}': {}",
                                exclude.display(),
                                error
                            ));
                        }
                    };
                    if !existing.lines().any(|line| line == ".demeteo/") {
                        let mut updated = existing;
                        if !updated.is_empty() && !updated.ends_with('\n') {
                            updated.push('\n');
                        }
                        updated.push_str(".demeteo/\n");
                        std::fs::write(&exclude, updated).map_err(|error| {
                            format!(
                                "Failed to update Git exclude file '{}': {}",
                                exclude.display(),
                                error
                            )
                        })?;
                    }
                }
            }

            let output = local_run_program_blocking(ProgramRequest {
                executable: "git".to_string(),
                args: vec![
                    "-C".to_string(),
                    repo_path.clone(),
                    "worktree".to_string(),
                    "add".to_string(),
                    "-b".to_string(),
                    branch,
                    sandbox_path,
                ],
                ..ProgramRequest::default()
            })?;
            println!(
                "[LocalSubprocessAdapter] Git Worktree provisioning output: {}",
                output
            );

            Ok(())
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn control_rpc(
        &self,
        _machine_id: &str,
        _method: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err(
            "Remote runs are only supported on remote Linux machines running \
             demeteo-runner, not the local machine"
                .to_string(),
        )
    }

    async fn resolve_home(&self, _machine_id: &str) -> Result<String, String> {
        #[cfg(windows)]
        let home =
            std::env::var("USERPROFILE").or_else(|_| -> Result<String, std::env::VarError> {
                let drive = std::env::var("HOMEDRIVE")?;
                let path = std::env::var("HOMEPATH")?;
                Ok(format!("{}{}", drive, path))
            });
        #[cfg(not(windows))]
        let home = std::env::var("HOME");
        let home =
            home.map_err(|_| "local home directory environment variable is not set".to_string())?;
        let path = std::path::PathBuf::from(home);
        if !path.is_absolute() {
            return Err(format!(
                "Resolved local home is not absolute: '{}'",
                path.display()
            ));
        }
        Ok(path.to_string_lossy().into_owned())
    }

    /// The one transport that gets this for free: the target *is* the host, so
    /// the answer is the compiler's and costs no probe. It is still routed
    /// through the port rather than read as a `cfg!` at the call site, because
    /// a caller that reads `cfg!` has no way to be right about a remote.
    async fn resolve_platform(&self, _machine_id: &str) -> Result<Platform, String> {
        Platform::from_target_os(std::env::consts::OS).ok_or_else(|| {
            format!(
                "Demeteo does not ship a desktop for '{}'",
                std::env::consts::OS
            )
        })
    }

    async fn resolve_user(&self, _machine_id: &str) -> Result<String, String> {
        // Local agent: forward the GUI process's own USER so the agent
        // sees the same identity the rest of the desktop does. Prefer
        // USER (login identity) over LOGNAME; some minimal macOS GUI
        // launches set only LOGNAME, but USER is what `bash -c 'echo
        // $USER'` and most CLIs look at.
        #[cfg(windows)]
        let user = std::env::var("USERNAME");
        #[cfg(not(windows))]
        let user = std::env::var("USER").or_else(|_| std::env::var("LOGNAME"));
        user.map_err(|_| "local username environment variable is not set".to_string())
    }

    fn spawn_interactive(
        &self,
        _machine_id: &str,
        binary: &str,
        args: &[String],
        cwd: &str,
        env: &HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        let mut cmd = Command::new(program_path(binary));
        cmd.args(args);
        cmd.current_dir(cwd);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        sanitize_child_env(&mut cmd);
        harden_child_spawn(&mut cmd);
        let guard = ProcessGuard::armed();
        let child = cmd.spawn().map_err(|e| {
            unspawnable_arguments(binary, &e)
                .unwrap_or_else(|| format!("failed to spawn '{}': {}", binary, e))
        })?;
        guard.adopt_sync(&child);
        Ok(Box::new(LocalChildProcess::new(child, guard)))
    }
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/local/execution.rs"]
mod tests;

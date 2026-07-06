use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::ports::execution::SftpEntry;
use crate::ports::execution::{ExecutionPort, InteractiveHandle, ShellOptions};
use crate::shared::proc::sanitize_child_env;
use crate::shared::shell;

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
}

impl LocalChildProcess {
    fn new(mut child: std::process::Child) -> Self {
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().map(BufReader::new);
        let stderr = child.stderr.take().map(BufReader::new);
        Self {
            child: Arc::new(Mutex::new(child)),
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(stdout)),
            _stderr: Arc::new(Mutex::new(stderr)),
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
        let mut child = self.child.lock().unwrap();
        child.kill().map_err(|e| e.to_string())
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

/// Run `cmd` locally honouring `opts` (see [`ShellOptions`]). Honours the
/// same login/non-login + cwd + env contract the SSH adapter does, so both
/// transports behave identically for identical options (D2):
/// * login shell ⇒ `bash -l -c <body>` (profile sourced), else `sh -c <body>`;
/// * `cwd` is applied via the child's working directory;
/// * `env` is exported *inside* the body so it wins over a login profile,
///   matching the SSH construction exactly.
fn local_run_command_with(cmd: &str, opts: &ShellOptions) -> Result<String, String> {
    let exports = shell::export_prefix(&opts.env);
    // cwd is applied via `current_dir` below, so it is not baked into the
    // body here (unlike the SSH adapter, which has no separate cwd channel).
    let body = shell::command_body(None, &exports, cmd);

    let mut command = if opts.login_shell {
        let mut c = Command::new("bash");
        c.arg("-l").arg("-c").arg(&body);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(&body);
        c
    };
    if let Some(cwd) = &opts.cwd {
        command.current_dir(cwd);
    }
    sanitize_child_env(&mut command);
    let output = command
        .output()
        .map_err(|e| format!("Failed to execute command: {}", e))?;

    let mut result = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&stderr);
        return Err(format!(
            "Command failed (exit code: {:?}): {}",
            output.status.code(),
            result
        ));
    }

    Ok(result)
}

/// Non-login, default-cwd, no-extra-env convenience used by the adapter's
/// own internal helpers (`setup_worktree`, `resolve_home`). Equivalent to
/// `run_command`.
fn local_run_command(cmd: &str) -> Result<String, String> {
    local_run_command_with(cmd, &ShellOptions::default())
}

#[async_trait]
impl ExecutionPort for LocalSubprocessAdapter {
    async fn test_connection(&self, _machine_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn run_command_with(
        &self,
        _machine_id: &str,
        cmd: &str,
        opts: ShellOptions,
    ) -> Result<String, String> {
        // The underlying `std::process::Command` is sync; run it on
        // the blocking pool so we don't stall the tokio worker
        // thread. The error type stays `String` to match the port
        // signature; the `?` conversions happen inside the closure.
        // `run_command` (no override) delegates here via the trait
        // default with `ShellOptions::default()`.
        let cmd = cmd.to_string();
        tokio::task::spawn_blocking(move || local_run_command_with(&cmd, &opts))
            .await
            .map_err(|e| format!("blocking task panicked: {}", e))?
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
            local_run_command(&format!("mkdir -p {}/.demeteo/worktrees", repo_path))?;

            let git_exclude_cmd = format!(
                "if [ -d \"{0}/.git\" ]; then mkdir -p \"{0}/.git/info\"; if ! grep -q \".demeteo/\" \"{0}/.git/info/exclude\" 2>/dev/null; then echo \".demeteo/\" >> \"{0}/.git/info/exclude\"; fi; fi",
                repo_path
            );
            let _ = local_run_command(&git_exclude_cmd);

            let worktree_add_cmd = format!(
                "git -C \"{}\" worktree add -b \"{}\" \"{}\"",
                repo_path, branch, sandbox_path
            );
            let output = local_run_command(&worktree_add_cmd)?;
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
        let raw = std::env::var("HOME")
            .map_err(|_| "HOME environment variable is not set on the local process".to_string())?;
        tokio::task::spawn_blocking(move || -> Result<String, String> {
            let expanded = if raw == "~" || raw.starts_with("~/") {
                local_run_command("printf %s \"$HOME\"")?
            } else {
                raw
            };
            let trimmed = expanded.trim().to_string();
            if trimmed.is_empty() {
                return Err("Resolved local HOME is empty".to_string());
            }
            if !trimmed.starts_with('/') {
                return Err(format!(
                    "Resolved local HOME is not absolute: '{}'",
                    trimmed
                ));
            }
            Ok(trimmed)
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn resolve_user(&self, _machine_id: &str) -> Result<String, String> {
        // Local agent: forward the GUI process's own USER so the agent
        // sees the same identity the rest of the desktop does. Prefer
        // USER (login identity) over LOGNAME; some minimal macOS GUI
        // launches set only LOGNAME, but USER is what `bash -c 'echo
        // $USER'` and most CLIs look at.
        std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .map_err(|_| "Neither USER nor LOGNAME is set on the local process".to_string())
    }

    fn spawn_interactive(
        &self,
        _machine_id: &str,
        binary: &str,
        args: &[String],
        cwd: &str,
        env: &HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        let mut cmd = Command::new(binary);
        cmd.args(args);
        cmd.current_dir(cwd);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        sanitize_child_env(&mut cmd);
        let child = cmd
            .spawn()
            .map_err(|e| format!("failed to spawn '{}': {}", binary, e))?;
        Ok(Box::new(LocalChildProcess::new(child)))
    }
}

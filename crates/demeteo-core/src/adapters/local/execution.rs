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

/// The `(program, args)` a set of [`ShellOptions`] resolves to. Shared by the
/// sync and async run paths so the two can never drift into invoking different
/// shells for the same options.
///
/// * login shell ⇒ `bash -l -c <body>` (profile sourced), else `sh -c <body>`;
/// * `env` is exported *inside* the body so it wins over a login profile,
///   matching the SSH construction exactly (D2).
///
/// `cwd` is deliberately not baked into the body: the local adapter has a
/// `current_dir` channel the SSH one lacks.
fn shell_invocation(cmd: &str, opts: &ShellOptions) -> (&'static str, Vec<String>) {
    let exports = shell::export_prefix(&opts.env);
    let body = format!(
        "{}{}",
        shell::job_control_prefix(opts.interactive),
        shell::command_body(None, &exports, cmd)
    );

    if opts.login_shell {
        let mut args = vec!["-l".to_string()];
        // Interactive login also sources `~/.bashrc` (mise/asdf/nvm tool
        // activation); see `ShellOptions::interactive`. Kept in lockstep with
        // the SSH adapter so both transports resolve the same PATH (D2).
        if opts.interactive {
            args.push("-i".to_string());
        }
        args.push("-c".to_string());
        args.push(body);
        ("bash", args)
    } else {
        ("sh", vec!["-c".to_string(), body])
    }
}

/// Apply the non-argument half of `opts` to a spawned child.
fn configure_child(command: &mut Command, opts: &ShellOptions) {
    if let Some(cwd) = &opts.cwd {
        command.current_dir(cwd);
    }
    // An interactive login shell (`bash -l -i -c`, used by the availability /
    // model probes so mise/asdf/nvm tools resolve) tries to grab the
    // controlling terminal for job control. When demeteo runs under a terminal
    // (e.g. `tauri dev`), that suspends the whole process group. Detach the
    // child into its own session so it has no controlling TTY. Harmless for the
    // non-interactive paths. See `detach_from_controlling_tty`.
    if opts.interactive {
        crate::shared::proc::detach_from_controlling_tty(command);
    }
    sanitize_child_env(command);
}

/// Assemble the D3 result shape from a finished child: stdout on success,
/// `Err(stdout + stderr)` on a non-zero exit — never `Ok("")`.
fn command_result(
    status_code: Option<i32>,
    ok: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<String, String> {
    let mut result = String::from_utf8_lossy(stdout).to_string();
    if !ok {
        let stderr = String::from_utf8_lossy(stderr);
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&stderr);
        return Err(format!(
            "Command failed (exit code: {:?}): {}",
            status_code, result
        ));
    }
    Ok(result)
}

/// Kill a spawned child's whole **process group** on drop.
///
/// `kill_on_drop` alone is not enough for this adapter: every command runs as
/// `bash -c <body>`, so killing the direct child reaps the shell and orphans
/// whatever it spawned — the `npm test` inside a hung `bash -c "npm test"`
/// would keep running (and keep writing into a worktree that is about to be
/// torn down). Killing the group takes the tree.
///
/// Armed **only** when the child called `setsid` (the `interactive` path,
/// which is what `harness_shell_options` — and therefore every `command` node
/// — uses). A child that did not `setsid` shares *demeteo's own* process
/// group, and `killpg` on that would kill the app. When disarmed the caller
/// still gets `kill_on_drop`'s direct-child kill, which is the correct floor.
struct KillGroupOnDrop {
    pid: Option<u32>,
    own_session: bool,
}

impl KillGroupOnDrop {
    /// The child exited on its own; there is no group left to signal (and a
    /// recycled pid must never be).
    fn disarm(&mut self) {
        self.pid = None;
    }
}

impl Drop for KillGroupOnDrop {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            if let (Some(pid), true) = (self.pid, self.own_session) {
                // SAFETY: `pid` is a child we spawned with `setsid`, so it is
                // its own session and process-group leader — the group can
                // contain nothing but its descendants. ESRCH (already gone) is
                // the expected benign error and is ignored.
                unsafe {
                    libc::killpg(pid as libc::pid_t, libc::SIGKILL);
                }
            }
        }
    }
}

/// Run `cmd` locally honouring `opts`, **owning the deadline** so an expiry
/// actually stops the work (see [`ShellOptions::timeout`]).
///
/// Cancel-safe by construction: the group kill hangs off `Drop`, so abandoning
/// this future — a timeout, a cancelled step, an aborted task — kills the
/// command tree just as the deadline does. That is what lets the `command`
/// node treat "cancelled" as immediate.
async fn local_run_command_async(cmd: &str, opts: &ShellOptions) -> Result<String, String> {
    use tokio::io::AsyncReadExt;

    let (program, args) = shell_invocation(cmd, opts);
    let mut command = tokio::process::Command::new(program);
    command.args(&args);
    configure_child(command.as_std_mut(), opts);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // The floor when the group kill is disarmed (non-`setsid` children).
        .kill_on_drop(true);

    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to execute command: {}", e))?;
    let mut guard = KillGroupOnDrop {
        pid: child.id(),
        own_session: opts.interactive,
    };

    let mut out_pipe = child.stdout.take().expect("stdout piped above");
    let mut err_pipe = child.stderr.take().expect("stderr piped above");
    // Drain both pipes *while* waiting. Waiting first and reading after would
    // deadlock the moment a build fills the 64K pipe buffer.
    let run = async {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let (_, _, status) = tokio::join!(
            out_pipe.read_to_end(&mut stdout),
            err_pipe.read_to_end(&mut stderr),
            child.wait(),
        );
        (stdout, stderr, status)
    };

    let (stdout, stderr, status) = match opts.timeout {
        Some(limit) => match tokio::time::timeout(limit, run).await {
            Ok(finished) => finished,
            Err(_) => {
                // `guard` drops on return and takes the process group with it.
                return Err(format!(
                    "{}command exceeded its {}s ceiling",
                    crate::ports::execution::TIMEOUT_ERROR_PREFIX,
                    limit.as_secs()
                ));
            }
        },
        None => run.await,
    };

    guard.disarm();
    let status = status.map_err(|e| format!("Failed to await command: {}", e))?;
    command_result(status.code(), status.success(), &stdout, &stderr)
}

/// Blocking twin of [`local_run_command_async`] for the adapter's own
/// synchronous helpers (`setup_worktree`, `resolve_home`), which run short
/// fixed commands and need no deadline. `opts.timeout` is not honoured here —
/// [`ExecutionPort::run_command_with`] routes through the async path.
fn local_run_command_with(cmd: &str, opts: &ShellOptions) -> Result<String, String> {
    let (program, args) = shell_invocation(cmd, opts);
    let mut command = Command::new(program);
    command.args(&args);
    configure_child(&mut command, opts);
    let output = command
        .output()
        .map_err(|e| format!("Failed to execute command: {}", e))?;
    command_result(
        output.status.code(),
        output.status.success(),
        &output.stdout,
        &output.stderr,
    )
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

#[cfg(test)]
#[path = "../../../tests/infrastructure/local_execution.rs"]
mod local_execution_tests;

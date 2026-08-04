use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::ports::execution::SftpEntry;
use crate::ports::execution::{ExecutionPort, InteractiveHandle, ProgramRequest, ShellOptions};
use crate::shared::proc::sanitize_child_env;
use crate::shared::shell;

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

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
    #[cfg(windows)]
    job: WindowsJob,
}

impl LocalChildProcess {
    fn new(mut child: std::process::Child) -> Result<Self, String> {
        #[cfg(windows)]
        let job = WindowsJob::attach(child.as_raw_handle() as HANDLE)?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().map(BufReader::new);
        let stderr = child.stderr.take().map(BufReader::new);
        Ok(Self {
            child: Arc::new(Mutex::new(child)),
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(stdout)),
            _stderr: Arc::new(Mutex::new(stderr)),
            #[cfg(windows)]
            job,
        })
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
        #[cfg(windows)]
        self.job.terminate()?;
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

/// Owns a Windows Job Object configured to terminate every assigned process
/// when the owner is dropped. This is the Windows equivalent of the Unix
/// session/process-group guard below: a timeout or cancellation must not
/// orphan grandchildren such as a compiler started by a package manager.
#[cfg(windows)]
struct WindowsJob(HANDLE);

#[cfg(windows)]
impl WindowsJob {
    fn attach(process: HANDLE) -> Result<Self, String> {
        // SAFETY: null name requests an unnamed Job Object owned by this
        // wrapper. The returned handle is closed exactly once in Drop.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(format!(
                "failed to create Windows Job Object: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: limits points to a fully initialized value for the exact
        // information class requested, and handle is valid above.
        let configured = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            // SAFETY: handle was created by CreateJobObjectW and has not moved.
            unsafe { CloseHandle(handle) };
            return Err(format!(
                "failed to configure Windows Job Object: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: process is the raw handle of the newly spawned child and
        // handle remains owned by this wrapper for the child's lifetime.
        if unsafe { AssignProcessToJobObject(handle, process) } == 0 {
            // SAFETY: handle was created above and is still owned here.
            unsafe { CloseHandle(handle) };
            return Err(format!(
                "failed to assign process to Windows Job Object: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self(handle))
    }

    fn terminate(&self) -> Result<(), String> {
        // SAFETY: self.0 is a live Job Object handle until Drop.
        if unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1) } == 0 {
            return Err(format!(
                "failed to terminate Windows Job Object: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    fn disarm(&self) -> Result<(), String> {
        let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        // SAFETY: limits is the exact structure required by this information
        // class and self.0 is valid until Drop.
        if unsafe {
            SetInformationJobObject(
                self.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(format!(
                "failed to disarm Windows Job Object: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        // SAFETY: self.0 is exclusively owned by this wrapper. KILL_ON_JOB_CLOSE
        // ensures all remaining descendants terminate before the handle closes.
        unsafe { CloseHandle(self.0) };
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

/// Execute an argv request directly so owned operations never depend on shell quoting.
async fn local_run_program(request: ProgramRequest) -> Result<String, String> {
    use tokio::io::AsyncReadExt;
    let mut command = tokio::process::Command::new(&request.executable);
    command.args(&request.args);
    if let Some(cwd) = &request.cwd {
        command.current_dir(cwd);
    }
    command.envs(&request.env);
    sanitize_child_env(command.as_std_mut());
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to execute '{}': {}", request.executable, e))?;
    #[cfg(windows)]
    let _job = WindowsJob::attach(child.as_raw_handle() as HANDLE)?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout pipe was not available".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "stderr pipe was not available".to_string())?;
    let run = async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let (_, _, status) = tokio::join!(
            stdout.read_to_end(&mut out),
            stderr.read_to_end(&mut err),
            child.wait()
        );
        (out, err, status)
    };
    let (out, err, status) = match request.timeout {
        Some(limit) => tokio::time::timeout(limit, run).await.map_err(|_| {
            format!(
                "{}program exceeded its {}s ceiling",
                crate::ports::execution::TIMEOUT_ERROR_PREFIX,
                limit.as_secs()
            )
        })?,
        None => run.await,
    };
    #[cfg(windows)]
    _job.disarm()?;
    let status = status.map_err(|e| format!("Failed to await program: {}", e))?;
    command_result(status.code(), status.success(), &out, &err)
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
    #[cfg(windows)]
    let _job = WindowsJob::attach(child.as_raw_handle() as HANDLE)?;
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
    #[cfg(windows)]
    _job.disarm()?;
    let status = status.map_err(|e| format!("Failed to await command: {}", e))?;
    command_result(status.code(), status.success(), &stdout, &stderr)
}

/// Blocking structured-program helper for a few short adapter-owned setup
/// operations. User-authored scripts always go through [`local_run_program`].
fn local_run_program_blocking(request: ProgramRequest) -> Result<String, String> {
    let mut command = Command::new(&request.executable);
    command.args(&request.args);
    if let Some(cwd) = &request.cwd {
        command.current_dir(cwd);
    }
    command.envs(&request.env);
    sanitize_child_env(&mut command);
    let output = command
        .output()
        .map_err(|e| format!("Failed to execute '{}': {}", request.executable, e))?;
    command_result(
        output.status.code(),
        output.status.success(),
        &output.stdout,
        &output.stderr,
    )
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

    async fn remove_dir_all(&self, _machine_id: &str, path: &str) -> Result<(), String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            std::fs::remove_dir_all(&path)
                .map_err(|e| format!("Failed to remove directory '{}': {}", path, e))
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

    async fn set_file_mode(&self, _machine_id: &str, path: &str, mode: u32) -> Result<(), String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                    .map_err(|e| format!("Failed to set permissions on '{}': {}", path, e))
            }
            #[cfg(windows)]
            {
                let _ = mode;
                std::fs::metadata(&path).map(|_| ()).map_err(|e| {
                    format!(
                        "Failed to stat '{}' before setting permissions: {}",
                        path, e
                    )
                })
            }
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
            {
                use std::os::unix::fs::PermissionsExt;

                Ok(!metadata.is_dir() && metadata.permissions().mode() & 0o111 != 0)
            }
            #[cfg(windows)]
            {
                Ok(!metadata.is_dir())
            }
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
        let home = std::env::var("USERPROFILE").or_else(|_| {
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
        Ok(Box::new(LocalChildProcess::new(child)?))
    }
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/local_execution.rs"]
mod local_execution_tests;

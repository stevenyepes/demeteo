use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::ports::execution::SftpEntry;
use crate::ports::execution::{ExecutionPort, InteractiveHandle, ProgramRequest, ShellOptions};
use crate::shared::proc::{harden_child_spawn, sanitize_child_env};
use crate::shared::shell;

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
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

/// What the job limits, and the one thing it deliberately permits.
///
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (`0x2000`) is the reaping: every
/// process still in the job dies when the last handle to it closes, which is
/// what takes a compiler down with the package manager that started it.
///
/// `JOB_OBJECT_LIMIT_BREAKAWAY_OK` (`0x800`) is for Hermes, which launches its
/// gateway with `CREATE_BREAKAWAY_FROM_JOB`; inside a job without this bit that
/// call fails `ERROR_ACCESS_DENIED` and the gateway never starts. Its
/// neighbour `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK` (`0x1000`) grants the same
/// exit without the asking — every ordinary agent child would leave the tree
/// whether it meant to or not, and the reaping above would be reaping an empty
/// job. Do not reach for it because something failed to break away: a process
/// that has to say so is the entire distinction.
#[cfg_attr(not(windows), allow(dead_code))]
const JOB_LIMIT_FLAGS: u32 = 0x2000 | 0x0800;

#[cfg(windows)]
const _: () = {
    use windows_sys::Win32::System::JobObjects::{
        JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
    };

    assert!(JOB_LIMIT_FLAGS == JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK);
    assert!(JOB_LIMIT_FLAGS & JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK == 0);
};

/// The Windows Job Object a spawned tree is confined to — and nothing at all
/// anywhere else, where the process group [`KillGroupOnDrop`] signals is the
/// same guarantee by other means. Callers hold one either way, so no spawn
/// site branches on the platform.
///
/// Constructed **before** the spawn it guards. Job membership is forward-only:
/// a grandchild started before the child is assigned escapes for good, and
/// there is no re-parenting it afterwards — so the two configuration syscalls
/// that used to sit inside that window (`CreateJobObjectW`,
/// `SetInformationJobObject`) are lifted out of it, leaving only
/// `AssignProcessToJobObject`.
///
/// **The window is narrowed, not closed, and nothing here should be read as
/// claiming otherwise.** Closing it means spawning `CREATE_SUSPENDED` and
/// resuming only once the assignment has landed, and `ResumeThread` needs the
/// primary thread's handle — which `std::process` closes on the way out and
/// never exposes. Reaching it costs either an open-coded `CreateProcessW`,
/// re-implementing std's stdio and handle-inheritance plumbing along with it,
/// or a crate; the crate is an AGENTS.md §6 gate and has not been asked for.
///
/// Every operation is best effort. A job that could not be created, could not
/// be configured, or could not adopt its child degrades to `kill_on_drop`'s
/// direct-child kill — the guarantee this adapter had before any job existed —
/// and says so at debug level. Returning `Err` instead, as this once did, makes
/// a teardown guarantee a precondition for running anything at all, and
/// `AssignProcessToJobObject` answers `ERROR_ACCESS_DENIED` for a process that
/// has already exited: a command fast enough to finish first would have failed
/// *because* it succeeded. Cargo's `util/job.rs` degrades for the same reason.
#[derive(Default)]
struct ProcessGuard {
    #[cfg(windows)]
    job: Option<WindowsJob>,
}

impl ProcessGuard {
    /// Create the job the next spawn is to be assigned to.
    fn armed() -> Self {
        Self {
            #[cfg(windows)]
            job: WindowsJob::create(),
        }
    }

    #[cfg_attr(not(windows), allow(unused_variables))]
    fn adopt(&self, child: &tokio::process::Child) {
        // `tokio::process::Child` exposes its handle as an `Option` rather
        // than implementing `AsRawHandle`: the handle is gone once the child
        // has been reaped, and nothing needs confining after that.
        #[cfg(windows)]
        self.assign(child.raw_handle().map(|handle| handle as HANDLE));
    }

    #[cfg_attr(not(windows), allow(unused_variables))]
    fn adopt_sync(&self, child: &std::process::Child) {
        #[cfg(windows)]
        self.assign(Some(child.as_raw_handle() as HANDLE));
    }

    #[cfg(windows)]
    fn assign(&self, process: Option<HANDLE>) {
        let Some(job) = self.job.as_ref() else {
            return;
        };
        match process {
            Some(process) => job.adopt(process),
            None => tracing::debug!("child exited before it could be confined to a job"),
        }
    }

    /// Kill the tree now. The caller kills the direct child regardless, so a
    /// failure here costs the grandchildren and nothing else.
    fn terminate(&self) {
        #[cfg(windows)]
        if let Some(job) = self.job.as_ref() {
            job.terminate();
        }
    }

    /// The command finished on its own, so what is still running is what it
    /// deliberately left running: clearing the limits lets that outlive the
    /// handle. [`KillGroupOnDrop::disarm`] concedes the same thing at the same
    /// point on Unix, and a command whose background daemon survives on one
    /// platform and not the other is a parity break.
    fn disarm(&self) {
        #[cfg(windows)]
        if let Some(job) = self.job.as_ref() {
            job.disarm();
        }
    }
}

#[cfg(windows)]
struct WindowsJob(HANDLE);

// SAFETY: a Win32 `HANDLE` is a process-wide kernel-object reference, not a
// pointer into this process's address space; every operation performed on it
// here (`SetInformationJobObject`, `AssignProcessToJobObject`,
// `TerminateJobObject`, `CloseHandle`) is documented as thread-safe. The
// wrapper owns it exclusively and closes it
// exactly once, in `Drop`. Needed because `LocalChildProcess` is handed out as
// a `Box<dyn InteractiveHandle>`, which the port requires to be `Send + Sync`.
#[cfg(windows)]
unsafe impl Send for WindowsJob {}
#[cfg(windows)]
unsafe impl Sync for WindowsJob {}

#[cfg(windows)]
impl WindowsJob {
    fn create() -> Option<Self> {
        // SAFETY: a null name asks for an unnamed job nothing else can open,
        // and a null attribute pointer for the default security descriptor.
        // The returned handle is owned by the value below and closed exactly
        // once, in Drop.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            tracing::debug!(
                error = %std::io::Error::last_os_error(),
                "no Windows Job Object for this spawn"
            );
            return None;
        }
        let job = Self(handle);
        job.set_limits(JOB_LIMIT_FLAGS).then_some(job)
    }

    fn set_limits(&self, flags: u32) -> bool {
        // SAFETY: every field of this structure is an integer or a pointer,
        // for all of which all-zero is a valid value — it is the documented
        // way to build one, since a set bit in `LimitFlags` is what gives any
        // other field meaning.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = flags;
        // SAFETY: `limits` is a fully initialised value of exactly the type
        // `JobObjectExtendedLimitInformation` names, the length passed is that
        // type's own size, and `self.0` is live until Drop.
        let configured = unsafe {
            SetInformationJobObject(
                self.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            tracing::debug!(
                error = %std::io::Error::last_os_error(),
                flags,
                "Windows Job Object limits not applied"
            );
        }
        configured != 0
    }

    fn adopt(&self, process: HANDLE) {
        // SAFETY: `process` is the raw handle of a child this adapter spawned
        // and still owns, so it cannot be closed or recycled during the call;
        // `self.0` is live until Drop.
        if unsafe { AssignProcessToJobObject(self.0, process) } == 0 {
            tracing::debug!(
                error = %std::io::Error::last_os_error(),
                "child not confined to a job; its own children will outlive a tree kill"
            );
        }
    }

    fn terminate(&self) {
        // SAFETY: self.0 is a live Job Object handle until Drop.
        if unsafe { TerminateJobObject(self.0, 1) } == 0 {
            tracing::debug!(
                error = %std::io::Error::last_os_error(),
                "Windows Job Object not terminated"
            );
        }
    }

    fn disarm(&self) {
        self.set_limits(0);
    }
}

#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        // SAFETY: self.0 is exclusively owned by this wrapper and closed here
        // exactly once. Unless `disarm` ran first, KILL_ON_JOB_CLOSE takes
        // every process still inside with it.
        unsafe { CloseHandle(self.0) };
    }
}

/// The `(program, args)` a set of [`ShellOptions`] resolves to.
///
/// Split in two because only one half varies by platform: [`shell_args`] is
/// the program text and is composed by identical code everywhere, while
/// [`shell_program`] is the file that interprets it. That is the entire
/// Windows difference — see `docs/WINDOWS_PARITY.md`.
fn shell_invocation(cmd: &str, opts: &ShellOptions) -> Result<(PathBuf, Vec<String>), String> {
    Ok((shell_program(opts.login_shell)?, shell_args(cmd, opts)))
}

/// The argv for one user-authored script body.
///
/// * login shell ⇒ `bash -l -c <body>` (profile sourced), else `sh -c <body>`;
/// * `env` is exported *inside* the body so it wins over a login profile,
///   matching the SSH construction exactly (D2).
///
/// `cwd` is deliberately not baked into the body: the local adapter has a
/// `current_dir` channel the SSH one lacks. On Windows that is also what keeps
/// a `C:\…` path out of a body where `\` is an escape character.
fn shell_args(cmd: &str, opts: &ShellOptions) -> Vec<String> {
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
        args
    } else {
        vec!["-c".to_string(), body]
    }
}

/// The interpreter that runs the body: bash for a login shell, sh otherwise.
///
/// On Unix these stay the bare names `execvp` resolves through `PATH`, exactly
/// as before. On Windows they are absolute paths inside the Git for Windows
/// installation [`crate::shared::win::posix_shell`] located, because a bare
/// `bash` there is `C:\Windows\System32\bash.exe` — the WSL launcher, which
/// resolves none of the paths Demeteo passes.
///
/// The bash/sh split is mirrored rather than collapsed so a local `sh -c` and
/// a remote `sh -c` remain the same interpreter family.
#[cfg(not(windows))]
fn shell_program(login_shell: bool) -> Result<PathBuf, String> {
    Ok(PathBuf::from(if login_shell { "bash" } else { "sh" }))
}

#[cfg(windows)]
fn shell_program(login_shell: bool) -> Result<PathBuf, String> {
    let shell =
        crate::shared::win::posix_shell::posix_shell().map_err(|e| no_posix_shell_error(&e))?;
    Ok(if login_shell {
        shell.bash.clone()
    } else {
        shell.sh.clone()
    })
}

/// Marks the one `ExecutionPort` failure that is neither a verdict nor a
/// broken connection: this machine has no interpreter to run a user-authored
/// script with.
///
/// It travels inside the D3 transport class because the alternative — a bare
/// `Err` — reads as a non-zero exit, i.e. as the project's own command having
/// been run and found wanting. But it is not a blip either: every remaining
/// command on this machine will fail the same way until something is
/// installed, which is why `adapters::step_executor::preflight` singles it out
/// instead of treating it as no evidence.
pub(crate) const NO_POSIX_SHELL_ERROR: &str = "no POSIX shell on this machine: ";

/// Render a failed resolution as that error. Kept out of the `#[cfg(windows)]`
/// arm above so the Linux host can assert the round trip against the preflight
/// that has to recognise it — no Windows toolchain exists here to observe it
/// any other way.
#[cfg(any(windows, test))]
pub(crate) fn no_posix_shell_error(
    missing: &crate::shared::win::posix_shell::ShellMissing,
) -> String {
    format!(
        "{}{}{}",
        crate::ports::execution::TRANSPORT_ERROR_PREFIX,
        NO_POSIX_SHELL_ERROR,
        missing
    )
}

/// Marks the spawn failure that is a statement about **how the command is
/// configured**, not about anything it did: it never started, and starting it
/// again changes nothing.
///
/// Windows raises it for the shape Demeteo hits by design. Since
/// CVE-2024-24576 `std` refuses to spawn a `.bat`/`.cmd` target carrying an
/// argument it cannot escape safely for `cmd.exe`, and every agent invocation
/// passes a prompt — arbitrary feature and ticket prose — as an argument to a
/// runtime that npm installed as exactly such a shim. Unix raises the same
/// `ErrorKind` for an interior NUL in the program or an argument, which is the
/// same statement about the caller, so the classification carries no `#[cfg]`.
///
/// It rides [`TRANSPORT_ERROR_PREFIX`](crate::ports::execution::TRANSPORT_ERROR_PREFIX)
/// because that is what
/// [`classify_exec_failure`](crate::domain::harness_failure::classify_exec_failure)
/// reads, and it must not reach the rework loop: an agent handed this is being
/// asked to repair source code that was never run, on every attempt, forever.
/// The same reasoning as [`NO_POSIX_SHELL_ERROR`], and the same shape — a
/// marker at a fixed position that a matcher can find.
pub(crate) const UNSPAWNABLE_ARGUMENTS_ERROR: &str =
    "the arguments cannot be passed to this program: ";

/// Render a spawn failure as that configuration error, or `None` when it is an
/// ordinary one the caller words itself.
///
/// Pure over the error kind and `cfg`-free, so the message a Windows user
/// meets — and the side of the triage it lands on — is decided and tested on
/// the host that has no Windows.
fn unspawnable_arguments(executable: &str, error: &std::io::Error) -> Option<String> {
    if error.kind() != std::io::ErrorKind::InvalidInput {
        return None;
    }
    Some(format!(
        "{}{}'{}' ({}). Nothing ran, and no source change can affect it. The cause on Windows \
         is a program that resolves to a `.bat` or `.cmd` shim being handed an argument the \
         interpreter cannot be made to quote safely (CVE-2024-24576): point the command at the \
         `.exe` behind the shim, or keep the offending text out of the argument list.",
        crate::ports::execution::TRANSPORT_ERROR_PREFIX,
        UNSPAWNABLE_ARGUMENTS_ERROR,
        executable,
        error,
    ))
}

/// The file a program name names.
///
/// Everywhere but Windows that is the name itself, resolved by `execvp`. There
/// it is the `PATHEXT` search [`crate::shared::win::exe`] performs and Rust's
/// `Command` does not, which is what makes `run_program("npm", …)` find the
/// `.cmd` shim npm actually installs. Failing to resolve falls through to the
/// bare name, so an unknown program still fails as `CreateProcess`'s own
/// missing-file error rather than as a Demeteo one.
fn program_path(name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(resolved) = crate::shared::win::exe::resolve_on_path(name) {
            return resolved;
        }
    }
    PathBuf::from(name)
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
    harden_child_spawn(command);
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
    let mut command = tokio::process::Command::new(program_path(&request.executable));
    command.args(&request.args);
    if let Some(cwd) = &request.cwd {
        command.current_dir(cwd);
    }
    command.envs(&request.env);
    sanitize_child_env(command.as_std_mut());
    harden_child_spawn(command.as_std_mut());
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let guard = ProcessGuard::armed();
    let mut child = command.spawn().map_err(|e| {
        unspawnable_arguments(&request.executable, &e)
            .unwrap_or_else(|| format!("Failed to execute '{}': {}", request.executable, e))
    })?;
    guard.adopt(&child);
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
    guard.disarm();
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
    #[cfg_attr(
        not(unix),
        expect(
            dead_code,
            reason = "the group kill it arms is the Unix arm of Drop; Windows takes the tree \
                      through the job object instead"
        )
    )]
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

    let (program, args) = shell_invocation(cmd, opts)?;
    let mut command = tokio::process::Command::new(&program);
    command.args(&args);
    configure_child(command.as_std_mut(), opts);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // The floor when the group kill is disarmed (non-`setsid` children).
        .kill_on_drop(true);

    let job = ProcessGuard::armed();
    let mut child = command.spawn().map_err(|e| {
        unspawnable_arguments(&program.to_string_lossy(), &e)
            .unwrap_or_else(|| format!("Failed to execute command: {}", e))
    })?;
    job.adopt(&child);
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
    job.disarm();
    let status = status.map_err(|e| format!("Failed to await command: {}", e))?;
    command_result(status.code(), status.success(), &stdout, &stderr)
}

/// Blocking structured-program helper for a few short adapter-owned setup
/// operations. User-authored scripts always go through [`local_run_program`].
///
/// The one spawn here that takes no [`ProcessGuard`]. `Command::output` never
/// yields a handle to confine, and the call it would confine cannot be
/// abandoned — there is no deadline and no cancellation point — so the job
/// would only ever fire on an unwind, where it would kill whatever `git` had
/// legitimately left running.
fn local_run_program_blocking(request: ProgramRequest) -> Result<String, String> {
    let mut command = Command::new(program_path(&request.executable));
    command.args(&request.args);
    if let Some(cwd) = &request.cwd {
        command.current_dir(cwd);
    }
    command.envs(&request.env);
    sanitize_child_env(&mut command);
    harden_child_spawn(&mut command);
    let output = command.output().map_err(|e| {
        unspawnable_arguments(&request.executable, &e)
            .unwrap_or_else(|| format!("Failed to execute '{}': {}", request.executable, e))
    })?;
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
#[path = "../../../tests/infrastructure/local_execution.rs"]
mod local_execution_tests;

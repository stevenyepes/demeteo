//! What gets started, and how it is named.
//!
//! Three questions the `ExecutionPort` impl asks before every spawn and
//! answers nowhere else: which interpreter runs a user-authored script body,
//! which file a program name resolves to, and which spawn failures are
//! statements about the *configuration* rather than verdicts on the command.
//!
//! Almost none of it is `#[cfg]`-gated, and that is deliberate — the Windows
//! answers are decisions, and a decision behind a `cfg` is one no local test
//! reaches (AGENTS.md §7). Only [`shell_program`] has two bodies.

use std::path::PathBuf;

use crate::ports::execution::ShellOptions;
use crate::shared::shell;

/// The `(program, args)` a set of [`ShellOptions`] resolves to.
///
/// Split in two because only one half varies by platform: [`shell_args`] is
/// the program text and is composed by identical code everywhere, while
/// [`shell_program`] is the file that interprets it. That is the entire
/// Windows difference — see `docs/WINDOWS_PARITY.md`.
pub(super) fn shell_invocation(
    cmd: &str,
    opts: &ShellOptions,
) -> Result<(PathBuf, Vec<String>), String> {
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
pub(super) fn unspawnable_arguments(executable: &str, error: &std::io::Error) -> Option<String> {
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
pub(super) fn program_path(name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(resolved) = crate::shared::win::exe::resolve_on_path(name) {
            return resolved;
        }
    }
    PathBuf::from(name)
}

/// Whether Git would run a file of this shape as a hook, given its Unix mode
/// where the platform has one.
///
/// `None` is Windows, and `true` for every non-directory is the answer there
/// rather than a stand-in for one: Git's `mingw_access` masks `X_OK` off, so
/// `find_hook` cannot test a bit and Git attempts the file regardless. See
/// [`ExecutionPort::is_executable`], which this decides for.
///
/// Free and `cfg`-free so the Windows answer is reachable from a test on a
/// host that has no Windows.
pub(super) fn git_would_run_hook(is_dir: bool, unix_mode: Option<u32>) -> bool {
    !is_dir && unix_mode.is_none_or(|mode| mode & 0o111 != 0)
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/local/invocation.rs"]
mod tests;

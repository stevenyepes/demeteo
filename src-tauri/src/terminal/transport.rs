use crate::domain::models::Machine;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use ssh2::Session;
use std::io::Write;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

#[allow(unused_imports)]
use super::hooks::{cmd_double_quote, upload_remote_settings};
use super::model::{ReadSource, SessionHandles, SessionKeepalive, WriteSink};

/// Re-exported by the facade so `forward.rs` and any other caller can keep
/// using `crate::terminal::connect_ssh` unchanged.
pub fn connect_ssh(
    machine: &Machine,
    secret: Option<String>,
) -> Result<(Session, TcpStream), String> {
    crate::ssh_util::connect(machine, secret)
}

/// Pick the shell to spawn for a local PTY. Split by platform because the
/// env var that names the interactive shell differs: POSIX exports `SHELL`,
/// Windows has no such variable and instead names the command processor via
/// `COMSPEC`. Spawning `/bin/bash` on Windows makes ConPTY/`portable-pty`
/// return Err and the session dies with "Failed to spawn shell", so the
/// Windows arm falls back to `cmd.exe` — the one interpreter guaranteed to
/// exist there.
#[cfg(target_os = "windows")]
pub(crate) fn select_local_shell() -> String {
    std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
}

/// POSIX: honour the user's `$SHELL`, falling back to `/bin/bash`.
#[cfg(not(target_os = "windows"))]
pub(crate) fn select_local_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
}

pub(crate) fn start_local_pty(
    machine_id: &str,
    work_dir: &Option<String>,
    work_branch: &Option<String>,
    cols: u16,
    rows: u16,
) -> Result<SessionHandles, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Failed to open PTY: {}", e))?;

    let shell = select_local_shell();
    let mut cmd = CommandBuilder::new(&shell);
    cmd.env("TERM", "xterm-256color");
    // Ensure a UTF-8 locale on macOS. A GUI launch (Finder/Dock) can hand the
    // app an environment with no `LANG`/`LC_*`, dropping the shell into the C
    // locale where it mishandles multibyte prompt output. `en_US.UTF-8` and the
    // bare `UTF-8` `LC_CTYPE` are Darwin idioms that are always valid there.
    //
    // Deliberately macOS-only: on Linux/BSD these exact values are risky —
    // `en_US.UTF-8` is frequently not generated and `LC_CTYPE=UTF-8` is not a
    // valid locale name, so forcing them yields `setlocale` warnings and a C
    // fallback. Desktop launchers on those platforms already propagate the
    // session locale, so no override is needed. Only set when absent so a
    // user's real locale is never overridden.
    #[cfg(target_os = "macos")]
    {
        if std::env::var_os("LANG").is_none() {
            cmd.env("LANG", "en_US.UTF-8");
        }
        if std::env::var_os("LC_CTYPE").is_none() {
            cmd.env("LC_CTYPE", "UTF-8");
        }
    }
    if let Some(dir) = work_dir {
        cmd.cwd(dir);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("Failed to spawn shell: {}", e))?;
    // Capture the shell pid before `child` is moved into the keepalive — the
    // foreground-agent detector walks the process tree rooted here.
    let child_pid = child.process_id();

    // Close the slave end in the parent — the child inherited it.
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("Failed to clone PTY reader: {}", e))?;
    // take_writer can only be called once — do it before moving master into keepalive.
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("Failed to take PTY writer: {}", e))?;

    // Bootstrap the branch if requested. The shell may not be ready to
    // accept input instantly, but stdin pipes buffer the bytes; bash will
    // process them on startup. A non-existent branch is swallowed: the
    // PTY remains usable on whatever branch the repo is currently on.
    if let Some(bootstrap) = branch_bootstrap_line(work_branch) {
        let _ = writer.write_all(bootstrap.as_bytes());
        let _ = writer.flush();
    }

    let read_source = ReadSource::LocalPty(Arc::new(Mutex::new(reader)));
    let write_sink = WriteSink::LocalPty(Arc::new(Mutex::new(writer)));
    let keepalive = Arc::new(Mutex::new(SessionKeepalive::LocalPty {
        master: pair.master,
        child,
    }));

    let _ = machine_id; // suppress unused warning
                        // A local session never places a remote settings file (T4.1).
    Ok((read_source, write_sink, keepalive, child_pid, None))
}

/// Build the POSIX-shell bootstrap line that performs a `git checkout` of the
/// supplied feature branch on shell startup. Returns `None` when no branch was
/// supplied, so callers can skip the write entirely for `ProjectHome`-style
/// flows (no pipeline context).
///
/// The line is shell-escaped defensively — the feature id is generated, but
/// a branch containing a stray quote or `;` would otherwise become a
/// command-injection vector. The trailing `clear` mirrors the existing
/// `cd … && clear` behaviour in the SSH path so the prompt lands cleanly.
/// Missing-branch failures are intentionally tolerated (`|| true`-style
/// fallback) so a not-yet-started feature still opens a usable terminal.
///
/// Always compiled — the syntax targets a POSIX shell, which is the correct
/// choice for **every remote SSH host regardless of client OS** (`start_ssh_
/// session` calls this directly) and for local sessions on non-Windows hosts
/// (via [`branch_bootstrap_line`]). The Windows/cmd.exe split lives in
/// [`branch_bootstrap_line`] and applies only to the *local* PTY, never to the
/// remote shell.
pub(crate) fn branch_bootstrap_line_posix(branch: &Option<String>) -> Option<String> {
    let raw = branch.as_ref()?.trim();
    if raw.is_empty() {
        return None;
    }
    let safe = crate::paths::shell_escape_posix(raw);
    Some(format!(
        "git checkout {safe} 2>/dev/null || git switch {safe} 2>/dev/null; clear\n"
    ))
}

/// Build the bootstrap line for the **local** PTY, choosing shell syntax by the
/// compile-time host OS: the POSIX form on non-Windows, the cmd.exe form on
/// Windows. This selector is used ONLY for the local shell — the SSH/remote
/// path deliberately bypasses it and always calls
/// [`branch_bootstrap_line_posix`] because the remote host is unconditionally a
/// POSIX shell. Both arms share the same `None`-on-absent/blank contract and
/// the same checkout-then-switch tolerance.
#[cfg(not(target_os = "windows"))]
pub(crate) fn branch_bootstrap_line(branch: &Option<String>) -> Option<String> {
    branch_bootstrap_line_posix(branch)
}

/// cmd.exe variant of [`branch_bootstrap_line`]. Emits `2>nul` (cmd's null
/// sink), `||`/`&` command chaining, and `cls` (cmd's screen clear), with a
/// CRLF terminator so cmd.exe treats the buffered bytes as one finished
/// command line. The branch is quoted with [`cmd_double_quote`] rather than
/// POSIX single quotes. Used only for the local Windows PTY — never for the
/// SSH path (see [`branch_bootstrap_line_posix`]).
#[cfg(target_os = "windows")]
pub(crate) fn branch_bootstrap_line(branch: &Option<String>) -> Option<String> {
    let raw = branch.as_ref()?.trim();
    if raw.is_empty() {
        return None;
    }
    let safe = cmd_double_quote(raw);
    Some(format!(
        "git checkout {safe} 2>nul || git switch {safe} 2>nul & cls\r\n"
    ))
}

pub(crate) fn start_ssh_session(
    machine: &Machine,
    work_dir: &Option<String>,
    work_branch: &Option<String>,
    cols: u16,
    rows: u16,
    remote_settings: Option<(String, &str)>,
) -> Result<SessionHandles, String> {
    let secret = match machine.auth_type.as_str() {
        "password" | "key" => {
            let key = format!("machine_{}", machine.id);
            crate::credential_cache::get_or_fetch(&key, || {
                let entry = keyring::Entry::new("demeteo", &key)
                    .map_err(|e| format!("Keyring error: {}", e))?;
                entry
                    .get_password()
                    .map_err(|e| format!("Keyring error: {}", e))
            })
            .ok()
        }
        _ => None,
    };

    let (sess, tcp) = connect_ssh(machine, secret)?;
    sess.set_keepalive(true, 30);
    let mut ssh_chan = sess
        .channel_session()
        .map_err(|e| format!("Failed to open SSH channel: {}", e))?;
    ssh_chan
        .request_pty(
            "xterm-256color",
            None,
            Some((cols as u32, rows as u32, 0, 0)),
        )
        .map_err(|e| format!("Failed to request PTY: {}", e))?;
    ssh_chan
        .shell()
        .map_err(|e| format!("Failed to start shell: {}", e))?;

    // Place the reporter-hooks settings file on the far host over SFTP for a
    // hooked launch (T4.1). Done here — session still blocking, drain not yet
    // spawned — so the synchronous write never races the interactive read.
    // A failure yields `None`, and the caller degrades the launch to unhooked
    // rather than pointing `claude --settings` at a missing file.
    let remote_settings_path = match remote_settings {
        Some((path, json)) => match upload_remote_settings(&sess, &path, json) {
            Ok(()) => Some(path),
            Err(e) => {
                eprintln!("[terminal] remote activity settings SFTP failed: {e}");
                None
            }
        },
        None => None,
    };

    if let Some(dir) = work_dir {
        let cd_cmd = format!("cd {} && clear\n", crate::paths::shell_escape_posix(dir));
        let _ = ssh_chan.write_all(cd_cmd.as_bytes());
        let _ = ssh_chan.flush();
    }
    // The remote is ALWAYS a POSIX shell, so the bootstrap must use POSIX
    // syntax regardless of the client OS. Calling `branch_bootstrap_line` here
    // would send cmd.exe syntax (`2>nul`, `& cls`) to a remote `bash`/`sh` on a
    // Windows client — creating a stray `nul` file and a `cls: command not
    // found` error in the remote repo. `branch_bootstrap_line_posix` is
    // compile-time-independent of the client OS and keeps the SSH path correct.
    if let Some(bootstrap) = branch_bootstrap_line_posix(work_branch) {
        let _ = ssh_chan.write_all(bootstrap.as_bytes());
        let _ = ssh_chan.flush();
    }

    sess.set_blocking(false);
    let arc_chan = Arc::new(Mutex::new(ssh_chan));
    let read_source = ReadSource::Ssh(arc_chan.clone());
    let write_sink = WriteSink::Ssh(arc_chan);
    let keepalive = Arc::new(Mutex::new(SessionKeepalive::Ssh { session: sess, tcp }));
    // No local pid for a remote session — the shell runs on the far host.
    Ok((
        read_source,
        write_sink,
        keepalive,
        None,
        remote_settings_path,
    ))
}

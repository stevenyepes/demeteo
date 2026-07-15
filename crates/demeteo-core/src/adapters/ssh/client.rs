use crate::paths;
use crate::ports::db::MachineRepository;
use crate::ports::execution::SftpEntry;
use crate::ports::execution::{ExecutionPort, InteractiveHandle, ShellOptions};
use crate::shared::shell;
use async_trait::async_trait;
use ssh2::{Channel, Session, Sftp};
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Outer wall-clock bound for draining a single one-shot SSH command. The
/// keepalive-aware loop in [`drain_stream`] lets a command stay silent for as
/// long as it needs without the session's 10s blocking-call timeout aborting
/// it (that was the "`cargo test` compiles silently for >10s → prepare command
/// spuriously fails" bug — see the conformance suite's long-silent-command
/// clause). We still cap total drain time so a genuinely wedged remote can't
/// hang a step forever. This is a transport backstop, not a per-command tuning
/// knob — finer wall-clock limits and cooperative cancellation belong to the
/// caller's timeout layer.
const TRANSPORT_WALL_CAP: Duration = Duration::from_secs(30 * 60);

/// How long a drain may go with **no sign of life** — no bytes read *and* no
/// successful keepalive round-trip — before the transport is declared dead and
/// the drain aborts. This is the difference between "quiet but alive" and
/// "connection wedged": a silent-but-healthy command (a `cargo test` compiling
/// in silence) keeps answering keepalives every ~30s, so its life clock is
/// continually reset and it survives up to [`TRANSPORT_WALL_CAP`]. A
/// black-holed connection stops acking keepalives, so this trips in ~2 min
/// instead of keepalive-looping to the full 30-minute cap — which used to
/// freeze not just the step but every SSH op queued behind the pooled session.
/// Deliberately larger than the 30s keepalive interval so a single transient
/// blip never false-positives, and smaller than the wall cap so it fails fast.
const NO_PROGRESS_ABORT: Duration = Duration::from_secs(120);

/// Has a silent drain crossed from "quiet but alive" into "the transport is
/// dead"? `since_last_life` is how long since we last saw *either* bytes on the
/// wire *or* a keepalive round-trip. Extracted (and kept free of `Session`) so
/// the boundary is unit-testable without a live socket.
fn no_progress_expired(since_last_life: Duration) -> bool {
    since_last_life >= NO_PROGRESS_ABORT
}

/// Tag `msg` as a *transport/connection* failure (the machine could not be
/// reached or the channel broke) rather than a *command* failure (it ran and
/// exited non-zero). Callers distinguish the two via
/// [`crate::ports::execution::TRANSPORT_ERROR_PREFIX`] (C0.2, D3) — e.g. the
/// verifier routes a transport failure to `Infrastructure` (non-retryable)
/// instead of a `Verdict` that would pointlessly re-run a failing build.
fn transport_err(msg: impl std::fmt::Display) -> String {
    format!("{}{}", crate::ports::execution::TRANSPORT_ERROR_PREFIX, msg)
}

/// Drain `reader` (an ssh2 channel or its stderr stream) to EOF into
/// `buf_out`, tolerating the session's blocking-call timeout the way the
/// interactive [`RemoteChannelHandle::try_read`] path does: a `TimedOut` /
/// `WouldBlock` read is **not** end-of-stream — libssh2 aborts a blocking read
/// the moment a keepalive comes due (~30s after handshake) even while the
/// command is alive and simply quiet. Send the keepalive it's waiting on and
/// retry, so a long silent compile drains to real EOF instead of failing with
/// "Timed out waiting on socket". `deadline` bounds the whole drain so a
/// wedged remote is still killable. Bytes are accumulated raw and decoded once
/// by the caller — decoding per chunk could split a multibyte UTF-8 sequence.
fn drain_stream<R: Read>(
    reader: &mut R,
    session: &Session,
    buf_out: &mut Vec<u8>,
    deadline: Instant,
    what: &str,
) -> Result<(), String> {
    let mut chunk = [0u8; 8192];
    // Last moment the transport showed life: either bytes arrived or a
    // keepalive round-tripped. A merely quiet command keeps this fresh (every
    // keepalive is answered); a wedged connection lets it go stale, which is
    // how we tell the two apart without killing healthy silent commands.
    let mut last_life = Instant::now();
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                buf_out.extend_from_slice(&chunk[..n]);
                last_life = Instant::now();
            }
            Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
                let now = Instant::now();
                if now >= deadline {
                    return Err(transport_err(format!(
                        "Timed out after the transport wall cap ({}s) waiting for {}",
                        TRANSPORT_WALL_CAP.as_secs(),
                        what
                    )));
                }
                // A blocking read times out every ~10s while a command is
                // simply quiet (see `ssh_util::connect`'s `set_timeout`). The
                // keepalive tells us whether the *transport* is still alive:
                // on a live session it round-trips (`Ok`) and we refresh the
                // life clock; on a black-holed one it errors (or its socket
                // write times out), the clock goes stale, and we abort once it
                // crosses `NO_PROGRESS_ABORT` instead of looping to the wall
                // cap and freezing every SSH op behind the pooled session.
                if session.keepalive_send().is_ok() {
                    last_life = now;
                } else if no_progress_expired(now.duration_since(last_life)) {
                    return Err(transport_err(format!(
                        "Connection appears dead: no data and no keepalive response for {}s while waiting for {}",
                        NO_PROGRESS_ABORT.as_secs(),
                        what
                    )));
                }
            }
            Err(e) => return Err(transport_err(format!("Failed to read {}: {}", what, e))),
        }
    }
}

struct RemoteChannelHandle {
    channel: Mutex<Channel>,
    session: Session,
}

impl InteractiveHandle for RemoteChannelHandle {
    fn write_line(&self, line: &str) -> std::io::Result<usize> {
        let mut channel = self.channel.lock().unwrap();
        channel.write_all(line.as_bytes())?;
        channel.write_all(b"\n")?;
        channel.flush()?;
        Ok(line.len() + 1)
    }

    fn try_read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut channel = self.channel.lock().unwrap();
        match channel.read(buf) {
            Ok(n) => Ok(n),
            // A timeout here is NOT end-of-stream: the session carries a 10s
            // blocking-call timeout (see `ssh_util::connect`), and with
            // keepalive configured libssh2 aborts a blocking read the moment
            // a keepalive comes due (~30s after handshake) even while data
            // is flowing. Send the keepalive libssh2 is waiting on and
            // report `WouldBlock` so the caller retries instead of treating
            // a healthy mid-turn stream as ended. Covered live by the
            // ignored `remote_pty_stream_survives_keepalive_and_silence_live`
            // test.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                let _ = self.session.keepalive_send();
                Err(std::io::Error::new(std::io::ErrorKind::WouldBlock, e))
            }
            Err(e) => Err(e),
        }
    }

    fn kill(&self) -> Result<(), String> {
        let mut channel = self.channel.lock().unwrap();
        channel.close().map_err(|e| e.to_string())
    }

    fn try_wait(&self) -> Result<Option<i32>, String> {
        let channel = self.channel.lock().unwrap();
        // `exit_status()` returns Ok(0) while the remote process is still
        // running (libssh2 only learns the real status once the channel
        // reaches EOF). Answer "still running" until then so callers never
        // mistake a live agent for a clean exit.
        if !channel.eof() {
            return Ok(None);
        }
        match channel.exit_status() {
            Ok(code) => Ok(Some(code)),
            Err(e) => Err(e.to_string()),
        }
    }
}

pub struct SftpSession {
    pub sftp: Mutex<Sftp>,
    pub session: Session,
    pub tcp: TcpStream,
}

pub struct SshClientAdapter {
    pub machines: Arc<dyn MachineRepository>,
    pub sessions: Arc<Mutex<HashMap<String, Arc<SftpSession>>>>,
    /// Resolved remote HOME per machine_id. The remote HOME is stable
    /// for the lifetime of the user's account, so we cache it after the
    /// first successful resolve to avoid an extra `echo $HOME` round-trip
    /// on every path computation. Cleared on `disconnect_all` (which
    /// isn't called today, but the cache is keyed by `machine_id` so
    /// reconnects naturally pick up the cached value).
    home_cache: Arc<Mutex<HashMap<String, String>>>,
}

impl SshClientAdapter {
    pub fn new(machines: Arc<dyn MachineRepository>) -> Self {
        Self {
            machines,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            home_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Resolve the remote user's HOME directory by running `echo $HOME`
    /// over the SSH channel. Cached per `machine_id` so we only pay the
    /// round-trip once per session. Sync helper; the async
    /// `resolve_home` impl method wraps this in `spawn_blocking`.
    fn resolve_remote_home(&self, machine_id: &str) -> Result<String, String> {
        resolve_home_blocking(&self.machines, &self.sessions, &self.home_cache, machine_id)
    }
}

/// Probe `$HOME` over a fresh channel on an already-connected session.
/// `printf %s` avoids trailing newlines and respects quoting.
fn probe_home_over_channel(session: &Session) -> Result<String, String> {
    let mut channel = session
        .channel_session()
        .map_err(|e| format!("Failed to open SSH channel for HOME probe: {}", e))?;
    channel
        .exec("printf %s \"$HOME\"")
        .map_err(|e| format!("Failed to exec HOME probe over SSH: {}", e))?;
    let deadline = Instant::now() + TRANSPORT_WALL_CAP;
    let mut raw_bytes = Vec::new();
    drain_stream(
        &mut channel,
        session,
        &mut raw_bytes,
        deadline,
        "HOME probe output",
    )?;
    let raw = String::from_utf8_lossy(&raw_bytes).into_owned();
    channel
        .wait_close()
        .map_err(|e| format!("Failed to wait for HOME probe channel: {}", e))?;
    // ssh2's `wait_close` returns `Result<(), Error>`; the exit status is
    // on a separate method that returns `Result<i32, Error>` (0 on
    // success, non-zero on remote failure). Drain it so a broken shell
    // session doesn't get cached as a valid HOME.
    let exit_code = channel
        .exit_status()
        .map_err(|e| format!("Failed to read HOME probe exit status: {}", e))?;
    if exit_code != 0 {
        return Err(format!(
            "Remote HOME probe exited with status {}; the SSH session may be denying shell access",
            exit_code
        ));
    }

    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        return Err("Remote HOME is empty (HOME is not set on the SSH session)".to_string());
    }
    if !trimmed.starts_with('/') {
        return Err(format!(
            "Remote HOME is not an absolute path (got '{}')",
            trimmed
        ));
    }
    Ok(trimmed)
}

/// Free-function version of [`SshClientAdapter::resolve_remote_home`] that
/// only needs the shared `Arc`s, so it can also be called from inside a
/// `spawn_blocking` closure that has moved those out of `self` (see
/// `control_rpc_blocking`).
fn resolve_home_blocking(
    machines: &Arc<dyn MachineRepository>,
    sessions: &Mutex<HashMap<String, Arc<SftpSession>>>,
    home_cache: &Mutex<HashMap<String, String>>,
    machine_id: &str,
) -> Result<String, String> {
    if let Ok(cache) = home_cache.lock() {
        if let Some(home) = cache.get(machine_id) {
            eprintln!(
                "[SshClientAdapter] resolve_remote_home({}) = {} (cache hit)",
                machine_id, home
            );
            return Ok(home.clone());
        }
    }

    let sftp_sess = get_sftp_blocking(machines, sessions, machine_id)?;
    let trimmed = probe_home_over_channel(&sftp_sess.session)?;

    eprintln!(
        "[SshClientAdapter] resolve_remote_home({}) = {} (fresh probe; cached)",
        machine_id, trimmed
    );
    if let Ok(mut cache) = home_cache.lock() {
        cache.insert(machine_id.to_string(), trimmed.clone());
    }
    Ok(trimmed)
}

/// M6.1: one request/response round-trip against `demeteo-runner`'s
/// control socket, reached via OpenSSH Unix-socket forwarding
/// (`channel_direct_streamlocal`, R4) over the same cached SSH session
/// `run_command`/SFTP use. Opens one fresh channel per call (the session
/// itself is what's cached/reused) — simple request/response, no
/// long-lived multiplexed connection to manage.
fn control_rpc_blocking(
    machines: &Arc<dyn MachineRepository>,
    sessions: &Mutex<HashMap<String, Arc<SftpSession>>>,
    home_cache: &Mutex<HashMap<String, String>>,
    machine_id: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let sftp_sess = get_sftp_blocking(machines, sessions, machine_id)?;
    let home = resolve_home_blocking(machines, sessions, home_cache, machine_id)?;
    let socket_path = format!("{}/.local/share/demeteo-runner/control.sock", home);

    let mut channel = sftp_sess
        .session
        .channel_direct_streamlocal(&socket_path, None)
        .map_err(|e| {
            format!(
                "Failed to reach demeteo-runner control socket at {}: {} \
                 (is the runner installed and running on this machine?)",
                socket_path, e
            )
        })?;

    let request = serde_json::json!({ "id": 1u64, "method": method, "params": params });
    let mut line = serde_json::to_string(&request).map_err(|e| e.to_string())?;
    line.push('\n');
    channel
        .write_all(line.as_bytes())
        .map_err(|e| format!("Failed to write control-RPC request: {}", e))?;
    channel
        .flush()
        .map_err(|e| format!("Failed to flush control-RPC request: {}", e))?;
    // Half-close our write side so the runner's line-reader loop sees
    // EOF right after our one request and closes its side in turn —
    // that's what unblocks the `read_to_string` below.
    channel
        .send_eof()
        .map_err(|e| format!("Failed to send EOF on control-RPC channel: {}", e))?;

    let deadline = Instant::now() + TRANSPORT_WALL_CAP;
    let mut raw_bytes = Vec::new();
    drain_stream(
        &mut channel,
        &sftp_sess.session,
        &mut raw_bytes,
        deadline,
        "control-RPC response",
    )?;
    let raw = String::from_utf8_lossy(&raw_bytes).into_owned();
    let _ = channel.close();
    let _ = channel.wait_close();

    let line = raw
        .lines()
        .next()
        .ok_or_else(|| "empty response from demeteo-runner control socket".to_string())?;

    #[derive(serde::Deserialize)]
    struct RpcResponse {
        result: Option<serde_json::Value>,
        error: Option<String>,
    }
    let resp: RpcResponse = serde_json::from_str(line)
        .map_err(|e| format!("invalid control-RPC response: {} (raw: {})", e, line))?;
    match resp.error {
        Some(e) => Err(e),
        None => Ok(resp.result.unwrap_or(serde_json::Value::Null)),
    }
}

/// Blocking helper used by the async `ExecutionPort` impl methods.
/// Opens (or returns a cached) SFTP session using only the shared
/// `Arc<MachineRepository>` and `Arc<Mutex<HashMap>>` so it can run
/// inside `tokio::task::spawn_blocking` without moving `&self`.
fn get_sftp_blocking(
    machines: &Arc<dyn crate::ports::db::MachineRepository>,
    sessions: &Mutex<HashMap<String, Arc<SftpSession>>>,
    machine_id: &str,
) -> Result<Arc<SftpSession>, String> {
    // Take a cheap `Arc` clone of any pooled session *under* the lock, then
    // release it before the liveness probe. The probe (`readdir`) is a blocking
    // network round-trip; running it while holding the global `sessions` mutex
    // means one wedged connection blocks every other machine's SSH ops behind
    // it — a pipeline-wide stall (the "stopped at validate" hang). Off the lock,
    // a slow probe only delays the caller that owns that connection.
    let pooled = {
        let sessions = sessions
            .lock()
            .map_err(|_| "Failed to lock SFTP state".to_string())?;
        sessions.get(machine_id).cloned()
    };

    if let Some(s) = pooled {
        let alive = match s.sftp.lock() {
            Ok(sftp) => sftp.readdir(std::path::Path::new(".")).is_ok(),
            Err(_) => false,
        };
        if alive {
            return Ok(s);
        }
        // Wedged/dead — evict it so the next caller reconnects. Only remove the
        // entry if it's still the same `Arc` we probed: a concurrent caller may
        // have already reconnected and inserted a fresh session while our probe
        // was blocking, and we must not drop that one on the floor.
        if let Ok(mut sessions) = sessions.lock() {
            if sessions
                .get(machine_id)
                .is_some_and(|cur| Arc::ptr_eq(cur, &s))
            {
                sessions.remove(machine_id);
            }
        }
    }

    // Connect new session
    let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
        &**machines,
        machine_id,
    )?;

    let secret = match machine.auth_type.as_str() {
        "password" | "key" => {
            let key = format!("machine_{}", machine.id);
            crate::credential_cache::get_or_fetch(&key, || {
                #[cfg(feature = "keyring")]
                {
                    let entry = keyring::Entry::new("demeteo", &key)
                        .map_err(|e| format!("Keyring error: {}", e))?;
                    entry
                        .get_password()
                        .map_err(|e| format!("Keyring error: {}", e))
                }
                #[cfg(not(feature = "keyring"))]
                {
                    Err("OS-keyring credential cache is disabled in this build".to_string())
                }
            })
            .ok()
        }
        _ => None,
    };

    let (sess, tcp) = crate::ssh_util::connect(&machine, secret)?;

    sess.set_blocking(true);
    let sftp = sess
        .sftp()
        .map_err(|e| format!("SFTP subsystem failed: {}", e))?;

    let sftp_session = Arc::new(SftpSession {
        sftp: Mutex::new(sftp),
        session: sess,
        tcp,
    });

    let mut sessions = sessions
        .lock()
        .map_err(|_| "Failed to lock SFTP state".to_string())?;
    sessions.insert(machine_id.to_string(), sftp_session.clone());
    Ok(sftp_session)
}

/// Open a fresh channel on `session`, exec `full_cmd`, drain stdout AND
/// stderr, and enforce the exit-code invariant (D3): a non-zero exit is an
/// `Err` carrying the captured stderr (never `Ok("")`). `full_cmd` is the
/// already-assembled shell invocation (login/non-login wrapper + cwd + env);
/// this helper is transport plumbing only and makes no assumptions about it.
///
/// This is the single drain-and-check path shared by `run_command_with`, so
/// the exit-status handling that root-caused "UI says HEALTHY but the dir
/// doesn't exist" lives in exactly one place.
fn exec_over_channel(session: &Session, full_cmd: &str) -> Result<String, String> {
    let mut channel = session
        .channel_session()
        .map_err(|e| transport_err(format!("Failed to open SSH channel: {}", e)))?;
    channel
        .exec(full_cmd)
        .map_err(|e| transport_err(format!("Failed to execute command: {}", e)))?;

    // Timeout-tolerant drain: a long silent command (e.g. `cargo test`
    // compiling) must not be aborted by the session's 10s blocking-call
    // timeout. See `drain_stream` / `TRANSPORT_WALL_CAP`.
    let deadline = Instant::now() + TRANSPORT_WALL_CAP;
    let mut stdout_bytes = Vec::new();
    drain_stream(
        &mut channel,
        session,
        &mut stdout_bytes,
        deadline,
        "command stdout",
    )?;
    let stdout = String::from_utf8_lossy(&stdout_bytes).into_owned();

    // ssh2 keeps stderr on a separate stream. Drain it so the remote
    // shell's error message is included in the Err variant.
    let mut stderr_bytes = Vec::new();
    {
        let mut err_stream = channel.stderr();
        let _ = drain_stream(
            &mut err_stream,
            session,
            &mut stderr_bytes,
            deadline,
            "command stderr",
        );
    }
    let stderr = String::from_utf8_lossy(&stderr_bytes).into_owned();

    channel
        .wait_close()
        .map_err(|e| transport_err(format!("Failed to wait for channel close: {}", e)))?;
    let exit_code = channel
        .exit_status()
        .map_err(|e| transport_err(format!("Failed to read command exit status: {}", e)))?;

    if exit_code != 0 {
        let detail = if stderr.trim().is_empty() {
            format!("exit code: {}", exit_code)
        } else {
            stderr.trim().to_string()
        };
        return Err(format!("Command failed ({}): {}", detail, full_cmd));
    }

    Ok(stdout)
}

#[async_trait]
impl ExecutionPort for SshClientAdapter {
    async fn test_connection(&self, machine_id: &str) -> Result<(), String> {
        let machine_id = machine_id.to_string();
        let machines = self.machines.clone();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
                &*machines,
                &machine_id,
            )?;

            // Local machines don't use SSH – trivially valid
            if machine.auth_type == "local" {
                return Ok(());
            }

            let secret = match machine.auth_type.as_str() {
                "password" | "key" => {
                    let key = format!("machine_{}", machine.id);
                    crate::credential_cache::get_or_fetch(&key, || {
                        #[cfg(feature = "keyring")]
                        {
                            let entry = keyring::Entry::new("demeteo", &key)
                                .map_err(|e| format!("Keyring error: {}", e))?;
                            entry
                                .get_password()
                                .map_err(|e| format!("Keyring error: {}", e))
                        }
                        #[cfg(not(feature = "keyring"))]
                        {
                            Err("OS-keyring credential cache is disabled in this build".to_string())
                        }
                    })
                    .ok()
                }
                _ => None,
            };

            let (sess, _tcp) = crate::ssh_util::connect(&machine, secret)?;

            // Connection is valid – disconnect cleanly
            let _ = sess.disconnect(None, "test complete", None);
            Ok(())
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn run_command_with(
        &self,
        machine_id: &str,
        cmd: &str,
        opts: ShellOptions,
    ) -> Result<String, String> {
        // The underlying `ssh2` API is fully sync (TCP + SFTP + Channel
        // I/O). Run the work on the blocking pool so we don't stall
        // the tokio worker thread. The error type stays `String` to
        // match the port signature.
        //
        // `run_command` (no override) delegates here via the trait default
        // with `ShellOptions::default()` — a non-login `sh -c` in the login
        // directory with no extra env, matching the historical behaviour of
        // the previous bare `channel.exec`, but now with cwd/env/login
        // honoured identically to the local adapter when the caller opts in.
        let machine_id = machine_id.to_string();
        let cmd = cmd.to_string();
        let machines = self.machines.clone();
        let sessions = self.sessions.clone();
        tokio::task::spawn_blocking(move || -> Result<String, String> {
            // A failure to establish/reuse the session is a transport failure,
            // not a command failure — tag it so callers (e.g. the verifier)
            // don't misclassify an unreachable machine as a red build.
            let sftp_sess = get_sftp_blocking(&machines, &sessions, &machine_id).map_err(|e| {
                if e.starts_with(crate::ports::execution::TRANSPORT_ERROR_PREFIX) {
                    e
                } else {
                    transport_err(e)
                }
            })?;

            // Assemble the shell invocation identically to the local
            // adapter: exports run *inside* the body (after a login shell
            // sources its profile) so the caller's env wins; `cd` is baked
            // into the body so a failed `cd` aborts before the command runs.
            let exports = shell::export_prefix(&opts.env);
            let body = shell::command_body(opts.cwd.as_deref(), &exports, &cmd);
            let full_cmd = if opts.login_shell {
                // `-i` (interactive) sources `~/.bashrc`, where tool-managers
                // (mise/asdf/nvm) put their PATH activation behind the standard
                // non-interactive guard; a plain `-l` login shell misses them.
                // See `ShellOptions::interactive`.
                let flags = if opts.interactive {
                    "-l -i -c"
                } else {
                    "-l -c"
                };
                format!("bash {} {}", flags, paths::shell_escape_posix(&body))
            } else {
                format!("sh -c {}", paths::shell_escape_posix(&body))
            };

            exec_over_channel(&sftp_sess.session, &full_cmd)
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn read_file(&self, machine_id: &str, path: &str) -> Result<String, String> {
        let machine_id = machine_id.to_string();
        let path = path.to_string();
        let machines = self.machines.clone();
        let sessions = self.sessions.clone();
        tokio::task::spawn_blocking(move || -> Result<String, String> {
            let sftp_sess = get_sftp_blocking(&machines, &sessions, &machine_id)?;
            let sftp = sftp_sess
                .sftp
                .lock()
                .map_err(|_| "Failed to lock SFTP".to_string())?;

            let path_buf = std::path::Path::new(&path);
            let mut file = sftp.open(path_buf).map_err(|e| {
                if let Ok(mut sessions) = sessions.lock() {
                    sessions.remove(&machine_id);
                }
                format!("Failed to open file: {}", e)
            })?;

            let mut contents = String::new();
            file.read_to_string(&mut contents)
                .map_err(|e| format!("Failed to read file content: {}", e))?;
            Ok(contents)
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn write_file(&self, machine_id: &str, path: &str, content: &str) -> Result<(), String> {
        let machine_id = machine_id.to_string();
        let path = path.to_string();
        let content = content.to_string();
        let machines = self.machines.clone();
        let sessions = self.sessions.clone();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let sftp_sess = get_sftp_blocking(&machines, &sessions, &machine_id)?;
            let sftp = sftp_sess
                .sftp
                .lock()
                .map_err(|_| "Failed to lock SFTP".to_string())?;

            let path_buf = std::path::Path::new(&path);
            let mut file = sftp.create(path_buf).map_err(|e| {
                if let Ok(mut sessions) = sessions.lock() {
                    sessions.remove(&machine_id);
                }
                format!("Failed to create file: {}", e)
            })?;

            file.write_all(content.as_bytes())
                .map_err(|e| format!("Failed to write file: {}", e))?;
            file.flush()
                .map_err(|e| format!("Failed to flush file: {}", e))?;
            Ok(())
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn write_file_bytes(
        &self,
        machine_id: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), String> {
        let machine_id = machine_id.to_string();
        let path = path.to_string();
        let content = content.to_vec();
        let machines = self.machines.clone();
        let sessions = self.sessions.clone();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let sftp_sess = get_sftp_blocking(&machines, &sessions, &machine_id)?;
            let sftp = sftp_sess
                .sftp
                .lock()
                .map_err(|_| "Failed to lock SFTP".to_string())?;

            let path_buf = std::path::Path::new(&path);
            let mut file = sftp.create(path_buf).map_err(|e| {
                if let Ok(mut sessions) = sessions.lock() {
                    sessions.remove(&machine_id);
                }
                format!("Failed to create file: {}", e)
            })?;

            file.write_all(&content)
                .map_err(|e| format!("Failed to write file: {}", e))?;
            file.flush()
                .map_err(|e| format!("Failed to flush file: {}", e))?;
            Ok(())
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn get_metadata(&self, machine_id: &str, path: &str) -> Result<SftpEntry, String> {
        let machine_id = machine_id.to_string();
        let path = path.to_string();
        let machines = self.machines.clone();
        let sessions = self.sessions.clone();
        tokio::task::spawn_blocking(move || -> Result<SftpEntry, String> {
            let sftp_sess = get_sftp_blocking(&machines, &sessions, &machine_id)?;
            let sftp = sftp_sess
                .sftp
                .lock()
                .map_err(|_| "Failed to lock SFTP".to_string())?;

            let path_buf = std::path::Path::new(&path);
            let stat = sftp.stat(path_buf).map_err(|e| {
                if let Ok(mut sessions) = sessions.lock() {
                    sessions.remove(&machine_id);
                }
                format!("Failed to stat file: {}", e)
            })?;

            let name = path_buf
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            let size = stat.size.unwrap_or(0);
            let modified = stat.mtime.unwrap_or(0);
            let is_dir = stat.is_dir();

            Ok(SftpEntry {
                name,
                path: path.clone(),
                is_dir,
                size,
                modified,
            })
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn list_dir(&self, machine_id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        let machine_id = machine_id.to_string();
        let path = path.to_string();
        let machines = self.machines.clone();
        let sessions = self.sessions.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<SftpEntry>, String> {
            let sftp_sess = get_sftp_blocking(&machines, &sessions, &machine_id)?;
            let sftp = sftp_sess
                .sftp
                .lock()
                .map_err(|_| "Failed to lock SFTP".to_string())?;

            let path_buf = std::path::Path::new(&path);
            let entries = sftp.readdir(path_buf).map_err(|e| {
                if let Ok(mut sessions) = sessions.lock() {
                    sessions.remove(&machine_id);
                }
                format!("Failed to read directory: {}", e)
            })?;

            let mut list = Vec::new();
            for (p, stat) in entries {
                let name = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();

                if name == "." || name == ".." {
                    continue;
                }

                let path_str = p.to_str().unwrap_or("").to_string();
                let size = stat.size.unwrap_or(0);
                let modified = stat.mtime.unwrap_or(0);
                let is_dir = stat.is_dir();

                list.push(SftpEntry {
                    name,
                    path: path_str,
                    is_dir,
                    size,
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
        machine_id: &str,
        repo_path: &str,
        branch: &str,
        sandbox_path: &str,
    ) -> Result<(), String> {
        // Step 1: Ensure directory setup
        self.run_command(
            machine_id,
            &format!("mkdir -p {}/.demeteo/worktrees", repo_path),
        )
        .await?;

        // Step 2: Configure git info exclude
        let git_exclude_cmd = format!(
            "if [ -d \"{0}/.git\" ]; then mkdir -p \"{0}/.git/info\"; if ! grep -q \".demeteo/\" \"{0}/.git/info/exclude\" 2>/dev/null; then echo \".demeteo/\" >> \"{0}/.git/info/exclude\"; fi; fi",
            repo_path
        );
        let _ = self.run_command(machine_id, &git_exclude_cmd).await;

        // Step 3: Run git worktree add
        let worktree_add_cmd = format!(
            "git -C \"{}\" worktree add -b \"{}\" \"{}\"",
            repo_path, branch, sandbox_path
        );
        let output = self.run_command(machine_id, &worktree_add_cmd).await?;
        println!(
            "[SshClientAdapter] Git Worktree provisioning output: {}",
            output
        );

        Ok(())
    }

    async fn resolve_home(&self, machine_id: &str) -> Result<String, String> {
        if machine_id.is_empty() || machine_id == "local" {
            return Err("Cannot resolve remote HOME for local machine_id".to_string());
        }
        self.resolve_remote_home(machine_id)
    }

    async fn resolve_user(&self, machine_id: &str) -> Result<String, String> {
        if machine_id.is_empty() || machine_id == "local" {
            return Err("Cannot resolve remote USER for local machine_id".to_string());
        }
        // The SSH channel authenticates as `Machine.username`, so the
        // remote passwd entry's USER matches the machine record.
        // Return the record's value verbatim — if the user typed in a
        // machine with an empty username, the error from the lookup
        // below will surface that loud rather than the agent
        // silently running as the GUI's user.
        let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
            &*self.machines,
            machine_id,
        )?;
        if machine.username.is_empty() {
            return Err(format!(
                "Machine '{}' has no username configured; cannot resolve remote USER",
                machine_id
            ));
        }
        Ok(machine.username.clone())
    }

    async fn control_rpc(
        &self,
        machine_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let machine_id = machine_id.to_string();
        let method = method.to_string();
        let machines = self.machines.clone();
        let sessions = self.sessions.clone();
        let home_cache = self.home_cache.clone();
        tokio::task::spawn_blocking(move || {
            control_rpc_blocking(
                &machines,
                &sessions,
                &home_cache,
                &machine_id,
                &method,
                params,
            )
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    fn spawn_interactive(
        &self,
        machine_id: &str,
        binary: &str,
        args: &[String],
        cwd: &str,
        env: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
            &*self.machines,
            machine_id,
        )?;

        let secret = match machine.auth_type.as_str() {
            "password" | "key" => {
                let key = format!("machine_{}", machine.id);
                crate::credential_cache::get_or_fetch(&key, || {
                    #[cfg(feature = "keyring")]
                    {
                        let entry = keyring::Entry::new("demeteo", &key)
                            .map_err(|e| format!("Keyring error: {}", e))?;
                        entry
                            .get_password()
                            .map_err(|e| format!("Keyring error: {}", e))
                    }
                    #[cfg(not(feature = "keyring"))]
                    {
                        Err("OS-keyring credential cache is disabled in this build".to_string())
                    }
                })
                .ok()
            }
            _ => None,
        };

        let (sess, _tcp) = crate::ssh_util::connect(&machine, secret)?;
        sess.set_keepalive(true, 30);

        let mut channel = sess
            .channel_session()
            .map_err(|e| format!("Failed to open SSH channel: {}", e))?;

        channel
            .request_pty("xterm-256color", None, None)
            .map_err(|e| format!("Failed to request PTY on SSH channel: {}", e))?;

        let use_login_shell = machine.use_login_shell.unwrap_or(false);

        // Env composition (HOME / USER / LOGNAME resolution against
        // the remote machine's identity) is the caller's
        // responsibility — `agent_base_env` in `ports/agent_runtime.rs`
        // is the single owner and consults `ExecutionPort::resolve_home`
        // + `ExecutionPort::resolve_user`. This adapter is now a
        // "pure" executor: it forwards whatever env the caller built,
        // no business logic, no identity assumptions. The previous
        // override here (HOME/USER/LOGNAME) was a defense-in-depth
        // band-aid for a class of bugs the port-side refactor
        // eliminates by construction.
        let mut env_str = String::new();
        for (k, v) in env {
            let escaped = v.replace('\'', "'\\''");
            env_str.push_str(&format!("export {}='{}'; ", k, escaped));
        }
        let args_str = args
            .iter()
            .map(|a| paths::shell_escape_posix(a))
            .collect::<Vec<_>>()
            .join(" ");

        let cmd = if use_login_shell {
            let inner = format!(
                "{} command cd {} && {{ command -v mise >/dev/null 2>&1 && mise trust --yes . || :; }} 2>/dev/null && exec {} {}",
                env_str,
                paths::shell_escape_posix(cwd),
                paths::shell_escape_posix(binary),
                args_str
            );
            // Interactive (`-i`) so `~/.bashrc` is sourced — that's where
            // mise/asdf/nvm activate the toolchain that puts the agent binary
            // on PATH. A non-interactive `bash -l` login shell hits the
            // standard `.bashrc` non-interactive guard and never activates
            // them, so `exec opencode` would fail with "command not found"
            // even though the binary is installed. A PTY is always requested
            // above, so `-i` has a controlling terminal and stays quiet. This
            // mirrors the interactive availability probe (see
            // `ShellOptions::interactive`) so "available" and "runnable" agree.
            format!("bash -l -i -c {}", paths::shell_escape_posix(&inner))
        } else {
            format!(
                "cd {} && {} {} {}",
                paths::shell_escape_posix(cwd),
                env_str,
                paths::shell_escape_posix(binary),
                args_str
            )
        };

        eprintln!("[SshClientAdapter] spawn_interactive cmd: {}", cmd);
        channel
            .exec(&cmd)
            .map_err(|e| format!("Failed to exec agent over SSH: {}", e))?;

        Ok(Box::new(RemoteChannelHandle {
            channel: Mutex::new(channel),
            session: sess,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fast-abort window is only correct if it sits strictly between the
    /// keepalive interval and the wall cap: larger than the interval so a
    /// quiet-but-alive command (which answers a keepalive every ~30s) never
    /// trips it, and smaller than the wall cap so a dead connection fails fast
    /// instead of hanging the pipeline for the full 30 minutes. Lock that
    /// ordering so a future tweak to any one constant can't silently break it.
    #[test]
    fn no_progress_abort_sits_between_keepalive_and_wall_cap() {
        // Keepalive interval configured on the session (`set_keepalive(true, 30)`).
        const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
        assert!(
            NO_PROGRESS_ABORT > KEEPALIVE_INTERVAL,
            "must outlast a keepalive cycle so silent-but-alive commands survive",
        );
        assert!(
            NO_PROGRESS_ABORT < TRANSPORT_WALL_CAP,
            "must fire before the wall cap so a dead connection fails fast",
        );
    }

    /// The boundary is inclusive at exactly `NO_PROGRESS_ABORT` and never trips
    /// before it — so a healthy session whose keepalives keep resetting the
    /// life clock (`since_last_life` stays near zero) is never declared dead.
    #[test]
    fn no_progress_expires_only_at_or_past_the_window() {
        assert!(!no_progress_expired(Duration::from_secs(0)));
        assert!(!no_progress_expired(
            NO_PROGRESS_ABORT - Duration::from_millis(1)
        ));
        assert!(no_progress_expired(NO_PROGRESS_ABORT));
        assert!(no_progress_expired(
            NO_PROGRESS_ABORT + Duration::from_secs(300)
        ));
    }
}

use crate::domain::models::Machine;
use crate::state::AppContext;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::Serialize;
use ssh2::Session;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{ipc::Channel, AppHandle, Emitter, Manager, Runtime, State};

// Phase-2 drain OSC scanner (TERMINAL_ACTIVITY_PLAN §2b). Built and tested as a
// self-contained unit here; the live drain does not use it yet — see the
// `TODO(T2.3)` notes in `drain_local` / `drain_ssh` for where it wires in.
// `allow(dead_code)` until that wiring lands (the scanner's API is exercised
// only by its own tests for now).
#[allow(dead_code)]
pub(crate) mod activity_scanner;

static SESSION_COUNTER: AtomicUsize = AtomicUsize::new(1);

pub enum ReadSource {
    Ssh(Arc<Mutex<ssh2::Channel>>),
    LocalPty(Arc<Mutex<Box<dyn Read + Send>>>),
}

pub enum WriteSink {
    Ssh(Arc<Mutex<ssh2::Channel>>),
    LocalPty(Arc<Mutex<Box<dyn Write + Send>>>),
}

pub struct ActiveSession {
    pub read_source: ReadSource,
    pub write_sink: WriteSink,
    /// Kept alive for the lifetime of the session.
    pub _keepalive: Arc<Mutex<SessionKeepalive>>,
    pub machine_id: String,
    /// Friendly machine name (`Machine.name`) resolved at start, so
    /// `list_terminal_sessions` can hand the frontend a human label for
    /// restored tabs instead of the raw machine id.
    pub machine_name: String,
    pub created_at: u64,
    /// OS process id of the local shell child, used by the foreground-agent
    /// detector to walk the process tree. `None` for SSH sessions (the
    /// child lives on the remote host, out of reach of local `ps`).
    pub child_pid: Option<u32>,
    /// Coding-agent kind currently detected in this session (`"claude-code"`,
    /// `"opencode"`, …) or `None` for a plain shell. Seeded from the launch
    /// command; the detector overwrites it for local sessions as the
    /// foreground process changes.
    pub agent: Mutex<Option<String>>,
    /// Wall-clock instant of the most recent chunk this session read off its
    /// PTY/SSH transport. Written by the drain thread on every chunk (local
    /// and SSH feed this one shared field) and read by the activity sweep to
    /// resolve `working` (recent output) vs `awaiting_input` (gone quiet).
    /// Seeded to the session's start instant so a freshly-started session
    /// reads as recently-active. Shared (`Arc`) so the drain thread can
    /// update it after the transport is swapped in on reconnect.
    pub last_output_at: Arc<Mutex<Instant>>,
    /// Output fan-out + scrollback for the session (TERMINALS_VIEW_SPEC
    /// §3). The drain thread appends every chunk to the scrollback ring
    /// and broadcasts it to every attached channel; a freshly-attached
    /// surface replays the accumulated scrollback so no output is ever
    /// lost between `start` and the first `attach`, and none is doubled.
    pub frontend_channel: Arc<Mutex<Broadcast>>,
    /// User-supplied tab title. `None` until the frontend calls
    /// `rename_terminal_session`; truncated/trimmed server-side.
    pub display_title: Mutex<Option<String>>,
    /// Spawn parameters retained so `reconnect_terminal_session` can
    /// rebuild an identical transport (same cwd / branch bootstrap) after
    /// an unexpected drop (TERMINALS_VIEW_SPEC §3.1).
    pub work_dir: Option<String>,
    pub work_branch: Option<String>,
    /// `true` while a live PTY/SSH child is attached. The drain thread
    /// flips it to `false` when it exits on an unexpected transport drop;
    /// `reconnect_terminal_session` flips it back to `true`. Reconnect
    /// refuses to run while this is `true` (transport still live).
    pub connected: Arc<AtomicBool>,
}

/// Maximum bytes retained in a session's scrollback ring. Caps backend
/// memory per session; trimming happens on whole-chunk boundaries so a
/// replay never starts mid-escape-sequence (TERMINALS_VIEW_SPEC §3, §8).
const SCROLLBACK_MAX_BYTES: usize = 256 * 1024;

/// Fallback PTY dimensions used when the frontend does not supply a size at
/// session start (e.g. reconnect, or a caller that has not measured its
/// surface yet). Kept at the classic 80x24 — crucially *narrower* than any
/// realistic terminal viewport so the shell's first prompt never wraps wider
/// than the visible area (a wider default made Powerlevel10k's full-width
/// frame wrap and the command line appear duplicated). The frontend sends the
/// real size via `resize_terminal_session` right after it mounts and fits.
const DEFAULT_TERM_COLS: u16 = 80;
const DEFAULT_TERM_ROWS: u16 = 24;

/// Per-session output fan-out with a bounded scrollback buffer, all
/// guarded by a single mutex so attach-replay and live-broadcast are
/// exactly ordered (TERMINALS_VIEW_SPEC §3). Replaces the PR #58
/// seed-channel / `consumeStartupReplay` mechanism, which duplicated
/// output during the attach window and leaked a phantom subscriber.
pub struct Broadcast {
    /// Every attached surface is one element. The drain thread clones a
    /// snapshot under the lock then `send`s outside it, so a slow/dead
    /// subscriber never blocks the others.
    pub channels: Vec<Channel<Vec<u8>>>,
    /// Whole output chunks, oldest first. Never split a chunk — trimming
    /// on chunk boundaries keeps the first repaint from cutting an escape
    /// sequence.
    pub scrollback: VecDeque<Vec<u8>>,
    /// Running total of `scrollback` byte lengths (avoids re-summing the
    /// ring on every chunk).
    pub scrollback_bytes: usize,
}

impl Broadcast {
    pub(crate) fn new() -> Self {
        Broadcast {
            channels: Vec::new(),
            scrollback: VecDeque::new(),
            scrollback_bytes: 0,
        }
    }

    /// Append a chunk to the scrollback ring and trim to the byte cap on
    /// whole-chunk boundaries. Called under the `Broadcast` lock.
    fn push_scrollback(&mut self, chunk: &[u8]) {
        self.scrollback.push_back(chunk.to_vec());
        self.scrollback_bytes += chunk.len();
        while self.scrollback_bytes > SCROLLBACK_MAX_BYTES {
            match self.scrollback.pop_front() {
                Some(dropped) => self.scrollback_bytes -= dropped.len(),
                None => break,
            }
        }
    }

    /// Concatenate the whole scrollback ring into one buffer for replay
    /// to a newly-attached channel. Called under the `Broadcast` lock.
    fn snapshot_scrollback(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.scrollback_bytes);
        for chunk in &self.scrollback {
            out.extend_from_slice(chunk);
        }
        out
    }
}

pub enum SessionKeepalive {
    Ssh {
        #[allow(dead_code)]
        session: Session,
        #[allow(dead_code)]
        tcp: TcpStream,
    },
    LocalPty {
        /// Kept alive for PTY resize operations.
        master: Box<dyn portable_pty::MasterPty + Send>,
        #[allow(dead_code)]
        child: Box<dyn portable_pty::Child + Send + Sync>,
    },
}

/// The I/O handles a freshly started session hands back: the reader, the
/// writer, a shared keepalive guard, and the child shell's pid (`None` when
/// the transport can't report one). Shared by the local-PTY and SSH start
/// paths so their signatures stay in lock-step.
type SessionHandles = (
    ReadSource,
    WriteSink,
    Arc<Mutex<SessionKeepalive>>,
    Option<u32>,
);

#[derive(Default)]
pub struct SessionState {
    pub sessions: Mutex<HashMap<String, ActiveSession>>,
}

#[derive(Serialize, Clone)]
pub struct SessionInfo {
    pub session_id: String,
    pub machine_id: String,
    pub created_at: u64,
    /// User-supplied tab title (`None` until renamed). New optional field
    /// — additive on the wire, ignored by older frontends (spec §2.1).
    pub title: Option<String>,
    /// Friendly machine name (`Machine.name`, e.g. "prod-gpu"), resolved at
    /// session start. `None` on lifecycle events (disconnect/ended/running)
    /// where the frontend already knows the label from the existing tab.
    /// Lets startup-reconcile show a human name instead of the raw id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_name: Option<String>,
    /// Coding-agent kind currently running in the session (e.g.
    /// `"claude-code"`, `"opencode"`), or `None` for a plain shell. Seeded
    /// from the launch command and, for local sessions, kept live by the
    /// foreground-process detector. Additive on the wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// Payload for the additive `terminal-session-activity` event
/// (TERMINAL_ACTIVITY_PLAN §2). `state` is one of `"working"`,
/// `"awaiting_input"`, or `"exit"` (the latter reserved for Phase 2 — the
/// Phase 1 cadence sweep only ever emits the first two). Kept a distinct
/// struct from `SessionInfo` so the activity wire shape stays minimal
/// (`{ session_id, state }`) and independent of the session-lifecycle
/// envelope. serde serialises the field names as-is.
#[derive(Serialize, Clone)]
pub struct ActivityInfo {
    pub session_id: String,
    pub state: String,
}

const IDLE_TIMEOUT_SECS: u64 = 600;

#[tauri::command]
pub fn set_machine_secret(machine_id: String, secret: String) -> Result<(), String> {
    let entry = keyring::Entry::new("demeteo", &format!("machine_{}", machine_id))
        .map_err(|e| format!("Keyring error: {}", e))?;
    entry
        .set_password(&secret)
        .map_err(|e| format!("Failed to store secret in keyring: {}", e))?;
    crate::credential_cache::set(&format!("machine_{}", machine_id), &secret);
    Ok(())
}

#[tauri::command]
pub fn delete_machine_secret(machine_id: String) -> Result<(), String> {
    let entry = keyring::Entry::new("demeteo", &format!("machine_{}", machine_id))
        .map_err(|e| format!("Keyring error: {}", e))?;
    let _ = entry.delete_credential();
    crate::credential_cache::invalidate(&format!("machine_{}", machine_id));
    Ok(())
}

pub fn connect_ssh(
    machine: &Machine,
    secret: Option<String>,
) -> Result<(Session, TcpStream), String> {
    crate::ssh_util::connect(machine, secret)
}

/// Starts a terminal session on the given machine.
///
/// `agent_kind` is the coding-agent kind the frontend is launching into this
/// fresh session (e.g. `"claude-code"`), or `None` for a plain shell. It
/// seeds the session's agent label immediately so the tab shows the badge
/// before the foreground detector has run its first pass.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn start_terminal_session(
    app: AppHandle,
    ctx: State<'_, AppContext>,
    session_state: State<'_, SessionState>,
    machine_id: String,
    work_dir: Option<String>,
    work_branch: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    agent_kind: Option<String>,
) -> Result<String, String> {
    let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
        &*ctx.machines,
        &machine_id,
    )?;
    let machine_name = machine.name.clone();
    // Resolve the initial PTY size so the shell draws its very first prompt at
    // (near) the real terminal width. `0` is treated as "unset" defensively —
    // a zero-column PTY would make prompts render incoherently.
    let cols = cols.filter(|c| *c > 0).unwrap_or(DEFAULT_TERM_COLS);
    let rows = rows.filter(|r| *r > 0).unwrap_or(DEFAULT_TERM_ROWS);

    let session_id = format!("sess_{}", SESSION_COUNTER.fetch_add(1, Ordering::SeqCst));
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Seed an empty `Broadcast`: no subscriber and no permanent seed
    // channel. Output produced before the surface attaches (shell
    // startup, the `git checkout` bootstrap, the first prompt) flows
    // into the scrollback ring and is replayed on the first
    // `attach_terminal_session`, so nothing is lost and nothing races
    // the attach (TERMINALS_VIEW_SPEC §3).
    let frontend_channel: Arc<Mutex<Broadcast>> = Arc::new(Mutex::new(Broadcast::new()));
    let display_title: Mutex<Option<String>> = Mutex::new(None);
    let connected = Arc::new(AtomicBool::new(true));
    // Seed the last-output instant to "now" so the session reads as
    // recently-active until its first quiet window elapses.
    let last_output_at: Arc<Mutex<Instant>> = Arc::new(Mutex::new(Instant::now()));

    let (read_source, write_sink, keepalive, child_pid) = if machine.auth_type == "local" {
        start_local_pty(&machine_id, &work_dir, &work_branch, cols, rows)?
    } else {
        start_ssh_session(&machine, &work_dir, &work_branch, cols, rows)?
    };
    // Normalise an empty agent kind to `None` so a plain shell never carries
    // a phantom badge.
    let agent_kind = agent_kind.filter(|k| !k.trim().is_empty());

    spawn_drain(
        &read_source,
        app.clone(),
        session_id.clone(),
        machine_id.clone(),
        created_at,
        frontend_channel.clone(),
        connected.clone(),
        last_output_at.clone(),
    );

    let mut sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    sessions.insert(
        session_id.clone(),
        ActiveSession {
            read_source,
            write_sink,
            _keepalive: keepalive,
            machine_id: machine_id.clone(),
            machine_name: machine_name.clone(),
            created_at,
            child_pid,
            agent: Mutex::new(agent_kind.clone()),
            last_output_at,
            frontend_channel,
            display_title,
            work_dir,
            work_branch,
            connected,
        },
    );

    let _ = app.emit(
        "terminal-session-started",
        SessionInfo {
            session_id: session_id.clone(),
            machine_id,
            created_at,
            title: None,
            machine_name: Some(machine_name),
            agent: agent_kind,
        },
    );

    Ok(session_id)
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

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
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
    Ok((read_source, write_sink, keepalive, child_pid))
}

/// Build the bootstrap line that performs a `git checkout` of the supplied
/// feature branch on PTY/SSH startup. Returns `None` when no branch was
/// supplied, so callers can skip the write entirely for `ProjectHome`-style
/// flows (no pipeline context).
///
/// The line is shell-escaped defensively — the feature id is generated, but
/// a branch containing a stray quote or `;` would otherwise become a
/// command-injection vector. The trailing `clear` mirrors the existing
/// `cd … && clear` behaviour in the SSH path so the prompt lands cleanly.
/// Missing-branch failures are intentionally tolerated (`|| true`-style
/// fallback) so a not-yet-started feature still opens a usable terminal.
fn branch_bootstrap_line(branch: &Option<String>) -> Option<String> {
    let raw = branch.as_ref()?.trim();
    if raw.is_empty() {
        return None;
    }
    let safe = crate::paths::shell_escape_posix(raw);
    Some(format!(
        "git checkout {safe} 2>/dev/null || git switch {safe} 2>/dev/null; clear\n"
    ))
}

fn start_ssh_session(
    machine: &Machine,
    work_dir: &Option<String>,
    work_branch: &Option<String>,
    cols: u16,
    rows: u16,
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

    if let Some(dir) = work_dir {
        let cd_cmd = format!("cd {} && clear\n", crate::paths::shell_escape_posix(dir));
        let _ = ssh_chan.write_all(cd_cmd.as_bytes());
        let _ = ssh_chan.flush();
    }
    if let Some(bootstrap) = branch_bootstrap_line(work_branch) {
        let _ = ssh_chan.write_all(bootstrap.as_bytes());
        let _ = ssh_chan.flush();
    }

    sess.set_blocking(false);
    let arc_chan = Arc::new(Mutex::new(ssh_chan));
    let read_source = ReadSource::Ssh(arc_chan.clone());
    let write_sink = WriteSink::Ssh(arc_chan);
    let keepalive = Arc::new(Mutex::new(SessionKeepalive::Ssh { session: sess, tcp }));
    // No local pid for a remote session — the shell runs on the far host.
    Ok((read_source, write_sink, keepalive, None))
}

/// Spawns the appropriate drain thread for a freshly-built transport,
/// forwarding output into the session's `Broadcast`. Shared by
/// `start_terminal_session` and `reconnect_terminal_session` so both wire
/// up the drain identically (TERMINALS_VIEW_SPEC §3.1).
#[allow(clippy::too_many_arguments)]
fn spawn_drain<R: Runtime>(
    read_source: &ReadSource,
    app: AppHandle<R>,
    session_id: String,
    machine_id: String,
    created_at: u64,
    frontend_channel: Arc<Mutex<Broadcast>>,
    connected: Arc<AtomicBool>,
    last_output_at: Arc<Mutex<Instant>>,
) {
    match read_source {
        ReadSource::Ssh(ch) => {
            let ch = ch.clone();
            thread::spawn(move || {
                drain_ssh(
                    ch,
                    app,
                    session_id,
                    machine_id,
                    created_at,
                    frontend_channel,
                    connected,
                    last_output_at,
                );
            });
        }
        ReadSource::LocalPty(reader) => {
            let reader = reader.clone();
            thread::spawn(move || {
                drain_local(
                    reader,
                    app,
                    session_id,
                    machine_id,
                    created_at,
                    frontend_channel,
                    connected,
                    last_output_at,
                );
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn drain_ssh<R: Runtime>(
    ch: Arc<Mutex<ssh2::Channel>>,
    app: AppHandle<R>,
    session_id: String,
    machine_id: String,
    created_at: u64,
    frontend_channel: Arc<Mutex<Broadcast>>,
    connected: Arc<AtomicBool>,
    last_output_at: Arc<Mutex<Instant>>,
) {
    // Re-seed to "now" at drain start so the idle timeout (and the activity
    // sweep) measure from this transport's lifetime, not a stale value carried
    // over the shared field from a long-ago disconnect on reconnect.
    touch_last_output(&last_output_at);
    let mut buffer = [0u8; 8192];
    loop {
        let result = ch.lock().unwrap().read(&mut buffer);
        match result {
            Ok(0) => {
                emit_disconnected(&app, &session_id, &machine_id, created_at, &connected);
                break;
            }
            Ok(n) => {
                // Feed the one shared last-output field (also read by the
                // idle-timeout check below and the activity sweep) so both
                // transports have a single source of truth.
                touch_last_output(&last_output_at);
                // TODO(T2.3): wire into drain_local/drain_ssh — run this chunk
                // through the session's `activity_scanner::ActivityScanner`
                // here, broadcast `ScanOutput.forward` (with our OSC stripped)
                // instead of the raw chunk, and emit `terminal-session-activity`
                // for each parsed `ScanOutput.events` state. Left out of this
                // task so the drain hot path is untouched until the launch-line
                // work (T2.3) lands the per-session nonce.
                let chunk = buffer[..n].to_vec();
                send_chunk(&frontend_channel, chunk);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if elapsed_since_last_output(&last_output_at).as_secs() > IDLE_TIMEOUT_SECS {
                    emit_disconnected(&app, &session_id, &machine_id, created_at, &connected);
                    break;
                }
                thread::sleep(Duration::from_millis(15));
            }
            Err(_) => {
                emit_disconnected(&app, &session_id, &machine_id, created_at, &connected);
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_local<R: Runtime>(
    reader: Arc<Mutex<Box<dyn Read + Send>>>,
    app: AppHandle<R>,
    session_id: String,
    machine_id: String,
    created_at: u64,
    frontend_channel: Arc<Mutex<Broadcast>>,
    connected: Arc<AtomicBool>,
    last_output_at: Arc<Mutex<Instant>>,
) {
    // Re-seed to "now" at drain start so the activity sweep measures from this
    // transport's lifetime rather than a stale value carried over on reconnect.
    touch_last_output(&last_output_at);
    let mut buffer = [0u8; 8192];
    loop {
        let result = reader.lock().unwrap().read(&mut buffer);
        match result {
            Ok(0) | Err(_) => {
                emit_disconnected(&app, &session_id, &machine_id, created_at, &connected);
                break;
            }
            Ok(n) => {
                // Feed the shared last-output field the activity sweep reads.
                touch_last_output(&last_output_at);
                // TODO(T2.3): wire into drain_local/drain_ssh — same seam as the
                // SSH drain above: feed the chunk through the session's
                // `activity_scanner::ActivityScanner`, broadcast the stripped
                // `forward` bytes, and emit `terminal-session-activity` per
                // parsed event. Deferred with the launch-line/nonce work (T2.3).
                let chunk = buffer[..n].to_vec();
                send_chunk(&frontend_channel, chunk);
            }
        }
    }
}

/// Stamp a session's shared last-output instant to "now". Called by both
/// drain transports on every chunk so the activity sweep has one source of
/// truth for cadence. A poisoned lock is swallowed — a missed stamp only
/// makes the sweep briefly read the session as quieter than it is, never a
/// crash on the hot output path.
fn touch_last_output(last_output_at: &Arc<Mutex<Instant>>) {
    if let Ok(mut slot) = last_output_at.lock() {
        *slot = Instant::now();
    }
}

/// How long since a session last produced output. A poisoned lock reads as
/// zero elapsed (treated as recently-active) so a transient poisoning never
/// spuriously flips a session to `awaiting_input`.
fn elapsed_since_last_output(last_output_at: &Arc<Mutex<Instant>>) -> Duration {
    last_output_at
        .lock()
        .map(|slot| slot.elapsed())
        .unwrap_or(Duration::ZERO)
}

/// The transport (PTY/SSH child) dropped unexpectedly. Mark the session
/// disconnected but KEEP it in the map — its `Broadcast` (scrollback +
/// title) survives so `reconnect_terminal_session` can rebuild the
/// transport in place (TERMINALS_VIEW_SPEC §3.1). Distinct from
/// `emit_ended`, which fires only on an explicit close that removes the
/// session.
fn emit_disconnected<R: Runtime>(
    app: &AppHandle<R>,
    session_id: &str,
    machine_id: &str,
    created_at: u64,
    connected: &Arc<AtomicBool>,
) {
    // Emit only on the genuine connected→disconnected transition. An
    // explicit close (`close_terminal_session` / `close_machine_sessions`)
    // pre-sets `connected = false` before tearing the transport down, so
    // the drain thread's subsequent EOF is swallowed here instead of
    // racing a spurious `terminal-session-disconnected` in *after* the
    // `terminal-session-ended` the close already emitted. `swap` returns
    // the previous value: `false` means someone (a close, or an earlier
    // disconnect) already claimed the transition, so we bail.
    if !connected.swap(false, Ordering::SeqCst) {
        return;
    }
    let _ = app.emit(
        "terminal-session-disconnected",
        SessionInfo {
            session_id: session_id.to_string(),
            machine_id: machine_id.to_string(),
            created_at,
            title: None,
            machine_name: None,
            agent: None,
        },
    );
}

/// The session was explicitly closed (tab close / kill-all / tray
/// cleanup) and removed from the map. Fires `terminal-session-ended` so
/// listeners can distinguish a permanent close from a recoverable
/// disconnect (TERMINALS_VIEW_SPEC §3.1).
fn emit_ended<R: Runtime>(app: &AppHandle<R>, session_id: &str, machine_id: &str, created_at: u64) {
    let _ = app.emit(
        "terminal-session-ended",
        SessionInfo {
            session_id: session_id.to_string(),
            machine_id: machine_id.to_string(),
            created_at,
            title: None,
            machine_name: None,
            agent: None,
        },
    );
}

/// Appends a chunk to the session's scrollback and broadcasts it to
/// every currently-attached subscriber.
///
/// Under the lock we do two cheap things: push the chunk into the
/// scrollback ring (trimming to the byte cap on whole-chunk
/// boundaries) and clone the channel list. The actual `send()` calls
/// happen outside the lock so a slow/dead subscriber cannot block
/// attach/detach or the scrollback bookkeeping. Per-channel errors are
/// swallowed: one dead subscriber must not prevent the others from
/// receiving output. When a `send` fails, the dead channel is pruned so
/// subsequent chunks don't keep cloning the payload for a subscriber
/// that can never receive it (TERMINALS_VIEW_SPEC §3).
pub(crate) fn send_chunk(frontend_channel: &Arc<Mutex<Broadcast>>, chunk: Vec<u8>) {
    let snapshot: Vec<Channel<Vec<u8>>> = {
        let mut guard = match frontend_channel.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        guard.push_scrollback(&chunk);
        guard.channels.clone()
    };
    for chan in &snapshot {
        if chan.send(chunk.clone()).is_err() {
            if let Ok(mut guard) = frontend_channel.lock() {
                guard.channels.retain(|c| c.id() != chan.id());
            }
        }
    }
}

#[tauri::command]
pub fn write_terminal_session(
    session_state: State<'_, SessionState>,
    session_id: String,
    data: String,
) -> Result<(), String> {
    let sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    if let Some(active) = sessions.get(&session_id) {
        match &active.write_sink {
            WriteSink::Ssh(ch) => {
                let mut chan = ch
                    .lock()
                    .map_err(|_| "Failed to lock channel".to_string())?;
                chan.write_all(data.as_bytes())
                    .map_err(|e| format!("Failed to write to terminal: {}", e))?;
                chan.flush()
                    .map_err(|e| format!("Failed to flush terminal: {}", e))?;
            }
            WriteSink::LocalPty(writer) => {
                let mut w = writer
                    .lock()
                    .map_err(|_| "Failed to lock PTY writer".to_string())?;
                w.write_all(data.as_bytes())
                    .map_err(|e| format!("Failed to write to PTY: {}", e))?;
                w.flush()
                    .map_err(|e| format!("Failed to flush PTY: {}", e))?;
            }
        }
        Ok(())
    } else {
        Err("Session not found".to_string())
    }
}

#[tauri::command]
pub fn resize_terminal_session(
    session_state: State<'_, SessionState>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    let sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    if let Some(active) = sessions.get(&session_id) {
        match &active.write_sink {
            WriteSink::Ssh(ch) => {
                let mut chan = ch
                    .lock()
                    .map_err(|_| "Failed to lock channel".to_string())?;
                chan.request_pty_size(cols, rows, None, None)
                    .map_err(|e| format!("Failed to resize terminal: {}", e))?;
            }
            WriteSink::LocalPty(_) => {
                if let Ok(keepalive) = active._keepalive.lock() {
                    if let SessionKeepalive::LocalPty { master, .. } = &*keepalive {
                        master
                            .resize(PtySize {
                                rows: rows as u16,
                                cols: cols as u16,
                                pixel_width: 0,
                                pixel_height: 0,
                            })
                            .map_err(|e| format!("Failed to resize PTY: {}", e))?;
                    }
                }
            }
        }
        Ok(())
    } else {
        Err("Session not found".to_string())
    }
}

#[tauri::command]
pub fn close_terminal_session(
    app: AppHandle,
    session_state: State<'_, SessionState>,
    session_id: String,
) -> Result<(), String> {
    let mut sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    if let Some(active) = sessions.remove(&session_id) {
        // Claim the connected→disconnected transition before tearing the
        // transport down, so the drain thread's EOF does not emit a
        // spurious `terminal-session-disconnected` after the
        // `terminal-session-ended` below.
        active.connected.store(false, Ordering::SeqCst);
        match &active.write_sink {
            WriteSink::Ssh(ch) => {
                let mut chan = ch
                    .lock()
                    .map_err(|_| "Failed to lock channel".to_string())?;
                let _ = chan.close();
            }
            WriteSink::LocalPty(_) => {
                // Child is killed when keepalive drops
            }
        }
        // Explicit close → permanent end. Drop the sessions lock before
        // emitting to keep the critical section tight.
        let machine_id = active.machine_id.clone();
        let created_at = active.created_at;
        drop(sessions);
        emit_ended(&app, &session_id, &machine_id, created_at);
        Ok(())
    } else {
        Err("Session not found".to_string())
    }
}

#[tauri::command]
pub fn list_terminal_sessions(
    session_state: State<'_, SessionState>,
) -> Result<Vec<SessionInfo>, String> {
    let sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    Ok(sessions
        .iter()
        .map(|(id, s)| SessionInfo {
            session_id: id.clone(),
            machine_id: s.machine_id.clone(),
            created_at: s.created_at,
            title: s.display_title.lock().ok().and_then(|g| g.clone()),
            machine_name: Some(s.machine_name.clone()),
            agent: s.agent.lock().ok().and_then(|g| g.clone()),
        })
        .collect())
}

#[tauri::command]
pub fn close_machine_sessions(
    app: AppHandle,
    session_state: State<'_, SessionState>,
    machine_id: String,
) -> Result<usize, String> {
    let ended: Vec<(String, u64)> = {
        let mut sessions = session_state
            .sessions
            .lock()
            .map_err(|_| "Failed to lock sessions".to_string())?;
        let to_close: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| s.machine_id == machine_id)
            .map(|(id, _)| id.clone())
            .collect();
        to_close
            .into_iter()
            .filter_map(|id| {
                sessions.remove(&id).map(|s| {
                    // Claim the transition so the drain thread's EOF is
                    // swallowed rather than emitting a spurious
                    // `terminal-session-disconnected` after the ended
                    // event below.
                    s.connected.store(false, Ordering::SeqCst);
                    (id, s.created_at)
                })
            })
            .collect()
    };
    let count = ended.len();
    // Explicit kill-all → each removed session is a permanent end.
    for (id, created_at) in &ended {
        emit_ended(&app, id, &machine_id, *created_at);
    }
    Ok(count)
}

#[tauri::command]
pub fn attach_terminal_session(
    session_state: State<'_, SessionState>,
    session_id: String,
    tauri_channel: Channel<Vec<u8>>,
) -> Result<(), String> {
    // Clone the `Broadcast` handle and drop the sessions lock before the
    // (potentially large) scrollback replay, so a replay never blocks
    // other session commands.
    let broadcast = {
        let sessions = session_state
            .sessions
            .lock()
            .map_err(|_| "Failed to lock sessions".to_string())?;
        sessions
            .get(&session_id)
            .ok_or_else(|| "Session not found".to_string())?
            .frontend_channel
            .clone()
    };

    let mut guard = broadcast
        .lock()
        .map_err(|_| "Failed to lock frontend channel".to_string())?;
    // Deduplicate by channel id: a rapid remount (useEffect cleanup
    // racing the next mount's attach) can otherwise pile duplicate
    // subscribers onto the Vec, and every output chunk would then
    // clone the payload once per stale entry. If the same id is
    // already present we replace it in place so the existing position
    // is preserved (LIFO detach stays predictable).
    let new_id = tauri_channel.id();
    if let Some(pos) = guard.channels.iter().position(|c| c.id() == new_id) {
        guard.channels[pos] = tauri_channel.clone();
    } else {
        guard.channels.push(tauri_channel.clone());
    }
    // Replay the accumulated scrollback to ONLY the newly-attached
    // channel, while still holding the `Broadcast` lock. Holding the
    // lock guarantees the ordering `scrollback → live`: any concurrent
    // `send_chunk` serializes on this lock, so it cannot enqueue a live
    // chunk to this channel before the scrollback lands, and — because
    // the chunk was appended to scrollback under the same lock — the
    // replay never duplicates a live chunk this subscriber also sees.
    // Existing subscribers are untouched: the replay targets the new
    // channel alone.
    if guard.scrollback_bytes > 0 {
        let replay = guard.snapshot_scrollback();
        let _ = tauri_channel.send(replay);
    }
    Ok(())
}

#[tauri::command]
pub fn detach_terminal_session(
    session_state: State<'_, SessionState>,
    session_id: String,
    channel_id: Option<u32>,
) -> Result<(), String> {
    let sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    if let Some(active) = sessions.get(&session_id) {
        if let Ok(mut guard) = active.frontend_channel.lock() {
            match channel_id {
                Some(id) => {
                    // Channel-specific detach: the caller knows exactly
                    // which subscriber it owns (via `Channel::id()` on
                    // the frontend side) and only that entry is removed.
                    // This is race-safe against a fresh attach that
                    // happens to be racing the unmount cleanup of a
                    // previous surface — the cleanup can't accidentally
                    // pop the new channel.
                    let before = guard.channels.len();
                    guard.channels.retain(|c| c.id() != id);
                    if guard.channels.len() == before {
                        // Unknown id → no-op. We deliberately do NOT
                        // fall back to LIFO pop here: the caller
                        // committed to a specific id, and a stale id
                        // means either the attach raced us (and is now
                        // gone) or the caller is mistaken. Either way
                        // we don't want to evict a peer subscriber.
                    }
                }
                None => {
                    // Backward-compat fallback for callers that don't
                    // track channel identity: LIFO pop. Matches the V1
                    // single-subscriber semantics — the typical V1 case
                    // has only one channel attached per session, so
                    // the pop removes it.
                    guard.channels.pop();
                }
            }
        }
        Ok(())
    } else {
        Err("Session not found".to_string())
    }
}

/// Persist a user-supplied tab title for the session. The frontend calls
/// this on every commit of the inline rename input. We trim
/// surrounding whitespace and length-cap at 64 chars server-side so a
/// runaway UI input cannot balloon the panel or leak into log lines.
/// An empty string after trim is treated as "clear the title" and
/// stores `None` so the panel falls back to its default-name strategy.
#[tauri::command]
pub fn rename_terminal_session(
    session_state: State<'_, SessionState>,
    session_id: String,
    title: String,
) -> Result<(), String> {
    const TITLE_MAX_CHARS: usize = 64;
    let trimmed = title.trim();
    let stored = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(TITLE_MAX_CHARS).collect::<String>())
    };
    let mut sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    let active = sessions
        .get_mut(&session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    let mut title_slot = active
        .display_title
        .lock()
        .map_err(|_| "Failed to lock title".to_string())?;
    *title_slot = stored;
    Ok(())
}

/// Re-establish the transport for a disconnected session in place. The
/// session shell — id, scrollback, subscribers, title — survived the
/// drop; here we spawn a fresh PTY/SSH child (re-running the same cwd /
/// branch bootstrap), attach it to the SAME `Broadcast`, spawn a new
/// drain thread on it, and emit `terminal-session-running`. Scrollback is
/// preserved as history and the new child's output appends after it
/// (TERMINALS_VIEW_SPEC §3.1). Errors if the session id is unknown or the
/// session is still connected.
#[tauri::command]
pub fn reconnect_terminal_session(
    app: AppHandle,
    ctx: State<'_, AppContext>,
    session_state: State<'_, SessionState>,
    session_id: String,
) -> Result<(), String> {
    let machine_id = {
        let sessions = session_state
            .sessions
            .lock()
            .map_err(|_| "Failed to lock sessions".to_string())?;
        sessions
            .get(&session_id)
            .ok_or_else(|| "Session not found".to_string())?
            .machine_id
            .clone()
    };
    let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
        &*ctx.machines,
        &machine_id,
    )?;
    reconnect_with_machine(&app, &machine, &session_state, &session_id)
}

/// Command-agnostic core of `reconnect_terminal_session` (machine already
/// resolved). Rebuilds the transport for a disconnected session in place,
/// re-attaches it to the existing `Broadcast`, spawns a fresh drain
/// thread, and emits `terminal-session-running`. Split out so it can be
/// tested against a `local_machine()` without a full `AppContext`.
pub(crate) fn reconnect_with_machine<R: Runtime>(
    app: &AppHandle<R>,
    machine: &Machine,
    session_state: &SessionState,
    session_id: &str,
) -> Result<(), String> {
    // Snapshot the spawn params + shared handles, then release the lock.
    let (
        machine_id,
        work_dir,
        work_branch,
        frontend_channel,
        created_at,
        connected,
        last_output_at,
    ) = {
        let sessions = session_state
            .sessions
            .lock()
            .map_err(|_| "Failed to lock sessions".to_string())?;
        let active = sessions
            .get(session_id)
            .ok_or_else(|| "Session not found".to_string())?;
        (
            active.machine_id.clone(),
            active.work_dir.clone(),
            active.work_branch.clone(),
            active.frontend_channel.clone(),
            active.created_at,
            active.connected.clone(),
            active.last_output_at.clone(),
        )
    };

    // Atomically claim the reconnect: `true` means a live transport is
    // still attached, so refuse (spec: "errors on an already-connected
    // id"). This also serialises two racing reconnect calls — only one
    // wins the CAS.
    if connected
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("Session is already connected".to_string());
    }

    // Reconnect has no surface size to hand off yet; spawn at the default and
    // let the frontend's post-mount `resize_terminal_session` correct it.
    let built = if machine.auth_type == "local" {
        start_local_pty(
            &machine_id,
            &work_dir,
            &work_branch,
            DEFAULT_TERM_COLS,
            DEFAULT_TERM_ROWS,
        )
    } else {
        start_ssh_session(
            machine,
            &work_dir,
            &work_branch,
            DEFAULT_TERM_COLS,
            DEFAULT_TERM_ROWS,
        )
    };
    let (read_source, write_sink, keepalive, child_pid) = match built {
        Ok(t) => t,
        Err(e) => {
            connected.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };

    spawn_drain(
        &read_source,
        app.clone(),
        session_id.to_string(),
        machine_id.clone(),
        created_at,
        frontend_channel,
        connected.clone(),
        last_output_at,
    );

    // Swap the fresh transport onto the existing session.
    {
        let mut sessions = session_state
            .sessions
            .lock()
            .map_err(|_| "Failed to lock sessions".to_string())?;
        let active = match sessions.get_mut(session_id) {
            Some(a) => a,
            None => {
                // The session was closed while we were rebuilding. Undo
                // the claim; the freshly-built transport drops here and
                // its drain thread exits on the resulting EOF.
                connected.store(false, Ordering::SeqCst);
                return Err("Session not found".to_string());
            }
        };
        active.read_source = read_source;
        active.write_sink = write_sink;
        active._keepalive = keepalive;
        // The rebuilt transport spawned a fresh shell child — repoint the
        // detector at its pid (or clear it for a remote reconnect).
        active.child_pid = child_pid;
    }

    let _ = app.emit(
        "terminal-session-running",
        SessionInfo {
            session_id: session_id.to_string(),
            machine_id,
            created_at,
            title: None,
            machine_name: None,
            agent: None,
        },
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Foreground coding-agent detection
// ---------------------------------------------------------------------------
//
// A background poller walks the local process tree every few seconds and
// reports whether a known coding-agent CLI (Claude, OpenCode, …) is running
// under each *local* session's shell. When a session's detected agent
// changes it emits `terminal-session-agent { session_id, agent }`, so a tab
// shows an accurate "running Claude" badge even when the user launched the
// agent by hand (typed `claude` in an already-open shell) rather than via
// the New menu.
//
// SSH sessions are deliberately skipped: their shell runs on the far host,
// out of reach of a local `ps`, and probing over the shared libssh2 session
// concurrently with the interactive drain thread is unsafe. Remote tabs keep
// the agent label seeded from the launch command instead.

/// Time between foreground-agent detection passes. A few seconds keeps the
/// badge feeling live without spawning `ps` in a tight loop.
const AGENT_DETECT_INTERVAL: Duration = Duration::from_secs(3);

/// Maps a coding-agent CLI binary name to the "kind" the frontend labels.
/// Mirrors the frontend `AGENT_CLI` table (NewTerminalMenu) so the launch
/// path and the detector agree on the same identifiers.
fn agent_kind_for_binary(binary: &str) -> Option<&'static str> {
    match binary {
        "claude" => Some("claude-code"),
        "opencode" => Some("opencode"),
        "hermes" => Some("hermes"),
        "codex" => Some("codex"),
        _ => None,
    }
}

/// Scan a full command line for a known agent CLI. Matches on whole-token
/// basenames (with a `.js`/`.mjs` script suffix stripped) rather than raw
/// substrings, so a path that merely *contains* an agent name doesn't
/// false-positive. Catches native launchers (`claude`, `opencode`, `codex`)
/// directly; node-script installs that appear as `node …/claude` match on
/// the `claude` token.
fn detect_agent_in_command(command: &str) -> Option<&'static str> {
    for token in command.split_whitespace() {
        let base = token.rsplit('/').next().unwrap_or(token);
        let base = base
            .strip_suffix(".js")
            .or_else(|| base.strip_suffix(".mjs"))
            .unwrap_or(base);
        if let Some(kind) = agent_kind_for_binary(base) {
            return Some(kind);
        }
    }
    None
}

/// A one-shot snapshot of the local process table: each pid's parent and
/// the agent (if any) its command line names, plus a ppid→children index
/// for walking a session's subtree.
struct ProcessTree {
    agent_by_pid: HashMap<u32, Option<&'static str>>,
    children: HashMap<u32, Vec<u32>>,
}

impl ProcessTree {
    /// Capture the process table via `ps`. Returns `None` if `ps` is
    /// unavailable or fails (e.g. a locked-down sandbox) — the detector
    /// then simply skips the pass.
    fn capture() -> Option<ProcessTree> {
        let output = std::process::Command::new("ps")
            .args(["-axo", "pid=,ppid=,command="])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let mut agent_by_pid = HashMap::new();
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            let pid = match parts.next().and_then(|s| s.parse::<u32>().ok()) {
                Some(p) => p,
                None => continue,
            };
            let ppid = match parts.next().and_then(|s| s.parse::<u32>().ok()) {
                Some(p) => p,
                None => continue,
            };
            let command = parts.collect::<Vec<_>>().join(" ");
            agent_by_pid.insert(pid, detect_agent_in_command(&command));
            children.entry(ppid).or_default().push(pid);
        }
        Some(ProcessTree {
            agent_by_pid,
            children,
        })
    }

    /// Find the first known agent running at or below `root` (the session's
    /// shell pid). Iterative DFS with a visited guard so a pathological
    /// process table can never spin.
    fn find_agent_under(&self, root: u32) -> Option<&'static str> {
        let mut stack = vec![root];
        let mut visited = std::collections::HashSet::new();
        while let Some(pid) = stack.pop() {
            if !visited.insert(pid) {
                continue;
            }
            if let Some(Some(kind)) = self.agent_by_pid.get(&pid) {
                return Some(kind);
            }
            if let Some(kids) = self.children.get(&pid) {
                stack.extend(kids.iter().copied());
            }
        }
        None
    }
}

/// Spawn the background foreground-agent detector. Runs for the lifetime of
/// the app (a cheap sleeping thread); call once from setup.
pub fn spawn_agent_detector<R: Runtime>(app: AppHandle<R>) {
    thread::spawn(move || loop {
        thread::sleep(AGENT_DETECT_INTERVAL);
        detect_agents_once(&app);
    });
}

/// One detection pass: snapshot local sessions, capture the process table
/// once, diff each session's detected agent against its stored value, and
/// emit `terminal-session-agent` for the changes.
fn detect_agents_once<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<SessionState>();

    // Snapshot (id, shell pid, current agent) for every local session, then
    // release the lock before the (slower) `ps` call.
    let locals: Vec<(String, u32, Option<String>)> = {
        let sessions = match state.sessions.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        sessions
            .iter()
            .filter_map(|(id, s)| {
                let pid = s.child_pid?;
                let current = s.agent.lock().ok().and_then(|g| g.clone());
                Some((id.clone(), pid, current))
            })
            .collect()
    };
    if locals.is_empty() {
        return;
    }

    let tree = match ProcessTree::capture() {
        Some(t) => t,
        None => return,
    };

    let mut changes: Vec<(String, Option<String>)> = Vec::new();
    for (id, pid, current) in locals {
        let detected = tree.find_agent_under(pid).map(|k| k.to_string());
        if detected != current {
            changes.push((id, detected));
        }
    }
    if changes.is_empty() {
        return;
    }

    // Write the new values back (skipping sessions closed since the
    // snapshot), then emit outside the lock.
    {
        let sessions = match state.sessions.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        for (id, detected) in &changes {
            if let Some(s) = sessions.get(id) {
                if let Ok(mut g) = s.agent.lock() {
                    *g = detected.clone();
                }
            }
        }
    }
    for (id, detected) in changes {
        let _ = app.emit(
            "terminal-session-agent",
            SessionInfo {
                session_id: id,
                machine_id: String::new(),
                created_at: 0,
                title: None,
                machine_name: None,
                agent: detected,
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Activity cadence sweep (working ↔ awaiting_input)
// ---------------------------------------------------------------------------
//
// A second background poller — modelled on `spawn_agent_detector` — resolves
// the universal working/waiting floor from the byte cadence the drain already
// records in `ActiveSession.last_output_at` (TERMINAL_ACTIVITY_PLAN §3, §4).
// Every tick it snapshots each session under the lock, releases it, then for
// each session WITH an agent present resolves `working` (output within the
// cadence window) vs `awaiting_input` (gone quiet) and emits
// `terminal-session-activity` ONLY when that state changed since the last
// emit. Plain-shell sessions (agent `None`) are never emitted for.

/// Cadence window: output seen within this of a sweep tick reads as
/// `working`; quieter than this reads as `awaiting_input`
/// (TERMINAL_ACTIVITY_PLAN §7.2 — start at ~1s, tune against real agents).
const CADENCE_WINDOW: Duration = Duration::from_millis(1000);

/// Time between activity sweeps. ~250ms keeps `working` appearing within one
/// tick of the first byte and `awaiting_input` settling ≤ ~1s after silence
/// (TERMINAL_ACTIVITY_PLAN §5).
const ACTIVITY_SWEEP_INTERVAL: Duration = Duration::from_millis(250);

/// Pure cadence decision for a single session, factored out of the sweep loop
/// so it is unit-testable without spinning a real thread. Given the state we
/// last emitted for the session (`None` if we never have), whether an agent is
/// present, and how long since the session last produced output, return the
/// state to emit — or `None` to stay silent.
///
/// Two reasons to stay silent: the session has no agent (a plain shell must
/// NEVER emit — the agent-gate), or the resolved state is unchanged from the
/// last emit (dedup — emit only on real change). Otherwise resolve `working`
/// within the cadence window, else `awaiting_input`.
fn next_activity_emit(
    last_emitted: Option<&str>,
    has_agent: bool,
    since_last_output: Duration,
) -> Option<&'static str> {
    if !has_agent {
        return None;
    }
    let resolved = if since_last_output <= CADENCE_WINDOW {
        "working"
    } else {
        "awaiting_input"
    };
    if last_emitted == Some(resolved) {
        None
    } else {
        Some(resolved)
    }
}

/// Spawn the background activity sweep. Runs for the lifetime of the app (a
/// cheap sleeping thread, like `spawn_agent_detector`); call once from setup.
pub fn spawn_activity_sweep<R: Runtime>(app: AppHandle<R>) {
    thread::spawn(move || {
        // The last state emitted per session, owned by this loop — the single
        // place that decides whether a state is a real change worth emitting.
        let mut last_states: HashMap<String, String> = HashMap::new();
        loop {
            thread::sleep(ACTIVITY_SWEEP_INTERVAL);
            sweep_activity_once(&app, &mut last_states);
        }
    });
}

/// One sweep pass: snapshot each session's (agent-present, quiet-for) under
/// the lock, release it, drop map entries for sessions that disappeared, then
/// resolve + emit outside the lock. Mirrors `detect_agents_once`'s
/// snapshot-then-emit-outside-the-lock shape.
fn sweep_activity_once<R: Runtime>(app: &AppHandle<R>, last_states: &mut HashMap<String, String>) {
    let state = app.state::<SessionState>();

    // Snapshot (id, agent-present, elapsed-since-last-output) for every
    // session, then release the sessions lock before emitting.
    let snapshot: Vec<(String, bool, Duration)> = {
        let sessions = match state.sessions.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        sessions
            .iter()
            .map(|(id, s)| {
                let has_agent = s.agent.lock().map(|g| g.is_some()).unwrap_or(false);
                let elapsed = elapsed_since_last_output(&s.last_output_at);
                (id.clone(), has_agent, elapsed)
            })
            .collect()
    };

    // Forget sessions that disappeared since the last sweep so their stale
    // last-emitted state can't linger (and so a reused id starts clean). No
    // `exit` emit here — that is a Phase 2 concern.
    let live: std::collections::HashSet<&str> =
        snapshot.iter().map(|(id, _, _)| id.as_str()).collect();
    last_states.retain(|id, _| live.contains(id.as_str()));

    for (id, has_agent, elapsed) in &snapshot {
        let last = last_states.get(id).map(|s| s.as_str());
        if let Some(next) = next_activity_emit(last, *has_agent, *elapsed) {
            last_states.insert(id.clone(), next.to_string());
            let _ = app.emit(
                "terminal-session-activity",
                ActivityInfo {
                    session_id: id.clone(),
                    state: next.to_string(),
                },
            );
        }
    }
}

#[cfg(test)]
#[path = "../tests/infrastructure/terminal.rs"]
mod tests;

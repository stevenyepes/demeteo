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
use std::time::Duration;
use tauri::{ipc::Channel, AppHandle, Emitter, Runtime, State};

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
    pub created_at: u64,
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

#[tauri::command]
pub fn start_terminal_session(
    app: AppHandle,
    ctx: State<'_, AppContext>,
    session_state: State<'_, SessionState>,
    machine_id: String,
    work_dir: Option<String>,
    work_branch: Option<String>,
) -> Result<String, String> {
    let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
        &*ctx.machines,
        &machine_id,
    )?;

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

    let (read_source, write_sink, keepalive) = if machine.auth_type == "local" {
        start_local_pty(&machine_id, &work_dir, &work_branch)?
    } else {
        start_ssh_session(&machine, &work_dir, &work_branch)?
    };

    spawn_drain(
        &read_source,
        app.clone(),
        session_id.clone(),
        machine_id.clone(),
        created_at,
        frontend_channel.clone(),
        connected.clone(),
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
            created_at,
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
        },
    );

    Ok(session_id)
}

pub(crate) fn start_local_pty(
    machine_id: &str,
    work_dir: &Option<String>,
    work_branch: &Option<String>,
) -> Result<(ReadSource, WriteSink, Arc<Mutex<SessionKeepalive>>), String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 220,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Failed to open PTY: {}", e))?;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let mut cmd = CommandBuilder::new(&shell);
    cmd.env("TERM", "xterm-256color");
    if let Some(dir) = work_dir {
        cmd.cwd(dir);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("Failed to spawn shell: {}", e))?;

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
    Ok((read_source, write_sink, keepalive))
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
) -> Result<(ReadSource, WriteSink, Arc<Mutex<SessionKeepalive>>), String> {
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
        .request_pty("xterm-256color", None, None)
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
    Ok((read_source, write_sink, keepalive))
}

/// Spawns the appropriate drain thread for a freshly-built transport,
/// forwarding output into the session's `Broadcast`. Shared by
/// `start_terminal_session` and `reconnect_terminal_session` so both wire
/// up the drain identically (TERMINALS_VIEW_SPEC §3.1).
fn spawn_drain<R: Runtime>(
    read_source: &ReadSource,
    app: AppHandle<R>,
    session_id: String,
    machine_id: String,
    created_at: u64,
    frontend_channel: Arc<Mutex<Broadcast>>,
    connected: Arc<AtomicBool>,
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
                );
            });
        }
    }
}

fn drain_ssh<R: Runtime>(
    ch: Arc<Mutex<ssh2::Channel>>,
    app: AppHandle<R>,
    session_id: String,
    machine_id: String,
    created_at: u64,
    frontend_channel: Arc<Mutex<Broadcast>>,
    connected: Arc<AtomicBool>,
) {
    let mut buffer = [0u8; 8192];
    let mut last_activity = std::time::Instant::now();
    loop {
        let result = ch.lock().unwrap().read(&mut buffer);
        match result {
            Ok(0) => {
                emit_disconnected(&app, &session_id, &machine_id, created_at, &connected);
                break;
            }
            Ok(n) => {
                last_activity = std::time::Instant::now();
                let chunk = buffer[..n].to_vec();
                send_chunk(&frontend_channel, chunk);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if last_activity.elapsed().as_secs() > IDLE_TIMEOUT_SECS {
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

pub(crate) fn drain_local<R: Runtime>(
    reader: Arc<Mutex<Box<dyn Read + Send>>>,
    app: AppHandle<R>,
    session_id: String,
    machine_id: String,
    created_at: u64,
    frontend_channel: Arc<Mutex<Broadcast>>,
    connected: Arc<AtomicBool>,
) {
    let mut buffer = [0u8; 8192];
    loop {
        let result = reader.lock().unwrap().read(&mut buffer);
        match result {
            Ok(0) | Err(_) => {
                emit_disconnected(&app, &session_id, &machine_id, created_at, &connected);
                break;
            }
            Ok(n) => {
                let chunk = buffer[..n].to_vec();
                send_chunk(&frontend_channel, chunk);
            }
        }
    }
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
    let (machine_id, work_dir, work_branch, frontend_channel, created_at, connected) = {
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

    let built = if machine.auth_type == "local" {
        start_local_pty(&machine_id, &work_dir, &work_branch)
    } else {
        start_ssh_session(machine, &work_dir, &work_branch)
    };
    let (read_source, write_sink, keepalive) = match built {
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
    }

    let _ = app.emit(
        "terminal-session-running",
        SessionInfo {
            session_id: session_id.to_string(),
            machine_id,
            created_at,
            title: None,
        },
    );
    Ok(())
}

#[cfg(test)]
#[path = "../tests/infrastructure/terminal.rs"]
mod tests;

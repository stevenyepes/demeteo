use crate::domain::models::Machine;
use crate::state::AppContext;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::Serialize;
use ssh2::Session;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;
use tauri::{ipc::Channel, AppHandle, Emitter, State};

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
    /// Every attached subscriber is one element in the `Vec`. The drain
    /// thread holds this lock only long enough to clone a snapshot, then
    /// iterates the snapshot and calls `send` on each channel (spec §2.1
    /// — broadcast over the Vec, swallow per-channel errors so a dead
    /// subscriber never blocks the others).
    pub frontend_channel: Arc<Mutex<Vec<Channel<Vec<u8>>>>>,
    /// User-supplied tab title. `None` until the frontend calls
    /// `rename_terminal_session`; truncated/trimmed server-side.
    pub display_title: Mutex<Option<String>>,
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

/// Wraps the supplied channel in a `Vec`-backed `Mutex` so it can act
/// as the seed subscriber for a freshly-started terminal session. The
/// drain thread broadcasts every output chunk to every element of the
/// `Vec`, so the seed channel captures shell startup output that would
/// otherwise race the first `attach_terminal_session` call from the
/// panel surface.
pub(crate) fn seed_frontend_channel(
    channel: Channel<Vec<u8>>,
) -> Arc<Mutex<Vec<Channel<Vec<u8>>>>> {
    Arc::new(Mutex::new(vec![channel]))
}

#[tauri::command]
pub fn start_terminal_session(
    app: AppHandle,
    ctx: State<'_, AppContext>,
    session_state: State<'_, SessionState>,
    machine_id: String,
    tauri_channel: Channel<Vec<u8>>,
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
    // Seed the subscriber Vec with the channel the frontend already
    // created. Without this, shell startup output, the bootstrap
    // `git checkout`, and the first prompt all reach the drain thread
    // before the surface has a chance to attach and are irretrievably
    // lost (spec §1 AC #1: "the shell prompt reappears without
    // restart"). The supplied channel is a no-op for most callers (its
    // `onmessage` ignores bytes); `attach_terminal_session` later
    // appends the surface's render-bound channel so the user actually
    // sees the output. Id-based dedup avoids growing the Vec when the
    // frontend re-uses the same channel for both the start and the
    // post-mount attach.
    let frontend_channel: Arc<Mutex<Vec<Channel<Vec<u8>>>>> = seed_frontend_channel(tauri_channel);
    let display_title: Mutex<Option<String>> = Mutex::new(None);

    let (read_source, write_sink, keepalive) = if machine.auth_type == "local" {
        start_local_pty(&machine_id, &work_dir, &work_branch)?
    } else {
        start_ssh_session(&machine, &work_dir, &work_branch)?
    };

    let read_app = app.clone();
    let read_session_id = session_id.clone();
    let read_machine_id = machine_id.clone();
    let read_frontend_channel = frontend_channel.clone();

    match &read_source {
        ReadSource::Ssh(ch) => {
            let ch = ch.clone();
            thread::spawn(move || {
                drain_ssh(
                    ch,
                    read_app,
                    read_session_id,
                    read_machine_id,
                    created_at,
                    read_frontend_channel,
                );
            });
        }
        ReadSource::LocalPty(reader) => {
            let reader = reader.clone();
            thread::spawn(move || {
                drain_local(
                    reader,
                    read_app,
                    read_session_id,
                    read_machine_id,
                    created_at,
                    read_frontend_channel,
                );
            });
        }
    }

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

fn drain_ssh(
    ch: Arc<Mutex<ssh2::Channel>>,
    app: AppHandle,
    session_id: String,
    machine_id: String,
    created_at: u64,
    frontend_channel: Arc<Mutex<Vec<Channel<Vec<u8>>>>>,
) {
    let mut buffer = [0u8; 8192];
    let mut last_activity = std::time::Instant::now();
    loop {
        let result = ch.lock().unwrap().read(&mut buffer);
        match result {
            Ok(0) => {
                emit_ended(&app, &session_id, &machine_id, created_at);
                break;
            }
            Ok(n) => {
                last_activity = std::time::Instant::now();
                let chunk = buffer[..n].to_vec();
                send_chunk(&frontend_channel, chunk);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if last_activity.elapsed().as_secs() > IDLE_TIMEOUT_SECS {
                    emit_ended(&app, &session_id, &machine_id, created_at);
                    break;
                }
                thread::sleep(Duration::from_millis(15));
            }
            Err(_) => {
                emit_ended(&app, &session_id, &machine_id, created_at);
                break;
            }
        }
    }
}

pub(crate) fn drain_local(
    reader: Arc<Mutex<Box<dyn Read + Send>>>,
    app: AppHandle,
    session_id: String,
    machine_id: String,
    created_at: u64,
    frontend_channel: Arc<Mutex<Vec<Channel<Vec<u8>>>>>,
) {
    let mut buffer = [0u8; 8192];
    loop {
        let result = reader.lock().unwrap().read(&mut buffer);
        match result {
            Ok(0) | Err(_) => {
                emit_ended(&app, &session_id, &machine_id, created_at);
                break;
            }
            Ok(n) => {
                let chunk = buffer[..n].to_vec();
                send_chunk(&frontend_channel, chunk);
            }
        }
    }
}

fn emit_ended(app: &AppHandle, session_id: &str, machine_id: &str, created_at: u64) {
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

/// Broadcasts a single output chunk to every currently-attached subscriber.
///
/// Lock is held only long enough to clone the channel list (the Vec is
/// small — typically 0 or 1 element). The actual `send()` calls happen
/// outside the lock so a slow/dead subscriber cannot block
/// attach/detach. Per-channel errors are swallowed: one dead subscriber
/// must not prevent the others from receiving output. When a `send`
/// fails, the dead channel is pruned from the Vec so subsequent chunks
/// don't keep cloning the payload for a subscriber that can never
/// receive it.
pub(crate) fn send_chunk(frontend_channel: &Arc<Mutex<Vec<Channel<Vec<u8>>>>>, chunk: Vec<u8>) {
    let snapshot: Vec<Channel<Vec<u8>>> = match frontend_channel.lock() {
        Ok(guard) => guard.iter().cloned().collect(),
        Err(_) => return,
    };
    for chan in &snapshot {
        if chan.send(chunk.clone()).is_err() {
            if let Ok(mut guard) = frontend_channel.lock() {
                guard.retain(|c| c.id() != chan.id());
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
    session_state: State<'_, SessionState>,
    session_id: String,
) -> Result<(), String> {
    let mut sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    if let Some(active) = sessions.remove(&session_id) {
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
    session_state: State<'_, SessionState>,
    machine_id: String,
) -> Result<usize, String> {
    let mut sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    let to_close: Vec<String> = sessions
        .iter()
        .filter(|(_, s)| s.machine_id == machine_id)
        .map(|(id, _)| id.clone())
        .collect();
    let count = to_close.len();
    for id in to_close {
        sessions.remove(&id);
    }
    Ok(count)
}

#[tauri::command]
pub fn attach_terminal_session(
    session_state: State<'_, SessionState>,
    session_id: String,
    tauri_channel: Channel<Vec<u8>>,
) -> Result<(), String> {
    let sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    let active = sessions
        .get(&session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    let mut subscribers = active
        .frontend_channel
        .lock()
        .map_err(|_| "Failed to lock frontend channel".to_string())?;
    // Deduplicate by channel id: a rapid remount (useEffect cleanup
    // racing the next mount's attach) can otherwise pile duplicate
    // subscribers onto the Vec, and every output chunk would then
    // clone the payload once per stale entry. If the same id is
    // already present we replace it in place so the existing position
    // is preserved (LIFO detach stays predictable).
    let new_id = tauri_channel.id();
    if let Some(pos) = subscribers.iter().position(|c| c.id() == new_id) {
        subscribers[pos] = tauri_channel;
    } else {
        subscribers.push(tauri_channel);
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
        if let Ok(mut subscribers) = active.frontend_channel.lock() {
            match channel_id {
                Some(id) => {
                    // Channel-specific detach: the caller knows exactly
                    // which subscriber it owns (via `Channel::id()` on
                    // the frontend side) and only that entry is removed.
                    // This is race-safe against a fresh attach that
                    // happens to be racing the unmount cleanup of a
                    // previous surface — the cleanup can't accidentally
                    // pop the new channel.
                    let before = subscribers.len();
                    subscribers.retain(|c| c.id() != id);
                    if subscribers.len() == before {
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
                    subscribers.pop();
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

#[cfg(test)]
#[path = "../tests/infrastructure/terminal.rs"]
mod tests;

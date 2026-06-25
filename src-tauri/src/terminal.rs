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
    pub frontend_channel: Arc<Mutex<Option<Channel<Vec<u8>>>>>,
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
    tauri_channel: Channel<Vec<u8>>,
    work_dir: Option<String>,
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
    let frontend_channel: Arc<Mutex<Option<Channel<Vec<u8>>>>> =
        Arc::new(Mutex::new(Some(tauri_channel)));

    let (read_source, write_sink, keepalive) = if machine.auth_type == "local" {
        start_local_pty(&machine_id, &work_dir)?
    } else {
        start_ssh_session(&machine, &work_dir)?
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
        },
    );

    let _ = app.emit(
        "terminal-session-started",
        SessionInfo {
            session_id: session_id.clone(),
            machine_id,
            created_at,
        },
    );

    Ok(session_id)
}

fn start_local_pty(
    machine_id: &str,
    work_dir: &Option<String>,
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
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("Failed to take PTY writer: {}", e))?;

    let read_source = ReadSource::LocalPty(Arc::new(Mutex::new(reader)));
    let write_sink = WriteSink::LocalPty(Arc::new(Mutex::new(writer)));
    let keepalive = Arc::new(Mutex::new(SessionKeepalive::LocalPty {
        master: pair.master,
        child,
    }));

    let _ = machine_id; // suppress unused warning
    Ok((read_source, write_sink, keepalive))
}

fn start_ssh_session(
    machine: &Machine,
    work_dir: &Option<String>,
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
    frontend_channel: Arc<Mutex<Option<Channel<Vec<u8>>>>>,
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

fn drain_local(
    reader: Arc<Mutex<Box<dyn Read + Send>>>,
    app: AppHandle,
    session_id: String,
    machine_id: String,
    created_at: u64,
    frontend_channel: Arc<Mutex<Option<Channel<Vec<u8>>>>>,
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
        },
    );
}

fn send_chunk(frontend_channel: &Arc<Mutex<Option<Channel<Vec<u8>>>>>, chunk: Vec<u8>) {
    loop {
        let chan_opt = frontend_channel.lock().unwrap();
        if let Some(frontend) = chan_opt.as_ref() {
            if frontend.send(chunk.clone()).is_ok() {
                break;
            }
        } else {
            break;
        }
        drop(chan_opt);
        thread::sleep(Duration::from_millis(50));
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
    let mut slot = active
        .frontend_channel
        .lock()
        .map_err(|_| "Failed to lock frontend channel".to_string())?;
    *slot = Some(tauri_channel);
    Ok(())
}

#[tauri::command]
pub fn detach_terminal_session(
    session_state: State<'_, SessionState>,
    session_id: String,
) -> Result<(), String> {
    let sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    if let Some(active) = sessions.get(&session_id) {
        if let Ok(mut slot) = active.frontend_channel.lock() {
            *slot = None;
        }
        Ok(())
    } else {
        Err("Session not found".to_string())
    }
}

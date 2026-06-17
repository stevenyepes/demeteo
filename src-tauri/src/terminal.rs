use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};
use std::thread;
use std::time::Duration;
use serde::Serialize;
use ssh2::Session;
use tauri::{AppHandle, Emitter, ipc::Channel, State};
use crate::state::AppContext;
use crate::domain::models::Machine;

static SESSION_COUNTER: AtomicUsize = AtomicUsize::new(1);

/// Wraps either an SSH channel or a local subprocess stdout for reading.
pub enum ReadSource {
    Ssh(Arc<Mutex<ssh2::Channel>>),
    Local(Arc<Mutex<ChildStdout>>),
}

/// Wraps either an SSH channel or a local subprocess stdin for writing.
pub enum WriteSink {
    Ssh(Arc<Mutex<ssh2::Channel>>),
    Local(Arc<Mutex<ChildStdin>>),
}

pub struct ActiveSession {
    pub read_source: ReadSource,
    pub write_sink: WriteSink,
    /// Kept alive for the lifetime of the session (SSH or local process).
    pub _keepalive: Arc<Mutex<SessionKeepalive>>,
    pub machine_id: String,
    pub created_at: u64,
    pub frontend_channel: Arc<Mutex<Option<Channel<Vec<u8>>>>>,
}

/// SSH sessions need both the Session and TcpStream kept alive.
/// Local processes need the Child kept alive.
pub enum SessionKeepalive {
    Ssh {
        #[allow(dead_code)]
        session: Session,
        #[allow(dead_code)]
        tcp: TcpStream,
    },
    Local {
        child: Arc<Mutex<Child>>,
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
    entry.set_password(&secret)
        .map_err(|e| format!("Failed to store secret in keyring: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn delete_machine_secret(machine_id: String) -> Result<(), String> {
    let entry = keyring::Entry::new("demeteo", &format!("machine_{}", machine_id))
        .map_err(|e| format!("Keyring error: {}", e))?;
    let _ = entry.delete_credential();
    Ok(())
}

pub fn connect_ssh(machine: &Machine, secret: Option<String>) -> Result<(Session, TcpStream), String> {
    crate::ssh_util::connect(machine, secret)
}

fn spawn_local_shell() -> Result<(Child, ChildStdin, ChildStdout), String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
    let mut cmd = Command::new(&shell);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn local shell ({}): {}", shell, e))?;
    let stdin = child.stdin.take().ok_or_else(|| "Failed to capture shell stdin".to_string())?;
    let stdout = child.stdout.take().ok_or_else(|| "Failed to capture shell stdout".to_string())?;
    Ok((child, stdin, stdout))
}

#[tauri::command]
pub fn start_terminal_session(
    app: AppHandle,
    ctx: State<'_, AppContext>,
    session_state: State<'_, SessionState>,
    machine_id: String,
    tauri_channel: Channel<Vec<u8>>,
) -> Result<String, String> {
    let machines = ctx.machines.get_machines()?;
    let machine_id_typed = crate::domain::ids::MachineId::from(machine_id.clone());
    let machine = machines.into_iter().find(|m| m.id == machine_id_typed)
        .ok_or_else(|| "Machine not found".to_string())?;

    let secret = match machine.auth_type.as_str() {
        "password" | "key" => {
            let entry = keyring::Entry::new("demeteo", &format!("machine_{}", machine.id))
                .map_err(|e| format!("Keyring error: {}", e))?;
            entry.get_password().ok()
        }
        _ => None,
    };

    let session_id = format!("sess_{}", SESSION_COUNTER.fetch_add(1, Ordering::SeqCst));
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let frontend_channel: Arc<Mutex<Option<Channel<Vec<u8>>>>> = Arc::new(Mutex::new(Some(tauri_channel)));

    let (read_source, write_sink, keepalive) = if machine.auth_type == "local" {
        let (child, stdin, stdout) = spawn_local_shell()?;
        let child = Arc::new(Mutex::new(child));
        let stdin = Arc::new(Mutex::new(stdin));
        let stdout = Arc::new(Mutex::new(stdout));
        (
            ReadSource::Local(stdout),
            WriteSink::Local(stdin),
            SessionKeepalive::Local { child },
        )
    } else {
        let (sess, tcp) = connect_ssh(&machine, secret)?;
        let _ = sess.set_keepalive(true, 30);
        let mut ssh_chan = sess.channel_session().map_err(|e| format!("Failed to open SSH channel: {}", e))?;
        ssh_chan.request_pty("xterm-256color", None, None).map_err(|e| format!("Failed to request PTY: {}", e))?;
        ssh_chan.shell().map_err(|e| format!("Failed to start shell: {}", e))?;
        sess.set_blocking(false);
        let arc_chan = Arc::new(Mutex::new(ssh_chan));
        (
            ReadSource::Ssh(arc_chan.clone()),
            WriteSink::Ssh(arc_chan),
            SessionKeepalive::Ssh { session: sess, tcp },
        )
    };

    let keepalive = Arc::new(Mutex::new(keepalive));

    let read_app = app.clone();
    let read_session_id = session_id.clone();
    let read_machine_id = machine_id.clone();
    let read_frontend_channel = frontend_channel.clone();
    let read_source_for_thread = match &read_source {
        ReadSource::Ssh(ch) => ReadSource::Ssh(ch.clone()),
        ReadSource::Local(out) => ReadSource::Local(out.clone()),
    };

    thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        let mut last_activity = std::time::Instant::now();
        loop {
            let result = match &read_source_for_thread {
                ReadSource::Ssh(ch) => ch.lock().unwrap().read(&mut buffer),
                ReadSource::Local(out) => out.lock().unwrap().read(&mut buffer),
            };

            match result {
                Ok(0) => {
                    let _ = read_app.emit(
                        "terminal-session-ended",
                        SessionInfo {
                            session_id: read_session_id.clone(),
                            machine_id: read_machine_id.clone(),
                            created_at,
                        },
                    );
                    break;
                }
                Ok(n) => {
                    last_activity = std::time::Instant::now();
                    let chunk = buffer[..n].to_vec();
                    let chan_opt = read_frontend_channel.lock().unwrap();
                    if let Some(frontend) = chan_opt.as_ref() {
                        if frontend.send(chunk).is_err() {
                            drop(chan_opt);
                            thread::sleep(Duration::from_millis(50));
                            continue;
                        }
                    } else {
                        drop(chan_opt);
                        thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if last_activity.elapsed().as_secs() > IDLE_TIMEOUT_SECS {
                        let _ = read_app.emit(
                            "terminal-session-ended",
                            SessionInfo {
                                session_id: read_session_id.clone(),
                                machine_id: read_machine_id.clone(),
                                created_at,
                            },
                        );
                        break;
                    }
                    thread::sleep(Duration::from_millis(15));
                }
                Err(_) => {
                    let _ = read_app.emit(
                        "terminal-session-ended",
                        SessionInfo {
                            session_id: read_session_id.clone(),
                            machine_id: read_machine_id.clone(),
                            created_at,
                        },
                    );
                    break;
                }
            }
        }
    });

    let mut sessions = session_state.sessions.lock().map_err(|_| "Failed to lock sessions".to_string())?;
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

#[tauri::command]
pub fn write_terminal_session(
    session_state: State<'_, SessionState>,
    session_id: String,
    data: String,
) -> Result<(), String> {
    let sessions = session_state.sessions.lock().map_err(|_| "Failed to lock sessions".to_string())?;
    if let Some(active) = sessions.get(&session_id) {
        match &active.write_sink {
            WriteSink::Ssh(ch) => {
                let mut chan = ch.lock().map_err(|_| "Failed to lock channel".to_string())?;
                chan.write_all(data.as_bytes()).map_err(|e| format!("Failed to write to terminal: {}", e))?;
                chan.flush().map_err(|e| format!("Failed to flush terminal: {}", e))?;
            }
            WriteSink::Local(stdin) => {
                let mut stdin = stdin.lock().map_err(|_| "Failed to lock stdin".to_string())?;
                stdin.write_all(data.as_bytes()).map_err(|e| format!("Failed to write to terminal: {}", e))?;
                stdin.flush().map_err(|e| format!("Failed to flush terminal: {}", e))?;
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
    let sessions = session_state.sessions.lock().map_err(|_| "Failed to lock sessions".to_string())?;
    if let Some(active) = sessions.get(&session_id) {
        match &active.write_sink {
            WriteSink::Ssh(ch) => {
                let mut chan = ch.lock().map_err(|_| "Failed to lock channel".to_string())?;
                chan.request_pty_size(cols, rows, None, None)
                    .map_err(|e| format!("Failed to resize terminal: {}", e))?;
            }
            WriteSink::Local(_) => {
                // Local subprocess terminals don't support PTY resize
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
    let mut sessions = session_state.sessions.lock().map_err(|_| "Failed to lock sessions".to_string())?;
    if let Some(active) = sessions.remove(&session_id) {
        match &active.write_sink {
            WriteSink::Ssh(ch) => {
                let mut chan = ch.lock().map_err(|_| "Failed to lock channel".to_string())?;
                let _ = chan.close();
            }
            WriteSink::Local(_) => {
                if let Ok(keepalive) = active._keepalive.lock() {
                    if let SessionKeepalive::Local { ref child } = *keepalive {
                        if let Ok(mut child) = child.lock() {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
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
pub fn list_terminal_sessions(
    session_state: State<'_, SessionState>,
) -> Result<Vec<SessionInfo>, String> {
    let sessions = session_state.sessions.lock().map_err(|_| "Failed to lock sessions".to_string())?;
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
    let mut sessions = session_state.sessions.lock().map_err(|_| "Failed to lock sessions".to_string())?;
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
    let sessions = session_state.sessions.lock().map_err(|_| "Failed to lock sessions".to_string())?;
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
    let sessions = session_state.sessions.lock().map_err(|_| "Failed to lock sessions".to_string())?;
    if let Some(active) = sessions.get(&session_id) {
        if let Ok(mut slot) = active.frontend_channel.lock() {
            *slot = None;
        }
        Ok(())
    } else {
        Err("Session not found".to_string())
    }
}

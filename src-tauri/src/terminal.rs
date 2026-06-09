use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};
use std::thread;
use std::time::Duration;
use serde::Serialize;
use ssh2::Session;
use tauri::{AppHandle, Emitter, ipc::Channel, State};
use crate::DatabaseState;
use crate::domain::models::Machine;

static SESSION_COUNTER: AtomicUsize = AtomicUsize::new(1);

pub struct ActiveSession {
    pub channel: Arc<Mutex<ssh2::Channel>>,
    pub session: Session,
    pub _tcp: TcpStream,
    pub machine_id: String,
    pub created_at: u64,
    /// The data channel bound to the currently attached frontend. The read
    /// thread sends to whichever channel is installed here, so the frontend
    /// can rebind on remount without dropping the SSH connection.
    pub frontend_channel: Arc<Mutex<Option<Channel<Vec<u8>>>>>,
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

/// 10 minutes — sessions idle longer than this on the SSH layer are reaped
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

#[tauri::command]
pub fn start_terminal_session(
    app: AppHandle,
    state: State<'_, DatabaseState>,
    session_state: State<'_, SessionState>,
    machine_id: String,
    tauri_channel: Channel<Vec<u8>>,
) -> Result<String, String> {
    // 1. Get machine connection details from DB
    let machines = state.db.get_machines()?;
    let machine = machines.into_iter().find(|m| m.id == machine_id)
        .ok_or_else(|| "Machine not found".to_string())?;


    // 2. Resolve credentials from keyring if password/key passphrase is required
    let secret = match machine.auth_type.as_str() {
        "password" | "key" => {
            let entry = keyring::Entry::new("demeteo", &format!("machine_{}", machine.id))
                .map_err(|e| format!("Keyring error: {}", e))?;
            entry.get_password().ok() // None if it does not exist or empty
        }
        _ => None,
    };

    // 3. Connect and authenticate SSH
    let (sess, tcp) = connect_ssh(&machine, secret)?;

    // Enable SSH-level keepalive so NAT / firewall idle drops don't kill the session
    let _ = sess.set_keepalive(true, 30);

    // 6. Open channel, request PTY and spawn shell
    let mut ssh_chan = sess.channel_session().map_err(|e| format!("Failed to open SSH channel: {}", e))?;
    ssh_chan.request_pty("xterm-256color", None, None).map_err(|e| format!("Failed to request PTY: {}", e))?;
    ssh_chan.shell().map_err(|e| format!("Failed to start shell: {}", e))?;

    // 7. Configure non-blocking for streaming loop
    sess.set_blocking(false);

    let session_id = format!("sess_{}", SESSION_COUNTER.fetch_add(1, Ordering::SeqCst));
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let arc_chan = Arc::new(Mutex::new(ssh_chan));
    let frontend_channel: Arc<Mutex<Option<Channel<Vec<u8>>>>> = Arc::new(Mutex::new(Some(tauri_channel)));

    // Spawn read thread
    let read_chan = arc_chan.clone();
    let read_session_id = session_id.clone();
    let read_app = app.clone();
    let read_machine_id = machine_id.clone();
    let read_frontend_channel = frontend_channel.clone();
    thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        let mut last_activity = std::time::Instant::now();
        loop {
            let mut chan = read_chan.lock().unwrap();
            match chan.read(&mut buffer) {
                Ok(0) => {
                    // EOF - remote closed the channel (user ran `exit`, server reboot, etc.)
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
                            // Frontend channel is dead; keep the SSH session alive
                            // and wait — the frontend will rebind on remount.
                            drop(chan_opt);
                            drop(chan);
                            thread::sleep(Duration::from_millis(50));
                            continue;
                        }
                    } else {
                        // No frontend attached — keep the session warm in the background.
                        drop(chan_opt);
                        drop(chan);
                        thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    drop(chan);
                    if last_activity.elapsed().as_secs() > IDLE_TIMEOUT_SECS {
                        // Idle too long: tell the frontend and bail
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

    // 8. Save active session
    let mut sessions = session_state.sessions.lock().map_err(|_| "Failed to lock sessions".to_string())?;
    sessions.insert(
        session_id.clone(),
        ActiveSession {
            channel: arc_chan,
            session: sess,
            _tcp: tcp,
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
        let mut chan = active.channel.lock().map_err(|_| "Failed to lock channel".to_string())?;
        chan.write_all(data.as_bytes()).map_err(|e| format!("Failed to write to terminal: {}", e))?;
        chan.flush().map_err(|e| format!("Failed to flush terminal: {}", e))?;
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
        let mut chan = active.channel.lock().map_err(|_| "Failed to lock channel".to_string())?;
        chan.request_pty_size(cols, rows, None, None)
            .map_err(|e| format!("Failed to resize terminal: {}", e))?;
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
        let mut chan = active.channel.lock().map_err(|_| "Failed to lock channel".to_string())?;
        let _ = chan.close();
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
        if let Some(active) = sessions.remove(&id) {
            if let Ok(mut chan) = active.channel.lock() {
                let _ = chan.close();
            }
        }
    }
    Ok(count)
}

/// Re-bind a new frontend data channel to an existing SSH session.
/// Used when the frontend remounts (e.g. user switched to supervisor view
/// and back) so the SSH connection survives but live output is re-routed
/// to the new xterm instance.
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

/// Detach the frontend channel from a session. The SSH session keeps running
/// (with the read thread buffering skips for unattached periods), and a later
/// attach_terminal_session call will rebind it.
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

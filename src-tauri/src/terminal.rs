use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};
use std::thread;
use std::time::Duration;
use ssh2::Session;
use tauri::ipc::Channel;
use tauri::State;
use crate::DatabaseState;
use crate::domain::models::Machine;

static SESSION_COUNTER: AtomicUsize = AtomicUsize::new(1);

pub struct ActiveSession {
    pub channel: Arc<Mutex<ssh2::Channel>>,
    pub session: Session,
    pub _tcp: TcpStream,
}

#[derive(Default)]
pub struct SessionState {
    pub sessions: Mutex<HashMap<String, ActiveSession>>,
}

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
    // Connect TCP
    let tcp = TcpStream::connect(format!("{}:{}", machine.host, machine.port))
        .map_err(|e| format!("Failed to connect to host: {}", e))?;

    // SSH Handshake
    let mut sess = Session::new().map_err(|e| format!("Failed to create SSH session: {}", e))?;
    sess.set_tcp_stream(tcp.try_clone().map_err(|e| e.to_string())?);
    sess.handshake().map_err(|e| format!("SSH Handshake failed: {}", e))?;

    // Authenticate
    match machine.auth_type.as_str() {
        "password" => {
            let password = secret.ok_or_else(|| "Password not found in keyring".to_string())?;
            sess.userauth_password(&machine.username, &password)
                .map_err(|e| format!("Authentication failed: {}", e))?;
        }
        "key" => {
            let key_path_str = machine.key_path.as_deref().ok_or_else(|| "Key path not provided".to_string())?;
            // Expand tilde (~) in key path if any
            let resolved_path = if key_path_str.starts_with('~') {
                let home = std::env::var("HOME").map_err(|_| "Could not find HOME environment variable".to_string())?;
                key_path_str.replacen('~', &home, 1)
            } else {
                key_path_str.to_string()
            };
            let key_path = std::path::Path::new(&resolved_path);
            if !key_path.exists() {
                return Err(format!("Private key file does not exist: {}", resolved_path));
            }
            sess.userauth_pubkey_file(&machine.username, None, key_path, secret.as_deref())
                .map_err(|e| format!("Key authentication failed: {}", e))?;
        }
        "agent" => {
            sess.userauth_agent(&machine.username)
                .map_err(|e| format!("Agent authentication failed: {}", e))?;
        }
        _ => return Err(format!("Unknown auth type: {}", machine.auth_type)),
    }
    Ok((sess, tcp))
}

#[tauri::command]
pub fn start_terminal_session(
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

    // 6. Open channel, request PTY and spawn shell
    let mut ssh_chan = sess.channel_session().map_err(|e| format!("Failed to open SSH channel: {}", e))?;
    ssh_chan.request_pty("xterm-256color", None, None).map_err(|e| format!("Failed to request PTY: {}", e))?;
    ssh_chan.shell().map_err(|e| format!("Failed to start shell: {}", e))?;

    // 7. Configure non-blocking for streaming loop
    sess.set_blocking(false);

    let session_id = format!("sess_{}", SESSION_COUNTER.fetch_add(1, Ordering::SeqCst));
    let arc_chan = Arc::new(Mutex::new(ssh_chan));

    // Spawn read thread
    let read_chan = arc_chan.clone();
    let read_session_id = session_id.clone();
    thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            let mut chan = read_chan.lock().unwrap();
            match chan.read(&mut buffer) {
                Ok(0) => {
                    // EOF - SSH session ended
                    println!("[Terminal Read Thread] EOF reached for session {}", read_session_id);
                    break;
                }
                Ok(n) => {
                    let chunk = buffer[..n].to_vec();
                    if let Err(e) = tauri_channel.send(chunk) {
                        println!("[Terminal Read Thread] Error sending message to frontend: {}", e);
                        break;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // Release the lock and sleep briefly
                    drop(chan);
                    thread::sleep(Duration::from_millis(15));
                }
                Err(e) => {
                    println!("[Terminal Read Thread] Error reading from SSH channel: {}", e);
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

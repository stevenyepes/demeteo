use crate::state::AppContext;
use crate::terminal::connect_ssh;
use ssh2::Session;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;
use tauri::State;

pub struct ActiveForward {
    pub local_port: u16,
    pub session: Arc<Mutex<Session>>,
    pub _tcp: TcpStream,
    pub stop_flag: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct ForwardState {
    // Key is (machine_id, remote_port)
    pub forwards: Mutex<HashMap<(String, i32), ActiveForward>>,
}

#[tauri::command]
pub fn start_port_forward(
    ctx: State<'_, AppContext>,
    forward_state: State<'_, ForwardState>,
    machine_id: String,
    remote_port: i32,
) -> Result<u16, String> {
    let mut forwards = forward_state
        .forwards
        .lock()
        .map_err(|_| "Failed to lock forwards state".to_string())?;

    // 1. Check if already forwarding
    if let Some(forward) = forwards.get(&(machine_id.clone(), remote_port)) {
        return Ok(forward.local_port);
    }

    let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
        &*ctx.machines,
        &machine_id,
    )?;

    // Local machines don't need port forwarding
    if machine.auth_type == "local" {
        return Err("Port forwarding is not available for local machines".to_string());
    }

    // 3. Resolve keyring credentials
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

    // 4. Connect dedicated SSH session for forwarding
    let (sess, tcp) = connect_ssh(&machine, secret)?;
    sess.set_blocking(false);

    // 5. Bind local TcpListener on dynamic ephemeral port
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Failed to bind local listener: {}", e))?;
    let local_port = listener.local_addr().map_err(|e| e.to_string())?.port();

    let stop_flag = Arc::new(AtomicBool::new(false));
    let sess_arc = Arc::new(Mutex::new(sess));

    // Spawn accept loop thread
    let listener_clone = listener.try_clone().map_err(|e| e.to_string())?;
    let stop_clone = stop_flag.clone();
    let sess_clone = sess_arc.clone();
    thread::spawn(move || {
        while !stop_clone.load(Ordering::SeqCst) {
            if let Ok((mut local_stream, _)) = listener_clone.accept() {
                if stop_clone.load(Ordering::SeqCst) {
                    break;
                }
                let sess_inner = sess_clone.clone();
                let stop_inner = stop_clone.clone();

                if let Err(e) = local_stream.set_nonblocking(true) {
                    tracing::warn!(error = %e, "port forward: failed to set local stream to nonblocking");
                    continue;
                }

                thread::spawn(move || {
                    let mut channel = loop {
                        if stop_inner.load(Ordering::SeqCst) {
                            return;
                        }
                        let sess_guard = sess_inner.lock().unwrap();
                        match sess_guard.channel_direct_tcpip("127.0.0.1", remote_port as u16, None)
                        {
                            Ok(chan) => break chan,
                            Err(ref e) if e.code() == ssh2::ErrorCode::Session(-37) => {
                                drop(sess_guard);
                                thread::sleep(Duration::from_millis(15));
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "port forward: direct-tcpip failed");
                                return;
                            }
                        }
                    };

                    let mut local_buf = [0u8; 8192];
                    let mut remote_buf = [0u8; 8192];

                    loop {
                        if stop_inner.load(Ordering::SeqCst) {
                            break;
                        }
                        let mut active = false;

                        // 1. Read local -> Write remote
                        match local_stream.read(&mut local_buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                active = true;
                                let mut written = 0;
                                while written < n {
                                    if stop_inner.load(Ordering::SeqCst) {
                                        return;
                                    }
                                    let sess_guard = sess_inner.lock().unwrap();
                                    match channel.write(&local_buf[written..n]) {
                                        Ok(nw) => {
                                            written += nw;
                                        }
                                        Err(ref e)
                                            if e.kind() == std::io::ErrorKind::WouldBlock =>
                                        {
                                            drop(sess_guard);
                                            thread::sleep(Duration::from_millis(5));
                                        }
                                        Err(_) => return,
                                    }
                                }
                            }
                            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                            Err(_) => break,
                        }

                        // 2. Read remote -> Write local
                        let read_res = {
                            if stop_inner.load(Ordering::SeqCst) {
                                break;
                            }
                            let _sess_guard = sess_inner.lock().unwrap();
                            channel.read(&mut remote_buf)
                        };
                        match read_res {
                            Ok(0) => break,
                            Ok(n) => {
                                active = true;
                                let mut written = 0;
                                while written < n {
                                    if stop_inner.load(Ordering::SeqCst) {
                                        return;
                                    }
                                    match local_stream.write(&remote_buf[written..n]) {
                                        Ok(nw) => {
                                            written += nw;
                                        }
                                        Err(ref e)
                                            if e.kind() == std::io::ErrorKind::WouldBlock =>
                                        {
                                            thread::sleep(Duration::from_millis(5));
                                        }
                                        Err(_) => return,
                                    }
                                }
                            }
                            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                            Err(_) => break,
                        }

                        if !active {
                            thread::sleep(Duration::from_millis(15));
                        }
                    }

                    let _sess_guard = sess_inner.lock().unwrap();
                    let _ = channel.close();
                });
            }
        }
    });

    forwards.insert(
        (machine_id, remote_port),
        ActiveForward {
            local_port,
            session: sess_arc,
            _tcp: tcp,
            stop_flag,
        },
    );

    Ok(local_port)
}

#[tauri::command]
pub fn stop_port_forward(
    forward_state: State<'_, ForwardState>,
    machine_id: String,
    remote_port: i32,
) -> Result<(), String> {
    let mut forwards = forward_state
        .forwards
        .lock()
        .map_err(|_| "Failed to lock forwards state".to_string())?;

    if let Some(forward) = forwards.remove(&(machine_id, remote_port)) {
        forward.stop_flag.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(format!("127.0.0.1:{}", forward.local_port));
        let sess_guard = forward
            .session
            .lock()
            .map_err(|_| "Failed to lock session".to_string())?;
        let _ = sess_guard.disconnect(None, "Port forwarding stopped", None);
    }

    Ok(())
}

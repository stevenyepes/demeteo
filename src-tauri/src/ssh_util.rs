use crate::domain::models::Machine;
use ssh2::Session;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

/// Resolve host:port, TCP connect with 5s timeout, SSH handshake, and
/// authenticate using the machine's auth_type. Returns the connected
/// Session and TcpStream on success.
///
/// This is the single shared entry point for all SSH connections in
/// Demeteo. Callers are responsible for setting blocking mode,
/// keepalive, SFTP init, or disconnect on top of this.
pub fn connect(machine: &Machine, secret: Option<String>) -> Result<(Session, TcpStream), String> {
    let addr = format!("{}:{}", machine.host, machine.port)
        .to_socket_addrs()
        .map_err(|e| format!("Failed to resolve host: {}", e))?
        .next()
        .ok_or_else(|| format!("No addresses for host: {}", machine.host))?;

    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .map_err(|e| format!("Cannot reach {}:{} (timeout after 5s) — {}", machine.host, machine.port, e))?;
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = tcp.set_write_timeout(Some(Duration::from_secs(10)));

    let mut sess = Session::new()
        .map_err(|e| format!("Failed to create SSH session: {}", e))?;
    sess.set_tcp_stream(tcp.try_clone().map_err(|e| e.to_string())?);
    sess.set_timeout(10_000);
    sess.handshake()
        .map_err(|e| format!("SSH handshake failed: {}", e))?;

    match machine.auth_type.as_str() {
        "password" => {
            let password = secret.ok_or_else(|| "SSH password is required".to_string())?;
            sess.userauth_password(&machine.username, &password)
                .map_err(|e| format!("Password authentication failed: {}", e))?;
        }
        "key" => {
            let key_path_str = machine.key_path.as_deref()
                .ok_or_else(|| "Private key path is required".to_string())?;
            if key_path_str.trim_end().ends_with(".pub") {
                return Err("Key path points to a public key (.pub). Provide the private key instead (e.g. ~/.ssh/id_ed25519).".to_string());
            }
            let resolved = if key_path_str.starts_with('~') {
                let home = std::env::var("HOME")
                    .map_err(|_| "HOME environment variable not set".to_string())?;
                key_path_str.replacen('~', &home, 1)
            } else {
                key_path_str.to_string()
            };
            let key_file = Path::new(&resolved);
            if !key_file.exists() {
                return Err(format!("Private key file not found: {}", resolved));
            }
            sess.userauth_pubkey_file(&machine.username, None, key_file, secret.as_deref())
                .map_err(|e| format!("Key authentication failed: {}", e))?;
        }
        "agent" => {
            sess.userauth_agent(&machine.username)
                .map_err(|e| format!("SSH agent authentication failed: {}", e))?;
        }
        "local" => {}
        other => return Err(format!("Unknown auth type: {}", other)),
    }

    Ok((sess, tcp))
}

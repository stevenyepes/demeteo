use crate::domain::models::Machine;
use socket2::{SockRef, TcpKeepalive};
use ssh2::Session;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The home directory of the process itself, for expanding a `~`-prefixed
/// private-key path. The key is read on *this* host, so this is the local
/// home even when the connection is remote.
///
/// `ExecutionPort::resolve_home` owns this ladder and is the value an agent
/// is handed; it is async, while `connect` is synchronous and reached from
/// Tauri command handlers and from inside `spawn_blocking`. Until one of
/// those changes the two must be kept in step.
fn process_home() -> Result<String, String> {
    #[cfg(windows)]
    let home = std::env::var("USERPROFILE").or_else(|_| {
        let drive = std::env::var("HOMEDRIVE")?;
        let path = std::env::var("HOMEPATH")?;
        Ok::<_, std::env::VarError>(format!("{}{}", drive, path))
    });
    #[cfg(not(windows))]
    let home = std::env::var("HOME");
    home.map_err(|_| "Home directory environment variable is not set".to_string())
}

/// Resolve host:port, TCP connect with 5s timeout, SSH handshake, and
/// authenticate using the machine's auth_type. Returns the connected
/// Session and TcpStream on success.
///
/// This is the single shared entry point for all SSH connections in
/// Demeteo. Blocking-call timeouts and both kernel- and app-level keepalive
/// are configured here so every session is wedge-resistant; callers are
/// responsible for setting blocking mode, SFTP init, or disconnect on top.
pub fn connect(machine: &Machine, secret: Option<String>) -> Result<(Session, TcpStream), String> {
    // Local machines don't use SSH — this function should not be called
    // for local auth_type. Callers should check auth_type first.
    if machine.auth_type == "local" {
        return Err(format!(
            "Machine '{}' uses auth_type=local; SSH connection is not applicable",
            machine.id
        ));
    }

    let addr = format!("{}:{}", machine.host, machine.port)
        .to_socket_addrs()
        .map_err(|e| format!("Failed to resolve host: {}", e))?
        .next()
        .ok_or_else(|| format!("No addresses for host: {}", machine.host))?;

    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| {
        format!(
            "Cannot reach {}:{} (timeout after 5s) — {}",
            machine.host, machine.port, e
        )
    })?;
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = tcp.set_write_timeout(Some(Duration::from_secs(10)));

    // Kernel-level TCP keepalive so a *black-holed* connection (packets
    // silently dropped, no RST) is torn down by the OS instead of hanging a
    // blocking read forever. libssh2's own keepalive only helps when the
    // socket *write* fails; this makes the kernel probe an idle connection and
    // deliver an ECONNRESET, which surfaces as a read error and lets
    // `drain_stream` fail the step fast instead of looping to the 30-minute
    // wall cap (the "pipeline froze at validate/critic" wedge). Idle 30s,
    // probe every 10s, give up after 3 misses → dead detected in ~60s.
    let keepalive = TcpKeepalive::new()
        .with_time(Duration::from_secs(30))
        .with_interval(Duration::from_secs(10))
        .with_retries(3);
    let _ = SockRef::from(&tcp).set_tcp_keepalive(&keepalive);

    let mut sess = Session::new().map_err(|e| format!("Failed to create SSH session: {}", e))?;
    sess.set_tcp_stream(tcp.try_clone().map_err(|e| e.to_string())?);
    sess.set_timeout(10_000);
    sess.handshake()
        .map_err(|e| format!("SSH handshake failed: {}", e))?;

    // App-level keepalive on every session so `drain_stream`'s no-progress
    // abort has a signal to work with: with keepalive disabled (the default),
    // `keepalive_send()` returns `Ok` without touching the wire and the abort
    // never fires. `want_reply = true` asks the server to respond; 30s matches
    // the interval `drain_stream`/`NO_PROGRESS_ABORT` are tuned against. This
    // is the application-layer complement to the kernel keepalive above.
    sess.set_keepalive(true, 30);

    match machine.auth_type.as_str() {
        "password" => {
            let password = secret.ok_or_else(|| "SSH password is required".to_string())?;
            sess.userauth_password(&machine.username, &password)
                .map_err(|e| format!("Password authentication failed: {}", e))?;
        }
        "key" => {
            let key_path_str = machine
                .key_path
                .as_deref()
                .ok_or_else(|| "Private key path is required".to_string())?;
            if key_path_str.trim_end().ends_with(".pub") {
                return Err("Key path points to a public key (.pub). Provide the private key instead (e.g. ~/.ssh/id_ed25519).".to_string());
            }
            let key_file = match key_path_str.strip_prefix('~') {
                Some(rest) => {
                    Path::new(&process_home()?).join(rest.trim_start_matches(['/', '\\']))
                }
                None => PathBuf::from(key_path_str),
            };
            if !key_file.exists() {
                return Err(format!(
                    "Private key file not found: {}",
                    key_file.display()
                ));
            }
            sess.userauth_pubkey_file(&machine.username, None, &key_file, secret.as_deref())
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

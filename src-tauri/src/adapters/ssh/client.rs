use crate::ports::db::DatabasePort;
use crate::ports::execution::ExecutionPort;
use crate::sftp::SftpEntry;
use ssh2::{Session, Sftp};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct SftpSession {
    pub sftp: Mutex<Sftp>,
    pub session: Session,
    pub tcp: TcpStream,
}

pub struct SshClientAdapter {
    pub db: Arc<dyn DatabasePort>,
    pub sessions: Mutex<HashMap<String, Arc<SftpSession>>>,
}

impl SshClientAdapter {
    pub fn new(db: Arc<dyn DatabasePort>) -> Self {
        Self {
            db,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn get_sftp(&self, machine_id: &str) -> Result<Arc<SftpSession>, String> {
        let mut sessions = self.sessions.lock().map_err(|_| "Failed to lock SFTP state".to_string())?;

        if let Some(s) = sessions.get(machine_id) {
            // Test if connection is still alive by sending a keepalive or checking status
            // If it fails, we will reconnect.
            let sftp = s.sftp.lock().map_err(|_| "Failed to lock SFTP".to_string())?;
            if sftp.readdir(std::path::Path::new(".")).is_ok() {
                drop(sftp);
                return Ok(s.clone());
            }
            drop(sftp);
            sessions.remove(machine_id);
        }

        // Connect new session
        let machines = self.db.get_machines()?;
        let machine = machines.into_iter().find(|m| {
            m.id == machine_id
                || format!("{}@{}", m.username, m.host) == machine_id
                || m.host == machine_id
                || m.name == machine_id
        })
        .ok_or_else(|| "Machine not found".to_string())?;

        let secret = match machine.auth_type.as_str() {
            "password" | "key" => {
                let entry = keyring::Entry::new("demeteo", &format!("machine_{}", machine.id))
                    .ok();
                entry.and_then(|e| e.get_password().ok())
            }
            _ => None,
        };

        // Connect TCP with a 5s timeout so a black-holed host doesn't hang the whole command
        let addr = format!("{}:{}", machine.host, machine.port)
            .to_socket_addrs()
            .map_err(|e| format!("Failed to resolve host: {}", e))?
            .next()
            .ok_or_else(|| format!("No addresses for host: {}", machine.host))?;
        let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
            .map_err(|e| format!("Failed to connect to host (timeout after 5s): {}", e))?;
        let _ = tcp.set_read_timeout(Some(Duration::from_secs(10)));
        let _ = tcp.set_write_timeout(Some(Duration::from_secs(10)));

        // SSH Handshake
        let mut sess = Session::new().map_err(|e| format!("Failed to create SSH session: {}", e))?;
        sess.set_tcp_stream(tcp.try_clone().map_err(|e| e.to_string())?);
        sess.set_timeout(10_000);
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

        sess.set_blocking(true);
        let sftp = sess.sftp().map_err(|e| format!("SFTP subsystem failed: {}", e))?;

        let sftp_session = Arc::new(SftpSession {
            sftp: Mutex::new(sftp),
            session: sess,
            tcp,
        });

        sessions.insert(machine_id.to_string(), sftp_session.clone());
        Ok(sftp_session)
    }
}

impl ExecutionPort for SshClientAdapter {
    fn test_connection(&self, machine_id: &str) -> Result<(), String> {
        let machines = self.db.get_machines()?;
        let machine = machines
            .into_iter()
            .find(|m| {
                m.id == machine_id
                    || format!("{}@{}", m.username, m.host) == machine_id
                    || m.host == machine_id
                    || m.name == machine_id
            })
            .ok_or_else(|| "Machine not found".to_string())?;

        // Local machines don't use SSH – trivially valid
        if machine.auth_type == "local" {
            return Ok(());
        }

        let secret = match machine.auth_type.as_str() {
            "password" | "key" => {
                let entry = keyring::Entry::new("demeteo", &format!("machine_{}", machine.id)).ok();
                entry.and_then(|e| e.get_password().ok())
            }
            _ => None,
        };

        let addr = format!("{}:{}", machine.host, machine.port)
            .to_socket_addrs()
            .map_err(|e| format!("Failed to resolve host: {}", e))?
            .next()
            .ok_or_else(|| format!("No addresses for host: {}", machine.host))?;
        let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
            .map_err(|e| format!("Cannot reach host (timeout after 5s): {}", e))?;
        let _ = tcp.set_read_timeout(Some(Duration::from_secs(10)));
        let _ = tcp.set_write_timeout(Some(Duration::from_secs(10)));

        let mut sess =
            Session::new().map_err(|e| format!("Failed to create SSH session: {}", e))?;
        sess.set_tcp_stream(tcp);
        sess.handshake()
            .map_err(|e| format!("SSH handshake failed: {}", e))?;

        match machine.auth_type.as_str() {
            "password" => {
                let password =
                    secret.ok_or_else(|| "Password not found in keyring".to_string())?;
                sess.userauth_password(&machine.username, &password)
                    .map_err(|e| format!("Authentication failed: {}", e))?;
            }
            "key" => {
                let key_path_str = machine
                    .key_path
                    .as_deref()
                    .ok_or_else(|| "Key path not provided".to_string())?;
                let resolved_path = if key_path_str.starts_with('~') {
                    let home = std::env::var("HOME")
                        .map_err(|_| "Could not find HOME environment variable".to_string())?;
                    key_path_str.replacen('~', &home, 1)
                } else {
                    key_path_str.to_string()
                };
                let key_path = std::path::Path::new(&resolved_path);
                if !key_path.exists() {
                    return Err(format!(
                        "Private key file does not exist: {}",
                        resolved_path
                    ));
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

        // Connection is valid – disconnect cleanly
        let _ = sess.disconnect(None, "test complete", None);
        Ok(())
    }

    fn run_command(&self, machine_id: &str, cmd: &str) -> Result<String, String> {
        let sftp_sess = self.get_sftp(machine_id)?;
        let mut channel = sftp_sess.session.channel_session()
            .map_err(|e| format!("Failed to open SSH channel: {}", e))?;
        channel.exec(cmd).map_err(|e| format!("Failed to execute command: {}", e))?;
        
        let mut output = String::new();
        channel.read_to_string(&mut output).map_err(|e| format!("Failed to read command output: {}", e))?;
        let _ = channel.wait_close();
        
        Ok(output)
    }

    fn read_file(&self, machine_id: &str, path: &str) -> Result<String, String> {
        let sftp_sess = self.get_sftp(machine_id)?;
        let sftp = sftp_sess.sftp.lock().map_err(|_| "Failed to lock SFTP".to_string())?;
        
        let path_buf = std::path::Path::new(path);
        let mut file = sftp.open(path_buf).map_err(|e| {
            // Invalidate session cache if sftp connection went down
            if let Ok(mut sessions) = self.sessions.lock() {
                sessions.remove(machine_id);
            }
            format!("Failed to open file: {}", e)
        })?;
        
        let mut contents = String::new();
        file.read_to_string(&mut contents).map_err(|e| format!("Failed to read file content: {}", e))?;
        Ok(contents)
    }

    fn write_file(&self, machine_id: &str, path: &str, content: &str) -> Result<(), String> {
        let sftp_sess = self.get_sftp(machine_id)?;
        let sftp = sftp_sess.sftp.lock().map_err(|_| "Failed to lock SFTP".to_string())?;
        
        let path_buf = std::path::Path::new(path);
        let mut file = sftp.create(path_buf).map_err(|e| {
            if let Ok(mut sessions) = self.sessions.lock() {
                sessions.remove(machine_id);
            }
            format!("Failed to create file: {}", e)
        })?;
        
        file.write_all(content.as_bytes()).map_err(|e| format!("Failed to write file: {}", e))?;
        file.flush().map_err(|e| format!("Failed to flush file: {}", e))?;
        Ok(())
    }

    fn get_metadata(&self, machine_id: &str, path: &str) -> Result<SftpEntry, String> {
        let sftp_sess = self.get_sftp(machine_id)?;
        let sftp = sftp_sess.sftp.lock().map_err(|_| "Failed to lock SFTP".to_string())?;
        
        let path_buf = std::path::Path::new(path);
        let stat = sftp.stat(path_buf).map_err(|e| {
            if let Ok(mut sessions) = self.sessions.lock() {
                sessions.remove(machine_id);
            }
            format!("Failed to stat file: {}", e)
        })?;
        
        let name = path_buf.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
            
        let size = stat.size.unwrap_or(0);
        let modified = stat.mtime.unwrap_or(0) as u64;
        let is_dir = stat.is_dir();
        
        Ok(SftpEntry {
            name,
            path: path.to_string(),
            is_dir,
            size,
            modified,
        })
    }

    fn list_dir(&self, machine_id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        let sftp_sess = self.get_sftp(machine_id)?;
        let sftp = sftp_sess.sftp.lock().map_err(|_| "Failed to lock SFTP".to_string())?;
        
        let path_buf = std::path::Path::new(path);
        let entries = sftp.readdir(path_buf).map_err(|e| {
            if let Ok(mut sessions) = self.sessions.lock() {
                sessions.remove(machine_id);
            }
            format!("Failed to read directory: {}", e)
        })?;
        
        let mut list = Vec::new();
        for (p, stat) in entries {
            let name = p.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            
            if name == "." || name == ".." {
                continue;
            }

            let path_str = p.to_str().unwrap_or("").to_string();
            let size = stat.size.unwrap_or(0);
            let modified = stat.mtime.unwrap_or(0) as u64;
            let is_dir = stat.is_dir();
            
            list.push(SftpEntry {
                name,
                path: path_str,
                is_dir,
                size,
                modified,
            });
        }
        
        list.sort_by(|a, b| {
            if a.is_dir != b.is_dir {
                b.is_dir.cmp(&a.is_dir)
            } else {
                a.name.cmp(&b.name)
            }
        });

        Ok(list)
    }

    fn setup_worktree(&self, machine_id: &str, repo_path: &str, branch: &str, sandbox_path: &str) -> Result<(), String> {
        // Step 1: Ensure directory setup
        self.run_command(machine_id, &format!("mkdir -p {}/.demeteo/worktrees", repo_path))?;

        // Step 2: Configure git info exclude
        let git_exclude_cmd = format!(
            "if [ -d \"{0}/.git\" ]; then mkdir -p \"{0}/.git/info\"; if ! grep -q \".demeteo/\" \"{0}/.git/info/exclude\" 2>/dev/null; then echo \".demeteo/\" >> \"{0}/.git/info/exclude\"; fi; fi",
            repo_path
        );
        let _ = self.run_command(machine_id, &git_exclude_cmd);

        // Step 3: Run git worktree add
        let worktree_add_cmd = format!(
            "git -C \"{}\" worktree add -b \"{}\" \"{}\"",
            repo_path, branch, sandbox_path
        );
        let output = self.run_command(machine_id, &worktree_add_cmd)?;
        println!("[SshClientAdapter] Git Worktree provisioning output: {}", output);

        Ok(())
    }

    fn spawn_interactive(
        &self,
        machine_id: &str,
        binary: &str,
        args: &[String],
        cwd: &str,
        env: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
        use crate::adapters::agent::acp::transport_ssh::RemoteSshTransport;

        let machines = self.db.get_machines()?;
        let machine = machines.into_iter().find(|m| {
            m.id == machine_id
                || format!("{}@{}", m.username, m.host) == machine_id
                || m.host == machine_id
                || m.name == machine_id
        })
        .ok_or_else(|| "Machine not found".to_string())?;

        let secret = match machine.auth_type.as_str() {
            "password" | "key" => {
                let entry = keyring::Entry::new("demeteo", &format!("machine_{}", machine.id))
                    .ok();
                entry.and_then(|e| e.get_password().ok())
            }
            _ => None,
        };

        let (sess, tcp) = crate::terminal::connect_ssh(&machine, secret)?;
        let _ = sess.set_keepalive(true, 30);

        let mut channel = sess
            .channel_session()
            .map_err(|e| format!("Failed to open SSH channel: {}", e))?;

        // Build the remote command. We `cd` to the worktree first so the
        // agent's `cwd` matches what the user picked, then exec the
        // binary. Env vars are exported one per line.
        let mut env_str = String::new();
        for (k, v) in env {
            // Quote each value with single quotes; escape any embedded
            // single quotes by closing/reopening the quoted segment.
            let escaped = v.replace('\'', "'\\''");
            env_str.push_str(&format!("export {}='{}'; ", k, escaped));
        }
        let args_str = args
            .iter()
            .map(|a| shell_escape(a))
            .collect::<Vec<_>>()
            .join(" ");
        let cmd = format!(
            "cd {} && {} {} {}",
            shell_escape(cwd),
            env_str,
            shell_escape(binary),
            args_str
        );
        channel
            .exec(&cmd)
            .map_err(|e| format!("Failed to exec agent over SSH: {}", e))?;

        sess.set_blocking(false);

        Ok(Box::new(RemoteSshTransport::new(channel, sess, tcp, cmd)))
    }
}

/// Single-quote-escape a string for use in a POSIX shell command. We don't
/// shell-escape the whole assembled command because the caller composes
/// segments and we only need to defend against the binary path / args.
fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '=' | ':' | ',' | '@'))
    {
        return s.to_string();
    }
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

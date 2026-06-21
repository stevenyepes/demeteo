use crate::paths;
use crate::ports::db::MachineRepository;
use crate::ports::execution::{ExecutionPort, InteractiveHandle};
use crate::sftp::SftpEntry;
use ssh2::{Channel, Session, Sftp};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct RemoteChannelHandle {
    channel: Mutex<Channel>,
    _session: Session,
}

impl InteractiveHandle for RemoteChannelHandle {
    fn write_line(&self, line: &str) -> std::io::Result<usize> {
        let mut channel = self.channel.lock().unwrap();
        channel.write_all(line.as_bytes())?;
        channel.write_all(b"\n")?;
        channel.flush()?;
        Ok(line.len() + 1)
    }

    fn try_read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut channel = self.channel.lock().unwrap();
        channel.read(buf)
    }

    fn kill(&self) -> Result<(), String> {
        let mut channel = self.channel.lock().unwrap();
        channel.close().map_err(|e| e.to_string())
    }

    fn try_wait(&self) -> Result<Option<i32>, String> {
        let channel = self.channel.lock().unwrap();
        match channel.exit_status() {
            Ok(code) => Ok(Some(code)),
            Err(e) => Err(e.to_string()),
        }
    }
}

pub struct SftpSession {
    pub sftp: Mutex<Sftp>,
    pub session: Session,
    pub tcp: TcpStream,
}

pub struct SshClientAdapter {
    pub machines: Arc<dyn MachineRepository>,
    pub sessions: Mutex<HashMap<String, Arc<SftpSession>>>,
    /// Resolved remote HOME per machine_id. The remote HOME is stable
    /// for the lifetime of the user's account, so we cache it after the
    /// first successful resolve to avoid an extra `echo $HOME` round-trip
    /// on every path computation. Cleared on `disconnect_all` (which
    /// isn't called today, but the cache is keyed by `machine_id` so
    /// reconnects naturally pick up the cached value).
    home_cache: Mutex<HashMap<String, String>>,
}

impl SshClientAdapter {
    pub fn new(machines: Arc<dyn MachineRepository>) -> Self {
        Self {
            machines,
            sessions: Mutex::new(HashMap::new()),
            home_cache: Mutex::new(HashMap::new()),
        }
    }

    fn get_sftp(&self, machine_id: &str) -> Result<Arc<SftpSession>, String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Failed to lock SFTP state".to_string())?;

        if let Some(s) = sessions.get(machine_id) {
            // Test if connection is still alive by sending a keepalive or checking status
            // If it fails, we will reconnect.
            let sftp = s
                .sftp
                .lock()
                .map_err(|_| "Failed to lock SFTP".to_string())?;
            if sftp.readdir(std::path::Path::new(".")).is_ok() {
                drop(sftp);
                return Ok(s.clone());
            }
            drop(sftp);
            sessions.remove(machine_id);
        }

        // Connect new session
        let machines = self.machines.get_machines()?;
        let machine_id_typed = crate::domain::ids::MachineId::from(machine_id.to_string());
        let machine = machines
            .into_iter()
            .find(|m| {
                m.id == machine_id_typed
                    || format!("{}@{}", m.username, m.host) == machine_id
                    || m.host == machine_id
                    || m.name == machine_id
            })
            .ok_or_else(|| "Machine not found".to_string())?;

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
        let mut sess =
            Session::new().map_err(|e| format!("Failed to create SSH session: {}", e))?;
        sess.set_tcp_stream(tcp.try_clone().map_err(|e| e.to_string())?);
        sess.set_timeout(10_000);
        sess.handshake()
            .map_err(|e| format!("SSH Handshake failed: {}", e))?;

        // Authenticate
        match machine.auth_type.as_str() {
            "password" => {
                let password = secret.ok_or_else(|| "Password not found in keyring".to_string())?;
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

        sess.set_blocking(true);
        let sftp = sess
            .sftp()
            .map_err(|e| format!("SFTP subsystem failed: {}", e))?;

        let sftp_session = Arc::new(SftpSession {
            sftp: Mutex::new(sftp),
            session: sess,
            tcp,
        });

        sessions.insert(machine_id.to_string(), sftp_session.clone());
        Ok(sftp_session)
    }

    /// Resolve the remote user's HOME directory by running `echo $HOME`
    /// over the SSH channel. Cached per `machine_id` so we only pay the
    /// round-trip once per session.
    fn resolve_remote_home(&self, machine_id: &str) -> Result<String, String> {
        if let Ok(cache) = self.home_cache.lock() {
            if let Some(home) = cache.get(machine_id) {
                eprintln!(
                    "[SshClientAdapter] resolve_remote_home({}) = {} (cache hit)",
                    machine_id, home
                );
                return Ok(home.clone());
            }
        }

        let sftp_sess = self.get_sftp(machine_id)?;
        let sess = sftp_sess.session.clone();
        let mut channel = sess
            .channel_session()
            .map_err(|e| format!("Failed to open SSH channel for HOME probe: {}", e))?;
        // `printf %s` avoids trailing newlines and respects quoting.
        channel
            .exec("printf %s \"$HOME\"")
            .map_err(|e| format!("Failed to exec HOME probe over SSH: {}", e))?;
        let mut raw = String::new();
        channel
            .read_to_string(&mut raw)
            .map_err(|e| format!("Failed to read HOME probe output: {}", e))?;
        channel
            .wait_close()
            .map_err(|e| format!("Failed to wait for HOME probe channel: {}", e))?;
        // ssh2's `wait_close` returns `Result<(), Error>`; the exit
        // status is on a separate method that returns `Result<i32, Error>`
        // (0 on success, non-zero on remote failure). Drain it so a
        // broken shell session doesn't get cached as a valid HOME.
        let exit_code = channel
            .exit_status()
            .map_err(|e| format!("Failed to read HOME probe exit status: {}", e))?;
        if exit_code != 0 {
            return Err(format!(
                "Remote HOME probe exited with status {}; the SSH session may be denying shell access",
                exit_code
            ));
        }

        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() {
            return Err("Remote HOME is empty (HOME is not set on the SSH session)".to_string());
        }
        if !trimmed.starts_with('/') {
            return Err(format!(
                "Remote HOME is not an absolute path (got '{}')",
                trimmed
            ));
        }

        eprintln!(
            "[SshClientAdapter] resolve_remote_home({}) = {} (fresh probe; cached)",
            machine_id, trimmed
        );
        if let Ok(mut cache) = self.home_cache.lock() {
            cache.insert(machine_id.to_string(), trimmed.clone());
        }
        Ok(trimmed)
    }
}

impl ExecutionPort for SshClientAdapter {
    fn test_connection(&self, machine_id: &str) -> Result<(), String> {
        let machines = self.machines.get_machines()?;
        let machine_id_typed = crate::domain::ids::MachineId::from(machine_id.to_string());
        let machine = machines
            .into_iter()
            .find(|m| {
                m.id == machine_id_typed
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
                let password = secret.ok_or_else(|| "Password not found in keyring".to_string())?;
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
        let mut channel = sftp_sess
            .session
            .channel_session()
            .map_err(|e| format!("Failed to open SSH channel: {}", e))?;
        channel
            .exec(cmd)
            .map_err(|e| format!("Failed to execute command: {}", e))?;

        // Drain both stdout AND stderr. The previous implementation only
        // read stdout and ignored the channel's exit status, which meant
        // a command that failed (e.g. `cd /nonexistent`) returned
        // `Ok("")` and every caller — the workspace-health check in
        // particular — concluded the operation succeeded. That is the
        // root cause of "UI says CLONED + HEALTHY but the agent can't
        // find the dir": the health probe was returning Ok on a path
        // that didn't exist.
        let mut stdout = String::new();
        channel
            .read_to_string(&mut stdout)
            .map_err(|e| format!("Failed to read command stdout: {}", e))?;

        // ssh2 keeps stderr on a separate stream. Drain it so the
        // remote shell's error message is included in the Err variant
        // (otherwise the user sees a useless "exit code: 1" with no
        // context, which is what the original bash `cd: ...: No such
        // file or directory` message was hiding behind).
        let mut stderr = String::new();
        let mut err_stream = channel.stderr();
        let _ = err_stream.read_to_string(&mut stderr);

        channel
            .wait_close()
            .map_err(|e| format!("Failed to wait for channel close: {}", e))?;
        // `Channel::exit_status` returns the remote process's exit
        // code as `Result<i32, Error>` (0 on success, non-zero on
        // failure). We must check this — see the comment on
        // `run_command` above.
        let exit_code = channel
            .exit_status()
            .map_err(|e| format!("Failed to read command exit status: {}", e))?;

        if exit_code != 0 {
            // Preserve any captured stderr; fall back to a generic
            // message if the remote shell didn't write anything.
            let detail = if stderr.trim().is_empty() {
                format!("exit code: {}", exit_code)
            } else {
                stderr.trim().to_string()
            };
            return Err(format!("Command failed ({}): {}", detail, cmd));
        }

        Ok(stdout)
    }

    /// Resolve the remote user's HOME directory by running `echo $HOME`
    /// over the SSH channel. Cached per `machine_id` so we only pay the
    /// round-trip once per session.
    fn read_file(&self, machine_id: &str, path: &str) -> Result<String, String> {
        let sftp_sess = self.get_sftp(machine_id)?;
        let sftp = sftp_sess
            .sftp
            .lock()
            .map_err(|_| "Failed to lock SFTP".to_string())?;

        let path_buf = std::path::Path::new(path);
        let mut file = sftp.open(path_buf).map_err(|e| {
            // Invalidate session cache if sftp connection went down
            if let Ok(mut sessions) = self.sessions.lock() {
                sessions.remove(machine_id);
            }
            format!("Failed to open file: {}", e)
        })?;

        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .map_err(|e| format!("Failed to read file content: {}", e))?;
        Ok(contents)
    }

    fn write_file(&self, machine_id: &str, path: &str, content: &str) -> Result<(), String> {
        let sftp_sess = self.get_sftp(machine_id)?;
        let sftp = sftp_sess
            .sftp
            .lock()
            .map_err(|_| "Failed to lock SFTP".to_string())?;

        let path_buf = std::path::Path::new(path);
        let mut file = sftp.create(path_buf).map_err(|e| {
            if let Ok(mut sessions) = self.sessions.lock() {
                sessions.remove(machine_id);
            }
            format!("Failed to create file: {}", e)
        })?;

        file.write_all(content.as_bytes())
            .map_err(|e| format!("Failed to write file: {}", e))?;
        file.flush()
            .map_err(|e| format!("Failed to flush file: {}", e))?;
        Ok(())
    }

    fn get_metadata(&self, machine_id: &str, path: &str) -> Result<SftpEntry, String> {
        let sftp_sess = self.get_sftp(machine_id)?;
        let sftp = sftp_sess
            .sftp
            .lock()
            .map_err(|_| "Failed to lock SFTP".to_string())?;

        let path_buf = std::path::Path::new(path);
        let stat = sftp.stat(path_buf).map_err(|e| {
            if let Ok(mut sessions) = self.sessions.lock() {
                sessions.remove(machine_id);
            }
            format!("Failed to stat file: {}", e)
        })?;

        let name = path_buf
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let size = stat.size.unwrap_or(0);
        let modified = stat.mtime.unwrap_or(0);
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
        let sftp = sftp_sess
            .sftp
            .lock()
            .map_err(|_| "Failed to lock SFTP".to_string())?;

        let path_buf = std::path::Path::new(path);
        let entries = sftp.readdir(path_buf).map_err(|e| {
            if let Ok(mut sessions) = self.sessions.lock() {
                sessions.remove(machine_id);
            }
            format!("Failed to read directory: {}", e)
        })?;

        let mut list = Vec::new();
        for (p, stat) in entries {
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            if name == "." || name == ".." {
                continue;
            }

            let path_str = p.to_str().unwrap_or("").to_string();
            let size = stat.size.unwrap_or(0);
            let modified = stat.mtime.unwrap_or(0);
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

    fn setup_worktree(
        &self,
        machine_id: &str,
        repo_path: &str,
        branch: &str,
        sandbox_path: &str,
    ) -> Result<(), String> {
        // Step 1: Ensure directory setup
        self.run_command(
            machine_id,
            &format!("mkdir -p {}/.demeteo/worktrees", repo_path),
        )?;

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
        println!(
            "[SshClientAdapter] Git Worktree provisioning output: {}",
            output
        );

        Ok(())
    }

    fn resolve_home(&self, machine_id: &str) -> Result<String, String> {
        if machine_id.is_empty() || machine_id == "local" {
            return Err("Cannot resolve remote HOME for local machine_id".to_string());
        }
        self.resolve_remote_home(machine_id)
    }

    fn spawn_interactive(
        &self,
        machine_id: &str,
        binary: &str,
        args: &[String],
        cwd: &str,
        env: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        let machines = self.machines.get_machines()?;
        let machine_id_typed = crate::domain::ids::MachineId::from(machine_id.to_string());
        let machine = machines
            .into_iter()
            .find(|m| {
                m.id == machine_id_typed
                    || format!("{}@{}", m.username, m.host) == machine_id
                    || m.host == machine_id
                    || m.name == machine_id
            })
            .ok_or_else(|| "Machine not found".to_string())?;

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

        let (sess, _tcp) = crate::terminal::connect_ssh(&machine, secret)?;
        sess.set_keepalive(true, 30);

        let mut channel = sess
            .channel_session()
            .map_err(|e| format!("Failed to open SSH channel: {}", e))?;

        channel
            .request_pty("xterm-256color", None, None)
            .map_err(|e| format!("Failed to request PTY on SSH channel: {}", e))?;

        let use_login_shell = machine.use_login_shell.unwrap_or(false);

        let mut env_str = String::new();
        for (k, v) in env {
            let escaped = v.replace('\'', "'\\''");
            env_str.push_str(&format!("export {}='{}'; ", k, escaped));
        }
        let args_str = args
            .iter()
            .map(|a| paths::shell_escape_posix(a))
            .collect::<Vec<_>>()
            .join(" ");

        let cmd = if use_login_shell {
            let inner = format!(
                "{} command cd {} && {{ command -v mise >/dev/null 2>&1 && mise trust --yes . || :; }} 2>/dev/null && exec {} {}",
                env_str,
                paths::shell_escape_posix(cwd),
                paths::shell_escape_posix(binary),
                args_str
            );
            format!("bash -l -c {}", paths::shell_escape_posix(&inner))
        } else {
            format!(
                "cd {} && {} {} {}",
                paths::shell_escape_posix(cwd),
                env_str,
                paths::shell_escape_posix(binary),
                args_str
            )
        };

        eprintln!("[SshClientAdapter] spawn_interactive cmd: {}", cmd);
        channel
            .exec(&cmd)
            .map_err(|e| format!("Failed to exec agent over SSH: {}", e))?;

        Ok(Box::new(RemoteChannelHandle {
            channel: Mutex::new(channel),
            _session: sess,
        }))
    }
}

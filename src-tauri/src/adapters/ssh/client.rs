use crate::paths;
use crate::ports::db::MachineRepository;
use crate::ports::execution::{ExecutionPort, InteractiveHandle};
use crate::sftp::SftpEntry;
use async_trait::async_trait;
use ssh2::{Channel, Session, Sftp};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

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
    pub sessions: Arc<Mutex<HashMap<String, Arc<SftpSession>>>>,
    /// Resolved remote HOME per machine_id. The remote HOME is stable
    /// for the lifetime of the user's account, so we cache it after the
    /// first successful resolve to avoid an extra `echo $HOME` round-trip
    /// on every path computation. Cleared on `disconnect_all` (which
    /// isn't called today, but the cache is keyed by `machine_id` so
    /// reconnects naturally pick up the cached value).
    home_cache: Arc<Mutex<HashMap<String, String>>>,
}

impl SshClientAdapter {
    pub fn new(machines: Arc<dyn MachineRepository>) -> Self {
        Self {
            machines,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            home_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Resolve the remote user's HOME directory by running `echo $HOME`
    /// over the SSH channel. Cached per `machine_id` so we only pay the
    /// round-trip once per session. Sync helper; the async
    /// `resolve_home` impl method wraps this in `spawn_blocking`.
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

        let sftp_sess = get_sftp_blocking(&self.machines, &self.sessions, machine_id)?;
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

/// Blocking helper used by the async `ExecutionPort` impl methods.
/// Opens (or returns a cached) SFTP session using only the shared
/// `Arc<MachineRepository>` and `Arc<Mutex<HashMap>>` so it can run
/// inside `tokio::task::spawn_blocking` without moving `&self`.
fn get_sftp_blocking(
    machines: &Arc<dyn crate::ports::db::MachineRepository>,
    sessions: &Mutex<HashMap<String, Arc<SftpSession>>>,
    machine_id: &str,
) -> Result<Arc<SftpSession>, String> {
    {
        let mut sessions = sessions
            .lock()
            .map_err(|_| "Failed to lock SFTP state".to_string())?;

        if let Some(s) = sessions.get(machine_id) {
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
    }

    // Connect new session
    let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
        &**machines,
        machine_id,
    )?;

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

    let (sess, tcp) = crate::ssh_util::connect(&machine, secret)?;

    sess.set_blocking(true);
    let sftp = sess
        .sftp()
        .map_err(|e| format!("SFTP subsystem failed: {}", e))?;

    let sftp_session = Arc::new(SftpSession {
        sftp: Mutex::new(sftp),
        session: sess,
        tcp,
    });

    let mut sessions = sessions
        .lock()
        .map_err(|_| "Failed to lock SFTP state".to_string())?;
    sessions.insert(machine_id.to_string(), sftp_session.clone());
    Ok(sftp_session)
}

#[async_trait]
impl ExecutionPort for SshClientAdapter {
    async fn test_connection(&self, machine_id: &str) -> Result<(), String> {
        let machine_id = machine_id.to_string();
        let machines = self.machines.clone();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
                &*machines,
                &machine_id,
            )?;

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

            let (sess, _tcp) = crate::ssh_util::connect(&machine, secret)?;

            // Connection is valid – disconnect cleanly
            let _ = sess.disconnect(None, "test complete", None);
            Ok(())
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn run_command(&self, machine_id: &str, cmd: &str) -> Result<String, String> {
        // The underlying `ssh2` API is fully sync (TCP + SFTP + Channel
        // I/O). Run the work on the blocking pool so we don't stall
        // the tokio worker thread. The error type stays `String` to
        // match the port signature.
        let machine_id = machine_id.to_string();
        let cmd = cmd.to_string();
        let machines = self.machines.clone();
        let sessions = self.sessions.clone();
        tokio::task::spawn_blocking(move || -> Result<String, String> {
            let sftp_sess = get_sftp_blocking(&machines, &sessions, &machine_id)?;
            let mut channel = sftp_sess
                .session
                .channel_session()
                .map_err(|e| format!("Failed to open SSH channel: {}", e))?;
            channel
                .exec(&cmd)
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
            // failure). We must check this — see the comment above.
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
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn read_file(&self, machine_id: &str, path: &str) -> Result<String, String> {
        let machine_id = machine_id.to_string();
        let path = path.to_string();
        let machines = self.machines.clone();
        let sessions = self.sessions.clone();
        tokio::task::spawn_blocking(move || -> Result<String, String> {
            let sftp_sess = get_sftp_blocking(&machines, &sessions, &machine_id)?;
            let sftp = sftp_sess
                .sftp
                .lock()
                .map_err(|_| "Failed to lock SFTP".to_string())?;

            let path_buf = std::path::Path::new(&path);
            let mut file = sftp.open(path_buf).map_err(|e| {
                if let Ok(mut sessions) = sessions.lock() {
                    sessions.remove(&machine_id);
                }
                format!("Failed to open file: {}", e)
            })?;

            let mut contents = String::new();
            file.read_to_string(&mut contents)
                .map_err(|e| format!("Failed to read file content: {}", e))?;
            Ok(contents)
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn write_file(&self, machine_id: &str, path: &str, content: &str) -> Result<(), String> {
        let machine_id = machine_id.to_string();
        let path = path.to_string();
        let content = content.to_string();
        let machines = self.machines.clone();
        let sessions = self.sessions.clone();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let sftp_sess = get_sftp_blocking(&machines, &sessions, &machine_id)?;
            let sftp = sftp_sess
                .sftp
                .lock()
                .map_err(|_| "Failed to lock SFTP".to_string())?;

            let path_buf = std::path::Path::new(&path);
            let mut file = sftp.create(path_buf).map_err(|e| {
                if let Ok(mut sessions) = sessions.lock() {
                    sessions.remove(&machine_id);
                }
                format!("Failed to create file: {}", e)
            })?;

            file.write_all(content.as_bytes())
                .map_err(|e| format!("Failed to write file: {}", e))?;
            file.flush()
                .map_err(|e| format!("Failed to flush file: {}", e))?;
            Ok(())
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn get_metadata(&self, machine_id: &str, path: &str) -> Result<SftpEntry, String> {
        let machine_id = machine_id.to_string();
        let path = path.to_string();
        let machines = self.machines.clone();
        let sessions = self.sessions.clone();
        tokio::task::spawn_blocking(move || -> Result<SftpEntry, String> {
            let sftp_sess = get_sftp_blocking(&machines, &sessions, &machine_id)?;
            let sftp = sftp_sess
                .sftp
                .lock()
                .map_err(|_| "Failed to lock SFTP".to_string())?;

            let path_buf = std::path::Path::new(&path);
            let stat = sftp.stat(path_buf).map_err(|e| {
                if let Ok(mut sessions) = sessions.lock() {
                    sessions.remove(&machine_id);
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
                path: path.clone(),
                is_dir,
                size,
                modified,
            })
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn list_dir(&self, machine_id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        let machine_id = machine_id.to_string();
        let path = path.to_string();
        let machines = self.machines.clone();
        let sessions = self.sessions.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<SftpEntry>, String> {
            let sftp_sess = get_sftp_blocking(&machines, &sessions, &machine_id)?;
            let sftp = sftp_sess
                .sftp
                .lock()
                .map_err(|_| "Failed to lock SFTP".to_string())?;

            let path_buf = std::path::Path::new(&path);
            let entries = sftp.readdir(path_buf).map_err(|e| {
                if let Ok(mut sessions) = sessions.lock() {
                    sessions.remove(&machine_id);
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
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn setup_worktree(
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
        )
        .await?;

        // Step 2: Configure git info exclude
        let git_exclude_cmd = format!(
            "if [ -d \"{0}/.git\" ]; then mkdir -p \"{0}/.git/info\"; if ! grep -q \".demeteo/\" \"{0}/.git/info/exclude\" 2>/dev/null; then echo \".demeteo/\" >> \"{0}/.git/info/exclude\"; fi; fi",
            repo_path
        );
        let _ = self.run_command(machine_id, &git_exclude_cmd).await;

        // Step 3: Run git worktree add
        let worktree_add_cmd = format!(
            "git -C \"{}\" worktree add -b \"{}\" \"{}\"",
            repo_path, branch, sandbox_path
        );
        let output = self.run_command(machine_id, &worktree_add_cmd).await?;
        println!(
            "[SshClientAdapter] Git Worktree provisioning output: {}",
            output
        );

        Ok(())
    }

    async fn resolve_home(&self, machine_id: &str) -> Result<String, String> {
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
        let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
            &*self.machines,
            machine_id,
        )?;

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

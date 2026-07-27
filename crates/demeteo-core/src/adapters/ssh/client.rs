//! The `ExecutionPort` translation layer for SSH.
//!
//! Every method here adapts the port's `async` signature onto the synchronous
//! `ssh2` API — almost always by moving the work onto `tokio::task::spawn_blocking`
//! (which can't borrow `self`, hence the single `Arc<SessionPool>` the closures
//! clone) — and then delegates the real work to a sibling module. There is no
//! transport policy in this file; if you are looking for behaviour, it is in one
//! of these:
//!
//! - `session` — the pooled SSH/SFTP connections, their liveness and eviction
//!   policy, and the keyring credential lookup (`machine_secret`).
//! - `transport` — the keepalive-aware drain policy (`drain_stream`) and the
//!   transport-vs-command error tagging.
//! - `home` — resolving and caching the remote `$HOME`.
//! - `command` — one-shot commands: shell-invocation assembly, channel exec,
//!   drain, and the exit-code invariant.
//! - `sftp` — file read/write, metadata, and directory listing.
//! - `interactive` — PTY-backed agent sessions: the command line and the
//!   `InteractiveHandle` over the channel.
//! - `control_rpc` — the `demeteo-runner` control-socket round-trip and its
//!   response decoding.

use super::session::{machine_secret, SessionPool};
// The pooled-session type moved to `session`, but it was public at this path
// before the split — re-export it so the crate's surface is unchanged.
pub use super::session::SftpSession;
use super::{command, control_rpc, interactive, sftp};
use crate::ports::db::MachineRepository;
use crate::ports::execution::SftpEntry;
use crate::ports::execution::{ExecutionPort, InteractiveHandle, ShellOptions};
use async_trait::async_trait;
use std::sync::Arc;

pub struct SshClientAdapter {
    /// Every pooled SSH/SFTP connection plus the remote-HOME cache. Held
    /// behind an `Arc` because the blocking `ssh2` API forces each port method
    /// onto `spawn_blocking`, which can't borrow `self` — the closures clone
    /// this one handle.
    pool: Arc<SessionPool>,
}

impl SshClientAdapter {
    pub fn new(machines: Arc<dyn MachineRepository>) -> Self {
        Self {
            pool: Arc::new(SessionPool::new(machines)),
        }
    }
}

#[async_trait]
impl ExecutionPort for SshClientAdapter {
    async fn test_connection(&self, machine_id: &str) -> Result<(), String> {
        let machine_id = machine_id.to_string();
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
                pool.machines(),
                &machine_id,
            )?;

            // Local machines don't use SSH – trivially valid
            if machine.auth_type == "local" {
                return Ok(());
            }

            let secret = machine_secret(&machine);

            let (sess, _tcp) = crate::ssh_util::connect(&machine, secret)?;

            // Connection is valid – disconnect cleanly
            let _ = sess.disconnect(None, "test complete", None);
            Ok(())
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn run_command_with(
        &self,
        machine_id: &str,
        cmd: &str,
        opts: ShellOptions,
    ) -> Result<String, String> {
        // The underlying `ssh2` API is fully sync (TCP + SFTP + Channel
        // I/O). Run the work on the blocking pool so we don't stall
        // the tokio worker thread. The error type stays `String` to
        // match the port signature.
        //
        // `run_command` (no override) delegates here via the trait default
        // with `ShellOptions::default()` — a non-login `sh -c` in the login
        // directory with no extra env, matching the historical behaviour of
        // the previous bare `channel.exec`, but now with cwd/env/login
        // honoured identically to the local adapter when the caller opts in.
        let machine_id = machine_id.to_string();
        let cmd = cmd.to_string();
        let pool = self.pool.clone();
        let limit = opts.timeout;
        let work = tokio::task::spawn_blocking(move || -> Result<String, String> {
            command::run_blocking(&pool, &machine_id, &cmd, &opts)
        });

        // `ShellOptions::timeout`, to the extent this transport can honour it.
        // ssh2 is a synchronous API driving a channel we cannot signal from
        // here, so the deadline bounds *our* wait; the remote process keeps
        // running until it exits on its own. Documented on the field, and the
        // error is the same one the local adapter returns, so callers classify
        // an expiry identically on both transports.
        match limit {
            Some(limit) => match tokio::time::timeout(limit, work).await {
                Ok(joined) => joined.map_err(|e| format!("blocking task panicked: {}", e))?,
                Err(_) => Err(format!(
                    "{}command exceeded its {}s ceiling",
                    crate::ports::execution::TIMEOUT_ERROR_PREFIX,
                    limit.as_secs()
                )),
            },
            None => work
                .await
                .map_err(|e| format!("blocking task panicked: {}", e))?,
        }
    }

    async fn read_file(&self, machine_id: &str, path: &str) -> Result<String, String> {
        let machine_id = machine_id.to_string();
        let path = path.to_string();
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || sftp::read_file(&pool, &machine_id, &path))
            .await
            .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn write_file(&self, machine_id: &str, path: &str, content: &str) -> Result<(), String> {
        let machine_id = machine_id.to_string();
        let path = path.to_string();
        let content = content.to_string();
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || sftp::write_file(&pool, &machine_id, &path, &content))
            .await
            .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn write_file_bytes(
        &self,
        machine_id: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), String> {
        let machine_id = machine_id.to_string();
        let path = path.to_string();
        let content = content.to_vec();
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            sftp::write_file_bytes(&pool, &machine_id, &path, &content)
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn get_metadata(&self, machine_id: &str, path: &str) -> Result<SftpEntry, String> {
        let machine_id = machine_id.to_string();
        let path = path.to_string();
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || sftp::get_metadata(&pool, &machine_id, &path))
            .await
            .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn list_dir(&self, machine_id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        let machine_id = machine_id.to_string();
        let path = path.to_string();
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || sftp::list_dir(&pool, &machine_id, &path))
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
        // Unlike every other method here this does not hop onto
        // `spawn_blocking` — it calls straight into the pool's blocking probe.
        // Pre-existing behaviour, preserved by the module split; see the note
        // on `SessionPool::resolve_home`.
        self.pool.resolve_home(machine_id)
    }

    async fn resolve_user(&self, machine_id: &str) -> Result<String, String> {
        if machine_id.is_empty() || machine_id == "local" {
            return Err("Cannot resolve remote USER for local machine_id".to_string());
        }
        // The SSH channel authenticates as `Machine.username`, so the
        // remote passwd entry's USER matches the machine record.
        // Return the record's value verbatim — if the user typed in a
        // machine with an empty username, the error from the lookup
        // below will surface that loud rather than the agent
        // silently running as the GUI's user.
        let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
            self.pool.machines(),
            machine_id,
        )?;
        if machine.username.is_empty() {
            return Err(format!(
                "Machine '{}' has no username configured; cannot resolve remote USER",
                machine_id
            ));
        }
        Ok(machine.username.clone())
    }

    async fn control_rpc(
        &self,
        machine_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let machine_id = machine_id.to_string();
        let method = method.to_string();
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || control_rpc::call(&pool, &machine_id, &method, params))
            .await
            .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    fn spawn_interactive(
        &self,
        machine_id: &str,
        binary: &str,
        args: &[String],
        cwd: &str,
        env: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        interactive::spawn(&self.pool, machine_id, binary, args, cwd, env)
    }
}

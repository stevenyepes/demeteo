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
        // Goes to the blocking pool like every other method here. On a cache
        // hit this is a mutex lock, but a miss can cost a DNS lookup, a TCP
        // connect, an auth handshake and a channel round-trip. Running that on
        // the caller's worker pinned the thread for the whole probe, and the
        // callers make that expensive: `agent_base_env` reaches it per agent
        // turn, and the Machines view fires one runner-status probe per
        // configured machine at once — enough unreachable machines and every
        // worker is occupied at the same time, which stalls the whole backend.
        let machine_id = machine_id.to_string();
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || pool.resolve_home(&machine_id))
            .await
            .map_err(|e| format!("blocking task panicked: {}", e))?
    }

    async fn resolve_user(&self, machine_id: &str) -> Result<String, String> {
        if machine_id.is_empty() || machine_id == "local" {
            return Err("Cannot resolve remote USER for local machine_id".to_string());
        }
        // No network here, but the machine lookup is a synchronous SQLite
        // query behind a mutex, and unlike HOME it is not cached — every
        // `agent_base_env` call pays it. Contend that mutex with a slow
        // write and an unwrapped call would block a worker, so it takes the
        // same route as the rest.
        let machine_id = machine_id.to_string();
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            // The SSH channel authenticates as `Machine.username`, so the
            // remote passwd entry's USER matches the machine record.
            // Return the record's value verbatim — if the user typed in a
            // machine with an empty username, the error from the lookup
            // below will surface that loud rather than the agent
            // silently running as the GUI's user.
            let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
                pool.machines(),
                &machine_id,
            )?;
            if machine.username.is_empty() {
                return Err(format!(
                    "Machine '{}' has no username configured; cannot resolve remote USER",
                    machine_id
                ));
            }
            Ok(machine.username.clone())
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{AgentProfileId, MachineId};
    use crate::domain::models::{AgentProfile, Machine};
    use std::sync::mpsc;
    use std::time::Duration;

    /// Minimal single-machine repo, same shape as the conformance suite's stub.
    struct OneMachine(Machine);
    impl MachineRepository for OneMachine {
        fn get_machines(&self) -> Result<Vec<Machine>, String> {
            Ok(vec![self.0.clone()])
        }
        fn get_machine(&self, id: &MachineId) -> Result<Option<Machine>, String> {
            Ok((id.0 == self.0.id.0).then(|| self.0.clone()))
        }
        fn add(&self, _: Machine) -> Result<(), String> {
            unimplemented!()
        }
        fn update(&self, _: Machine) -> Result<(), String> {
            unimplemented!()
        }
        fn delete(&self, _: &MachineId) -> Result<(), String> {
            unimplemented!()
        }
        fn get_agent_profiles(&self, _: &MachineId) -> Result<Vec<AgentProfile>, String> {
            Ok(vec![])
        }
        fn add_agent_profile(&self, _: AgentProfile) -> Result<(), String> {
            unimplemented!()
        }
        fn delete_agent_profile(&self, _: &AgentProfileId) -> Result<(), String> {
            unimplemented!()
        }
    }

    /// Every `ssh2` call in this file is synchronous, so a port method that
    /// reaches one without `spawn_blocking` pins a tokio worker for the whole
    /// round-trip. HOME resolution is the easiest one to get wrong because a
    /// cache hit is just a mutex lock — but a miss costs a DNS lookup, a TCP
    /// connect and an auth handshake, and the Machines view fires one
    /// runner-status probe per configured machine at once. Enough slow or
    /// unreachable machines and every worker is occupied at the same time,
    /// which stalls the whole backend, not just the probes.
    ///
    /// The runtime here has exactly one worker, so "the worker is free" is
    /// observable: a task spawned while the probe is in flight either gets
    /// polled promptly or never. The remote is a local listener that accepts
    /// the connection and then goes silent, which makes the TCP connect
    /// succeed immediately and the SSH handshake block until `ssh_util::connect`
    /// gives up — deterministic on any host, unlike an unroutable address whose
    /// behaviour depends on the routing table. The signalling deliberately uses
    /// `std::sync::mpsc` rather than `tokio::time`: a pinned worker also stops
    /// the runtime's time driver, so tokio timers would be measuring the very
    /// thing they were supposed to detect.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn resolving_remote_home_leaves_the_tokio_worker_free() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("failed to bind listener");
        let port = listener.local_addr().expect("no local addr").port();

        // Accept, then hold the connection open and answer nothing until the
        // test releases it. Dropping the stream any earlier would let the
        // handshake fail fast, which is the one thing that would make this
        // test vacuous.
        let (release_tx, release_rx) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            let _held = listener.accept();
            let _ = release_rx.recv();
        });

        let machine_id = "silent-ssh-peer";
        let machine = Machine {
            id: MachineId(machine_id.to_string()),
            name: "silent-ssh-peer".to_string(),
            host: "127.0.0.1".to_string(),
            port: i32::from(port),
            username: "demeteo".to_string(),
            // Anything but "local" takes the SSH path; "agent" also keeps the
            // credential lookup away from the OS keyring, which a test host
            // may not have.
            auth_type: "agent".to_string(),
            key_path: None,
            agents: None,
            auto_approved_rules: None,
            use_login_shell: Some(false),
            setup_commands: None,
            notify_webhook_url: None,
        };
        let adapter = SshClientAdapter::new(Arc::new(OneMachine(machine)));

        // Wait until the probe task is actually on the worker before asking
        // whether the worker is still free, so the answer can't depend on the
        // order the scheduler happened to pick the two tasks up in.
        let (started_tx, started_rx) = mpsc::channel::<()>();
        let probe = tokio::spawn(async move {
            let _ = started_tx.send(());
            adapter.resolve_home(machine_id).await
        });
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the HOME probe task never reached the worker");

        let (pinged_tx, pinged_rx) = mpsc::channel::<()>();
        tokio::spawn(async move {
            let _ = pinged_tx.send(());
        });
        let responsive = pinged_rx.recv_timeout(Duration::from_millis(500));

        // Release the peer first: a failed assertion still has to let the
        // handshake unwedge, or the runtime's drop waits out the full ssh2
        // timeout before the failure is reported.
        drop(release_tx);
        probe.abort();

        assert!(
            responsive.is_ok(),
            "the tokio worker was still occupied by the HOME probe after 500ms — \
             resolve_home must hand the blocking ssh2 work to spawn_blocking",
        );
    }
}

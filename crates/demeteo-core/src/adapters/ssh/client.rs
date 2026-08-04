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
//! - `retry` — the re-establish-and-retry loop each method below wraps its
//!   blocking half in, and the rule that decides when a failure is eligible.
//!
//! Which methods retry, and why, is a policy statement rather than an
//! implementation detail, so it is stated once here (S4,
//! `docs/RELIABILITY_PLAN.md`):
//!
//! | method | retried | why |
//! |---|---|---|
//! | `run_command_with` | yes | only when the shell never received the command |
//! | `read_file` / `write_file(_bytes)` / `get_metadata` / `list_dir` | yes | same rule; the session could not be established |
//! | `resolve_home` | yes | same rule |
//! | `setup_worktree` | inherited | it is three `run_command` calls |
//! | `test_connection` | **no** | its answer *is* "can this connect right now"; retrying would report a flaky host as healthy |
//! | `resolve_user` | **no** | a database read; no network to drop |
//! | `control_rpc` | **no** | side-effecting runner methods, and it backs a reachability probe (see `control_rpc`) |
//! | `spawn_interactive` | **no** | a live PTY cannot be re-established under a caller already holding the handle |

use super::retry::{with_ssh_retry, SshFailure};
use super::session::{machine_secret, SessionPool};
// The pooled-session type moved to `session`, but it was public at this path
// before the split — re-export it so the crate's surface is unchanged.
pub use super::session::SftpSession;
use super::{command, control_rpc, interactive, sftp};
use crate::ports::db::MachineRepository;
use crate::ports::execution::SftpEntry;
use crate::ports::execution::{ExecutionPort, InteractiveHandle, ProgramRequest, ShellOptions};
use crate::ports::worktree_ops::{
    CreateTrustedTerminalWorktreeRequest, DependencyCacheMaterialization,
    MaterializeDependencyCacheRequest, RemoveTrustedTerminalWorktreeRequest,
    TrustedTerminalWorktreeCreated, TrustedTerminalWorktreeRemoved, TrustedWorktreePort,
};
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

    async fn run_program(
        &self,
        machine_id: &str,
        request: ProgramRequest,
    ) -> Result<String, String> {
        let mut argv = Vec::with_capacity(request.args.len() + 1);
        argv.push(crate::paths::shell_escape_posix(&request.executable));
        argv.extend(
            request
                .args
                .iter()
                .map(|arg| crate::paths::shell_escape_posix(arg)),
        );
        self.run_command_with(
            machine_id,
            &format!("exec {}", argv.join(" ")),
            ShellOptions {
                login_shell: false,
                interactive: false,
                cwd: request.cwd,
                env: request.env,
                timeout: request.timeout,
            },
        )
        .await
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
        //
        // `ShellOptions::timeout` is handed to `with_ssh_retry`, which spends
        // it across the *whole* call rather than per attempt: a caller asking
        // for a ceiling gets that ceiling, not `attempts × ceiling`. That also
        // keeps the expiry message byte-identical to the local adapter's, so
        // both transports classify an expiry the same way.
        //
        // This is the one method where re-running is genuinely dangerous — it
        // is arbitrary user shell — and the reason it can be retried at all is
        // that `command::run_blocking` reports *where* it failed. Only a
        // failure before `channel.exec` is eligible; see `super::retry`.
        let mid = machine_id.to_string();
        let cmd = cmd.to_string();
        let limit = opts.timeout;
        with_ssh_retry("run_command", machine_id, &self.pool, limit, || {
            let pool = self.pool.clone();
            let mid = mid.clone();
            let cmd = cmd.clone();
            let opts = opts.clone();
            async move {
                tokio::task::spawn_blocking(move || command::run_blocking(&pool, &mid, &cmd, &opts))
                    .await
                    .map_err(|e| SshFailure::answered(format!("blocking task panicked: {}", e)))?
            }
        })
        .await
    }

    async fn read_file(&self, machine_id: &str, path: &str) -> Result<String, String> {
        let mid = machine_id.to_string();
        let path = path.to_string();
        with_ssh_retry("read_file", machine_id, &self.pool, None, || {
            let pool = self.pool.clone();
            let mid = mid.clone();
            let path = path.clone();
            async move {
                tokio::task::spawn_blocking(move || sftp::read_file(&pool, &mid, &path))
                    .await
                    .map_err(|e| SshFailure::answered(format!("blocking task panicked: {}", e)))?
            }
        })
        .await
    }

    async fn write_file(&self, machine_id: &str, path: &str, content: &str) -> Result<(), String> {
        self.write_file_bytes(machine_id, path, content.as_bytes())
            .await
    }

    async fn write_file_bytes(
        &self,
        machine_id: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), String> {
        let mid = machine_id.to_string();
        let path = path.to_string();
        let content = content.to_vec();
        with_ssh_retry("write_file", machine_id, &self.pool, None, || {
            let pool = self.pool.clone();
            let mid = mid.clone();
            let path = path.clone();
            let content = content.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    sftp::write_file_bytes(&pool, &mid, &path, &content)
                })
                .await
                .map_err(|e| SshFailure::answered(format!("blocking task panicked: {}", e)))?
            }
        })
        .await
    }

    async fn create_dir_all(&self, machine_id: &str, path: &str) -> Result<(), String> {
        let mid = machine_id.to_string();
        let path = path.to_string();
        with_ssh_retry("create_dir_all", machine_id, &self.pool, None, || {
            let pool = self.pool.clone();
            let mid = mid.clone();
            let path = path.clone();
            async move {
                tokio::task::spawn_blocking(move || sftp::create_dir_all(&pool, &mid, &path))
                    .await
                    .map_err(|e| SshFailure::answered(format!("blocking task panicked: {}", e)))?
            }
        })
        .await
    }

    async fn remove_dir_all(&self, machine_id: &str, path: &str) -> Result<(), String> {
        let mid = machine_id.to_string();
        let path = path.to_string();
        with_ssh_retry("remove_dir_all", machine_id, &self.pool, None, || {
            let pool = self.pool.clone();
            let mid = mid.clone();
            let path = path.clone();
            async move {
                tokio::task::spawn_blocking(move || sftp::remove_dir_all(&pool, &mid, &path))
                    .await
                    .map_err(|e| SshFailure::answered(format!("blocking task panicked: {}", e)))?
            }
        })
        .await
    }

    async fn remove_file(&self, machine_id: &str, path: &str) -> Result<(), String> {
        let mid = machine_id.to_string();
        let path = path.to_string();
        with_ssh_retry("remove_file", machine_id, &self.pool, None, || {
            let pool = self.pool.clone();
            let mid = mid.clone();
            let path = path.clone();
            async move {
                tokio::task::spawn_blocking(move || sftp::remove_file(&pool, &mid, &path))
                    .await
                    .map_err(|e| SshFailure::answered(format!("blocking task panicked: {}", e)))?
            }
        })
        .await
    }

    async fn is_executable(&self, machine_id: &str, path: &str) -> Result<bool, String> {
        let mid = machine_id.to_string();
        let path = path.to_string();
        with_ssh_retry("is_executable", machine_id, &self.pool, None, || {
            let pool = self.pool.clone();
            let mid = mid.clone();
            let path = path.clone();
            async move {
                tokio::task::spawn_blocking(move || sftp::is_executable(&pool, &mid, &path))
                    .await
                    .map_err(|e| SshFailure::answered(format!("blocking task panicked: {}", e)))?
            }
        })
        .await
    }

    async fn get_metadata(&self, machine_id: &str, path: &str) -> Result<SftpEntry, String> {
        let mid = machine_id.to_string();
        let path = path.to_string();
        with_ssh_retry("get_metadata", machine_id, &self.pool, None, || {
            let pool = self.pool.clone();
            let mid = mid.clone();
            let path = path.clone();
            async move {
                tokio::task::spawn_blocking(move || sftp::get_metadata(&pool, &mid, &path))
                    .await
                    .map_err(|e| SshFailure::answered(format!("blocking task panicked: {}", e)))?
            }
        })
        .await
    }

    async fn list_dir(&self, machine_id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        let mid = machine_id.to_string();
        let path = path.to_string();
        with_ssh_retry("list_dir", machine_id, &self.pool, None, || {
            let pool = self.pool.clone();
            let mid = mid.clone();
            let path = path.clone();
            async move {
                tokio::task::spawn_blocking(move || sftp::list_dir(&pool, &mid, &path))
                    .await
                    .map_err(|e| SshFailure::answered(format!("blocking task panicked: {}", e)))?
            }
        })
        .await
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
        let mid = machine_id.to_string();
        with_ssh_retry("resolve_home", machine_id, &self.pool, None, || {
            let pool = self.pool.clone();
            let mid = mid.clone();
            async move {
                tokio::task::spawn_blocking(move || pool.resolve_home(&mid))
                    .await
                    .map_err(|e| SshFailure::answered(format!("blocking task panicked: {}", e)))?
            }
        })
        .await
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

#[async_trait]
impl TrustedWorktreePort for SshClientAdapter {
    async fn create_terminal_worktree(
        &self,
        _request: CreateTrustedTerminalWorktreeRequest,
    ) -> Result<TrustedTerminalWorktreeCreated, String> {
        Err(trusted_worktree_helper_unavailable())
    }

    async fn remove_terminal_worktree(
        &self,
        _request: RemoveTrustedTerminalWorktreeRequest,
    ) -> Result<TrustedTerminalWorktreeRemoved, String> {
        Err(trusted_worktree_helper_unavailable())
    }

    async fn materialize_dependency_cache(
        &self,
        _request: MaterializeDependencyCacheRequest,
    ) -> Result<DependencyCacheMaterialization, String> {
        Err(trusted_worktree_helper_unavailable())
    }
}

fn trusted_worktree_helper_unavailable() -> String {
    "Trusted worktree operations are unavailable over SSH: the remote trusted-worktree helper is not installed. These operations require one remote helper transaction so the target host can enforce no-follow checks and prevent check-to-use substitution; Demeteo will not compose SFTP or independent commands as a fallback.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{AgentProfileId, MachineId};
    use crate::domain::models::{AgentProfile, Machine};
    use crate::ports::worktree_ops::{TerminalWorktreeRequest, TrustedWorktreeTarget};
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn trusted_worktree_operations_fail_closed_without_the_remote_helper() {
        let error = trusted_worktree_helper_unavailable();
        assert!(error.contains("unavailable over SSH"));
        assert!(error.contains("one remote helper transaction"));
        assert!(error.contains("will not compose SFTP"));
    }

    #[tokio::test]
    async fn trusted_worktree_rejects_adversarial_remote_paths_without_connecting() {
        let adapter = SshClientAdapter::new(Arc::new(OneMachine(Machine {
            id: MachineId("remote".to_string()),
            name: "remote".to_string(),
            host: "127.0.0.1".to_string(),
            port: 1,
            username: "demeteo".to_string(),
            auth_type: "agent".to_string(),
            key_path: None,
            agents: None,
            auto_approved_rules: None,
            use_login_shell: Some(false),
            setup_commands: None,
            notify_webhook_url: None,
        })));
        let target = TrustedWorktreeTarget::from_resolved(
            Some("remote".to_string()),
            "/repo; touch should-not-run".to_string(),
            "/project/../project-link".to_string(),
        );

        let error = TrustedWorktreePort::create_terminal_worktree(
            &adapter,
            CreateTrustedTerminalWorktreeRequest {
                target: target.clone(),
                terminal: TerminalWorktreeRequest {
                    branch: "terminal/$(touch should-not-run)".to_string(),
                    base_branch: Some("main; touch should-not-run".to_string()),
                    worktree_name: "../../should-not-run".to_string(),
                },
            },
        )
        .await
        .expect_err("SSH must not compose a fallback transaction");

        assert_eq!(error, trusted_worktree_helper_unavailable());

        let error = TrustedWorktreePort::remove_terminal_worktree(
            &adapter,
            RemoveTrustedTerminalWorktreeRequest {
                target: target.clone(),
                worktree_name: "../../should-not-run".to_string(),
                force: true,
            },
        )
        .await
        .expect_err("SSH must not remove an adversarial path without the helper");
        assert_eq!(error, trusted_worktree_helper_unavailable());

        let error = TrustedWorktreePort::materialize_dependency_cache(
            &adapter,
            MaterializeDependencyCacheRequest {
                target,
                worktree_dir: "/worktree/../../should-not-run".to_string(),
                feature_cache_dir: "/cache/$(touch should-not-run)".to_string(),
            },
        )
        .await
        .expect_err("SSH must not materialize an adversarial cache path without the helper");
        assert_eq!(error, trusted_worktree_helper_unavailable());
    }

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

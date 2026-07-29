//! Shared behavioural conformance suite for `ExecutionPort` (C2.1,
//! `docs/EXECUTION_PARITY.md`).
//!
//! This is the single place that defines "correct" for *every* transport
//! (`LocalSubprocessAdapter`, `SshClientAdapter`, `RouterExecutionPort`).
//! Each assertion maps to a clause of the trait-level contract on
//! [`ExecutionPort`]:
//!
//! * write → read round-trip (`write_file`/`read_file`);
//! * `cwd` honoured by `run_command_with`;
//! * non-zero exit ⇒ `Err` carrying stderr (D3) — never `Ok("")`;
//! * missing file ⇒ `Err`, not `Ok("")` (D3);
//! * `list_dir` entry shape (name/is_dir, `.`/`..` filtered);
//! * login-shell env resolution (D2 — the caller's env crosses the boundary);
//! * a command silent longer than the transport's blocking-call timeout still
//!   drains to EOF and returns its output (D3 — a slow, silent command is not
//!   a transport failure; this is leak #1: local drains a pipe forever, SSH
//!   used to abort at the 10s session timeout).
//!
//! New behaviour is added *here*, not bug-hunted onto each adapter. The SSH
//! target is wired the same way against a loopback `sshd` in C2.2 (feature-
//! gated so a Docker-less `cargo test` still passes); running the identical
//! `exec_contract` against both adapters is what proves local/SSH parity.

use std::sync::Arc;

use crate::adapters::local::execution::LocalSubprocessAdapter;
use crate::ports::execution::{ExecutionPort, ShellOptions};

/// Exercise the full `ExecutionPort` contract against `port`, using
/// `machine_id` to address the target and `workdir` as a pre-existing,
/// writable directory on that target. `workdir` is a parameter (rather than
/// hard-coded) precisely so the SSH variant can pass a remote temp dir and
/// run the byte-identical assertions.
pub async fn exec_contract(port: Arc<dyn ExecutionPort>, machine_id: &str, workdir: &str) {
    let base = workdir.trim_end_matches('/');
    // Canonicalize `base` when it exists locally so the local leg's
    // `pwd` assertion doesn't trip over macOS symlinked prefixes
    // (`/var/folders/...` resolves to `/private/var/folders/...` once
    // a subprocess `current_dir`'s into it). Falls back to the literal
    // when canonicalization fails — the SSH workdir doesn't exist on
    // the local FS, so a remote `pwd` returning the same literal path
    // (the typical Linux-container case has no symlinks) still
    // round-trips. Mirrors the same fix already used in
    // `tests/infrastructure/worktree/git_ops.rs::test_list_worktrees_with_one_extra_worktree`.
    let base = std::fs::canonicalize(base)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| base.to_string());

    // --- write → read round-trip -----------------------------------------
    let file_path = format!("{base}/conformance-roundtrip.txt");
    let body = "hello from the conformance suite\nsecond line\n";
    port.write_file(machine_id, &file_path, body)
        .await
        .expect("write_file should succeed into a writable dir");
    let read_back = port
        .read_file(machine_id, &file_path)
        .await
        .expect("read_file should return the bytes just written");
    assert_eq!(read_back, body, "write → read must round-trip exactly");

    // --- cwd honoured ----------------------------------------------------
    let out = port
        .run_command_with(
            machine_id,
            "pwd",
            ShellOptions {
                cwd: Some(base.to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("pwd in an explicit cwd should succeed");
    assert_eq!(
        out.trim(),
        base,
        "run_command_with must honour the explicit cwd",
    );

    // --- non-zero exit ⇒ Err carrying stderr (never Ok(\"\")) -------------
    let err = port
        .run_command(machine_id, "echo boom-on-stderr 1>&2; exit 3")
        .await
        .expect_err("a non-zero exit must be Err, never Ok(\"\")");
    assert!(
        err.contains("boom-on-stderr"),
        "the Err must carry the command's stderr, got: {err}",
    );

    // --- missing file ⇒ Err, not Ok(\"\") --------------------------------
    let missing = format!("{base}/definitely-does-not-exist-xyz.txt");
    port.read_file(machine_id, &missing)
        .await
        .expect_err("reading a missing file must be Err, never Ok(\"\")");

    // --- list_dir entry shape -------------------------------------------
    let entries = port
        .list_dir(machine_id, &base)
        .await
        .expect("list_dir on the workdir should succeed");
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(
        names.contains(&"conformance-roundtrip.txt"),
        "list_dir must include the file we wrote; got {names:?}",
    );
    assert!(
        !names.iter().any(|n| *n == "." || *n == ".."),
        "list_dir must filter out `.` and `..`; got {names:?}",
    );
    let file_entry = entries
        .iter()
        .find(|e| e.name == "conformance-roundtrip.txt")
        .unwrap();
    assert!(!file_entry.is_dir, "the written file must not be is_dir");

    // --- login-shell env resolution (D2) ---------------------------------
    // The caller's env must cross the transport boundary and win inside the
    // (login) shell body. This is the exact mechanism that closes the
    // "works local, missing remote" PATH gap.
    let mut env = std::collections::BTreeMap::new();
    env.insert(
        "DEMETEO_CONFORMANCE".to_string(),
        "marker-value".to_string(),
    );
    let echoed = port
        .run_command_with(
            machine_id,
            "printf %s \"$DEMETEO_CONFORMANCE\"",
            ShellOptions {
                login_shell: true,
                env,
                ..Default::default()
            },
        )
        .await
        .expect("login-shell command with env should succeed");
    assert_eq!(
        echoed.trim(),
        "marker-value",
        "caller-supplied env must be visible inside the login shell",
    );

    // --- long silent command survives the blocking-call timeout (D3) -----
    // A command that produces no output for longer than the SSH session's
    // 10s blocking-call timeout (`ssh_util::connect`) must still drain to
    // EOF and return its output — never abort with "Timed out waiting on
    // socket". Locally there is no socket timeout so this passes trivially;
    // on SSH it reproduces the reported drift where `cargo test` compiles
    // silently for >10s and the prepare command spuriously "fails"
    // (`docs/EXECUTION_PARITY.md`, leak #1). The 13s silence clears
    // the 10s timeout with margin. This is the failing assertion that the
    // shared timeout-tolerant drain helper (B) must turn green on SSH.
    let out = port
        .run_command(machine_id, "sleep 13; printf %s survived-the-silence")
        .await
        .expect("a command silent longer than the session timeout must still complete");
    assert_eq!(
        out.trim(),
        "survived-the-silence",
        "output produced after a >timeout silent gap must be captured to EOF",
    );

    // --- silent command survives across a full keepalive cycle -------------
    // The drain distinguishes "quiet but alive" from "dead connection" by
    // whether keepalives still round-trip (`NO_PROGRESS_ABORT` in the SSH
    // client). A 35s silence crosses the 30s keepalive interval, so the life
    // clock is only kept fresh if answered keepalives reset it — the exact
    // property that must NOT regress into killing healthy silent commands.
    // Only meaningful (and only worth its wall-clock cost) on the SSH leg;
    // locally there is no keepalive/timeout, so we skip it to keep the default
    // suite fast.
    if machine_id != "local" {
        let out = port
            .run_command(machine_id, "sleep 35; printf %s survived-a-keepalive-cycle")
            .await
            .expect("a command silent across a keepalive cycle must still complete");
        assert_eq!(
            out.trim(),
            "survived-a-keepalive-cycle",
            "a silent-but-alive command must survive past the keepalive interval",
        );
    }
}

/// Create a fresh, unique, writable temp directory for a local run and
/// return its absolute path. Kept local to the suite so the assertions
/// don't depend on any external temp-dir crate.
fn fresh_local_workdir() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("demeteo-exec-contract-{nanos}"));
    std::fs::create_dir_all(&dir).expect("failed to create local conformance workdir");
    dir.to_string_lossy().into_owned()
}

#[tokio::test]
async fn local_subprocess_adapter_satisfies_the_contract() {
    let workdir = fresh_local_workdir();
    let port: Arc<dyn ExecutionPort> = Arc::new(LocalSubprocessAdapter::new());
    exec_contract(port, "local", &workdir).await;
    let _ = std::fs::remove_dir_all(&workdir);
}

// ─────────────────────────────────────────────────────────────────────────
// C2.2 — the same `exec_contract`, against a real SSH target
// ─────────────────────────────────────────────────────────────────────────
//
// Running the byte-identical assertions above against `SshClientAdapter` is
// the only thing that proves local/SSH parity: a regression that reintroduces
// the bare non-login `channel.exec` (dropping cwd/env/login-shell) turns *this*
// red while the local leg stays green.
//
// Gated behind the `ssh-conformance` feature so a Docker-less `cargo test`
// still passes. The CI job — and `tests/conformance/run-ssh-conformance.sh`
// for local runs — stands up a throwaway loopback `sshd`
// (tests/conformance/sshd/Dockerfile) and exports the connection as env:
//
//   DEMETEO_SSH_CONFORMANCE_HOST      (default 127.0.0.1)
//   DEMETEO_SSH_CONFORMANCE_PORT      (default 2222)
//   DEMETEO_SSH_CONFORMANCE_USER      (default demeteo)
//   DEMETEO_SSH_CONFORMANCE_PASSWORD  (required)
//   DEMETEO_SSH_CONFORMANCE_WORKDIR   (default /home/<user>/conformance)
/// The loopback container's connection details and the wiring both SSH legs
/// need. Kept in one place so a second test against the same sshd does not
/// re-derive the env contract above.
#[cfg(feature = "ssh-conformance")]
mod ssh_target {
    use crate::adapters::ssh::client::SshClientAdapter;
    use crate::domain::ids::{AgentProfileId, MachineId};
    use crate::domain::models::{AgentProfile, Machine};
    use crate::ports::db::MachineRepository;
    use crate::ports::execution::ExecutionPort;
    use std::sync::Arc;

    /// Minimal single-machine repo pointing the adapter at the container.
    pub struct OneMachine(pub Machine);
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

    pub struct Target {
        pub host: String,
        pub port: i32,
        pub user: String,
        pub password: String,
        pub workdir: String,
    }

    pub fn target() -> Target {
        let host = std::env::var("DEMETEO_SSH_CONFORMANCE_HOST")
            .unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = std::env::var("DEMETEO_SSH_CONFORMANCE_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(2222);
        let user =
            std::env::var("DEMETEO_SSH_CONFORMANCE_USER").unwrap_or_else(|_| "demeteo".to_string());
        let password = std::env::var("DEMETEO_SSH_CONFORMANCE_PASSWORD")
            .expect("DEMETEO_SSH_CONFORMANCE_PASSWORD must be set for the ssh-conformance test");
        let workdir = std::env::var("DEMETEO_SSH_CONFORMANCE_WORKDIR")
            .unwrap_or_else(|_| format!("/home/{user}/conformance"));
        Target {
            host,
            port,
            user,
            password,
            workdir,
        }
    }

    /// An adapter addressing `machine_id`, reaching the container on
    /// `port` — which is the container's own port for the contract leg and a
    /// proxy's for the drop leg.
    pub fn adapter(t: &Target, port: i32, machine_id: &str) -> Arc<dyn ExecutionPort> {
        let machine = Machine {
            id: MachineId(machine_id.to_string()),
            name: machine_id.to_string(),
            host: t.host.clone(),
            port,
            username: t.user.clone(),
            auth_type: "password".to_string(),
            key_path: None,
            agents: None,
            auto_approved_rules: None,
            use_login_shell: Some(false),
            setup_commands: None,
            notify_webhook_url: None,
        };
        // Seed the in-process credential cache so the adapter's password lookup
        // never reaches the OS keyring (which the test build may lack, and
        // which has no secret for this throwaway host anyway).
        crate::credential_cache::set(&format!("machine_{machine_id}"), &t.password);
        Arc::new(SshClientAdapter::new(Arc::new(OneMachine(machine))))
    }
}

#[cfg(feature = "ssh-conformance")]
#[tokio::test]
async fn ssh_client_adapter_satisfies_the_contract() {
    let t = ssh_target::target();
    let machine_id = "ssh-conformance";
    let port = ssh_target::adapter(&t, t.port, machine_id);

    // Ensure the workdir exists, mirroring `fresh_local_workdir` for the local
    // leg — the shared `exec_contract` assumes a pre-existing writable dir.
    port.run_command(machine_id, &format!("mkdir -p {}", t.workdir))
        .await
        .expect("failed to create the remote conformance workdir");

    exec_contract(port, machine_id, &t.workdir).await;
}

// ─────────────────────────────────────────────────────────────────────────
// S4 — a genuine dropped connection, absorbed and then not absorbed
// ─────────────────────────────────────────────────────────────────────────

/// A TCP relay in front of the container's sshd that can be told to drop every
/// live connection and refuse new ones.
///
/// This is the only instrument available that produces a *real* dropped SSH
/// session: killing the container would end the test target, and a fake
/// `ExecutionPort` cannot exercise the thing under test at all — the retry
/// depends on `SessionPool` evicting a corpse and `ssh_util::connect`
/// re-handshaking, neither of which exists in a double. Cutting the transport
/// underneath a live adapter reproduces the field failure exactly: the pooled
/// session's liveness probe fails, the reconnect finds nothing listening back,
/// and everything the driver would have seen is what the assertions read.
#[cfg(feature = "ssh-conformance")]
struct FlakyProxy {
    port: u16,
    allow: std::sync::Arc<std::sync::atomic::AtomicBool>,
    live: std::sync::Arc<std::sync::Mutex<Vec<std::net::TcpStream>>>,
    stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(feature = "ssh-conformance")]
impl FlakyProxy {
    fn start(upstream: String) -> Self {
        use std::net::{TcpListener, TcpStream};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};

        let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind the proxy");
        let port = listener.local_addr().expect("no proxy addr").port();
        let allow = Arc::new(AtomicBool::new(true));
        let live: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));

        let (a, l, s) = (allow.clone(), live.clone(), stopped.clone());
        std::thread::spawn(move || {
            for incoming in listener.incoming() {
                if s.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(client) = incoming else { continue };
                // Refusing by accepting and closing (rather than not listening)
                // keeps the TCP connect fast and pushes the failure into the
                // SSH handshake — which is the shape a dropped session
                // actually presents, and one `PERMANENT_MARKERS` must not
                // mistake for a bad credential.
                if !a.load(Ordering::SeqCst) {
                    let _ = client.shutdown(std::net::Shutdown::Both);
                    continue;
                }
                let Ok(server) = TcpStream::connect(&upstream) else {
                    let _ = client.shutdown(std::net::Shutdown::Both);
                    continue;
                };
                if let (Ok(mut held), Ok(c), Ok(sv)) =
                    (l.lock(), client.try_clone(), server.try_clone())
                {
                    held.push(c);
                    held.push(sv);
                }
                for (mut from, mut to) in [
                    (
                        client.try_clone().expect("clone client"),
                        server.try_clone().expect("clone server"),
                    ),
                    (server, client),
                ] {
                    std::thread::spawn(move || {
                        let _ = std::io::copy(&mut from, &mut to);
                    });
                }
            }
        });

        Self {
            port,
            allow,
            live,
            stopped,
        }
    }

    /// Drop every live connection and refuse new ones.
    fn cut(&self) {
        use std::sync::atomic::Ordering;
        self.allow.store(false, Ordering::SeqCst);
        if let Ok(mut held) = self.live.lock() {
            for s in held.drain(..) {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
        }
    }

    fn restore(&self) {
        self.allow.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn allow_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.allow.clone()
    }
}

#[cfg(feature = "ssh-conformance")]
impl Drop for FlakyProxy {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        self.stopped.store(true, Ordering::SeqCst);
        // The accept loop is parked in `accept`; one connection wakes it so the
        // thread observes the flag and exits instead of outliving the test.
        let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
    }
}

/// S4 end-to-end, against a real sshd: a network that goes away for a moment
/// must not end a run, and one that stays away must still be reported as a
/// transport failure.
///
/// Both halves matter and they pull in opposite directions. Only the first
/// would let a wrapper pass that swallowed every failure into a retry loop;
/// only the second would let one pass that never retried at all. Together they
/// pin the two properties the plan entry names — absorb the blip, preserve the
/// distinction the verifier depends on.
#[cfg(feature = "ssh-conformance")]
#[tokio::test]
async fn ssh_client_adapter_absorbs_a_brief_drop_and_still_reports_a_lasting_one() {
    use crate::adapters::step_executor::driver::verifier::{
        classify_exec_failure, HarnessExecFailure,
    };
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    let t = ssh_target::target();
    let proxy = FlakyProxy::start(format!("{}:{}", t.host, t.port));
    let machine_id = "ssh-drop-recovery";
    let port = ssh_target::adapter(&t, i32::from(proxy.port), machine_id);

    // Establish and pool a session, so the drop below is a *live* session
    // dying rather than a machine that was never reachable.
    let first = port
        .run_command(machine_id, "printf %s established")
        .await
        .expect("the proxied session must work before anything is cut");
    assert_eq!(first.trim(), "established");

    // --- a blip the retry must absorb --------------------------------------
    proxy.cut();
    let allow = proxy.allow_flag();
    let healer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        allow.store(true, Ordering::SeqCst);
    });
    let recovered = port.run_command(machine_id, "printf %s recovered").await;
    healer.await.expect("the healer task must not panic");
    assert_eq!(
        recovered.as_deref().map(str::trim),
        Ok("recovered"),
        "a dropped session that comes back must be re-established transparently",
    );

    // --- an outage that outlasts the whole retry budget ---------------------
    // This is the regression guard for the verifier chain, asserted through
    // the classifier rather than the string: exhausted retries must still be
    // `Transport`, which routes to a non-retryable `Infrastructure` instead of
    // sending an agent to fix code that was never tested (C0.2 / D3).
    proxy.cut();
    let err = port
        .run_command(machine_id, "printf %s unreachable")
        .await
        .expect_err("a host that never comes back must fail");
    assert_eq!(
        classify_exec_failure(&err),
        HarnessExecFailure::Transport,
        "an exhausted retry must still read as a transport failure, got: {err}",
    );
    proxy.restore();
}

//! Shared behavioural conformance suite for `ExecutionPort` (C2.1,
//! `docs/EXECUTION_CONSISTENCY_PLAN.md`).
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
        .list_dir(machine_id, base)
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
    // (`docs/EXECUTION_CONSISTENCY_PLAN.md`, leak #1). The 13s silence clears
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
#[cfg(feature = "ssh-conformance")]
#[tokio::test]
async fn ssh_client_adapter_satisfies_the_contract() {
    use crate::adapters::ssh::client::SshClientAdapter;
    use crate::domain::ids::{AgentProfileId, MachineId};
    use crate::domain::models::{AgentProfile, Machine};
    use crate::ports::db::MachineRepository;

    // Minimal single-machine repo pointing the adapter at the container.
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

    let host =
        std::env::var("DEMETEO_SSH_CONFORMANCE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port_num: i32 = std::env::var("DEMETEO_SSH_CONFORMANCE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(2222);
    let user =
        std::env::var("DEMETEO_SSH_CONFORMANCE_USER").unwrap_or_else(|_| "demeteo".to_string());
    let password = std::env::var("DEMETEO_SSH_CONFORMANCE_PASSWORD")
        .expect("DEMETEO_SSH_CONFORMANCE_PASSWORD must be set for the ssh-conformance test");
    let workdir = std::env::var("DEMETEO_SSH_CONFORMANCE_WORKDIR")
        .unwrap_or_else(|_| format!("/home/{user}/conformance"));

    let machine_id = "ssh-conformance";
    let machine = Machine {
        id: MachineId(machine_id.to_string()),
        name: "ssh-conformance".to_string(),
        host,
        port: port_num,
        username: user,
        auth_type: "password".to_string(),
        key_path: None,
        agents: None,
        auto_approved_rules: None,
        use_login_shell: Some(false),
        setup_commands: None,
        notify_webhook_url: None,
    };
    // Seed the in-process credential cache so the adapter's password lookup
    // never reaches the OS keyring (which the test build may lack, and which
    // has no secret for this throwaway host anyway).
    crate::credential_cache::set(&format!("machine_{machine_id}"), &password);

    let port: Arc<dyn ExecutionPort> =
        Arc::new(SshClientAdapter::new(Arc::new(OneMachine(machine))));

    // Ensure the workdir exists, mirroring `fresh_local_workdir` for the local
    // leg — the shared `exec_contract` assumes a pre-existing writable dir.
    port.run_command(machine_id, &format!("mkdir -p {workdir}"))
        .await
        .expect("failed to create the remote conformance workdir");

    exec_contract(port, machine_id, &workdir).await;
}

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
//! * login-shell env resolution (D2 — the caller's env crosses the boundary).
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

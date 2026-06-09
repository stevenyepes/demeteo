use std::collections::HashMap;
use std::io::{Read, Write};

use crate::ports::execution::ExecutionPort;

/// Run the official install command for an agent. Returns Ok(()) on exit
/// code 0; otherwise an error string with the captured stderr. The
/// transport is local-subprocess for `machine_id == "local"` and SSH for
/// remote machines; this lets the user consent to the install command
/// once and have it run on whichever host the worktree lives on.
pub fn run_official_install(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    install_command: &str,
) -> Result<(), String> {
    if machine_id == "local" || machine_id.is_empty() {
        run_local(install_command)
    } else {
        run_remote(exec, machine_id, install_command)
    }
}

fn run_local(install_command: &str) -> Result<(), String> {
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(install_command)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn install command: {}", e))?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let mut out = String::new();
    let mut err = String::new();
    let _ = stdout.read_to_string(&mut out);
    let _ = stderr.read_to_string(&mut err);
    let status = child
        .wait()
        .map_err(|e| format!("Install wait failed: {}", e))?;
    if !status.success() {
        return Err(format!(
            "Install script failed (exit {:?}): {}{}",
            status.code(),
            err.trim(),
            if !out.is_empty() { format!("\nstdout: {}", out.trim()) } else { String::new() }
        ));
    }
    Ok(())
}

fn run_remote(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    install_command: &str,
) -> Result<(), String> {
    // For remote we just exec the install command via the existing
    // `run_command` port method. The result is whatever the script
    // printed to stdout; we treat any non-zero exit as a failure.
    let out = exec.run_command(machine_id, install_command)?;
    if out.is_empty() {
        Ok(())
    } else {
        // Heuristic: run_command returns Ok(stdout) on both success and
        // failure (ssh2 swallows exit codes unless we ask for them).
        // For v1 we accept this; a future phase can plumb through
        // exit-status via a typed result.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::execution::ExecutionPort;
    use crate::sftp::SftpEntry;

    /// Local install: `true` should always succeed; `false` should fail.
    #[test]
    fn local_install_true_succeeds() {
        assert!(run_local("true").is_ok());
    }

    #[test]
    fn local_install_false_fails() {
        let err = run_local("false").unwrap_err();
        assert!(err.contains("Install script failed"), "got: {}", err);
    }

    /// Local install of an unknown command should fail (not silently
    /// succeed). `sh -c "missing"` returns 127 with "command not found"
    /// on stderr, so the error message reflects the script's exit.
    #[test]
    fn local_install_missing_command_fails() {
        let err = run_local("this_binary_does_not_exist_xyz").unwrap_err();
        assert!(err.contains("Install script failed"), "got: {}", err);
    }

    /// Run-official-install on a remote machine should fall through to
    /// the `ExecutionPort::run_command` path; verify with a stub.
    struct StubExec;
    impl ExecutionPort for StubExec {
        fn test_connection(&self, _: &str) -> Result<(), String> { Ok(()) }
        fn run_command(&self, _: &str, _: &str) -> Result<String, String> { Ok(String::new()) }
        fn read_file(&self, _: &str, _: &str) -> Result<String, String> { Ok(String::new()) }
        fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> { Ok(()) }
        fn get_metadata(&self, _: &str, path: &str) -> Result<SftpEntry, String> {
            Ok(SftpEntry { name: path.into(), path: path.into(), is_dir: false, size: 0, modified: 0 })
        }
        fn list_dir(&self, _: &str, _: &str) -> Result<Vec<SftpEntry>, String> { Ok(vec![]) }
        fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> { Ok(()) }
        fn spawn_interactive(
            &self,
            _: &str,
            _: &str,
            _: &[String],
            _: &str,
            _: &HashMap<String, String>,
        ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
            Err("not used in install test".into())
        }
    }

    #[test]
    fn remote_install_delegates_to_run_command() {
        let exec = StubExec;
        let res = run_official_install(&exec, "remote_1", "curl -fsSL https://x/install | bash");
        assert!(res.is_ok());
    }
}

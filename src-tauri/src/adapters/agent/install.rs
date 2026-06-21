use std::io::Read;

use crate::ports::execution::ExecutionPort;
use crate::sftp::SftpEntry;

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
            if !out.is_empty() {
                format!("\nstdout: {}", out.trim())
            } else {
                String::new()
            }
        ));
    }
    Ok(())
}

fn run_remote(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    install_command: &str,
) -> Result<(), String> {
    exec.run_command(machine_id, install_command)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::execution::InteractiveHandle;

    struct StubExec;
    impl ExecutionPort for StubExec {
        fn test_connection(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn run_command(&self, _: &str, _: &str) -> Result<String, String> {
            Ok(String::new())
        }
        fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
            Ok(String::new())
        }
        fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn get_metadata(&self, _: &str, path: &str) -> Result<SftpEntry, String> {
            Ok(SftpEntry {
                name: path.into(),
                path: path.into(),
                is_dir: false,
                size: 0,
                modified: 0,
            })
        }
        fn list_dir(&self, _: &str, _: &str) -> Result<Vec<SftpEntry>, String> {
            Ok(vec![])
        }
        fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn resolve_home(&self, _: &str) -> Result<String, String> {
            Ok("/tmp".to_string())
        }
        fn spawn_interactive(
            &self,
            _: &str,
            _: &str,
            _: &[String],
            _: &str,
            _: &std::collections::HashMap<String, String>,
        ) -> Result<Box<dyn InteractiveHandle>, String> {
            Err("stub".to_string())
        }
    }

    #[test]
    fn local_install_true_succeeds() {
        assert!(run_local("true").is_ok());
    }

    #[test]
    fn local_install_false_fails() {
        let err = run_local("false").unwrap_err();
        assert!(err.contains("Install script failed"), "got: {}", err);
    }

    #[test]
    fn local_install_missing_command_fails() {
        let err = run_local("this_binary_does_not_exist_xyz").unwrap_err();
        assert!(err.contains("Install script failed"), "got: {}", err);
    }

    #[test]
    fn remote_install_delegates_to_run_command() {
        let exec = StubExec;
        let res = run_official_install(&exec, "remote_1", "curl -fsSL https://x/install | bash");
        assert!(res.is_ok());
    }
}

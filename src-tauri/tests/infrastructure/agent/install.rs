use super::*;
use crate::ports::execution::InteractiveHandle;
use crate::sftp::SftpEntry;

struct StubExec;
#[async_trait::async_trait]
impl ExecutionPort for StubExec {
    async fn test_connection(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn run_command(&self, _: &str, _: &str) -> Result<String, String> {
        Ok(String::new())
    }
    async fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
        Ok(String::new())
    }
    async fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn get_metadata(&self, _: &str, path: &str) -> Result<SftpEntry, String> {
        Ok(SftpEntry {
            name: path.into(),
            path: path.into(),
            is_dir: false,
            size: 0,
            modified: 0,
        })
    }
    async fn list_dir(&self, _: &str, _: &str) -> Result<Vec<SftpEntry>, String> {
        Ok(vec![])
    }
    async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn resolve_home(&self, _: &str) -> Result<String, String> {
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

#[tokio::test]
async fn remote_install_delegates_to_run_command() {
    let exec = StubExec;
    let res = run_official_install(&exec, "remote_1", "curl -fsSL https://x/install | bash").await;
    assert!(res.is_ok());
}

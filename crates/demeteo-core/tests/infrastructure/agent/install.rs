use super::*;
use crate::ports::execution::InteractiveHandle;
use crate::ports::execution::SftpEntry;
use crate::ports::execution::ShellOptions;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct StubExec {
    last_opts: Arc<Mutex<Option<ShellOptions>>>,
}
#[async_trait::async_trait]
impl ExecutionPort for StubExec {
    async fn test_connection(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn run_command(&self, _: &str, _: &str) -> Result<String, String> {
        Ok(String::new())
    }
    async fn run_command_with(
        &self,
        _: &str,
        _: &str,
        opts: ShellOptions,
    ) -> Result<String, String> {
        *self.last_opts.lock().unwrap() = Some(opts);
        Ok(String::new())
    }
    async fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
        Ok(String::new())
    }
    async fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn write_file_bytes(&self, _: &str, _: &str, _: &[u8]) -> Result<(), String> {
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
    async fn resolve_user(&self, _: &str) -> Result<String, String> {
        Ok("test".to_string())
    }
    async fn control_rpc(
        &self,
        _: &str,
        _: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err("control_rpc not supported by this stub".to_string())
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
async fn remote_install_runs_under_interactive_login_shell() {
    // A remote agent install must resolve the user's package managers
    // (npm via nvm, mise, asdf, brew), which only an interactive login shell
    // puts on PATH — regression guard for the C1.3 explicit-context migration.
    let exec = StubExec::default();
    let res = run_official_install(&exec, "remote_1", "curl -fsSL https://x/install | bash").await;
    assert!(res.is_ok());
    let opts = exec
        .last_opts
        .lock()
        .unwrap()
        .clone()
        .expect("opts recorded");
    assert!(opts.login_shell, "remote install must use a login shell");
    assert!(opts.interactive, "remote install must be interactive");
}

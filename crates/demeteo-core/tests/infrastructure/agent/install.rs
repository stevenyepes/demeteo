use super::*;
use crate::ports::execution::InteractiveHandle;
use crate::ports::execution::SftpEntry;
use crate::ports::execution::ShellOptions;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct StubExec {
    last_opts: Arc<Mutex<Option<ShellOptions>>>,
    last_call: Arc<Mutex<Option<(String, String)>>>,
    /// What `run_command_with` answers. `None` is the success case; `Some` is
    /// the transport's verbatim failure, which is the only thing the installer
    /// has to tell the human who pressed the button.
    fails_with: Option<String>,
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
        machine_id: &str,
        cmd: &str,
        opts: ShellOptions,
    ) -> Result<String, String> {
        *self.last_opts.lock().unwrap() = Some(opts);
        *self.last_call.lock().unwrap() = Some((machine_id.to_string(), cmd.to_string()));
        match &self.fails_with {
            Some(err) => Err(err.clone()),
            None => Ok(String::new()),
        }
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

async fn install_on(machine_id: &str) -> (StubExec, Result<(), String>) {
    let exec = StubExec::default();
    let res = run_official_install(&exec, machine_id, "curl -fsSL https://x/install | bash").await;
    (exec, res)
}

fn recorded(exec: &StubExec) -> (String, String, ShellOptions) {
    let (machine_id, cmd) = exec
        .last_call
        .lock()
        .unwrap()
        .clone()
        .expect("call recorded");
    let opts = exec
        .last_opts
        .lock()
        .unwrap()
        .clone()
        .expect("opts recorded");
    (machine_id, cmd, opts)
}

#[tokio::test]
async fn every_install_runs_under_an_interactive_login_shell() {
    // An install must resolve the user's package managers (npm via nvm, mise,
    // asdf, brew), which only an interactive login shell puts on PATH, and it
    // must be the *same* shell the availability probe uses — otherwise the
    // install succeeds and the probe that follows it reports the agent
    // missing. Regression guard for the C1.3 explicit-context migration.
    for machine_id in ["local", "", "remote_1"] {
        let (exec, res) = install_on(machine_id).await;
        assert!(res.is_ok(), "{}: {:?}", machine_id, res);
        let (routed_to, _, opts) = recorded(&exec);
        assert_eq!(routed_to, machine_id, "the router picks the transport");
        assert!(opts.login_shell, "{}: must use a login shell", machine_id);
        assert!(opts.interactive, "{}: must be interactive", machine_id);
    }
}

#[tokio::test]
async fn the_local_install_goes_through_the_port_like_every_other_one() {
    // It used to spawn `sh` by name, which names nothing on Windows: the Git
    // installation's `sh.exe` is not on PATH, so local installation there
    // could not run at all. Routing it through the port is what makes the
    // shell the transport's problem rather than this function's.
    let (exec, _) = install_on("local").await;
    let (machine_id, cmd, _) = recorded(&exec);
    assert_eq!(machine_id, "local");
    assert_eq!(cmd, "curl -fsSL https://x/install | bash");
}

#[tokio::test]
async fn a_failed_install_reports_what_the_installer_said_and_what_was_run() {
    // A human pressed a button and is waiting on this string.
    let exec = StubExec {
        fails_with: Some("Command failed (exit code: Some(1)): npm ERR! EACCES".to_string()),
        ..StubExec::default()
    };
    let err = run_official_install(&exec, "local", "npm i -g opencode-ai")
        .await
        .unwrap_err();
    assert!(err.contains("Install script failed"), "got: {}", err);
    assert!(err.contains("npm ERR! EACCES"), "got: {}", err);
    assert!(err.contains("npm i -g opencode-ai"), "got: {}", err);
}

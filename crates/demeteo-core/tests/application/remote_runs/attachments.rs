use super::{attachment_spool_dir, spool_attachments, MAX_DETACHED_ATTACHMENT_BYTES};

#[test]
fn attachment_spool_dir_returns_xdg_under_home() {
    assert_eq!(
        attachment_spool_dir("/home/alice", "laptop-1"),
        "/home/alice/.local/share/demeteo-runner/attachment-spool/laptop-1"
    );
    assert_eq!(MAX_DETACHED_ATTACHMENT_BYTES, 25 * 1024 * 1024);
}

#[test]
fn attachment_spool_dir_uses_run_id() {
    let first = attachment_spool_dir("/home/alice", "laptop-1");
    let second = attachment_spool_dir("/home/alice", "laptop-2");
    assert!(first.ends_with("/laptop-1"));
    assert!(second.ends_with("/laptop-2"));
    assert_ne!(first, second);
}

mod spool {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use base64::Engine;
    use serde_json::Value;

    use super::{attachment_spool_dir, spool_attachments, MAX_DETACHED_ATTACHMENT_BYTES};
    use crate::adapters::notification_noop::NoopNotificationAdapter;
    use crate::application::attachments::{
        resolve_agent_attachments, AgentAttachment, MAX_ATTACHMENT_BYTES,
    };
    use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
    use crate::domain::models::Platform;
    use crate::ports::execution::{ExecutionPort, InteractiveHandle, SftpEntry};
    use crate::state::AppContext;

    const MACHINE: &str = "m-1";
    const HOME: &str = "/home/runner";
    const RUN_ID: &str = "run-7";
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-not-a-real-image";

    /// Answers `resolve_home`, the spool's `mkdir -p`, and `write_file_bytes`;
    /// every other call is `Err`. It records what it was asked, so a test sees
    /// the bytes that reached the runner and the commands that did not.
    #[derive(Default)]
    struct SpoolExec {
        calls: Mutex<Vec<String>>,
        writes: Mutex<Vec<(String, Vec<u8>)>>,
    }

    impl SpoolExec {
        fn record(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn writes(&self) -> Vec<(String, Vec<u8>)> {
            self.writes.lock().unwrap().clone()
        }

        fn unscripted<T>(&self, call: &str) -> Result<T, String> {
            self.record(call.to_string());
            Err(format!("unscripted {call}"))
        }
    }

    #[async_trait]
    impl ExecutionPort for SpoolExec {
        async fn test_connection(&self, _machine_id: &str) -> Result<(), String> {
            self.unscripted("test_connection")
        }
        async fn run_command(&self, _machine_id: &str, cmd: &str) -> Result<String, String> {
            self.record(format!("run_command {cmd}"));
            let spool = attachment_spool_dir(HOME, RUN_ID);
            if cmd == format!("mkdir -p {spool}") {
                Ok(String::new())
            } else {
                Err(format!("unscripted command {cmd}"))
            }
        }
        async fn read_file(&self, _machine_id: &str, _path: &str) -> Result<String, String> {
            self.unscripted("read_file")
        }
        async fn write_file(
            &self,
            _machine_id: &str,
            _path: &str,
            _content: &str,
        ) -> Result<(), String> {
            self.unscripted("write_file")
        }
        async fn write_file_bytes(
            &self,
            _machine_id: &str,
            path: &str,
            content: &[u8],
        ) -> Result<(), String> {
            self.record(format!("write_file_bytes {path}"));
            self.writes
                .lock()
                .unwrap()
                .push((path.to_string(), content.to_vec()));
            Ok(())
        }
        async fn get_metadata(&self, _machine_id: &str, _path: &str) -> Result<SftpEntry, String> {
            self.unscripted("get_metadata")
        }
        async fn list_dir(&self, _machine_id: &str, _path: &str) -> Result<Vec<SftpEntry>, String> {
            self.unscripted("list_dir")
        }
        async fn setup_worktree(
            &self,
            _machine_id: &str,
            _repo_path: &str,
            _branch: &str,
            _sandbox_path: &str,
        ) -> Result<(), String> {
            self.unscripted("setup_worktree")
        }
        async fn resolve_home(&self, _machine_id: &str) -> Result<String, String> {
            self.record("resolve_home".to_string());
            Ok(HOME.to_string())
        }
        async fn resolve_platform(&self, _machine_id: &str) -> Result<Platform, String> {
            self.unscripted("resolve_platform")
        }
        async fn resolve_user(&self, _machine_id: &str) -> Result<String, String> {
            self.unscripted("resolve_user")
        }
        async fn control_rpc(
            &self,
            _machine_id: &str,
            method: &str,
            _params: Value,
        ) -> Result<Value, String> {
            self.unscripted(&format!("rpc {method}"))
        }
        fn spawn_interactive(
            &self,
            _machine_id: &str,
            _binary: &str,
            _args: &[String],
            _cwd: &str,
            _env: &HashMap<String, String>,
        ) -> Result<Box<dyn InteractiveHandle>, String> {
            self.unscripted("spawn_interactive")
        }
    }

    fn ctx_with(exec: Arc<SpoolExec>) -> (AppContext, crate::support::test_dir::TestDir) {
        let dir = crate::support::test_dir::TestDir::new("demeteo-spool-attachments");
        let mut ctx = build_core_context(
            CoreConfig {
                app_data_dir: dir.path().to_path_buf(),
                execution_mode: ExecutionMode::LocalOnly,
            },
            Arc::new(NoopNotificationAdapter),
            tokio::runtime::Handle::current(),
        );
        ctx.exec = exec;
        (ctx, dir)
    }

    fn inline(bytes: &[u8], filename: &str) -> AgentAttachment {
        AgentAttachment {
            path: None,
            content_base64: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
            mime: None,
            filename: Some(filename.to_string()),
        }
    }

    #[tokio::test]
    async fn an_mcp_inline_attachment_reaches_the_runner_as_it_was_sent() {
        let staged = resolve_agent_attachments(vec![inline(PNG, "mock.png")], &[]).unwrap();
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].source_path, "", "an inline item has no path");
        assert!(staged[0].bytes.is_some());

        let exec = Arc::new(SpoolExec::default());
        let (ctx, _dir) = ctx_with(exec.clone());
        let out = spool_attachments(&ctx, MACHINE, RUN_ID, staged)
            .await
            .expect("spools");

        let staged_path = format!("{}/0-mock.png", attachment_spool_dir(HOME, RUN_ID));
        assert_eq!(exec.writes(), [(staged_path.clone(), PNG.to_vec())]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].staged_path, staged_path);
        assert_eq!(out[0].mime.as_deref(), Some("image/png"));
        assert_eq!(out[0].source_filename.as_deref(), Some("mock.png"));
        assert_eq!(
            exec.calls(),
            [
                "resolve_home".to_string(),
                format!(
                    "run_command mkdir -p {}",
                    attachment_spool_dir(HOME, RUN_ID)
                ),
                format!("write_file_bytes {staged_path}"),
            ],
            "nothing but home, mkdir and the write"
        );
    }

    #[tokio::test]
    async fn an_mcp_path_attachment_reaches_the_runner_from_the_bytes_read_at_resolve_time() {
        let dir = crate::support::test_dir::TestDir::new("demeteo-spool-path-src");
        let src = dir.path().join("shot.png");
        std::fs::write(&src, PNG).unwrap();
        let item = AgentAttachment {
            path: Some(src.to_string_lossy().into_owned()),
            content_base64: None,
            mime: None,
            filename: None,
        };
        let staged = resolve_agent_attachments(vec![item], &[]).unwrap();
        // The file is gone before the spool runs: only the staged bytes can arrive.
        std::fs::remove_file(&src).unwrap();

        let exec = Arc::new(SpoolExec::default());
        let (ctx, _ctx_dir) = ctx_with(exec.clone());
        let out = spool_attachments(&ctx, MACHINE, RUN_ID, staged)
            .await
            .expect("spools");

        let staged_path = format!("{}/0-shot.png", attachment_spool_dir(HOME, RUN_ID));
        assert_eq!(exec.writes(), [(staged_path.clone(), PNG.to_vec())]);
        assert_eq!(out[0].staged_path, staged_path);
        assert_eq!(out[0].mime.as_deref(), Some("image/png"));
        assert_eq!(out[0].source_filename.as_deref(), Some("shot.png"));
    }

    #[tokio::test]
    async fn an_inline_attachment_over_the_detached_cap_is_refused_before_anything_is_written() {
        const _: () = assert!(MAX_DETACHED_ATTACHMENT_BYTES as u64 <= MAX_ATTACHMENT_BYTES);
        let mut big = PNG.to_vec();
        big.resize(MAX_DETACHED_ATTACHMENT_BYTES + 1024 * 1024, 0);
        let staged = resolve_agent_attachments(vec![inline(&big, "big.png")], &[])
            .expect("under the 100 MiB local cap, so the resolver accepts it");

        let exec = Arc::new(SpoolExec::default());
        let (ctx, _dir) = ctx_with(exec.clone());
        let err = spool_attachments(&ctx, MACHINE, RUN_ID, staged)
            .await
            .unwrap_err();

        assert!(
            err.contains("detached runs cap attachments at 25 MB"),
            "{err}"
        );
        assert!(exec.writes().is_empty());
    }
}

// Tests extracted from `crates/demeteo-core/src/application/projects.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::ports::execution::{InteractiveHandle, SftpEntry};
use async_trait::async_trait;

/// Configurable `ExecutionPort` double: `test_connection` succeeds only for
/// machine ids listed in `reachable`. Every other method is an unused
/// no-op stub — `check_liveness`/`liveness_result` only ever calls
/// `test_connection`.
struct FakeExec {
    reachable: Vec<&'static str>,
}

#[async_trait]
impl ExecutionPort for FakeExec {
    async fn test_connection(&self, machine_id: &str) -> Result<(), String> {
        if self.reachable.contains(&machine_id) {
            Ok(())
        } else {
            Err(format!("transport: unreachable: {}", machine_id))
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
        Err("control_rpc not supported by this fake".to_string())
    }
    fn spawn_interactive(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: &str,
        _: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        Err("spawn_interactive not supported by this fake".to_string())
    }
}

#[tokio::test]
async fn local_machine_is_always_online() {
    let exec = FakeExec {
        reachable: vec!["local"],
    };
    let result = liveness_result(&exec, "p1".to_string(), "local").await;
    assert_eq!(result.project_id, "p1");
    assert_eq!(result.liveness, "online");
    assert!(!result.checked_at.is_empty());
}

#[tokio::test]
async fn remote_machine_reports_online_when_reachable() {
    let exec = FakeExec {
        reachable: vec!["box1"],
    };
    let result = liveness_result(&exec, "p2".to_string(), "box1").await;
    assert_eq!(result.liveness, "online");
}

#[tokio::test]
async fn remote_machine_reports_offline_when_unreachable() {
    let exec = FakeExec { reachable: vec![] };
    let result = liveness_result(&exec, "p3".to_string(), "box1").await;
    assert_eq!(result.liveness, "offline");
}

#[test]
fn iso8601_now_has_the_expected_shape() {
    let ts = iso8601_now();
    // "YYYY-MM-DDTHH:MM:SSZ"
    assert_eq!(ts.len(), 20);
    assert!(ts.ends_with('Z'));
    assert_eq!(ts.as_bytes()[4], b'-');
    assert_eq!(ts.as_bytes()[7], b'-');
    assert_eq!(ts.as_bytes()[10], b'T');
    assert_eq!(ts.as_bytes()[13], b':');
    assert_eq!(ts.as_bytes()[16], b':');
}

#[test]
fn civil_from_unix_days_matches_known_dates() {
    // 1970-01-01 is day 0.
    assert_eq!(civil_from_unix_days(0), (1970, 1, 1));
    // 2000-01-01 is day 10957.
    assert_eq!(civil_from_unix_days(10_957), (2000, 1, 1));
    // 2024-02-29 (leap day) is day 19782.
    assert_eq!(civil_from_unix_days(19_782), (2024, 2, 29));
}

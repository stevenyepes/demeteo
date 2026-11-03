use super::*;

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::adapters::database::SqliteAdapter;
use crate::domain::ids::MachineId;
use crate::domain::models::{Machine, Platform};
use crate::ports::db::MachineRepository;
use crate::ports::execution::{InteractiveHandle, SftpEntry};

const MACHINE: &str = "m-1";
const HUB: &str = "https://hub.example.com";

/// Answers the one `control_rpc` it was scripted with, for [`MACHINE`];
/// every other call is an `Err`, and every call is recorded.
struct OneRpc {
    method: &'static str,
    answer: Result<Value, String>,
    calls: Mutex<Vec<String>>,
}

impl OneRpc {
    fn answering(method: &'static str, answer: Result<Value, &str>) -> Self {
        Self {
            method,
            answer: answer.map_err(str::to_owned),
            calls: Mutex::default(),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn unscripted<T>(&self, call: &str) -> Result<T, String> {
        self.calls.lock().unwrap().push(call.to_owned());
        Err(format!("unscripted {call}"))
    }
}

#[async_trait]
impl ExecutionPort for OneRpc {
    async fn test_connection(&self, _machine_id: &str) -> Result<(), String> {
        self.unscripted("test_connection")
    }
    async fn read_file(&self, _machine_id: &str, _path: &str) -> Result<String, String> {
        self.unscripted("read_file")
    }
    async fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        self.unscripted("write_file")
    }
    async fn write_file_bytes(&self, _: &str, _: &str, _: &[u8]) -> Result<(), String> {
        self.unscripted("write_file_bytes")
    }
    async fn get_metadata(&self, _: &str, _: &str) -> Result<SftpEntry, String> {
        self.unscripted("get_metadata")
    }
    async fn list_dir(&self, _: &str, _: &str) -> Result<Vec<SftpEntry>, String> {
        self.unscripted("list_dir")
    }
    async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
        self.unscripted("setup_worktree")
    }
    async fn resolve_home(&self, _: &str) -> Result<String, String> {
        self.unscripted("resolve_home")
    }
    async fn resolve_platform(&self, _: &str) -> Result<Platform, String> {
        self.unscripted("resolve_platform")
    }
    async fn resolve_user(&self, _: &str) -> Result<String, String> {
        self.unscripted("resolve_user")
    }
    async fn control_rpc(
        &self,
        machine_id: &str,
        method: &str,
        _params: Value,
    ) -> Result<Value, String> {
        if machine_id != MACHINE || method != self.method {
            return self.unscripted(&format!("rpc {method} on {machine_id}"));
        }
        self.calls.lock().unwrap().push(format!("rpc {method}"));
        self.answer.clone()
    }
    fn spawn_interactive(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: &str,
        _: &HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        self.unscripted("spawn_interactive")
    }
}

fn db_with_machine() -> SqliteAdapter {
    let db = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    MachineRepository::add(
        &db,
        Machine {
            id: MachineId::from(MACHINE),
            name: "runner".to_owned(),
            host: "runner.invalid".to_owned(),
            port: 22,
            username: "run".to_owned(),
            auth_type: "agent".to_owned(),
            key_path: None,
            agents: None,
            auto_approved_rules: None,
            use_login_shell: None,
            setup_commands: None,
            notify_webhook_url: None,
        },
    )
    .unwrap();
    db
}

fn known(paired: bool) -> RunnerHubState {
    RunnerHubState {
        paired,
        hub_url: Some(HUB.to_owned()),
        instance_id: Some("inst-old".to_owned()),
        checked_at: 1,
    }
}

async fn refreshed(exec: &OneRpc, db: &SqliteAdapter) -> Option<RunnerHubState> {
    refresh_runner_hub_state_via(exec, db, db, MACHINE)
        .await
        .expect("the row is writable")
}

#[tokio::test]
async fn a_paired_runner_is_stored_as_its_hub_status_says() {
    let db = db_with_machine();
    let exec = OneRpc::answering(
        "hub_status",
        Ok(json!({
            "status": { "state": "paired", "hub_url": HUB, "instance_id": "inst-1" },
            "store_unavailable": null,
        })),
    );

    let state = refreshed(&exec, &db).await.expect("an answer");

    assert!(state.paired);
    assert_eq!(state.hub_url.as_deref(), Some(HUB));
    assert_eq!(state.instance_id.as_deref(), Some("inst-1"));
    assert_eq!(
        MachineHubStateRepository::get(&db, MACHINE).unwrap(),
        Some(state)
    );
    assert_eq!(exec.calls(), ["rpc hub_status"]);
}

#[tokio::test]
async fn a_runner_without_the_hub_methods_is_stored_unpaired() {
    let db = db_with_machine();
    MachineHubStateRepository::put(&db, MACHINE, &known(true)).unwrap();
    let exec = OneRpc::answering("hub_status", Err("unknown method: hub_status"));

    let state = refreshed(&exec, &db).await.expect("an answer");

    assert!(!state.paired);
    assert_eq!(state.hub_url, None);
    assert_eq!(
        MachineHubStateRepository::get(&db, MACHINE).unwrap(),
        Some(state)
    );
}

#[tokio::test]
async fn an_unreachable_runner_leaves_its_last_known_state() {
    let db = db_with_machine();
    MachineHubStateRepository::put(&db, MACHINE, &known(true)).unwrap();
    let exec = OneRpc::answering("hub_status", Err("Timed out waiting on socket"));

    assert_eq!(refreshed(&exec, &db).await, None);

    assert_eq!(
        MachineHubStateRepository::get(&db, MACHINE).unwrap(),
        Some(known(true))
    );
    assert_eq!(exec.calls(), ["rpc hub_status"]);
}

#[test]
fn a_stored_state_round_trips_is_replaced_and_goes_with_its_machine() {
    let db = db_with_machine();
    assert_eq!(MachineHubStateRepository::get(&db, MACHINE).unwrap(), None);

    MachineHubStateRepository::put(&db, MACHINE, &known(true)).unwrap();
    let replaced = RunnerHubState {
        paired: false,
        hub_url: None,
        instance_id: None,
        checked_at: 2,
    };
    MachineHubStateRepository::put(&db, MACHINE, &replaced).unwrap();
    assert_eq!(
        MachineHubStateRepository::get(&db, MACHINE).unwrap(),
        Some(replaced.clone())
    );
    assert_eq!(
        MachineHubStateRepository::all(&db).unwrap(),
        HashMap::from([(MACHINE.to_owned(), replaced)])
    );

    MachineRepository::delete(&db, &MachineId::from(MACHINE)).unwrap();
    assert!(MachineHubStateRepository::all(&db).unwrap().is_empty());
}

#[test]
fn a_state_for_no_machine_is_refused() {
    let db = db_with_machine();
    assert!(MachineHubStateRepository::put(&db, "m-gone", &known(true)).is_err());
}

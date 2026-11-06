use super::*;

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use rusqlite::Connection;

use crate::adapters::database::SqliteAdapter;
use crate::domain::hub_pairing::HubPairingState;
use crate::domain::hub_request::log::{HubRequestLogEntry, LogKind, LogOutcome};
use crate::domain::hub_socket::RunnerHubState;
use crate::domain::ids::MachineId;
use crate::domain::models::{Machine, Platform};
use crate::ports::db::MachineRepository;
use crate::ports::execution::{InteractiveHandle, SftpEntry};

const MACHINE: &str = "m-1";

/// Answers a control RPC only when its method *and* params (less the stamped
/// `client_id`) are the scripted ones; every other call of any kind is `Err`.
#[derive(Default)]
struct ScriptedRpc {
    answers: Vec<(String, Value, Result<Value, String>)>,
    calls: Mutex<Vec<(String, String, Value)>>,
}

impl ScriptedRpc {
    fn answering(mut self, method: &str, params: Value, answer: Result<Value, &str>) -> Self {
        self.answers
            .push((method.to_string(), params, answer.map_err(str::to_string)));
        self
    }

    fn calls(&self) -> Vec<(String, String, Value)> {
        self.calls.lock().unwrap().clone()
    }

    fn unscripted<T>(&self, call: &str) -> Result<T, String> {
        Err(format!("unscripted {call}"))
    }
}

#[async_trait]
impl ExecutionPort for ScriptedRpc {
    async fn test_connection(&self, _machine_id: &str) -> Result<(), String> {
        self.unscripted("test_connection")
    }
    async fn run_command(&self, _machine_id: &str, _cmd: &str) -> Result<String, String> {
        self.unscripted("run_command")
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
        _path: &str,
        _content: &[u8],
    ) -> Result<(), String> {
        self.unscripted("write_file_bytes")
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
        self.unscripted("resolve_home")
    }
    async fn resolve_platform(&self, _machine_id: &str) -> Result<Platform, String> {
        self.unscripted("resolve_platform")
    }
    async fn resolve_user(&self, _machine_id: &str) -> Result<String, String> {
        self.unscripted("resolve_user")
    }
    async fn control_rpc(
        &self,
        machine_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        self.calls.lock().unwrap().push((
            machine_id.to_string(),
            method.to_string(),
            params.clone(),
        ));
        let mut asked = params;
        if let Some(obj) = asked.as_object_mut() {
            if obj.remove("client_id").is_none() {
                return Err(format!("{method} was sent without a client_id"));
            }
        }
        if machine_id != MACHINE {
            return Err(format!("unscripted machine {machine_id}"));
        }
        self.answers
            .iter()
            .find(|(m, p, _)| m == method && *p == asked)
            .map(|(_, _, answer)| answer.clone())
            .unwrap_or_else(|| Err(format!("unscripted rpc {method} {asked}")))
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

fn settings() -> SqliteAdapter {
    SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap()
}

/// [`settings`] holding [`MACHINE`], whose Hub state can therefore be stored.
fn db_with_machine() -> SqliteAdapter {
    let db = settings();
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

fn stored_paired(db: &SqliteAdapter) {
    let state = RunnerHubState {
        paired: true,
        hub_url: Some("https://hub.example".into()),
        instance_id: Some("inst-old".into()),
        checked_at: 1,
    };
    MachineHubStateRepository::put(db, MACHINE, &state).unwrap();
}

fn status(state: &str, instance_id: Option<&str>) -> Value {
    json!({
        "state": state,
        "hub_url": instance_id.map(|_| "https://hub.example"),
        "instance_id": instance_id,
        "fingerprint": null,
        "rp_id": null,
        "scopes": [],
        "paired_at": null,
        "keyring_backend": "sealed",
        "keyring_available": true,
    })
}

fn hub_status_answer(state: &str, instance_id: Option<&str>) -> Value {
    json!({ "status": status(state, instance_id), "store_unavailable": null })
}

fn methods(exec: &ScriptedRpc) -> Vec<String> {
    exec.calls()
        .into_iter()
        .map(|(_, method, _)| method)
        .collect()
}

fn ticket() -> DeviceTicket {
    DeviceTicket {
        user_code: "ABCD-EFGH".into(),
        verification_uri: "https://hub.example/device".into(),
        verification_uri_complete: "https://hub.example/device?user_code=ABCD-EFGH".into(),
        fingerprint: "SHA256:aaaa bbbb cccc dddd".into(),
        expires_at: 1_700_000_000,
    }
}

fn policy() -> HubLocalPolicy {
    HubLocalPolicy {
        read_enabled: true,
        spend_enabled: false,
        gates_enabled: true,
        run_ceiling_cents: Some(500),
        daily_ceiling_cents: None,
    }
}

#[tokio::test]
async fn pairing_begins_with_the_hub_url_and_reads_back_the_ticket() {
    let exec = ScriptedRpc::default().answering(
        "hub_pair_begin",
        json!({ "hub_url": "https://hub.example" }),
        Ok(serde_json::to_value(ticket()).unwrap()),
    );
    let settings = settings();
    let rpc = RunnerHubRpc {
        exec: &exec,
        settings: &settings,
        states: &settings,
    };

    let got = rpc
        .pair_begin(MACHINE, "https://hub.example")
        .await
        .expect("the runner answered a ticket");

    assert_eq!(got, ticket());
    assert_eq!(exec.calls().len(), 1);
}

#[tokio::test]
async fn a_pending_pair_state_carries_its_ticket() {
    let exec = ScriptedRpc::default().answering(
        "hub_pair_state",
        json!({}),
        Ok(json!({ "state": "pending", "ticket": ticket() })),
    );
    let settings = settings();
    let rpc = RunnerHubRpc {
        exec: &exec,
        settings: &settings,
        states: &settings,
    };

    assert_eq!(
        rpc.pair_state(MACHINE).await.expect("parsed"),
        PairStateView::Pending { ticket: ticket() }
    );
}

#[tokio::test]
async fn the_runners_own_refusal_reaches_the_caller_unchanged() {
    let refusal = "cannot pair: the sealed store is unavailable; run `demeteo-runner hub seal`, \
                   then restart the runner (`systemctl --user restart demeteo-runner`)";
    let exec = ScriptedRpc::default().answering(
        "hub_pair_begin",
        json!({ "hub_url": "https://hub.example" }),
        Err(refusal),
    );
    let settings = settings();
    let rpc = RunnerHubRpc {
        exec: &exec,
        settings: &settings,
        states: &settings,
    };

    assert_eq!(
        rpc.pair_begin(MACHINE, "https://hub.example").await,
        Err(refusal.to_string())
    );
}

#[tokio::test]
async fn a_status_answer_parses_into_the_view() {
    let answer = json!({
        "status": {
            "state": "paired",
            "hub_url": "https://hub.example",
            "instance_id": "inst-1",
            "fingerprint": "SHA256:aaaa",
            "rp_id": "hub.example",
            "scopes": ["read"],
            "paired_at": 1_700_000_000,
            "keyring_backend": "sealed",
            "keyring_available": true,
        },
        "store_unavailable": null,
        "connection": {
            "state": "connected",
            "stop_reason": null,
            "latency_ms": 12,
            "reported_through": 40,
            "sync": "in_sync",
            "last_heard_at_ms": 1_700_000_000_000_i64,
        },
        "machine": {
            "instance_name": "runner-1",
            "os": "linux",
            "arch": "x86_64",
            "app_version": "1.2.3",
        },
    });
    let exec = ScriptedRpc::default().answering("hub_status", json!({}), Ok(answer));
    let settings = settings();
    let rpc = RunnerHubRpc {
        exec: &exec,
        settings: &settings,
        states: &settings,
    };

    let view = rpc
        .status(MACHINE)
        .await
        .expect("parsed")
        .expect("the runner knows hub_status");

    assert_eq!(view.status.state, HubPairingState::Paired);
    assert_eq!(view.status.instance_id.as_deref(), Some("inst-1"));
    assert_eq!(view.connection.latency_ms, Some(12));
    assert_eq!(view.machine.instance_name, "runner-1");
}

#[tokio::test]
async fn a_runner_from_before_pairing_reads_as_no_status() {
    let exec = ScriptedRpc::default().answering(
        "hub_status",
        json!({}),
        Err("unknown method: hub_status"),
    );
    let settings = settings();
    let rpc = RunnerHubRpc {
        exec: &exec,
        settings: &settings,
        states: &settings,
    };

    assert_eq!(rpc.status(MACHINE).await, Ok(None));
}

#[tokio::test]
async fn a_runner_without_an_http_client_says_so_rather_than_reading_as_old() {
    let refusal = "Hub pairing is not available on this runner: it has no HTTP client for the Hub \
                   (see its log)";
    let exec = ScriptedRpc::default().answering("hub_status", json!({}), Err(refusal));
    let settings = settings();
    let rpc = RunnerHubRpc {
        exec: &exec,
        settings: &settings,
        states: &settings,
    };

    assert_eq!(rpc.status(MACHINE).await, Err(refusal.to_string()));
}

#[tokio::test]
async fn setting_the_policy_sends_all_of_it_and_answers_what_the_runner_stored() {
    let stored = HubLocalPolicy {
        run_ceiling_cents: Some(400),
        ..policy()
    };
    let exec = ScriptedRpc::default().answering(
        "hub_policy_set",
        json!({ "policy": {
            "read_enabled": true,
            "spend_enabled": false,
            "gates_enabled": true,
            "run_ceiling_cents": 500,
            "daily_ceiling_cents": null,
        }}),
        Ok(serde_json::to_value(&stored).unwrap()),
    );
    let settings = settings();
    let rpc = RunnerHubRpc {
        exec: &exec,
        settings: &settings,
        states: &settings,
    };

    assert_eq!(rpc.policy_set(MACHINE, &policy()).await, Ok(stored));
}

#[tokio::test]
async fn the_request_log_is_capped_and_parsed() {
    let row = HubRequestLogRow {
        id: 7,
        entry: HubRequestLogEntry {
            request_id: "req-1".into(),
            kind: LogKind::CancelFeature,
            scope: None,
            outcome: LogOutcome::Accepted,
            reason: None,
            feature_id: Some("f-1".into()),
            received_at_ms: 1_700_000_000_000,
        },
    };
    let exec = ScriptedRpc::default().answering(
        "hub_request_log",
        json!({ "limit": MAX_LOG_PAGE }),
        Ok(json!([row])),
    );
    let settings = settings();
    let rpc = RunnerHubRpc {
        exec: &exec,
        settings: &settings,
        states: &settings,
    };

    assert_eq!(rpc.request_log(MACHINE, 10_000).await, Ok(Some(vec![row])));
}

#[tokio::test]
async fn a_runner_from_before_its_activity_log_reads_as_no_log() {
    let exec = ScriptedRpc::default().answering(
        "hub_request_log",
        json!({ "limit": 50 }),
        Err("unknown method: hub_request_log"),
    );
    let settings = settings();
    let rpc = RunnerHubRpc {
        exec: &exec,
        settings: &settings,
        states: &settings,
    };

    assert_eq!(rpc.request_log(MACHINE, 50).await, Ok(None));
}

#[tokio::test]
async fn an_answer_in_another_shape_is_an_error_naming_the_method() {
    let exec = ScriptedRpc::default().answering(
        "hub_disconnect",
        json!({}),
        Ok(json!({ "paired": false })),
    );
    let settings = settings();
    let rpc = RunnerHubRpc {
        exec: &exec,
        settings: &settings,
        states: &settings,
    };

    let error = rpc.disconnect(MACHINE).await.expect_err("not a HubStatus");
    assert!(error.contains("hub_disconnect"), "{error}");
}

#[tokio::test]
async fn a_disconnect_stores_the_runner_as_no_longer_paired() {
    let db = db_with_machine();
    stored_paired(&db);
    let exec = ScriptedRpc::default()
        .answering("hub_disconnect", json!({}), Ok(status("unpaired", None)))
        .answering(
            "hub_status",
            json!({}),
            Ok(hub_status_answer("unpaired", None)),
        );
    let rpc = RunnerHubRpc {
        exec: &exec,
        settings: &db,
        states: &db,
    };

    let answer = rpc.disconnect(MACHINE).await.expect("disconnected");

    assert_eq!(answer.state, HubPairingState::Unpaired);
    assert_eq!(methods(&exec), ["hub_disconnect", "hub_status"]);
    let stored = MachineHubStateRepository::get(&db, MACHINE)
        .unwrap()
        .expect("a row");
    assert!(!stored.paired);
}

#[tokio::test]
async fn a_pairing_that_finished_stores_the_runner_as_paired() {
    let db = db_with_machine();
    let exec = ScriptedRpc::default()
        .answering("hub_pair_state", json!({}), Ok(json!({ "state": "idle" })))
        .answering(
            "hub_status",
            json!({}),
            Ok(hub_status_answer("paired", Some("inst-1"))),
        );
    let rpc = RunnerHubRpc {
        exec: &exec,
        settings: &db,
        states: &db,
    };

    assert_eq!(rpc.pair_state(MACHINE).await, Ok(PairStateView::Idle));

    assert_eq!(methods(&exec), ["hub_pair_state", "hub_status"]);
    let stored = MachineHubStateRepository::get(&db, MACHINE)
        .unwrap()
        .expect("a row");
    assert!(stored.paired);
    assert_eq!(stored.instance_id.as_deref(), Some("inst-1"));
}

#[tokio::test]
async fn a_pending_pairing_asks_nothing_more_of_the_runner() {
    let db = db_with_machine();
    let exec = ScriptedRpc::default().answering(
        "hub_pair_state",
        json!({}),
        Ok(json!({ "state": "pending", "ticket": ticket() })),
    );
    let rpc = RunnerHubRpc {
        exec: &exec,
        settings: &db,
        states: &db,
    };

    rpc.pair_state(MACHINE).await.expect("parsed");

    assert_eq!(methods(&exec), ["hub_pair_state"]);
    assert_eq!(MachineHubStateRepository::get(&db, MACHINE).unwrap(), None);
}

#[tokio::test]
async fn a_state_that_cannot_be_stored_does_not_fail_the_users_call() {
    // No machine row, so the refresh's write is refused.
    let db = settings();
    let exec = ScriptedRpc::default()
        .answering("hub_disconnect", json!({}), Ok(status("unpaired", None)))
        .answering("hub_pair_state", json!({}), Ok(json!({ "state": "idle" })))
        .answering(
            "hub_status",
            json!({}),
            Ok(hub_status_answer("unpaired", None)),
        );
    let rpc = RunnerHubRpc {
        exec: &exec,
        settings: &db,
        states: &db,
    };

    assert!(rpc.disconnect(MACHINE).await.is_ok());
    assert_eq!(rpc.pair_state(MACHINE).await, Ok(PairStateView::Idle));
    assert_eq!(
        methods(&exec),
        [
            "hub_disconnect",
            "hub_status",
            "hub_pair_state",
            "hub_status"
        ]
    );
}

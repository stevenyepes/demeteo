// Tests extracted from `crates/demeteo-runner/src/hub/keeper.rs` (mirrored-tests convention). `super` = that module.
//
// The fetcher is the real `HubGitTokens`, against a loopback responder that
// answers each request with the next scripted reply and anything past the
// script with a 599 it records, so an extra ask shows up as one.

use super::{hub_runs_to_keep, HubGitCredentials, KeptRun};
use crate::hub::keeper_policy::Timing;
use crate::hub::HUB_CLIENT_ID;
use crate::test_services::Rig;
use async_trait::async_trait;
use demeteo_core::adapters::hub::device_key::DeviceKey;
use demeteo_core::adapters::hub::git_tokens::HubGitTokens;
use demeteo_core::adapters::hub::socket::AccessTokens;
use demeteo_core::domain::hub_pairing::{RefreshAnswer, SecretString};
use demeteo_core::domain::hub_request::{HubRequestLogEntry, LogKind, LogOutcome};
use demeteo_core::domain::hub_socket::RefreshEnd;
use demeteo_core::shared::time::now_secs;
use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const RUN: &str = "hub-run-1";
const REQUEST: &str = "req-1";
const FEATURE: &str = "f-1";
const ACCESS: &str = "access-1";

const FAST: Timing = Timing {
    backoff_base: Duration::from_millis(10),
    backoff_cap: Duration::from_millis(40),
    // A token is replaced as soon as `min_hold` allows, unless it outlives
    // the margin by a lot.
    refresh_margin: Duration::from_secs(3600),
    min_hold: Duration::from_millis(30),
    not_yet_budget: Duration::from_secs(30),
    status_poll: Duration::from_millis(10),
};

/// One request as the responder read it.
#[derive(Debug, Clone)]
struct Seen {
    head: String,
    body: String,
}

#[derive(Clone, Default)]
struct FakeHub {
    script: Arc<Mutex<VecDeque<(u16, String)>>>,
    seen: Arc<Mutex<Vec<Seen>>>,
    unscripted: Arc<Mutex<usize>>,
}

impl FakeHub {
    fn token(token: &str, expires_in: u64) -> (u16, String) {
        (
            200,
            serde_json::json!({ "token": token, "expires_at": now_secs() + expires_in })
                .to_string(),
        )
    }

    fn status(status: u16) -> (u16, String) {
        (status, r#"{"error":"scripted"}"#.to_owned())
    }

    async fn start(replies: Vec<(u16, String)>) -> (Self, String) {
        let hub = Self::default();
        hub.script.lock().unwrap().extend(replies);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let serving = hub.clone();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                tokio::spawn(serving.clone().answer(tcp));
            }
        });
        (hub, origin)
    }

    async fn answer(self, mut tcp: tokio::net::TcpStream) {
        let mut request = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            if let Some(at) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                break at + 4;
            }
            match tcp.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => request.extend_from_slice(&chunk[..n]),
            }
        };
        let head = String::from_utf8_lossy(&request[..head_end]).into_owned();
        let length = head
            .lines()
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
            })
            .unwrap_or(0);
        while request.len() < head_end + length {
            match tcp.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => request.extend_from_slice(&chunk[..n]),
            }
        }
        let body = String::from_utf8_lossy(&request[head_end..head_end + length]).into_owned();
        self.seen.lock().unwrap().push(Seen { head, body });
        let next = self.script.lock().unwrap().pop_front();
        let (status, body) = next.unwrap_or_else(|| {
            *self.unscripted.lock().unwrap() += 1;
            (599, String::new())
        });
        let reply = format!(
            "HTTP/1.1 {status} Scripted\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = tcp.write_all(reply.as_bytes()).await;
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn unscripted(&self) -> usize {
        *self.unscripted.lock().unwrap()
    }
}

fn secret(value: &str) -> SecretString {
    let answer: RefreshAnswer = serde_json::from_value(serde_json::json!({
        "access_token": value,
        "token_type": "DPoP",
        "expires_in": 900,
        "refresh_token": "unused",
    }))
    .unwrap();
    answer.into_tokens().access
}

/// Always holds `ACCESS`; a refresh is not scripted here.
struct HeldAccess;

#[async_trait]
impl AccessTokens for HeldAccess {
    fn access_token(&self) -> Option<SecretString> {
        Some(secret(ACCESS))
    }
    fn access_expires_at(&self) -> Option<u64> {
        Some(now_secs() + 900)
    }
    async fn refresh_access(&self) -> Result<SecretString, RefreshEnd> {
        panic!("unscripted access refresh");
    }
}

fn keeper(rig: &Rig, origin: String) -> HubGitCredentials {
    let fetcher =
        HubGitTokens::new(origin, DeviceKey::generate().unwrap(), Arc::new(HeldAccess)).unwrap();
    HubGitCredentials::with_timing(rig.svc.clone(), Arc::new(fetcher), FAST)
}

/// A spec whose driver fails at once and touches nothing: its origin is one
/// no runner honours.
fn refused_origin_spec() -> String {
    serde_json::json!({
        "feature_id": FEATURE,
        "title": "t",
        "description": "d",
        "provider": { "kind": "github", "host": "github.com" },
        "repo_path": "acme/app",
        "workflow_json": {},
        "agent_kind": null,
        "model": null,
        "loop_iterations": null,
        "origin": { "kind": "from_the_future" },
    })
    .to_string()
}

/// A Hub run at `status` whose delivery window is open and that holds nothing.
fn seed_hub_run(rig: &Rig, status: &str) {
    let runs = &rig.svc.ctx.runner_runs;
    runs.get_or_create(RUN, &refused_origin_spec(), HUB_CLIENT_ID, 1)
        .unwrap();
    runs.update_status(RUN, status, None, Some(FEATURE), None, None, 2)
        .unwrap();
    rig.svc.creds.open_delivery(RUN);
}

fn kept() -> KeptRun {
    KeptRun {
        run_id: RUN.into(),
        request_id: REQUEST.into(),
        feature_id: FEATURE.into(),
        reopened: false,
    }
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn ended(handle: tokio::task::JoinHandle<()>) {
    tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("the keeper stopped")
        .expect("the keeper did not panic");
}

#[tokio::test]
async fn a_start_not_yet_accepted_is_asked_again_until_the_token_comes() {
    let rig = Rig::new("hub_keeper_not_yet");
    seed_hub_run(&rig, "running");
    let (hub, origin) =
        FakeHub::start(vec![FakeHub::status(404), FakeHub::token("ghs_one", 7200)]).await;

    let handle = keeper(&rig, origin).spawn(kept());
    until("the token", || rig.svc.creds.get(RUN).is_some()).await;

    assert_eq!(rig.svc.creds.get(RUN).as_deref(), Some("ghs_one"));
    let seen = hub.seen();
    assert_eq!(seen.len(), 2);
    for request in &seen {
        assert!(request.head.starts_with("POST /git-token HTTP/1.1\r\n"));
        let head = request.head.to_ascii_lowercase();
        assert!(head.contains(&format!("authorization: dpop {ACCESS}\r\n")));
        assert!(head.contains("\r\ndpop: "));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&request.body).unwrap(),
            serde_json::json!({ "request_id": REQUEST, "feature_id": FEATURE })
        );
    }

    rig.svc
        .ctx
        .runner_runs
        .update_status(RUN, "completed", None, None, None, None, 3)
        .unwrap();
    ended(handle).await;
    assert_eq!(rig.svc.creds.get(RUN), None, "a finished run holds nothing");
    assert_eq!(hub.unscripted(), 0);
}

#[tokio::test]
async fn a_token_near_expiry_is_replaced() {
    let rig = Rig::new("hub_keeper_refresh");
    seed_hub_run(&rig, "running");
    let (hub, origin) = FakeHub::start(vec![
        FakeHub::token("ghs_one", 3600),
        FakeHub::token("ghs_two", 7200),
    ])
    .await;

    let handle = keeper(&rig, origin).spawn(kept());
    until("the second token", || {
        rig.svc.creds.get(RUN).as_deref() == Some("ghs_two")
    })
    .await;

    assert_eq!(hub.seen().len(), 2);
    rig.svc
        .ctx
        .runner_runs
        .update_status(RUN, "failed", None, None, None, None, 3)
        .unwrap();
    ended(handle).await;
    assert_eq!(hub.unscripted(), 0);
}

#[tokio::test]
async fn a_run_the_hub_says_ended_stops_its_keeper() {
    let rig = Rig::new("hub_keeper_gone");
    seed_hub_run(&rig, "running");
    let (hub, origin) = FakeHub::start(vec![FakeHub::status(410)]).await;

    ended(keeper(&rig, origin).spawn(kept())).await;

    assert_eq!(hub.seen().len(), 1);
    assert_eq!(rig.svc.creds.get(RUN), None);
    assert!(
        !rig.svc.creds.deliver(RUN, "late".into()),
        "the keeper closed the window"
    );
    assert_eq!(rig.run(RUN).status, "running", "the run is the driver's");
}

/// The driver wipes before it writes its terminal status, so a keeper can
/// read `running` after the wipe. The closed window is what refuses it.
#[tokio::test]
async fn a_token_that_arrives_after_the_wipe_is_never_stored() {
    let rig = Rig::new("hub_keeper_after_wipe");
    seed_hub_run(&rig, "running");
    rig.svc.creds.remove(RUN);
    let (hub, origin) = FakeHub::start(vec![FakeHub::token("ghs_late", 7200)]).await;

    ended(keeper(&rig, origin).spawn(kept())).await;

    assert_eq!(hub.seen().len(), 1);
    assert_eq!(rig.svc.creds.get(RUN), None);
}

#[tokio::test]
async fn a_run_already_over_is_never_asked_for() {
    let rig = Rig::new("hub_keeper_over");
    seed_hub_run(&rig, "cancelled");
    let (hub, origin) = FakeHub::start(Vec::new()).await;

    ended(keeper(&rig, origin).spawn(kept())).await;

    assert!(hub.seen().is_empty());
    assert_eq!(rig.svc.creds.get(RUN), None);
}

/// The path `inject_credentials` takes: a parked run is re-driven once its
/// credential is there. This spec's driver then fails at once on its origin,
/// which only a driver writes.
#[tokio::test]
async fn a_run_parked_for_its_credential_is_re_driven_once_the_token_is_held() {
    let rig = Rig::new("hub_keeper_parked");
    seed_hub_run(&rig, "needs-credentials");
    let (_hub, origin) = FakeHub::start(vec![FakeHub::token("ghs_one", 7200)]).await;

    ended(keeper(&rig, origin).spawn(kept())).await;

    let run = rig.run(RUN);
    assert_eq!(run.status, "failed");
    assert!(
        run.error.as_deref().unwrap_or_default().contains("origin"),
        "{:?}",
        run.error
    );
    assert_eq!(rig.svc.creds.get(RUN), None, "the driver wiped it");
}

/// Holds `ACCESS` until refreshed, then `access-2`; counts the refreshes.
#[derive(Default)]
struct RefreshingAccess {
    refreshes: Mutex<u32>,
}

#[async_trait]
impl AccessTokens for RefreshingAccess {
    fn access_token(&self) -> Option<SecretString> {
        Some(secret(match *self.refreshes.lock().unwrap() {
            0 => ACCESS,
            _ => "access-2",
        }))
    }
    fn access_expires_at(&self) -> Option<u64> {
        Some(now_secs() + 900)
    }
    async fn refresh_access(&self) -> Result<SecretString, RefreshEnd> {
        *self.refreshes.lock().unwrap() += 1;
        Ok(secret("access-2"))
    }
}

#[tokio::test]
async fn a_401_earns_one_refresh_and_one_second_ask() {
    let rig = Rig::new("hub_keeper_unauthorized");
    seed_hub_run(&rig, "running");
    let (hub, origin) =
        FakeHub::start(vec![FakeHub::status(401), FakeHub::token("ghs_one", 7200)]).await;
    let access = Arc::new(RefreshingAccess::default());
    let fetcher =
        HubGitTokens::new(origin, DeviceKey::generate().unwrap(), access.clone()).unwrap();
    let handle =
        HubGitCredentials::with_timing(rig.svc.clone(), Arc::new(fetcher), FAST).spawn(kept());

    until("the token", || rig.svc.creds.get(RUN).is_some()).await;

    assert_eq!(*access.refreshes.lock().unwrap(), 1);
    let auth: Vec<_> = hub
        .seen()
        .iter()
        .map(|s| {
            s.head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("authorization: ")
                        .map(str::to_owned)
                })
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(auth, [format!("dpop {ACCESS}"), "dpop access-2".to_owned()]);
    rig.svc
        .ctx
        .runner_runs
        .update_status(RUN, "completed", None, None, None, None, 3)
        .unwrap();
    ended(handle).await;
    assert_eq!(hub.unscripted(), 0);
}

fn log_start(rig: &Rig, request_id: &str, feature_id: &str, outcome: LogOutcome) {
    rig.svc
        .ctx
        .hub_requests
        .append_log(&HubRequestLogEntry {
            request_id: request_id.into(),
            kind: LogKind::StartFeature,
            scope: None,
            outcome,
            reason: None,
            feature_id: Some(feature_id.into()),
            received_at_ms: 1,
        })
        .unwrap();
}

#[tokio::test]
async fn a_restart_keeps_the_live_hub_runs_under_the_start_they_were_accepted_as() {
    let rig = Rig::new("hub_keeper_restart");
    let runs = &rig.svc.ctx.runner_runs;
    for (run_id, feature_id, owner, status) in [
        ("hub-live", "f-live", HUB_CLIENT_ID, "running"),
        ("hub-parked", "f-parked", HUB_CLIENT_ID, "needs-credentials"),
        ("hub-done", "f-done", HUB_CLIENT_ID, "pr_ready"),
        ("hub-unlogged", "f-unlogged", HUB_CLIENT_ID, "running"),
        ("desk-live", "f-desk", "laptop-1", "running"),
    ] {
        runs.get_or_create(run_id, "{}", owner, 1).unwrap();
        runs.update_status(run_id, status, None, Some(feature_id), None, None, 2)
            .unwrap();
    }
    log_start(&rig, "req-refused", "f-live", LogOutcome::Refused);
    log_start(&rig, "req-live", "f-live", LogOutcome::Accepted);
    log_start(&rig, "req-parked", "f-parked", LogOutcome::Accepted);
    log_start(&rig, "req-done", "f-done", LogOutcome::Accepted);
    log_start(&rig, "req-desk", "f-desk", LogOutcome::Accepted);

    let mut kept: Vec<_> = hub_runs_to_keep(&rig.svc)
        .into_iter()
        .map(|k| (k.run_id, k.request_id, k.feature_id))
        .collect();
    kept.sort();

    assert_eq!(
        kept,
        [
            ("hub-live".into(), "req-live".into(), "f-live".into()),
            ("hub-parked".into(), "req-parked".into(), "f-parked".into()),
        ] as [(String, String, String); 2]
    );
    assert!(rig.svc.creds.deliver("hub-live", "t".into()));
    assert!(!rig.svc.creds.deliver("hub-done", "t".into()));
    assert!(!rig.svc.creds.deliver("hub-unlogged", "t".into()));
}

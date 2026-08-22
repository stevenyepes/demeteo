// Tests extracted from `crates/demeteo-core/src/application/discovery/turn.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::application::discovery::decompose;
use crate::application::discovery::running::RunningTurns;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{MachineId, ProjectId, WorkflowId, LOCAL_MACHINE};
use crate::domain::models::{Project, ProjectSettings, ProjectWorkflowOverride, Repository};
use crate::ports::db::ProjectRepository;

#[test]
fn a_resumed_turn_that_said_nothing_and_failed_is_retried_from_the_transcript() {
    assert!(should_reseed_and_retry(true, false, TurnEnding::Failed));
    assert!(should_reseed_and_retry(
        true,
        false,
        TurnEnding::Environmental
    ));
}

#[test]
fn a_resumed_turn_that_answered_before_failing_reached_the_model() {
    assert!(!should_reseed_and_retry(true, true, TurnEnding::Failed));
    assert!(!should_reseed_and_retry(
        true,
        true,
        TurnEnding::Environmental
    ));
}

#[test]
fn a_turn_that_already_carried_the_transcript_has_nothing_to_fall_back_to() {
    assert!(!should_reseed_and_retry(false, false, TurnEnding::Failed));
    assert!(!should_reseed_and_retry(
        false,
        false,
        TurnEnding::Environmental
    ));
}

#[test]
fn a_stop_is_the_user_declining_the_turn_not_a_lost_session() {
    assert!(!should_reseed_and_retry(
        true,
        false,
        TurnEnding::Interrupted
    ));
}

#[test]
fn a_turn_that_worked_is_never_run_twice() {
    assert!(!should_reseed_and_retry(true, false, TurnEnding::Success));
}

#[test]
fn every_ending_but_a_stop_reports_what_it_spent() {
    let outcome = || TurnOutcome {
        text: "said something".to_string(),
        produced_artifacts: Vec::new(),
        cost_usd: 0.31,
        tokens: 12_400,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    };
    for result in [
        TurnResult::Success(outcome()),
        TurnResult::Failed {
            reason: "rate limited".to_string(),
            spent: outcome(),
        },
        TurnResult::Environmental {
            reason: "wall cap".to_string(),
            spent: outcome(),
        },
    ] {
        let (_, _, spent) = split(result);
        assert_eq!(spent.cost_usd, 0.31);
        assert_eq!(spent.tokens, 12_400);
    }
    let (ending, reason, spent) = split(TurnResult::Interrupted);
    assert_eq!(ending, TurnEnding::Interrupted);
    assert_eq!(reason, None);
    assert_eq!(spent.cost_usd, 0.0);
}

#[test]
fn the_interviewer_may_read_and_run_but_never_write() {
    let p = interviewer_permissions();
    assert_eq!(p.read_fs, Access::Allow);
    assert_eq!(p.write_fs, Access::Deny);
    assert_eq!(p.execute, Access::Allow);
    assert_eq!(p.network, Access::Allow);
}

fn discovery() -> Discovery {
    Discovery {
        id: DiscoveryId::from("d-1".to_string()),
        project_id: ProjectId::from("p-1".to_string()),
        title: "multi-client runner".to_string(),
        status: DiscoveryStatus::Open,
        machine_id: MachineId::from(LOCAL_MACHINE.to_string()),
        agent_kind: "claude-code".to_string(),
        model: None,
        effort: None,
        resume_session_id: None,
        worktree_path: None,
        attachments: Vec::new(),
        total_cost: 0.0,
        tokens: 0,
        created_at: 0,
        updated_at: 0,
    }
}

/// Every status put on the wire, each paired with whether the Discovery read
/// as running at the instant it was sent — which is the half of the claim's
/// contract no later assertion can recover.
type Wire = Arc<std::sync::Mutex<Vec<(String, bool)>>>;

fn recorder(turns: Arc<RunningTurns>) -> (impl Fn(&str, serde_json::Value), Wire) {
    let wire: Wire = Arc::new(std::sync::Mutex::new(Vec::new()));
    let written = wire.clone();
    let emit = move |event: &str, payload: serde_json::Value| {
        assert_eq!(event, EVENT_DISCOVERY_TURN_STATUS);
        written.lock().unwrap().push((
            payload["status"].as_str().unwrap_or_default().to_string(),
            turns.running(&DiscoveryId::from("d-1".to_string())),
        ));
    };
    (emit, wire)
}

#[tokio::test]
async fn a_turn_says_it_is_setting_up_before_it_starts_setting_up() {
    let turns = Arc::new(RunningTurns::default());
    let (emit, wire) = recorder(turns.clone());
    let d = discovery();

    let heard = wire.clone();
    let held = turns
        .clone()
        .try_claim(d.id.as_str())
        .expect("nothing else holds it");
    let (announced_by_then, claim) = announced(&emit, &d, held, async move {
        Ok::<Vec<(String, bool)>, String>(heard.lock().unwrap().clone())
    })
    .await
    .expect("preparing succeeded");

    assert_eq!(
        announced_by_then,
        vec![(STATUS_SETTING_UP.to_string(), true)],
        "the surface learns a turn is setting up only after it has finished setting up"
    );
    drop(claim);
    assert!(!turns.running(&d.id));
}

#[tokio::test]
async fn a_setup_that_failed_gives_the_claim_back_before_it_says_so() {
    let turns = Arc::new(RunningTurns::default());
    let (emit, wire) = recorder(turns.clone());
    let d = discovery();

    let held = turns
        .clone()
        .try_claim(d.id.as_str())
        .expect("nothing else holds it");
    let outcome = announced(&emit, &d, held, async {
        Err::<(), String>("This project has no checkout on 'builder'".to_string())
    })
    .await;

    assert!(outcome.is_err());
    assert_eq!(
        wire.lock().unwrap().as_slice(),
        [
            (STATUS_SETTING_UP.to_string(), true),
            (STATUS_ERROR.to_string(), false)
        ]
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// What `send` does and does not wait for
// ─────────────────────────────────────────────────────────────────────────────

/// A `ProjectRepository` that parks the first call setup makes until the test
/// lets it through.
///
/// Which is the only way to assert that `send` did not wait: a caller that
/// awaited setup could not have answered while this is still parked, so
/// re-awaiting it turns the test's timeout red instead of leaving the property
/// untested.
///
/// A channel rather than a `Barrier` because dropping the sender releases it
/// too — a test that fails its assertion must not leave a worker thread parked
/// where the runtime's own shutdown will wait on it forever.
struct HeldProjects {
    gate: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

macro_rules! reject_project_call {
    () => {
        panic!("setup read the project repository through a call this double was not given")
    };
}

impl ProjectRepository for HeldProjects {
    fn get_repositories_for(&self, _: &ProjectId) -> Result<Vec<Repository>, String> {
        let _ = self
            .gate
            .lock()
            .expect("the gate is not poisoned")
            .recv_timeout(std::time::Duration::from_secs(30));
        Ok(Vec::new())
    }
    fn get_projects(&self) -> Result<Vec<Project>, String> {
        reject_project_call!()
    }
    fn get_project(&self, _: &ProjectId) -> Result<Option<Project>, String> {
        reject_project_call!()
    }
    fn add(&self, _: Project) -> Result<(), String> {
        reject_project_call!()
    }
    fn update(&self, _: Project) -> Result<(), String> {
        reject_project_call!()
    }
    fn update_status(&self, _: &ProjectId, _: &str) -> Result<(), String> {
        reject_project_call!()
    }
    fn delete(&self, _: &ProjectId) -> Result<(), String> {
        reject_project_call!()
    }
    fn delete_repositories_for(&self, _: &ProjectId) -> Result<(), String> {
        reject_project_call!()
    }
    fn add_repository(&self, _: Repository) -> Result<(), String> {
        reject_project_call!()
    }
    fn get_settings(&self, _: &ProjectId) -> Result<Option<ProjectSettings>, String> {
        reject_project_call!()
    }
    fn save_settings(&self, _: ProjectSettings) -> Result<(), String> {
        reject_project_call!()
    }
    fn list_workflow_overrides(
        &self,
        _: &ProjectId,
    ) -> Result<Vec<ProjectWorkflowOverride>, String> {
        reject_project_call!()
    }
    fn list_overrides_for_workflow(
        &self,
        _: &ProjectId,
        _: &WorkflowId,
    ) -> Result<Vec<ProjectWorkflowOverride>, String> {
        reject_project_call!()
    }
    fn upsert_workflow_override(&self, _: ProjectWorkflowOverride) -> Result<(), String> {
        reject_project_call!()
    }
}

/// One Discovery on a project with no repository, so setup gets as far as
/// resolving one and no further.
fn fixture(tag: &str) -> (AppContext, DiscoveryId) {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-discovery-send-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos()
    ));
    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
    let d = discovery();
    ctx.projects
        .add(Project {
            id: d.project_id.clone(),
            name: "interview fixture".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .expect("the project is stored");
    ctx.discoveries.create(&d).expect("the discovery is stored");
    (ctx, d.id)
}

type Statuses = Arc<std::sync::Mutex<Vec<(String, Option<String>)>>>;

fn statuses() -> (
    impl Fn(&str, serde_json::Value) + Send + Sync + 'static,
    Statuses,
) {
    let seen: Statuses = Arc::new(std::sync::Mutex::new(Vec::new()));
    let written = seen.clone();
    let emit = move |event: &str, payload: serde_json::Value| {
        if event != EVENT_DISCOVERY_TURN_STATUS {
            return;
        }
        written.lock().expect("the recorder is not poisoned").push((
            payload["status"].as_str().unwrap_or_default().to_string(),
            payload["reason"].as_str().map(str::to_string),
        ));
    };
    (emit, seen)
}

fn silent() -> impl Fn(&str, serde_json::Value) + Send + Sync + 'static {
    |_: &str, _: serde_json::Value| {}
}

async fn awaited(seen: &Statuses, status: &str) -> Option<String> {
    for _ in 0..400 {
        let found = seen
            .lock()
            .expect("the recorder is not poisoned")
            .iter()
            .find(|(seen, _)| seen == status)
            .cloned();
        if let Some((_, reason)) = found {
            return reason;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("'{status}' never reached the surface");
}

/// The held repository, and the sender that lets its one parked call through.
fn held() -> (Arc<HeldProjects>, std::sync::mpsc::Sender<()>) {
    let (release, gate) = std::sync::mpsc::channel();
    (
        Arc::new(HeldProjects {
            gate: std::sync::Mutex::new(gate),
        }),
        release,
    )
}

async fn until_claimed(ctx: &AppContext, id: &DiscoveryId) {
    for _ in 0..400 {
        if ctx.discovery_turns.running(id) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("nothing ever claimed the discovery");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_users_message_comes_back_before_the_turn_is_set_up() {
    let (mut ctx, id) = fixture("early-return");
    let (projects, release) = held();
    ctx.projects = projects;
    let (emit, _seen) = statuses();

    // On its own task, so that a `send` which waited for setup blocks only
    // itself and this one is still here to time out and say so.
    let sending = tokio::spawn({
        let ctx = ctx.clone();
        let id = id.clone();
        async move { send(&ctx, &id, "what should this do?".to_string(), emit).await }
    });
    let stored = tokio::time::timeout(std::time::Duration::from_secs(10), sending)
        .await
        .expect("send awaited setup: the bubble the user just typed must not queue behind it")
        .expect("the send task did not panic")
        .expect("the message is stored");

    assert_eq!(stored.role, MessageRole::User);
    assert_eq!(stored.content, "what should this do?");

    release.send(()).expect("setup is still parked on the gate");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_setup_that_failed_is_told_as_an_error_status_with_its_reason() {
    let (ctx, id) = fixture("setup-failure");
    let (emit, seen) = statuses();

    send(&ctx, &id, "what should this do?".to_string(), emit)
        .await
        .expect("a turn that cannot be set up is still accepted");

    let reason = awaited(&seen, STATUS_ERROR)
        .await
        .expect("an error status says what stopped it");
    assert!(
        reason.contains("No repository configured"),
        "the caller's Err became this event, so it has to carry the same reason: {reason}"
    );
    assert!(
        !ctx.discovery_turns.running(&id),
        "a failed setup gives the claim back"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_running_turn_refuses_a_second_turn_and_a_pass() {
    let (mut ctx, id) = fixture("turn-refuses");
    let (projects, release) = held();
    ctx.projects = projects;
    let (emit, _seen) = statuses();

    send(&ctx, &id, "first".to_string(), emit)
        .await
        .expect("the first turn is accepted");

    let second = send(&ctx, &id, "second".to_string(), silent()).await;
    assert_eq!(second.err().as_deref(), Some(ALREADY_RUNNING));
    let pass = decompose::run(&ctx, &id, silent()).await;
    assert_eq!(pass.err().as_deref(), Some(ALREADY_RUNNING));

    assert!(
        ctx.discovery_turns.running(&id),
        "a refusal must not release the claim it was refused by"
    );
    assert_eq!(
        ctx.discoveries
            .list_messages(&id)
            .expect("the transcript reads")
            .len(),
        1,
        "a refused turn leaves no bubble behind"
    );

    release.send(()).expect("setup is still parked on the gate");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn a_running_pass_refuses_a_turn() {
    let (mut ctx, id) = fixture("pass-refuses");
    let (projects, release) = held();
    ctx.projects = projects;

    let passing = tokio::spawn({
        let ctx = ctx.clone();
        let id = id.clone();
        async move { decompose::run(&ctx, &id, silent()).await }
    });
    until_claimed(&ctx, &id).await;

    let refused = send(&ctx, &id, "meanwhile".to_string(), silent()).await;
    assert_eq!(refused.err().as_deref(), Some(ALREADY_RUNNING));

    release
        .send(())
        .expect("the pass is still parked on the gate");
    assert!(
        passing.await.expect("the pass ran to its end").is_err(),
        "the pass fails on its own setup, not on the refusal"
    );
    assert!(!ctx.discovery_turns.running(&id));
}

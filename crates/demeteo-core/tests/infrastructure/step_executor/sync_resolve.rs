// `super` = `adapters::step_executor::sync_resolve`.
//
// The bundle is the subject: it is assembled here out of doubles and an
// in-memory database, with no `ExecutionDriver` anywhere, which is the property
// that makes the button's path and the workflow node's path the same code
// (AGENTS.md §3). The `ExecutionPort` double errors on anything it was not
// scripted, so a turn that reached for git this test did not anticipate reddens
// rather than reading as an empty answer.

use super::*;

use crate::adapters::agent::test_stubs::StubAgentExec;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::agent_event::{AgentEvent, StopReason, Usage};
use crate::domain::ids::{StepExecutionId, StepId};
use crate::domain::models::{Availability, EffortLevel, SessionInfo, StepExecution};
use crate::domain::sync_session::SyncSessionStatus;
use crate::ports::agent_runtime::{
    AgentCapabilities, AgentRuntime, AgentSession, AgentStartError, PersonalizationSupport,
};
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::ports::sync_session::{SyncSession, SyncSessionPort};
use rusqlite::Connection;
use std::pin::Pin;
use std::time::Instant;
use tokio_stream::Stream;

const REPO: &str = "/repos/demeteo";
const WT: &str = "/repos/demeteo_wt_sync_feature-f-1";
const PORCELAIN: &str =
    "git -C /repos/demeteo_wt_sync_feature-f-1 status --porcelain --untracked-files=no";
const MERGE_HEAD: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse --verify MERGE_HEAD";
const ADD_ALL: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 add -A";
const PENDING_MERGE_HEAD: &str =
    "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse --verify --quiet MERGE_HEAD";
const PENDING_STATUS: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 status --porcelain";
const COMMIT: &str = "git -c core.hooksPath=/dev/null -C /repos/demeteo_wt_sync_feature-f-1 \
                      -c user.email=demeteo@local -c user.name=demeteo commit -m \
                      'chore: resolve sync conflicts with origin/master'";
const PUSH: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 push origin feature/f-1";
const HEAD: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse HEAD";
const DISCARD: &str =
    "git -C /repos/demeteo worktree remove --force /repos/demeteo_wt_sync_feature-f-1";

/// Records what the turn told the UI, so a test can assert which row the
/// stream was keyed to rather than that a stream happened.
#[derive(Default)]
struct CapturingNotif {
    events: std::sync::Mutex<Vec<DomainEvent>>,
}
impl NotificationPort for CapturingNotif {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }
}

/// An [`AgentSession`] that replays a fixed script — the wire, minus the
/// process. Copied in shape from `tests/infrastructure/agent/event_stream.rs`;
/// what this file needs beyond it is a *runtime* the registry can spawn from.
struct ScriptedSession(Vec<AgentEvent>);
impl AgentSession for ScriptedSession {
    fn session_id(&self) -> &str {
        "scripted"
    }
    fn prompt(&self, _: &str) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
        Box::pin(tokio_stream::iter(self.0.clone()))
    }
    fn cancel(&self) -> Result<(), String> {
        Ok(())
    }
    fn set_mode(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn set_config_option(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn session_info(&self) -> SessionInfo {
        SessionInfo::default()
    }
}

struct ScriptedRuntime;
#[async_trait::async_trait]
impl AgentRuntime for ScriptedRuntime {
    fn kind(&self) -> &'static str {
        "opencode"
    }
    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            display_label: "Scripted",
            lists_models: false,
            model_listing: None,
            default_model: None,
            effort_levels: &[],
            personalization: PersonalizationSupport::Native,
            windows_agent_shell: crate::domain::models::WindowsAgentShell::Unknown,
        }
    }
    async fn availability(&self, _exec: &dyn ExecutionPort, _machine_id: &str) -> Availability {
        Availability::Installed
    }
    fn install_command(&self) -> &'static str {
        "echo scripted"
    }
    fn start(
        &self,
        _ctx: crate::ports::agent_runtime::AgentContext,
    ) -> Pin<
        Box<
            dyn std::future::Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async {
            let session: Arc<dyn AgentSession> = Arc::new(ScriptedSession(vec![
                AgentEvent::Text {
                    delta: "resolved".to_string(),
                },
                AgentEvent::Usage(Usage {
                    input_tokens: 400,
                    output_tokens: 600,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                    cost_usd: Some(1.25),
                }),
                AgentEvent::TurnComplete {
                    stop_reason: StopReason::EndOfTurn,
                    usage: None,
                },
            ]));
            Ok(session)
        })
    }
}

/// Every port the turn borrows, plus the three handles a test asserts through:
/// the concrete double (for its recorder), the database (for the row) and the
/// notifier (for what the UI was told).
struct Ports {
    exec: Arc<dyn ExecutionPort>,
    scripted: Arc<ScriptedExec>,
    db: Arc<SqliteAdapter>,
    registry: Arc<AgentRegistry>,
    notif: Arc<dyn NotificationPort>,
    capturing: Arc<CapturingNotif>,
    agent_exec: Arc<dyn AgentExecutionPort>,
    app_settings: Arc<dyn AppSettingsRepository>,
    git_ops: GitOpsHelper,
    merge_executor: Arc<dyn MergeExecutor>,
    pricing: Arc<dyn PricingTable>,
}

fn fid() -> FeatureId {
    FeatureId::from("f-1".to_string())
}

/// The persisted row the turn streams against. Named `se-f-1-s-sync` here
/// rather than built through `manual_sync_row`, because what this file is about
/// is the turn — the row's own derivation has its own test.
fn row() -> StepExecution {
    StepExecution {
        id: StepExecutionId::from("se-f-1-s-sync".to_string()),
        feature_id: fid(),
        step_id: StepId("s-sync".to_string()),
        step_index: 0,
        step_kind: "sync".to_string(),
        status: "running".to_string(),
        cost_usd: Some(0.0),
        tokens: Some(0),
        wall_clock_secs: Some(0),
        artifact_path: None,
        artifact_paths: Vec::new(),
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 0,
        updated_at: 0,
    }
}

fn ports(scripted: ScriptedExec, runtimes: Vec<Arc<dyn AgentRuntime>>) -> Ports {
    let db = Arc::new(SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap());
    {
        // The session cascades off the feature, and foreign keys are enforced.
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO projects (id, name, created_at) VALUES ('p-1', 'demeteo', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO features (id, project_id, title, created_at)
             VALUES ('f-1', 'p-1', 'sync me', 0)",
            [],
        )
        .unwrap();
    }
    let scripted = Arc::new(scripted);
    let exec: Arc<dyn ExecutionPort> = scripted.clone();
    let app_settings: Arc<dyn AppSettingsRepository> = db.clone();
    let merge_executor: Arc<dyn MergeExecutor> =
        Arc::new(crate::adapters::merge::SqliteMergeExecutor::new(
            db.clone(),
            db.clone(),
            GitOpsHelper::new(app_settings.clone(), exec.clone()),
            exec.clone(),
            std::path::PathBuf::from("/workspace"),
        ));
    let capturing = Arc::new(CapturingNotif::default());
    Ports {
        git_ops: GitOpsHelper::new(app_settings.clone(), exec.clone()),
        exec,
        scripted,
        db,
        registry: Arc::new(AgentRegistry::new(runtimes)),
        notif: capturing.clone(),
        capturing,
        agent_exec: Arc::new(StubAgentExec),
        app_settings,
        merge_executor,
        pricing: Arc::new(crate::adapters::pricing::HardcodedPricingTable::new()),
    }
}

fn open_conflicted(db: &Arc<SqliteAdapter>) {
    let sessions: &dyn SyncSessionPort = &**db;
    sessions
        .open(&SyncSession {
            feature_id: "f-1".to_string(),
            machine_id: crate::domain::ids::LOCAL_MACHINE.to_string(),
            repo_dir: REPO.to_string(),
            feature_branch: "feature/f-1".to_string(),
            base_branch: "master".to_string(),
            status: SyncSessionStatus::Conflicted,
            worktree_path: Some(WT.to_string()),
            head_before: Some("aaaaaaa".to_string()),
            merge_commit_sha: None,
            conflict_files: Vec::new(),
            raw_error: None,
            attempts: 0,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();
}

fn stored_status(db: &Arc<SqliteAdapter>) -> SyncSessionStatus {
    let sessions: &dyn SyncSessionPort = &**db;
    sessions.get(&fid()).unwrap().unwrap().status
}

/// Run one turn against `p`, with the spend and cancel a caller would own.
async fn run(
    p: &Ports,
    step_exec: &StepExecution,
    cancel: Option<watch::Receiver<bool>>,
    cost: &mut f64,
    tokens: &mut i64,
) -> Result<String, ResolveSyncError> {
    resolve_sync_conflicts(ResolveSyncContext {
        exec: &p.exec,
        registry: &p.registry,
        notif: &p.notif,
        agent_exec: &p.agent_exec,
        app_settings: &p.app_settings,
        git_ops: &p.git_ops,
        merge_executor: &p.merge_executor,
        feature_id: &fid(),
        repo_dir: REPO,
        resolved_cwd: WT,
        machine_str: crate::domain::ids::LOCAL_MACHINE,
        feature_branch: "feature/f-1",
        base_branch: "master",
        conflict_files: &["src/lib.rs".to_string()],
        step_exec,
        thread_id_prefix: SYNC_RESOLVER_THREAD_PREFIX,
        agent_kind: "opencode",
        override_model: None,
        effort: EffortLevel::DEFAULT,
        max_budget_usd: Some(10.0),
        cancel,
        spend: RunningSpend {
            cost,
            tokens,
            start: Instant::now(),
        },
        pricing: &p.pricing,
    })
    .await
}

/// A merge open over an index git already reports as resolved. That shape is
/// what lets one strict double answer the pre- and post-turn reads honestly:
/// they are the same `status --porcelain` command, so a conflicted first answer
/// and a clean second one need the queue, and the queue is worth spending on
/// the assertions that need it rather than on every test here.
fn happy_path() -> ScriptedExec {
    ScriptedExec::new(&[
        (PORCELAIN, Ok("")),
        (MERGE_HEAD, Ok("b1b2b3b\n")),
        (ADD_ALL, Ok("")),
        (PENDING_MERGE_HEAD, Ok("b1b2b3b\n")),
        (
            COMMIT,
            Ok("[feature/f-1 c0ffee] chore: resolve sync conflicts"),
        ),
        (PUSH, Ok("")),
        (HEAD, Ok("c0ffeec\n")),
        (DISCARD, Ok("")),
    ])
}

/// The turn that ends before it starts, and the row that has to say so.
///
/// Both callers used to record the verdict themselves — which meant only one of
/// them did, and a resolution the user asked for left the session reading
/// `conflicted` beside a tree nothing was working on. The bundle is what proves
/// the recording is the turn's own: nothing here is a driver, a workflow or a
/// step row.
#[tokio::test]
async fn a_turn_that_found_no_merge_leaves_the_verdict_on_the_session() {
    let p = ports(ScriptedExec::new(&[]), vec![]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run(&p, &row(), None, &mut cost, &mut tokens).await;

    assert!(
        matches!(&outcome, Err(ResolveSyncError::Failed(reason)) if reason.contains("No active merge")),
        "{outcome:?}"
    );
    assert_eq!(
        stored_status(&p.db),
        SyncSessionStatus::ResolutionFailed,
        "a resolution that failed must not leave the session mid-turn"
    );
    assert_eq!(
        p.scripted.commands(),
        vec![PORCELAIN.to_string(), MERGE_HEAD.to_string()],
        "a turn that never ran may not spawn an agent or tear down a worktree"
    );
}

/// What the turn cost has to land on the run's totals, and the stream has to
/// name a row the inspector can subscribe to.
///
/// The whole `TurnOutcome` was dropped and the stream was keyed to a
/// `se-sync-<millis>` id no row carried, so a resolution that ran for minutes
/// showed the user a spinner and billed them nothing.
#[tokio::test]
async fn the_turn_bills_the_totals_it_was_handed_and_streams_to_its_own_row() {
    let p = ports(happy_path(), vec![Arc::new(ScriptedRuntime)]);
    open_conflicted(&p.db);
    let step_exec = row();
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run(&p, &step_exec, None, &mut cost, &mut tokens).await;

    assert_eq!(outcome.unwrap(), "c0ffeec");
    assert_eq!(cost, 1.25, "the turn's dollars have to reach the caller");
    assert_eq!(tokens, 1000, "and so do its tokens");

    let events = p.capturing.events.lock().unwrap();
    assert!(
        events.iter().any(|e| matches!(
            e,
            DomainEvent::AgentStream { step_execution_id, .. } if *step_execution_id == step_exec.id
        )),
        "the stream has to be keyed to the persisted row: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            DomainEvent::StepProgress { step_id, status, .. }
                if step_id == "s-sync" && status == "running"
        )),
        "and the Activity panel's run-event feed reads StepProgress: {events:?}"
    );
}

/// Everything in the tree is staged, not the paths the merge reported.
///
/// The sync worktree is a throwaway checkout that is deleted the moment the
/// resolution lands, so a file the agent had to add — or a fourth file it had
/// to fix — was committed by nothing and then removed with the directory. The
/// commit looked clean; the tree did not build.
#[tokio::test]
async fn a_file_outside_the_reported_conflicts_is_staged_with_the_rest() {
    let scripted = happy_path()
        .with_queue(PORCELAIN, &[Ok("UU src/lib.rs\n"), Ok("")])
        .with_files(&[(
            "/repos/demeteo_wt_sync_feature-f-1/src/lib.rs",
            Ok("clean\n"),
        )]);
    let p = ports(scripted, vec![Arc::new(ScriptedRuntime)]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run(&p, &row(), None, &mut cost, &mut tokens).await;

    assert!(outcome.is_ok(), "{outcome:?}");
    let commands = p.scripted.commands();
    assert!(
        commands.iter().any(|c| c == ADD_ALL),
        "the whole worktree is the resolution: {commands:?}"
    );
    assert!(
        !commands.iter().any(|c| c.contains("add -- ")),
        "staging the reported paths is what dropped the rest: {commands:?}"
    );
}

/// An agent that committed the resolution itself has left nothing to commit,
/// and `git commit` says so by exiting non-zero. Committing unconditionally
/// failed a sync that had in fact succeeded.
///
/// Nothing scripts a commit here: if one is issued the strict double errors and
/// the resolution fails, which is the assertion.
#[tokio::test]
async fn an_agent_that_committed_on_its_own_does_not_fail_the_sync() {
    let scripted = ScriptedExec::new(&[
        (PORCELAIN, Ok("")),
        (MERGE_HEAD, Ok("b1b2b3b\n")),
        (ADD_ALL, Ok("")),
        (PENDING_MERGE_HEAD, Ok("")),
        (PENDING_STATUS, Ok("")),
        (PUSH, Ok("")),
        (HEAD, Ok("c0ffeec\n")),
        (DISCARD, Ok("")),
    ]);
    let p = ports(scripted, vec![Arc::new(ScriptedRuntime)]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run(&p, &row(), None, &mut cost, &mut tokens).await;

    assert_eq!(outcome.unwrap(), "c0ffeec");
    assert_eq!(
        stored_status(&p.db),
        SyncSessionStatus::Resolved,
        "the session has to read the resolution the agent committed"
    );
}

/// Stop, honoured before a dollar is spent.
///
/// The resolver passed `None` for the cancel watch on both paths, so a running
/// resolution could not be stopped at all — and a cancel that arrived while git
/// was mid-flight would have surfaced as an ordinary failure and left the row
/// reading `failed` rather than `interrupted`.
#[tokio::test]
async fn a_stop_that_arrived_first_spawns_no_agent() {
    let p = ports(happy_path(), vec![Arc::new(ScriptedRuntime)]);
    open_conflicted(&p.db);
    let (_tx, rx) = watch::channel(true);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run(&p, &row(), Some(rx), &mut cost, &mut tokens).await;

    assert!(
        matches!(outcome, Err(ResolveSyncError::Cancelled(_))),
        "{outcome:?}"
    );
    assert_eq!(cost, 0.0, "a stopped turn spends nothing");
    assert_eq!(
        p.scripted.commands(),
        vec![PORCELAIN.to_string(), MERGE_HEAD.to_string()],
        "the stop has to land before the spawn, not after it"
    );
}

#[test]
fn merge_markers_are_rejected_before_demeteo_stages_the_resolution() {
    assert!(has_conflict_marker(
        "const value = 1;\n<<<<<<< HEAD\nconst branch = 'feature';\n=======\nconst branch = 'main';\n>>>>>>> origin/master\n"
    ));
    assert!(!has_conflict_marker("const value = 1;\n"));
}

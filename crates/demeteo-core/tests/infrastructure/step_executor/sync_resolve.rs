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
const MERGE_HEAD: &str =
    "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse --verify --quiet MERGE_HEAD";
const ADD_ALL: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 add -A";
const PENDING_STATUS: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 status --porcelain";
const COMMIT: &str = "git -c core.hooksPath=/dev/null -C /repos/demeteo_wt_sync_feature-f-1 \
                      -c user.email=demeteo@local -c user.name=demeteo commit -m \
                      'chore: resolve sync conflicts with origin/master'";
const PUSH: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 push origin feature/f-1";
/// The push's own confirmation: `git push` exiting zero is a verdict about the
/// command, and this is the one about origin.
const CONTAINS: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 merge-base --is-ancestor \
                        c0ffeec refs/remotes/origin/feature/f-1";
const HEAD: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse HEAD";
const DISCARD: &str =
    "git -C /repos/demeteo worktree remove --force /repos/demeteo_wt_sync_feature-f-1";
const PRUNE: &str = "git -C /repos/demeteo worktree prune";
/// The teardown's confirmation read: only an observed-gone tree lets the row
/// stop naming the worktree.
const GIT_DIR: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse --git-dir";

/// A path inside the sync worktree as `ensure_conflict_markers_removed` builds
/// it, which is not the same string on every host.
fn resolved_file(rel: &str) -> String {
    crate::paths::join_on(
        WT,
        [rel],
        crate::paths::targets_windows_host(crate::domain::ids::LOCAL_MACHINE),
    )
}

fn transport_dead() -> String {
    format!(
        "{}Connection appears dead",
        crate::ports::execution::TRANSPORT_ERROR_PREFIX
    )
}

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

/// When a stop lands, relative to the turn it interrupts.
///
/// The two guards in the resolver are separate and only one of them is about
/// the agent: the watch handed to `stream_agent_turn` ends a turn in flight,
/// and the recheck afterwards catches a stop that arrived while git was
/// running. A fixture that delivers its whole script synchronously satisfies
/// both from the *recheck*, which is how "the resolver passes a cancel watch"
/// went untested while reading as covered.
#[derive(Clone, Copy, PartialEq)]
enum StopAt {
    /// As the agent is prompted. Only the watch inside the turn can see it, and
    /// the turn is then billed nothing.
    Prompt,
    /// As the stream closes, after the turn has run and been billed. The watch
    /// has nothing left to interrupt; only the recheck sees it.
    StreamEnd,
}

/// An [`AgentSession`] that replays a fixed script — the wire, minus the
/// process. Copied in shape from `tests/infrastructure/agent/event_stream.rs`;
/// what this file needs beyond it is a *runtime* the registry can spawn from.
///
/// `linger` is the pause between the first token and the rest, and it is what
/// makes a mid-turn stop deterministic rather than a race with the rest of the
/// script.
struct ScriptedSession {
    events: Vec<AgentEvent>,
    linger: Option<std::time::Duration>,
    stop: Option<(StopAt, watch::Sender<bool>)>,
    /// Trips when the registry reaps this session. The resolver's thread id is
    /// minted per turn from the clock, so nothing else in a test can tell a
    /// reaped session from a leaked one.
    killed: Arc<std::sync::atomic::AtomicBool>,
}

impl AgentSession for ScriptedSession {
    fn session_id(&self) -> &str {
        "scripted"
    }
    fn prompt(&self, _: &str) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
        if let Some((StopAt::Prompt, tx)) = &self.stop {
            let _ = tx.send(true);
        }
        let at_end = match &self.stop {
            Some((StopAt::StreamEnd, tx)) => Some(tx.clone()),
            _ => None,
        };
        Box::pin(futures::stream::unfold(
            (0usize, self.events.clone(), self.linger, at_end),
            |(i, events, linger, at_end)| async move {
                if i == 1 {
                    if let Some(d) = linger {
                        tokio::time::sleep(d).await;
                    }
                }
                if i < events.len() {
                    let event = events[i].clone();
                    return Some((event, (i + 1, events, linger, at_end)));
                }
                if let Some(tx) = &at_end {
                    let _ = tx.send(true);
                }
                None
            },
        ))
    }
    fn cancel(&self) -> Result<(), String> {
        Ok(())
    }
    fn kill(&self) -> Result<(), String> {
        self.killed.store(true, std::sync::atomic::Ordering::SeqCst);
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

/// The ceilings the turn asked the harness for, which are invisible to every
/// other assertion here: the registry hands the runtime an `AgentContext` and
/// nothing downstream reports what was in it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SpawnedWith {
    max_turns: Option<u32>,
    max_budget_usd: Option<f64>,
}

#[derive(Default)]
struct ScriptedRuntime {
    seen: std::sync::Mutex<Vec<SpawnedWith>>,
    linger: Option<std::time::Duration>,
    stop: Option<(StopAt, watch::Sender<bool>)>,
    killed: Arc<std::sync::atomic::AtomicBool>,
}

impl ScriptedRuntime {
    /// A turn that pauses after its first token and trips `stop` at `at`.
    fn stopping(at: StopAt, stop: watch::Sender<bool>) -> Self {
        Self {
            linger: Some(std::time::Duration::from_millis(300)),
            stop: Some((at, stop)),
            ..Default::default()
        }
    }
    fn spawns(&self) -> Vec<SpawnedWith> {
        self.seen.lock().unwrap().clone()
    }
    fn session_reaped(&self) -> bool {
        self.killed.load(std::sync::atomic::Ordering::SeqCst)
    }
    /// `TurnComplete` is what ends the turn, so the stop-at-close fixture omits
    /// it: the stream's own end is then the last thing that happens before the
    /// recheck, and there is no race with it.
    fn events(&self) -> Vec<AgentEvent> {
        let mut events = vec![
            AgentEvent::Text {
                delta: "resolved".to_string(),
            },
            AgentEvent::Usage(Usage {
                input_tokens: 400,
                output_tokens: 600,
                cache_read_input_tokens: 70,
                cache_creation_input_tokens: 30,
                cost_usd: Some(1.25),
            }),
        ];
        if !matches!(self.stop, Some((StopAt::StreamEnd, _))) {
            events.push(AgentEvent::TurnComplete {
                stop_reason: StopReason::EndOfTurn,
                usage: None,
            });
        }
        events
    }
}

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
            path_containment: crate::domain::models::PathContainment::UNFENCED,
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
        ctx: crate::ports::agent_runtime::AgentContext,
    ) -> Pin<
        Box<
            dyn std::future::Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>>
                + Send
                + '_,
        >,
    > {
        self.seen.lock().unwrap().push(SpawnedWith {
            max_turns: ctx.max_turns,
            max_budget_usd: ctx.max_budget_usd,
        });
        let events = self.events();
        let linger = self.linger;
        let stop = self.stop.clone();
        let killed = self.killed.clone();
        Box::pin(async move {
            let session: Arc<dyn AgentSession> = Arc::new(ScriptedSession {
                events,
                linger,
                stop,
                killed,
            });
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
            db.clone(),
            Arc::new(crate::application::sync_turns::SyncTurns::default()),
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
            blocked_stage: None,
            pushed_at: None,
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
///
/// `running` because that is the caller a plain `run` stands in for — a
/// workflow's `sync` node, whose driver still holds the feature — and it is
/// what makes the turn publish. The two publication facts are arguments rather
/// than a verdict for the reason
/// [`ResolveSyncContext::review_before_push`](super::ResolveSyncContext) gives:
/// with the verdict handed in, no test here could see the policy at all.
async fn run(
    p: &Ports,
    step_exec: &StepExecution,
    cancel: Option<watch::Receiver<bool>>,
    cost: &mut f64,
    tokens: &mut i64,
) -> Result<ResolvedSync, ResolveSyncError> {
    run_with(p, step_exec, cancel, cost, tokens, None, "running").await
}

#[allow(clippy::too_many_arguments)]
async fn run_with(
    p: &Ports,
    step_exec: &StepExecution,
    cancel: Option<watch::Receiver<bool>>,
    cost: &mut f64,
    tokens: &mut i64,
    review_before_push: Option<bool>,
    feature_status: &str,
) -> Result<ResolvedSync, ResolveSyncError> {
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
        review_before_push,
        feature_status,
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
        (
            COMMIT,
            Ok("[feature/f-1 c0ffee] chore: resolve sync conflicts"),
        ),
        (HEAD, Ok("c0ffeec\n")),
        (PUSH, Ok("")),
        (CONTAINS, Ok("")),
        (DISCARD, Ok("")),
        (PRUNE, Ok("")),
        // The teardown's confirmation: the tree really is gone.
        (GIT_DIR, Err("fatal: not a git repository")),
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
    let p = ports(
        ScriptedExec::new(&[
            (PORCELAIN, Ok("")),
            (MERGE_HEAD, Err("fatal: Needed a single revision")),
        ]),
        vec![],
    );
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
    let p = ports(happy_path(), vec![Arc::new(ScriptedRuntime::default())]);
    open_conflicted(&p.db);
    let step_exec = row();
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run(&p, &step_exec, None, &mut cost, &mut tokens).await;

    assert_eq!(outcome.unwrap().merge_commit_sha, "c0ffeec");
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
        // Built with the same `join_on` the marker check uses, not spelled: on
        // a Windows host that join produces a backslash and a POSIX-spelled key
        // matches nothing, so the read errors and the test asserts nothing
        // about the staging it is named for.
        .with_files(&[(resolved_file("src/lib.rs").as_str(), Ok("clean\n"))]);
    let p = ports(scripted, vec![Arc::new(ScriptedRuntime::default())]);
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
        (ADD_ALL, Ok("")),
        (PENDING_STATUS, Ok("")),
        (HEAD, Ok("c0ffeec\n")),
        (PUSH, Ok("")),
        (DISCARD, Ok("")),
        (PRUNE, Ok("")),
        (GIT_DIR, Err("fatal: not a git repository")),
    ])
    // Open when the turn starts, consumed by the agent's own commit by the
    // time Demeteo asks whether one is still owed.
    .with_queue(MERGE_HEAD, &[Ok("b1b2b3b\n"), Ok("")]);
    let p = ports(scripted, vec![Arc::new(ScriptedRuntime::default())]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run(&p, &row(), None, &mut cost, &mut tokens).await;

    assert_eq!(outcome.unwrap().merge_commit_sha, "c0ffeec");
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
    let p = ports(happy_path(), vec![Arc::new(ScriptedRuntime::default())]);
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

/// The resolution has to be *recorded*, and nothing said so.
///
/// Forcing the commit guard to `false` — so the merge is never committed at all
/// — left the whole suite green: the sha this file asserts comes from a
/// scripted `rev-parse HEAD` that answers the same whether or not a commit ran,
/// so the one write the turn exists to make could disappear while the session
/// was still filed `Resolved`. Only the skip direction was covered.
#[tokio::test]
async fn the_resolution_is_committed_before_it_is_published() {
    let p = ports(happy_path(), vec![Arc::new(ScriptedRuntime::default())]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run(&p, &row(), None, &mut cost, &mut tokens).await;
    assert!(outcome.is_ok(), "{outcome:?}");

    let commands = p.scripted.commands();
    let commit = commands.iter().position(|c| c == COMMIT);
    let push = commands.iter().position(|c| c == PUSH);
    assert!(
        commit.is_some(),
        "the merge the agent resolved has to be committed: {commands:?}"
    );
    assert!(
        commit < push,
        "and committed before it is published: {commands:?}"
    );
}

/// The prompt cache the turn used has to reach the row.
///
/// `cost_usd` and `tokens` were folded out of the `TurnOutcome` and the two
/// cache counters were dropped, so both callers wrote `CacheTokens::default()`
/// and the header's cache chips undercounted every conflict resolution.
#[tokio::test]
async fn the_turns_cache_telemetry_reaches_its_caller() {
    let p = ports(happy_path(), vec![Arc::new(ScriptedRuntime::default())]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let resolved = run(&p, &row(), None, &mut cost, &mut tokens)
        .await
        .expect("the happy path resolves");

    assert_eq!(
        resolved.cache,
        CacheTokens {
            read: Some(70),
            creation: Some(30)
        }
    );
}

/// A resolver turn the user started by hand is an unbounded spend unless the
/// ceilings reach the harness, and nothing downstream reports what the registry
/// was handed — so reverting both to `None` was invisible to every other test.
#[tokio::test]
async fn the_resolver_is_spawned_with_a_turn_cap_and_a_budget() {
    let runtime = Arc::new(ScriptedRuntime::default());
    let p = ports(happy_path(), vec![runtime.clone()]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    run(&p, &row(), None, &mut cost, &mut tokens)
        .await
        .expect("the happy path resolves");

    assert_eq!(
        runtime.spawns(),
        vec![SpawnedWith {
            max_turns: Some(RESOLVER_MAX_TURNS),
            max_budget_usd: Some(10.0),
        }]
    );
}

/// Stop, honoured *during* the turn — the only case the cancel watch exists
/// for, and the one a stop-before-the-spawn test cannot reach.
///
/// The turn is billed nothing, which is what separates this from the recheck
/// below: a resolver handed no watch at all runs its turn to completion, bills
/// it, and is only then refused.
#[tokio::test]
async fn a_stop_that_arrives_mid_turn_ends_it_before_the_turn_is_billed() {
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let runtime = Arc::new(ScriptedRuntime::stopping(StopAt::Prompt, cancel_tx));
    let p = ports(happy_path(), vec![runtime.clone()]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run(&p, &row(), Some(cancel_rx), &mut cost, &mut tokens).await;

    assert!(
        matches!(outcome, Err(ResolveSyncError::Cancelled(_))),
        "{outcome:?}"
    );
    assert_eq!(runtime.spawns().len(), 1, "the agent did start");
    assert_eq!(cost, 0.0, "an interrupted turn is billed nothing");
    let commands = p.scripted.commands();
    assert!(
        !commands
            .iter()
            .any(|c| c == ADD_ALL || c == COMMIT || c == PUSH),
        "a stopped turn stages, commits and publishes nothing: {commands:?}"
    );
    assert_eq!(
        stored_status(&p.db),
        SyncSessionStatus::ResolutionFailed,
        "and the session records the turn that did not land"
    );
}

/// A stop that lands once the turn is over is the other guard, and it is the
/// one that decides whether the row reads `interrupted` or `failed`: without
/// it the resolution walks on into staging and committing in a worktree the
/// user has asked to be left alone.
#[tokio::test]
async fn a_stop_that_arrives_after_the_turn_still_stops_the_resolution() {
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let runtime = Arc::new(ScriptedRuntime::stopping(StopAt::StreamEnd, cancel_tx));
    let p = ports(happy_path(), vec![runtime.clone()]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run(&p, &row(), Some(cancel_rx), &mut cost, &mut tokens).await;

    assert!(
        matches!(outcome, Err(ResolveSyncError::Cancelled(_))),
        "{outcome:?}"
    );
    assert_eq!(cost, 1.25, "this turn ran and has to be paid for");
    let commands = p.scripted.commands();
    assert!(
        !commands.iter().any(|c| c == ADD_ALL || c == PUSH),
        "nothing is staged or published after a stop: {commands:?}"
    );
}

/// Every other exit from the turn reaps the resolver; the push's `?` returned
/// straight out. The thread id is minted per turn from the clock, so nothing
/// later reclaims it: the agent process outlives the run and its registry entry
/// accumulates for the life of the app.
#[tokio::test]
async fn a_push_that_failed_still_reaps_the_resolver() {
    let scripted = happy_path().with_queue(PUSH, &[Err("fatal: remote rejected")]);
    let runtime = Arc::new(ScriptedRuntime::default());
    let p = ports(scripted, vec![runtime.clone()]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run(&p, &row(), None, &mut cost, &mut tokens).await;

    assert!(
        matches!(&outcome, Err(ResolveSyncError::Failed(reason)) if reason.contains("push to origin")),
        "{outcome:?}"
    );
    assert!(
        runtime.session_reaped(),
        "the resolver agent is still running"
    );
}

/// A channel that dies over the commit guard used to read as "the agent
/// committed it itself": the commit was skipped, the push succeeded as a no-op,
/// `rev-parse HEAD` answered the pre-merge sha, and the teardown then deleted
/// the worktree the resolution was sitting in — with the session filed
/// `Resolved`.
#[tokio::test]
async fn an_unreadable_commit_guard_keeps_the_worktree_and_the_verdict_honest() {
    let scripted = happy_path().with_queue(MERGE_HEAD, &[Ok("b1b2b3b\n"), Err(&transport_dead())]);
    let p = ports(scripted, vec![Arc::new(ScriptedRuntime::default())]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run(&p, &row(), None, &mut cost, &mut tokens).await;

    assert!(
        matches!(outcome, Err(ResolveSyncError::Failed(_))),
        "{outcome:?}"
    );
    let commands = p.scripted.commands();
    assert!(
        !commands.iter().any(|c| c == PUSH || c == DISCARD),
        "nothing may be published or deleted on an answer nobody got: {commands:?}"
    );
    assert_eq!(stored_status(&p.db), SyncSessionStatus::ResolutionFailed);
    let session: &dyn SyncSessionPort = &*p.db;
    assert_eq!(
        session
            .get(&fid())
            .unwrap()
            .unwrap()
            .worktree_path
            .as_deref(),
        Some(WT),
        "the row has to keep naming the tree the resolution is still in"
    );
}

/// An unreachable host is not a verdict about a merge, and a session it could
/// not read may not be rewritten from it. The row previously moved to
/// `resolution_failed` with `raw_error` replaced by advice — re-run Sync —
/// whose force-remove would then take the live conflicted worktree.
#[tokio::test]
async fn an_unreachable_worktree_leaves_the_conflicted_session_untouched() {
    let p = ports(
        ScriptedExec::new(&[(PORCELAIN, Err(&transport_dead()))]),
        vec![],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run(&p, &row(), None, &mut cost, &mut tokens).await;

    assert!(
        matches!(outcome, Err(ResolveSyncError::Failed(_))),
        "{outcome:?}"
    );
    assert_eq!(
        stored_status(&p.db),
        SyncSessionStatus::Conflicted,
        "nothing was observed, so nothing may be recorded"
    );
}

/// The teardown reports nothing, so the row may only stop naming the worktree
/// once something has *seen* it go — the rule
/// `application::sync_session::abort` takes at the same boundary.
#[tokio::test]
async fn a_worktree_the_teardown_could_not_confirm_stays_on_the_row() {
    let scripted = happy_path().with_queue(GIT_DIR, &[Err(&transport_dead())]);
    let p = ports(scripted, vec![Arc::new(ScriptedRuntime::default())]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    run(&p, &row(), None, &mut cost, &mut tokens)
        .await
        .expect("the resolution itself landed");

    let session: &dyn SyncSessionPort = &*p.db;
    let stored = session.get(&fid()).unwrap().unwrap();
    assert_eq!(stored.status, SyncSessionStatus::Resolved);
    assert_eq!(
        stored.worktree_path.as_deref(),
        Some(WT),
        "a path nobody confirmed gone is a directory nothing would ever reclaim"
    );
}

#[test]
fn merge_markers_are_rejected_before_demeteo_stages_the_resolution() {
    assert!(has_conflict_marker(
        "const value = 1;\n<<<<<<< HEAD\nconst branch = 'feature';\n=======\nconst branch = 'main';\n>>>>>>> origin/master\n"
    ));
    assert!(!has_conflict_marker("const value = 1;\n"));
}

/// The session as the row holds it, for the two facts publication turns on.
fn stored_session(db: &Arc<SqliteAdapter>) -> crate::ports::sync_session::SyncSession {
    let sessions: &dyn SyncSessionPort = &**db;
    sessions.get(&fid()).unwrap().unwrap()
}

/// A resolution nobody is watching publishes itself, exactly as every
/// resolution did before there was a choice — and the row says so, because
/// `resolved` alone stopped being the whole answer the moment an unpublished
/// resolution became a real state.
#[tokio::test]
async fn a_resolution_with_nobody_to_review_it_reaches_origin() {
    let p = ports(happy_path(), vec![Arc::new(ScriptedRuntime::default())]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let resolved = run(&p, &row(), None, &mut cost, &mut tokens)
        .await
        .expect("the happy path resolves");

    assert!(resolved.published);
    let session = stored_session(&p.db);
    assert_eq!(session.status, SyncSessionStatus::Resolved);
    assert!(
        session.pushed_at.is_some(),
        "a published resolution has to be recorded as one"
    );
}

/// The turn most likely to quietly drop a hunk was the only one that shipped
/// straight to the open pull request. Held, it commits and stops: the script
/// below scripts no `push`, and the double errors on anything it was not told
/// to answer, so a turn that published anyway reddens here rather than passing
/// on an empty answer.
///
/// It also scripts no teardown. The tree is what the review's Discard resets
/// the branch in, so keeping it is not an oversight — a resolution held for
/// review is the one success that leaves its worktree standing.
#[tokio::test]
async fn a_resolution_somebody_can_look_at_stops_at_the_commit() {
    let p = ports(
        ScriptedExec::new(&[
            (PORCELAIN, Ok("")),
            (MERGE_HEAD, Ok("b1b2b3b\n")),
            (ADD_ALL, Ok("")),
            (
                COMMIT,
                Ok("[feature/f-1 c0ffee] chore: resolve sync conflicts"),
            ),
            (HEAD, Ok("c0ffeec\n")),
        ]),
        vec![Arc::new(ScriptedRuntime::default())],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let resolved = run_with(&p, &row(), None, &mut cost, &mut tokens, None, "completed")
        .await
        .expect("holding the push is not a failure");

    assert!(!resolved.published);
    assert_eq!(resolved.merge_commit_sha, "c0ffeec");
    let session = stored_session(&p.db);
    assert_eq!(session.status, SyncSessionStatus::Resolved);
    assert_eq!(session.pushed_at, None);
    assert_eq!(
        session.worktree_path.as_deref(),
        Some(WT),
        "the tree the branch is checked out in is what Discard resets"
    );
    // The row would keep naming the directory whether or not it went:
    // `discard_sync_worktree` swallows every error and the held arm hard-codes
    // `worktree_discarded: false`. What the deletion is visible in is the
    // command log, so that is what this asserts on.
    assert!(
        !p.scripted
            .commands()
            .iter()
            .any(|c| c.contains("worktree remove") || c.contains("worktree prune")),
        "{:?}",
        p.scripted.commands()
    );
}

/// The project may take review away, and taking it away has to actually reach
/// the turn. Somebody is in a position to look here — the run is over — and the
/// resolution still publishes, which is the whole of what the setting does.
#[tokio::test]
async fn a_project_that_opted_out_of_review_publishes_with_somebody_watching() {
    let p = ports(happy_path(), vec![Arc::new(ScriptedRuntime::default())]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let resolved = run_with(
        &p,
        &row(),
        None,
        &mut cost,
        &mut tokens,
        Some(false),
        "completed",
    )
    .await
    .expect("opting out is not a failure");

    assert!(resolved.published);
    assert!(stored_session(&p.db).pushed_at.is_some());
}

/// V45's own invariant, and the one a mutation at either call site could
/// silently break: a resolution produced while a driver still holds the feature
/// must never wait. Nothing offers Publish there, so holding would leave the
/// merge on the branch with nobody able to publish it — and the project asking
/// for review, which is the strongest form of the request, may not buy it.
///
/// The script has no `push` removed and no teardown removed: it is the happy
/// path, which means a turn that held here would die on the strict double
/// rather than pass.
#[tokio::test]
async fn a_run_that_still_owns_its_branch_publishes_however_the_project_asked() {
    for feature_status in ["running", "awaiting_gate", "syncing_origin"] {
        let p = ports(happy_path(), vec![Arc::new(ScriptedRuntime::default())]);
        open_conflicted(&p.db);
        let (mut cost, mut tokens) = (0.0, 0);

        let resolved = run_with(
            &p,
            &row(),
            None,
            &mut cost,
            &mut tokens,
            Some(true),
            feature_status,
        )
        .await
        .expect("a run's own sync node resolves and publishes");

        assert!(resolved.published, "{feature_status}");
        assert!(
            stored_session(&p.db).pushed_at.is_some(),
            "{feature_status}"
        );
    }
}

/// `git push` exiting zero is a verdict about the command, not about origin.
/// The button's `publish` already refuses to record one it cannot confirm; this
/// path used to write `pushed_at` from the exit code alone, so a push that
/// never landed suppressed the review card forever for a merge the pull request
/// had never seen. Two paths writing one column on opposite evidence rules is
/// what the shared confirmation removes.
#[tokio::test]
async fn a_push_origin_did_not_confirm_leaves_the_resolution_waiting() {
    let script = [
        (PORCELAIN, Ok("")),
        (MERGE_HEAD, Ok("b1b2b3b\n")),
        (ADD_ALL, Ok("")),
        (
            COMMIT,
            Ok("[feature/f-1 c0ffee] chore: resolve sync conflicts"),
        ),
        (HEAD, Ok("c0ffeec\n")),
        (PUSH, Ok("")),
        (CONTAINS, Err("fatal: not an ancestor")),
    ];
    let p = ports(
        ScriptedExec::new(&script),
        vec![Arc::new(ScriptedRuntime::default())],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let resolved = run(&p, &row(), None, &mut cost, &mut tokens)
        .await
        .expect("the resolution is committed either way");

    assert!(!resolved.published);
    let session = stored_session(&p.db);
    assert_eq!(session.status, SyncSessionStatus::Resolved);
    assert_eq!(session.pushed_at, None);
    assert_eq!(
        session.worktree_path.as_deref(),
        Some(WT),
        "an unconfirmed push leaves the tree the review still needs"
    );
}

/// The base a review diff is measured from is the tip the *sync* recorded, and
/// a resolution may never move it.
///
/// `merge_commit^` reads correctly for a merge with one commit on top of it and
/// goes silently wrong the moment the resolver adds a follow-up — which is the
/// shape here: the agent committed on its own, so the sha the turn reads back
/// is not the merge's first parent at all. Nothing on the row could recover the
/// real base afterwards, which is why it is written before the merge and left
/// alone.
#[tokio::test]
async fn a_follow_up_commit_does_not_move_the_diffs_base() {
    let scripted = ScriptedExec::new(&[
        (PORCELAIN, Ok("")),
        (ADD_ALL, Ok("")),
        // The agent staged and committed for itself, so there is nothing left
        // for Demeteo to record — and the tip is its commit, not the merge.
        (PENDING_STATUS, Ok("")),
        (HEAD, Ok("f0110up\n")),
    ])
    .with_queue(MERGE_HEAD, &[Ok("b1b2b3b\n"), Ok("")]);
    let p = ports(scripted, vec![Arc::new(ScriptedRuntime::default())]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let resolved = run_with(&p, &row(), None, &mut cost, &mut tokens, None, "completed")
        .await
        .expect("an agent that committed for itself still resolved the merge");

    let session = stored_session(&p.db);
    assert_eq!(resolved.merge_commit_sha, "f0110up");
    assert_eq!(session.merge_commit_sha.as_deref(), Some("f0110up"));
    assert_eq!(
        session.head_before.as_deref(),
        Some("aaaaaaa"),
        "the pre-merge tip is the diff base and the turn does not own it"
    );
}

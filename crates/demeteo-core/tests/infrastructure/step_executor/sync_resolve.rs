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
use crate::ports::worktree_ops::MergeGate;
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
/// The push, as `run_program` renders it. Byte-identical to the shell string
/// it used to be — only the port changed, because a credential helper needs
/// argv and env, and a shell command string has nowhere to put either.
const PUSH: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 push origin feature/f-1";
/// Read before every push, to see whether the remote needs a credential at all.
const REMOTE_URL: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 remote get-url origin";
/// An ssh remote authenticates itself, so these tests push uncredentialed —
/// which is the shape every assertion here was written against.
const SSH_REMOTE: &str = "git@github.com:acme/widgets.git\n";
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
/// Which tracking branch the open merge is pulling in, as the tree answers it.
const POINTS_AT: &str =
    "git -C /repos/demeteo_wt_sync_feature-f-1 branch --remotes --points-at MERGE_HEAD";
/// What the base side did to the tree while this branch was away. Not a
/// conflict list: these are the paths git merged without asking.
const BASE_MOVES: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 diff --name-status -M \
                          --diff-filter=ADR HEAD...MERGE_HEAD";
/// The project's own checks, as `ProjectSettings.test_command` holds them.
const CHECKS: &str = "npm run checks:code";
const PREPARE: &str = "npm ci";

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
    /// What the turn actually asked for. Nothing downstream reports the prompt,
    /// so without this recorder every claim about what the resolver was told is
    /// only reachable through `build_resolver_prompt` — which is the half that
    /// cannot see whether the turn passed it anything.
    prompts: Arc<std::sync::Mutex<Vec<String>>>,
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
    fn prompt(&self, prompt: &str) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
        self.prompts.lock().unwrap().push(prompt.to_string());
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
    /// The turn's own ending, when the harness imposed one instead of the
    /// agent reporting back. Replaces the `TurnComplete` below, which is the
    /// only difference between a resolver that said "done" and one the CLI cut
    /// off — and the whole of what the tree-is-the-authority tests turn on.
    ends_with_error: bool,
    killed: Arc<std::sync::atomic::AtomicBool>,
    prompts: Arc<std::sync::Mutex<Vec<String>>>,
}

/// What a tripped `--max-turns` reaches the turn loop as: a non-recoverable
/// `cli_error` whose text is the adapter's, because claude's own error result
/// carries no `result` field to quote.
const CAP_TRIPPED: &str = "the agent stopped at its turn cap (--max-turns) without reporting back";

impl ScriptedRuntime {
    /// A turn that pauses after its first token and trips `stop` at `at`.
    fn stopping(at: StopAt, stop: watch::Sender<bool>) -> Self {
        Self {
            linger: Some(std::time::Duration::from_millis(300)),
            stop: Some((at, stop)),
            ..Default::default()
        }
    }

    /// A turn that spent its tokens and was then ended by its own harness,
    /// never reporting back.
    fn cut_off() -> Self {
        Self {
            ends_with_error: true,
            ..Default::default()
        }
    }
    fn spawns(&self) -> Vec<SpawnedWith> {
        self.seen.lock().unwrap().clone()
    }
    fn session_reaped(&self) -> bool {
        self.killed.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
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
        if self.ends_with_error {
            events.push(AgentEvent::Error {
                code: "cli_error".to_string(),
                message: CAP_TRIPPED.to_string(),
                recoverable: false,
                usage: None,
            });
        } else if !matches!(self.stop, Some((StopAt::StreamEnd, _))) {
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
        let prompts = self.prompts.clone();
        Box::pin(async move {
            let session: Arc<dyn AgentSession> = Arc::new(ScriptedSession {
                events,
                linger,
                stop,
                killed,
                prompts,
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
    run_with(
        p,
        step_exec,
        cancel,
        cost,
        tokens,
        None,
        "running",
        MergeGate::default(),
    )
    .await
}

/// The same turn, for a project that named checks. Every assertion about the
/// gate is about *this* argument, so a fixture that could not vary it would
/// leave the gate asserted against the empty one it defaults to.
async fn run_gated(
    p: &Ports,
    step_exec: &StepExecution,
    cost: &mut f64,
    tokens: &mut i64,
    gate: MergeGate<'_>,
) -> Result<ResolvedSync, ResolveSyncError> {
    run_with(p, step_exec, None, cost, tokens, None, "running", gate).await
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
    gate: MergeGate<'_>,
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
        gate,
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
    happy_path_with(&[])
}

/// The same script, plus whatever else a test needs answered beside it — which
/// is only ever the project's own checks. Extending it rather than writing a
/// second copy is what keeps a gated turn asserted against the *ungated* git
/// sequence.
fn happy_path_with(extra: &[(&str, Result<&str, &str>)]) -> ScriptedExec {
    let mut script = vec![
        (PORCELAIN, Ok("")),
        (MERGE_HEAD, Ok("b1b2b3b\n")),
        (POINTS_AT, Ok("  origin/master\n")),
        (BASE_MOVES, Ok("")),
        (ADD_ALL, Ok("")),
        (
            COMMIT,
            Ok("[feature/f-1 c0ffee] chore: resolve sync conflicts"),
        ),
        (HEAD, Ok("c0ffeec\n")),
        (CONTAINS, Ok("")),
        (DISCARD, Ok("")),
        (PRUNE, Ok("")),
        // The teardown's confirmation: the tree really is gone.
        (GIT_DIR, Err("fatal: not a git repository")),
    ];
    script.extend_from_slice(extra);
    ScriptedExec::new(&script).with_programs(&[(REMOTE_URL, Ok(SSH_REMOTE)), (PUSH, Ok(""))])
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
        (DISCARD, Ok("")),
        (PRUNE, Ok("")),
        (GIT_DIR, Err("fatal: not a git repository")),
    ])
    .with_programs(&[(REMOTE_URL, Ok(SSH_REMOTE)), (PUSH, Ok(""))])
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

    // One interleaved list, because the commit goes out as a shell command and
    // the push as a program: read from the two separate recorders, the order
    // between them cannot be seen at all.
    let calls = p.scripted.calls();
    let commit = calls.iter().position(|c| c == COMMIT);
    let push = calls.iter().position(|c| c == PUSH);
    assert!(
        commit.is_some(),
        "the merge the agent resolved has to be committed: {calls:?}"
    );
    assert!(push.is_some(), "and published: {calls:?}");
    assert!(
        commit < push,
        "and committed before it is published: {calls:?}"
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
        !commands.iter().any(|c| c == ADD_ALL || c == COMMIT),
        "a stopped turn stages and commits nothing: {commands:?}"
    );
    assert!(
        !p.scripted.programs().iter().any(|c| c == PUSH),
        "and publishes nothing"
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
        !commands.iter().any(|c| c == ADD_ALL),
        "nothing is staged after a stop: {commands:?}"
    );
    assert!(
        !p.scripted.programs().iter().any(|c| c == PUSH),
        "and nothing is published"
    );
}

/// Every other exit from the turn reaps the resolver; the push's `?` returned
/// straight out. The thread id is minted per turn from the clock, so nothing
/// later reclaims it: the agent process outlives the run and its registry entry
/// accumulates for the life of the app.
#[tokio::test]
async fn a_push_that_failed_still_reaps_the_resolver() {
    let scripted = happy_path().with_programs(&[
        (REMOTE_URL, Ok(SSH_REMOTE)),
        (PUSH, Err("fatal: remote rejected")),
    ]);
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
        !commands.iter().any(|c| c == DISCARD),
        "nothing may be deleted on an answer nobody got: {commands:?}"
    );
    assert!(
        !p.scripted.programs().iter().any(|c| c == PUSH),
        "nor published"
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

    let resolved = run_with(
        &p,
        &row(),
        None,
        &mut cost,
        &mut tokens,
        None,
        "completed",
        MergeGate::default(),
    )
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
        MergeGate::default(),
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
            MergeGate::default(),
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
        (CONTAINS, Err("fatal: not an ancestor")),
    ];
    let p = ports(
        ScriptedExec::new(&script).with_programs(&[(REMOTE_URL, Ok(SSH_REMOTE)), (PUSH, Ok(""))]),
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

    let resolved = run_with(
        &p,
        &row(),
        None,
        &mut cost,
        &mut tokens,
        None,
        "completed",
        MergeGate::default(),
    )
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

/// The turn's exit status is not a reading of the tree.
///
/// A resolver that tripped `--max-turns` four turns after writing a correct
/// resolution was answered with `resolution_failed` and a `raw_error` of
/// "agent error", while the resolution itself sat unstaged in a throwaway
/// worktree the teardown would take with it. The marker check, the `add -A`
/// and the index re-check below are the completion check this file exists to
/// keep honest, and they were all downstream of the early return that read the
/// agent's exit instead of git's index.
#[tokio::test]
async fn a_turn_the_harness_cut_off_over_a_resolved_tree_still_lands_the_resolution() {
    let p = ports(happy_path(), vec![Arc::new(ScriptedRuntime::cut_off())]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run(&p, &row(), None, &mut cost, &mut tokens).await;

    assert_eq!(
        outcome.unwrap().merge_commit_sha,
        "c0ffeec",
        "git said the conflicts were gone; the agent's exit code does not overrule it"
    );
    assert_eq!(
        stored_status(&p.db),
        SyncSessionStatus::Resolved,
        "the session has to read the resolution that is actually on the branch"
    );
    assert!(
        p.scripted.commands().iter().any(|c| c == ADD_ALL),
        "a resolution nothing staged is a resolution the teardown deletes"
    );
}

/// The tokens a cut-off turn burned are still spent.
///
/// `TurnResult::Failed` carried a string and nothing else, so the turn that
/// ran for three minutes and cost real dollars closed its row at `$0.00` —
/// the one class of turn whose spend is least visible reporting none at all.
#[tokio::test]
async fn a_turn_the_harness_cut_off_still_bills_what_it_spent() {
    let p = ports(happy_path(), vec![Arc::new(ScriptedRuntime::cut_off())]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let _ = run(&p, &row(), None, &mut cost, &mut tokens).await;

    assert_eq!(
        cost, 1.25,
        "the cut-off turn's dollars still reach the caller"
    );
    assert_eq!(tokens, 1000, "and so do its tokens");
}

/// When the tree really is unresolved, both halves of why reach the user.
///
/// The tree says *what* is wrong and cannot say why nobody fixed it; the
/// turn's ending is the other half and is worth exactly that much — an
/// explanation, never the verdict.
#[tokio::test]
async fn a_tree_still_conflicted_reports_the_turns_ending_beside_it() {
    let scripted = ScriptedExec::new(&[(ADD_ALL, Ok(""))])
        .with_queue(PORCELAIN, &[Ok("UU src/lib.rs\n"), Ok("UU src/lib.rs\n")])
        .with_files(&[(resolved_file("src/lib.rs").as_str(), Ok("no markers\n"))]);
    let p = ports(scripted, vec![Arc::new(ScriptedRuntime::cut_off())]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run(&p, &row(), None, &mut cost, &mut tokens).await;

    let Err(ResolveSyncError::Failed(reason)) = outcome else {
        panic!("an index git still reports as unmerged is a failed resolution: {outcome:?}");
    };
    assert!(
        reason.contains("did not resolve every conflicted file"),
        "the tree's own verdict has to lead: {reason}"
    );
    assert!(
        reason.contains(CAP_TRIPPED),
        "and the turn's ending is what explains it: {reason}"
    );
    assert_eq!(stored_status(&p.db), SyncSessionStatus::ResolutionFailed);
}

/// The verification line names the project's command, because the agent
/// cannot.
///
/// "Run the project's build / test suite" is not an instruction, it is a
/// search: one resolver resolved its conflict in four turns and spent the
/// other twenty guessing at cargo test targets before its cap ended it.
#[test]
fn the_prompt_names_the_projects_own_test_command() {
    let files = vec!["src/lib.rs".to_string()];
    let prompt = build_resolver_prompt(
        "feature/f-1",
        IncomingSide::Base("master"),
        &files,
        Verification::Gated {
            command: "npm run checks:code",
        },
        &[],
    );

    assert!(
        prompt.contains("`npm run checks:code`"),
        "the command has to be quoted verbatim: {prompt}"
    );
    assert!(
        prompt.contains("Do NOT go looking for another command"),
        "naming it is only half — the other half is not hunting for a second: {prompt}"
    );
    assert!(
        prompt.contains("will not commit it if that comes back red"),
        "the agent has to know the gate is not advisory: {prompt}"
    );
}

/// What the prompt promises is the *refusal*, not the run.
///
/// "Demeteo runs that same command itself before it commits anything" was true
/// of no branch that skips the harness — a prepare that failed, a transport
/// that dropped, a deadline that expired — and it was read by an agent that had
/// just been told to stop rather than look for another command. The refusal is
/// the half that holds in every branch, because every branch that skips it is a
/// branch where nothing came back red.
#[test]
fn the_verification_promise_is_about_the_refusal_not_the_run() {
    let files = vec!["src/lib.rs".to_string()];
    let prompt = build_resolver_prompt(
        "feature/f-1",
        IncomingSide::Base("master"),
        &files,
        Verification::Gated {
            command: "npm run checks:code",
        },
        &[],
    );

    assert!(
        !prompt.contains("Demeteo runs that same command itself before it commits"),
        "a promise the code cannot keep in every branch: {prompt}"
    );
    assert!(
        prompt.contains(
            "Demeteo runs `npm run checks:code` against your resolution and will not \
             commit it if that comes back red"
        ),
        "and the one it can: {prompt}"
    );
}

/// A worktree the project's own prepare command could not build is told so.
///
/// The alternative is what shipped: the agent is told to run a harness that
/// cannot answer here, and told in the same breath not to go looking for
/// another command — so it stops on a red about the missing install, and the
/// resolution it never got to finish is refused for it.
#[test]
fn an_unprepared_worktree_is_told_so_and_not_told_to_verify() {
    let files = vec!["src/lib.rs".to_string()];
    let prompt = build_resolver_prompt(
        "feature/f-1",
        IncomingSide::Base("master"),
        &files,
        Verification::Unprepared {
            prepare: "npm ci",
            command: "npm run checks:code",
        },
        &[],
    );

    assert!(
        prompt.contains("`npm ci` failed here"),
        "the agent has to know which command left the tree like this: {prompt}"
    );
    assert!(
        !prompt.contains("Do NOT go looking for another command"),
        "that instruction is what turns a spurious red into a stopped turn: {prompt}"
    );
    assert!(
        !prompt.contains("will not commit it if that comes back red"),
        "nothing runs the harness on this branch, so the promise would be the \
         lie it replaced: {prompt}"
    );
}

/// The scope clause is the merge's damage, not git's conflict list.
///
/// The two halves are one decision and neither survives alone. A blanket "do
/// not modify any other file" is what a resolver obeyed while leaving a caller
/// of a signature the other side had changed, and the merge commit it pushed
/// reddened every check on the pull request. An unbounded licence in its place
/// buys that back at the price of a refactor nobody merged.
#[test]
fn the_prompts_scope_is_what_the_merge_broke() {
    let files = vec!["src/lib.rs".to_string()];
    let prompt = build_resolver_prompt(
        "feature/f-1",
        IncomingSide::Base("master"),
        &files,
        Verification::Ungated,
        &[],
    );

    assert!(
        prompt.contains("only where the merge itself broke it"),
        "a file git merged silently and broke is in scope: {prompt}"
    );
    assert!(
        !prompt.contains("Do NOT modify any other file"),
        "a blanket ban is what put a tree that does not build on origin: {prompt}"
    );
    assert!(
        prompt.contains("Do not refactor, reformat, or fix anything the merge did not break"),
        "and in scope is not the same as open season: {prompt}"
    );
}

/// A project with no test command gets no verification line at all.
///
/// An instruction the agent cannot satisfy costs more than a missing one: it
/// is what a search is, and a search is turns.
#[test]
fn the_prompt_asks_for_no_verification_when_the_project_names_no_command() {
    let files = vec!["src/lib.rs".to_string()];
    let prompt = build_resolver_prompt(
        "feature/f-1",
        IncomingSide::Base("master"),
        &files,
        Verification::Ungated,
        &[],
    );

    assert!(
        !prompt.contains("verify"),
        "no command, no verification line: {prompt}"
    );
    assert!(
        prompt.contains("Do NOT stage or commit"),
        "the rest of the contract is unchanged: {prompt}"
    );
}

/// The incident, in miniature: a resolver that reported success over a tree
/// that does not build.
///
/// `ADD_ALL` is deliberately unscripted, so a turn that skipped the gate dies
/// on the strict double instead of quietly committing — which is what makes
/// this fail against a *missing* gate as well as a green one.
#[tokio::test]
async fn a_resolution_the_projects_checks_reddened_is_not_committed_or_pushed() {
    let p = ports(
        ScriptedExec::new(&[
            (PORCELAIN, Ok("")),
            (MERGE_HEAD, Ok("b1b2b3b\n")),
            (
                CHECKS,
                Err("Command failed (exit code: Some(101)): error[E0061]: this function takes 3 arguments"),
            ),
        ]),
        vec![Arc::new(ScriptedRuntime::default())],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run_gated(
        &p,
        &row(),
        &mut cost,
        &mut tokens,
        MergeGate {
            prepare: None,
            harness: Some(CHECKS),
        },
    )
    .await;

    let Err(ResolveSyncError::Failed(reason)) = outcome else {
        panic!("a tree that does not build is not a resolved conflict: {outcome:?}");
    };
    assert!(
        reason.contains(CHECKS) && reason.contains("E0061"),
        "the refusal has to carry the command and what it said, or the user \
         cannot act on it: {reason}"
    );
    assert_eq!(stored_status(&p.db), SyncSessionStatus::ResolutionFailed);
    assert!(
        !p.scripted
            .calls()
            .iter()
            .any(|c| c == ADD_ALL || c == COMMIT),
        "a red tree may not be staged or committed: {:?}",
        p.scripted.calls()
    );
    assert!(
        p.scripted.programs().is_empty(),
        "and it may certainly not reach origin: {:?}",
        p.scripted.programs()
    );
}

/// Before `git add -A`, not after.
///
/// A gate that runs afterwards has already staged the index by the time it goes
/// red, so the next attempt's marker check iterates an empty unmerged list and
/// passes over a tree nobody resolved. Nothing else here pins the order.
#[tokio::test]
async fn the_checks_run_before_anything_is_staged() {
    let p = ports(
        happy_path_with(&[(CHECKS, Ok("all checks passed"))]),
        vec![Arc::new(ScriptedRuntime::default())],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    run_gated(
        &p,
        &row(),
        &mut cost,
        &mut tokens,
        MergeGate {
            prepare: None,
            harness: Some(CHECKS),
        },
    )
    .await
    .expect("a green gate lets the resolution through");

    let calls = p.scripted.calls();
    let checks = calls.iter().position(|c| c == CHECKS);
    let staged = calls.iter().position(|c| c == ADD_ALL);
    assert!(
        matches!((checks, staged), (Some(c), Some(s)) if c < s),
        "{calls:?}"
    );
}

/// A build that never ran is not a red build, so the resolution lands.
///
/// The ordering half is where the teeth are: the outcome alone would pass
/// against a turn that never ran the gate at all.
#[tokio::test]
async fn checks_the_transport_cut_short_still_land_the_resolution() {
    let dead = transport_dead();
    let p = ports(
        happy_path_with(&[(CHECKS, Err(dead.as_str()))]),
        vec![Arc::new(ScriptedRuntime::default())],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let resolved = run_gated(
        &p,
        &row(),
        &mut cost,
        &mut tokens,
        MergeGate {
            prepare: None,
            harness: Some(CHECKS),
        },
    )
    .await
    .expect("a dropped connection says nothing about the tree");

    assert!(resolved.published);
    let calls = p.scripted.calls();
    assert!(
        calls.contains(&CHECKS.to_string()),
        "the gate has to have run for this to be about the gate: {calls:?}"
    );
}

/// Where the checks run and how long they may take.
///
/// Every other command in this turn goes through the bare `run_command`, whose
/// `ShellOptions` carry no deadline at all — copying that idiom here would hang
/// a resolution forever on a wedged build.
#[tokio::test]
async fn the_checks_run_in_the_worktree_under_the_projects_deadline() {
    let p = ports(
        happy_path_with(&[(CHECKS, Ok(""))]),
        vec![Arc::new(ScriptedRuntime::default())],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    run_gated(
        &p,
        &row(),
        &mut cost,
        &mut tokens,
        MergeGate {
            prepare: None,
            harness: Some(CHECKS),
        },
    )
    .await
    .expect("a green gate lets the resolution through");

    let seen = p
        .scripted
        .options()
        .into_iter()
        .zip(p.scripted.commands())
        .find(|(_, cmd)| cmd == CHECKS)
        .map(|(opts, _)| opts)
        .expect("the checks were run");
    assert_eq!(
        seen.cwd.as_deref(),
        Some(WT),
        "the merge is in the worktree"
    );
    assert!(
        seen.login_shell && seen.interactive,
        "a user-authored command needs the shell its toolchain shims live in"
    );
    assert_eq!(
        seen.timeout,
        Some(std::time::Duration::from_secs(1800)),
        "the run's own wall cap, not a second knob and not no cap at all"
    );
}

/// The agent is asked to run the harness during its turn, so the tree it runs
/// it in has to be prepared by then.
///
/// `BASE_MOVES` is the marker: it is issued after the spawn and before the
/// prompt, so a prepare that drifts back below the turn lands after it here.
#[tokio::test]
async fn prepare_runs_before_the_agent_is_ever_prompted() {
    let runtime = Arc::new(ScriptedRuntime::default());
    let p = ports(
        happy_path_with(&[(PREPARE, Ok("added 900 packages")), (CHECKS, Ok(""))]),
        vec![runtime.clone()],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    run_gated(
        &p,
        &row(),
        &mut cost,
        &mut tokens,
        MergeGate {
            prepare: Some(PREPARE),
            harness: Some(CHECKS),
        },
    )
    .await
    .expect("a green gate lets the resolution through");

    let calls = p.scripted.calls();
    let prepared = calls.iter().position(|c| c == PREPARE);
    let spawned = calls.iter().position(|c| c == BASE_MOVES);
    assert!(
        matches!((prepared, spawned), (Some(p), Some(s)) if p < s),
        "{calls:?}"
    );
    assert_eq!(
        runtime.prompts().len(),
        1,
        "and the turn it precedes actually happened"
    );
}

/// Once per resolution, not once per stage that wants a prepared tree.
///
/// Nothing downstream reports that prepare ran, so a second call — a gate that
/// went back to running its own prepare after the turn — costs an `npm ci` per
/// resolution and is invisible everywhere else.
#[tokio::test]
async fn prepare_runs_exactly_once_per_resolution() {
    let p = ports(
        happy_path_with(&[(PREPARE, Ok("added 900 packages")), (CHECKS, Ok(""))]),
        vec![Arc::new(ScriptedRuntime::default())],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    run_gated(
        &p,
        &row(),
        &mut cost,
        &mut tokens,
        MergeGate {
            prepare: Some(PREPARE),
            harness: Some(CHECKS),
        },
    )
    .await
    .expect("a green gate lets the resolution through");

    let calls = p.scripted.calls();
    assert_eq!(
        calls.iter().filter(|c| *c == PREPARE).count(),
        1,
        "{calls:?}"
    );
    assert_eq!(
        calls.iter().filter(|c| *c == CHECKS).count(),
        1,
        "{calls:?}"
    );
}

/// A tree the project's own prepare command could not build cannot answer the
/// question, so nothing asks it — and the resolution lands rather than being
/// refused over an environment fault Demeteo caused nothing of.
///
/// The clean half lands the same merge under the same broken registry. Refusing
/// here would decide the two by whether git happened to hit a textual conflict.
#[tokio::test]
async fn an_unprepared_tree_lands_the_resolution_without_running_the_harness() {
    let p = ports(
        happy_path_with(&[(PREPARE, Err("npm ERR! network ETIMEDOUT"))]),
        vec![Arc::new(ScriptedRuntime::default())],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let resolved = run_gated(
        &p,
        &row(),
        &mut cost,
        &mut tokens,
        MergeGate {
            prepare: Some(PREPARE),
            harness: Some(CHECKS),
        },
    )
    .await
    .expect("a worktree nobody could prepare says nothing about the merge");

    assert!(resolved.published);
    let calls = p.scripted.calls();
    assert!(
        !calls.iter().any(|c| c == CHECKS),
        "a harness run here would report the missing install as a broken merge: {calls:?}"
    );
}

/// And the agent is told which command left the tree like this, in the prompt
/// it is handed — the half `build_resolver_prompt` alone cannot see.
#[tokio::test]
async fn an_unprepared_turn_hands_the_agent_the_unprepared_prompt() {
    let runtime = Arc::new(ScriptedRuntime::default());
    let p = ports(
        happy_path_with(&[(PREPARE, Err("npm ERR! network ETIMEDOUT"))]),
        vec![runtime.clone()],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    run_gated(
        &p,
        &row(),
        &mut cost,
        &mut tokens,
        MergeGate {
            prepare: Some(PREPARE),
            harness: Some(CHECKS),
        },
    )
    .await
    .expect("a worktree nobody could prepare says nothing about the merge");

    let prompts = runtime.prompts();
    assert!(prompts[0].contains("`npm ci` failed here"), "{prompts:?}");
    assert!(
        !prompts[0].contains("Do NOT go looking for another command"),
        "{prompts:?}"
    );
}

/// A stop that arrives while prepare is running costs no agent at all.
///
/// Prepare sits above the spawn, which is what makes this reachable: before it
/// moved there, a stop during prepare was a stop *after* a turn that had
/// already been paid for.
#[tokio::test]
async fn a_stop_during_prepare_spawns_no_agent() {
    let runtime = Arc::new(ScriptedRuntime::default());
    let (tx, rx) = watch::channel(false);
    let p = ports(
        happy_path_with(&[(PREPARE, Ok("added 900 packages"))]).with_stop_on(PREPARE, tx),
        vec![runtime.clone()],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run_with(
        &p,
        &row(),
        Some(rx),
        &mut cost,
        &mut tokens,
        None,
        "running",
        MergeGate {
            prepare: Some(PREPARE),
            harness: Some(CHECKS),
        },
    )
    .await;

    assert!(matches!(outcome, Err(ResolveSyncError::Cancelled(_))));
    assert!(
        p.scripted.calls().iter().any(|c| c == PREPARE),
        "the stop has to have reached prepare for this to be about prepare: {:?}",
        p.scripted.calls()
    );
    assert!(runtime.prompts().is_empty(), "and no turn was paid for");
}

/// The no-regression half: a project that named no checks resolves exactly as
/// it did before the gate existed, and runs nothing extra to do it.
#[tokio::test]
async fn a_project_that_names_no_checks_resolves_exactly_as_it_did() {
    let p = ports(happy_path(), vec![Arc::new(ScriptedRuntime::default())]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    run(&p, &row(), None, &mut cost, &mut tokens)
        .await
        .expect("an absent gate withholds nothing");

    assert_eq!(
        p.scripted.commands(),
        vec![
            PORCELAIN.to_string(),
            MERGE_HEAD.to_string(),
            POINTS_AT.to_string(),
            BASE_MOVES.to_string(),
            ADD_ALL.to_string(),
            PORCELAIN.to_string(),
            MERGE_HEAD.to_string(),
            COMMIT.to_string(),
            HEAD.to_string(),
            CONTAINS.to_string(),
            DISCARD.to_string(),
            PRUNE.to_string(),
            GIT_DIR.to_string(),
        ],
        "an empty gate runs no command of its own"
    );
}

/// The file that broke the pull request, named in the prompt that would have
/// prevented it.
///
/// `crates/demeteo-core/tests/application/run_view.rs` existed on master alone,
/// so git merged it silently, so it was in no conflict list — and it called a
/// constructor the branch had given a fourth parameter. A resolver reading only
/// the conflicted paths could not have known it was there.
#[test]
fn the_prompt_names_what_the_base_side_moved() {
    let files = vec!["crates/demeteo-core/src/application/run_view.rs".to_string()];
    let moves = vec!["A\tcrates/demeteo-core/tests/application/run_view.rs".to_string()];

    let prompt = build_resolver_prompt(
        "feature/f-1",
        IncomingSide::Base("master"),
        &files,
        Verification::Ungated,
        &moves,
    );

    assert!(
        prompt.contains("A\tcrates/demeteo-core/tests/application/run_view.rs"),
        "the moved file has to be in the prompt verbatim: {prompt}"
    );
    assert!(
        prompt.contains("Git merged them without asking"),
        "and the resolver has to be told why a silent merge is the dangerous half: {prompt}"
    );
}

/// A merge that moved hundreds of files is where the hint would drown the
/// conflict it was meant to aim at, so past the cap it hands over the command
/// instead of the list.
#[test]
fn a_long_list_of_base_side_moves_is_capped_and_says_where_the_rest_is() {
    let files = vec!["src/lib.rs".to_string()];
    let moves: Vec<String> = (0..200).map(|i| format!("A\tsrc/gen/f{}.rs", i)).collect();

    let prompt = build_resolver_prompt(
        "feature/f-1",
        IncomingSide::Base("master"),
        &files,
        Verification::Ungated,
        &moves,
    );

    assert!(prompt.contains("A\tsrc/gen/f39.rs"), "{prompt}");
    assert!(
        !prompt.contains("A\tsrc/gen/f40.rs"),
        "the 41st entry is past the cap: {prompt}"
    );
    assert!(
        prompt.contains("…and 160 more"),
        "the tail has to be counted, not dropped: {prompt}"
    );
    assert!(
        prompt.contains("git diff --name-status -M --diff-filter=ADR HEAD...MERGE_HEAD"),
        "and reachable in one call: {prompt}"
    );
}

/// The incident's own arithmetic: the entry that mattered was the 69th of 252,
/// and a cap that took git's first forty would have dropped it.
///
/// Path order is not relevance order, and this is the whole of what the
/// ordering buys — a hint that is capped where the answer is not is a hint that
/// reads as reassurance.
#[test]
fn a_move_naming_a_conflicted_file_leads_however_long_the_list_is() {
    let files = vec!["crates/demeteo-core/src/application/run_view.rs".to_string()];
    let moved = "A\tcrates/demeteo-core/tests/application/run_view.rs";
    let mut moves: Vec<String> = (0..68).map(|i| format!("A\tsrc/gen/a{}.rs", i)).collect();
    moves.push(moved.to_string());
    moves.extend((0..183).map(|i| format!("A\tsrc/gen/z{}.rs", i)));

    let prompt = build_resolver_prompt(
        "feature/f-1",
        IncomingSide::Base("master"),
        &files,
        Verification::Ungated,
        &moves,
    );

    assert!(
        prompt.contains(moved),
        "the file that broke the build is the one entry that may not be capped away: {prompt}"
    );
    assert!(
        prompt.contains("…and 212 more"),
        "and the rest is still bounded: {prompt}"
    );
}

/// The list is `--diff-filter=ADR`, and the agent is told so.
///
/// A base side that *modified* a trait, a struct field or a signature merges
/// just as silently as one that moved a file, and none of that is in this
/// section. Read as complete — which is how the header read — it retires the
/// question it was meant to raise.
#[test]
fn the_hint_says_the_files_it_leaves_out() {
    let files = vec!["src/lib.rs".to_string()];
    let moves = vec!["A\tsrc/added.rs".to_string()];

    let prompt = build_resolver_prompt(
        "feature/f-1",
        IncomingSide::Base("master"),
        &files,
        Verification::Ungated,
        &moves,
    );

    assert!(
        prompt.contains("only *modified* are not in that list"),
        "the filter's exclusion has to be the agent's to see, not only the rustdoc's: {prompt}"
    );
    assert!(
        prompt.contains("`git diff --name-status -M HEAD...MERGE_HEAD`"),
        "and reachable, or it is a warning with nothing behind it: {prompt}"
    );
}

/// A filename is evidence only while it is rare.
///
/// Promotion costs a place under the cap, so promoting every `mod.rs` the base
/// side touched pushes the one entry that names a real relationship below the
/// line — the failure the ordering exists to prevent, reached from the other
/// side.
#[test]
fn a_shared_basename_leads_only_while_it_is_distinctive() {
    let files = vec![
        "src/a/mod.rs".to_string(),
        "crates/demeteo-core/src/application/run_view.rs".to_string(),
    ];
    let crowd: Vec<String> = ["b", "c", "d", "e"]
        .iter()
        .map(|d| format!("A\tsrc/{}/mod.rs", d))
        .collect();
    let aimed = "A\tcrates/demeteo-core/tests/application/run_view.rs";
    let mut moves = crowd.clone();
    moves.push(aimed.to_string());

    let ordered = aimed_first(&files, &moves);

    assert_eq!(
        ordered.first(),
        Some(&aimed),
        "one candidate for a conflicted name is an aim; four are a reshuffle: {ordered:?}"
    );
}

/// The frequency rule ranks basenames, and never demotes a path.
///
/// A move that names a conflicted path *itself* is the sharpest relationship
/// there is, and it sits in exactly the tree — a directory-wide `mod.rs`
/// reshuffle — where the rule above is busiest.
#[test]
fn a_move_naming_the_conflicted_path_itself_outranks_a_crowded_basename() {
    let files = vec!["src/a/mod.rs".to_string()];
    let mut moves: Vec<String> = ["b", "c", "d", "e"]
        .iter()
        .map(|d| format!("A\tsrc/{}/mod.rs", d))
        .collect();
    moves.push("D\tsrc/a/mod.rs".to_string());

    let ordered = aimed_first(&files, &moves);

    assert_eq!(
        ordered.first(),
        Some(&"D\tsrc/a/mod.rs"),
        "the base side deleting the file under the conflict is the one thing that cannot \
         be crowded out: {ordered:?}"
    );
}

/// `--name-status` is tab-separated, and a path may contain spaces.
///
/// Split on whitespace, `src/app/run view.rs` becomes `src/app/run` and
/// `view.rs`, neither of which is a filename anything conflicted shares — so
/// the one entry that aims at the conflict reads as noise.
#[test]
fn a_path_with_a_space_is_one_path() {
    let files = vec!["src/other/run view.rs".to_string()];
    let aimed = "A\tsrc/app/run view.rs";
    let mut moves: Vec<String> = (0..5).map(|i| format!("A\tsrc/gen/f{}.rs", i)).collect();
    moves.push(aimed.to_string());

    let ordered = aimed_first(&files, &moves);

    assert_eq!(ordered.first(), Some(&aimed), "{ordered:?}");
}

/// A merge with nothing to say about itself says nothing.
#[test]
fn a_merge_that_moved_nothing_adds_no_section() {
    let files = vec!["src/lib.rs".to_string()];

    let prompt = build_resolver_prompt(
        "feature/f-1",
        IncomingSide::Base("master"),
        &files,
        Verification::Ungated,
        &[],
    );

    assert!(
        !prompt.contains("also added, moved or deleted"),
        "an empty list is not a section: {prompt}"
    );
}

/// The read reaches the prompt, spelled the way git answers it.
///
/// The two halves are only wired together here: the pure function cannot see
/// whether the turn passed it anything, and the command string is invisible to
/// every other assertion in this file.
#[tokio::test]
async fn the_resolver_is_told_what_the_merge_moved_under_it() {
    let runtime = Arc::new(ScriptedRuntime::default());
    let p = ports(
        happy_path_with(&[(
            BASE_MOVES,
            Ok("A\tcrates/demeteo-core/tests/application/run_view.rs\n"),
        )]),
        vec![runtime.clone()],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    run(&p, &row(), None, &mut cost, &mut tokens)
        .await
        .expect("a hint withholds nothing");

    let prompts = runtime.prompts();
    assert!(
        prompts[0].contains("A\tcrates/demeteo-core/tests/application/run_view.rs"),
        "the merge's own damage has to reach the agent: {prompts:?}"
    );
}

/// The hint is the one read in this turn issued with an agent already alive.
///
/// Unbounded, a transport that stops answering mid-read holds that process open
/// for as long as it stays silent — the wedge shape, one step below where it is
/// usually found. The deadline is the run's own `fast_timeout_s`, so a user who
/// tightens "how long may something be silent" tightens this with it.
#[tokio::test]
async fn the_base_move_hint_is_read_under_a_deadline() {
    let p = ports(happy_path(), vec![Arc::new(ScriptedRuntime::default())]);
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    run(&p, &row(), None, &mut cost, &mut tokens)
        .await
        .expect("a hint withholds nothing");

    let seen = p
        .scripted
        .options()
        .into_iter()
        .zip(p.scripted.commands())
        .find(|(_, cmd)| cmd == BASE_MOVES)
        .map(|(opts, _)| opts)
        .expect("the hint was read");
    assert_eq!(
        seen.timeout,
        Some(std::time::Duration::from_secs(300)),
        "the run's own silence threshold, not no deadline at all"
    );
}

/// And a deadline that expires is a hint nobody got, not an empty answer.
#[tokio::test]
async fn a_hint_that_ran_out_of_time_is_no_hint() {
    let expired = format!(
        "{}npm run checks:code exceeded 300s",
        crate::ports::execution::TIMEOUT_ERROR_PREFIX
    );
    let runtime = Arc::new(ScriptedRuntime::default());
    let p = ports(
        happy_path_with(&[(BASE_MOVES, Err(expired.as_str()))]),
        vec![runtime.clone()],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let resolved = run(&p, &row(), None, &mut cost, &mut tokens)
        .await
        .expect("a hint that timed out is not a failed resolution");

    assert!(resolved.published);
    assert!(
        runtime.session_reaped(),
        "and the agent the read was holding open is still reaped"
    );
    let prompts = runtime.prompts();
    assert!(
        !prompts[0].contains("also added, moved or deleted"),
        "an answer that never arrived may not be rendered as an empty one: {prompts:?}"
    );
}

/// A hint is not a gate: an unreadable answer costs the section, not the turn.
#[tokio::test]
async fn an_unreadable_base_diff_still_resolves_and_says_nothing_about_it() {
    let dead = transport_dead();
    let runtime = Arc::new(ScriptedRuntime::default());
    let p = ports(
        happy_path_with(&[(BASE_MOVES, Err(dead.as_str()))]),
        vec![runtime.clone()],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let resolved = run(&p, &row(), None, &mut cost, &mut tokens)
        .await
        .expect("a hint nobody could read is not a failed resolution");

    assert!(resolved.published);
    let prompts = runtime.prompts();
    assert!(
        !prompts[0].contains("also added, moved or deleted"),
        "an answer nobody got may not be rendered as an empty one: {prompts:?}"
    );
}

/// Which branch a merge is pulling in, over everything `git branch
/// --remotes --points-at` can answer.
///
/// The empty and unparseable rows are the point of the table: a divergence
/// reconcile merges `origin/<feature>` and an ordinary sync merges
/// `origin/<base>`, so an answer that establishes neither may not be rounded to
/// the common one — the prompt built on it would tell the resolver whose
/// commits it is about, wrongly.
#[test]
fn the_incoming_side_is_whichever_tracking_tip_merge_head_sits_on() {
    let cases: &[(&str, IncomingSide)] = &[
        ("  origin/master\n", IncomingSide::Base("master")),
        (
            "  origin/feature/f-1\n",
            IncomingSide::OwnBranch("feature/f-1"),
        ),
        // Both tips on one commit is a base merge with nothing to conflict
        // over, so the branch that is only ever merged by a reconcile wins.
        (
            "  origin/master\n  origin/feature/f-1\n",
            IncomingSide::OwnBranch("feature/f-1"),
        ),
        ("", IncomingSide::Unknown),
        ("  origin/HEAD -> origin/master\n", IncomingSide::Unknown),
        (
            "fatal: malformed object name MERGE_HEAD",
            IncomingSide::Unknown,
        ),
        ("  origin/feature/f-10\n", IncomingSide::Unknown),
        ("  upstream/master\n", IncomingSide::Unknown),
    ];

    for (points_at, want) in cases {
        assert_eq!(
            tracking_tip_at_merge_head(points_at, "master", "feature/f-1"),
            *want,
            "{points_at:?}"
        );
    }
}

/// A reconcile is told the other side is this same branch, not upstream.
///
/// "We just merged origin/master into feature/f-1" is what the prompt said
/// whatever was merged, and after a divergence reconcile it is false: the
/// incoming commits are a colleague's work on the user's own branch, and a
/// resolver that reads them as upstream's has a rule for which side to defer
/// to.
#[test]
fn a_reconcile_tells_the_resolver_whose_commits_the_other_side_carries() {
    let files = vec!["src/lib.rs".to_string()];

    let prompt = build_resolver_prompt(
        "feature/f-1",
        IncomingSide::OwnBranch("feature/f-1"),
        &files,
        Verification::Ungated,
        &["A\tsrc/added.rs".to_string()],
    );

    assert!(
        prompt.contains("We just merged origin/feature/f-1 into feature/f-1"),
        "the branch actually merged is the one to name: {prompt}"
    );
    assert!(
        prompt.contains("this same branch as origin holds it")
            && prompt.contains("not a change from upstream"),
        "and what that makes the other side: {prompt}"
    );
    assert!(
        !prompt.contains("origin/master"),
        "a branch this merge never touched may not appear at all: {prompt}"
    );
}

/// An unread incoming side names no branch rather than the likely one.
///
/// The section still earns its place — the moved files are read from
/// `MERGE_HEAD` and are true whichever branch that is — but a name is the one
/// part of it that cannot be guessed.
#[test]
fn an_unnamed_incoming_side_names_no_branch_at_all() {
    let files = vec!["src/lib.rs".to_string()];

    let prompt = build_resolver_prompt(
        "feature/f-1",
        IncomingSide::Unknown,
        &files,
        Verification::Ungated,
        &["A\tsrc/added.rs".to_string()],
    );

    assert!(
        !prompt.contains("origin/"),
        "nothing was established, so nothing is claimed: {prompt}"
    );
    assert!(
        prompt.contains("A\tsrc/added.rs") && prompt.contains("Git merged them without asking"),
        "the moves are read from MERGE_HEAD and hold either way: {prompt}"
    );
}

/// The tree is what names the incoming side, not the row.
///
/// The session row this turn runs against says `base_branch: master`, and the
/// merge open in the worktree is `origin/feature/f-1`. Only the probe can tell
/// them apart, so a prompt built from the row alone reads as correct here and
/// is not.
#[tokio::test]
async fn a_reconcile_in_the_tree_outranks_the_rows_base_branch() {
    let runtime = Arc::new(ScriptedRuntime::default());
    let p = ports(
        happy_path_with(&[(POINTS_AT, Ok("  origin/feature/f-1\n"))]),
        vec![runtime.clone()],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    run(&p, &row(), None, &mut cost, &mut tokens)
        .await
        .expect("a reconcile resolves like any other merge");

    let prompts = runtime.prompts();
    assert!(
        prompts[0].contains("We just merged origin/feature/f-1 into feature/f-1"),
        "{prompts:?}"
    );
    assert!(
        !prompts[0].contains("origin/master"),
        "the row's base branch is not the incoming side here: {prompts:?}"
    );
}

/// A probe nobody could read costs the name, not the turn — and it is read
/// under the same deadline as the move hint, for the same reason: an agent is
/// already alive while it runs.
#[tokio::test]
async fn an_unreadable_incoming_side_still_resolves_and_names_no_branch() {
    let dead = transport_dead();
    let runtime = Arc::new(ScriptedRuntime::default());
    let p = ports(
        happy_path_with(&[(POINTS_AT, Err(dead.as_str()))]),
        vec![runtime.clone()],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let resolved = run(&p, &row(), None, &mut cost, &mut tokens)
        .await
        .expect("a probe nobody could read is not a failed resolution");

    assert!(resolved.published);
    let prompts = runtime.prompts();
    assert!(
        !prompts[0].contains("origin/"),
        "an answer nobody got may not be rendered as the likely one: {prompts:?}"
    );
    let seen = p
        .scripted
        .options()
        .into_iter()
        .zip(p.scripted.commands())
        .find(|(_, cmd)| cmd == POINTS_AT)
        .map(|(opts, _)| opts)
        .expect("the probe was issued");
    assert_eq!(
        seen.timeout,
        Some(std::time::Duration::from_secs(300)),
        "the run's own silence threshold, not no deadline at all"
    );
}

/// The repair prompt is about a build, not about a merge.
///
/// Rebuilt from `build_resolver_prompt` it would open by asking for markers a
/// turn has already removed, which is an instruction over nothing: the agent
/// no-ops, the gate reddens on the same error, and the ladder pays a harness
/// run per attempt to learn it again.
#[test]
fn the_repair_prompt_carries_the_command_and_what_it_said() {
    let prompt = build_repair_prompt(
        "feature/f-1",
        CHECKS,
        "error[E0061]: this function takes 3 arguments",
    );

    assert!(
        prompt.contains(CHECKS) && prompt.contains("E0061"),
        "the agent cannot fix what it was not shown: {prompt}"
    );
    assert!(
        !prompt.contains("Read the conflict markers"),
        "there are none left to read: {prompt}"
    );
    assert!(
        !prompt.contains("A merge conflict was detected"),
        "the conflict is resolved — what is broken is the build: {prompt}"
    );
    assert!(
        prompt.contains("Do NOT stage or commit"),
        "the rest of the contract is unchanged: {prompt}"
    );
}

/// A red gate buys the resolver a turn with the compiler's own output in it.
///
/// This is the whole of F2: without it the refusal goes only to the human, and
/// the caller's retry ladder — which rebuilds the resolution prompt from the
/// tree — re-opens over a marker-free worktree with nothing named to fix.
#[tokio::test]
async fn a_red_gate_runs_a_repair_turn_carrying_the_compiler_output() {
    let runtime = Arc::new(ScriptedRuntime::default());
    let p = ports(
        happy_path().with_queue(
            CHECKS,
            &[
                Err("Command failed (exit code: Some(101)): error[E0061]: this function takes 3 arguments"),
                Ok("all checks passed"),
            ],
        ),
        vec![runtime.clone()],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let resolved = run_gated(
        &p,
        &row(),
        &mut cost,
        &mut tokens,
        MergeGate {
            prepare: None,
            harness: Some(CHECKS),
        },
    )
    .await
    .expect("a tree the resolver was given a chance to fix is a resolution");

    let prompts = runtime.prompts();
    assert_eq!(
        prompts.len(),
        2,
        "the red gate bought a second turn: {prompts:?}"
    );
    assert!(
        prompts[1].contains("E0061") && prompts[1].contains(CHECKS),
        "carrying what the harness said and how to re-run it: {prompts:?}"
    );
    assert!(
        !prompts[1].contains("Read the conflict markers"),
        "and not asking for markers the first turn removed: {prompts:?}"
    );
    assert_eq!(
        runtime.spawns().len(),
        2,
        "the session before it was reaped ahead of the gate, so this is a fresh one"
    );
    assert!(resolved.published);
    assert!(
        p.scripted.calls().iter().any(|c| c == COMMIT),
        "and the repaired tree lands: {:?}",
        p.scripted.calls()
    );
}

/// The repair round spends the rest of the resolution's budget, not a copy of it.
///
/// `max_budget_usd` is one ceiling for the resolution. Spawning a fresh session
/// per round is what makes handing it the raw field a way to spend the cap
/// twice, so the round is handed the remainder instead.
#[tokio::test]
async fn a_repair_round_is_handed_what_is_left_of_the_budget() {
    let runtime = Arc::new(ScriptedRuntime::default());
    let p = ports(
        happy_path().with_queue(
            CHECKS,
            &[
                Err("Command failed (exit code: Some(101)): error[E0061]: this function takes 3 arguments"),
                Ok("all checks passed"),
            ],
        ),
        vec![runtime.clone()],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    run_gated(
        &p,
        &row(),
        &mut cost,
        &mut tokens,
        MergeGate {
            prepare: None,
            harness: Some(CHECKS),
        },
    )
    .await
    .expect("the repaired tree lands");

    let spawns = runtime.spawns();
    assert_eq!(
        spawns.len(),
        2,
        "a red gate bought a second round: {spawns:?}"
    );
    assert_eq!(
        spawns[0].max_budget_usd,
        Some(10.0),
        "the first round is handed the whole ceiling: {spawns:?}"
    );
    assert_eq!(
        spawns[1].max_budget_usd,
        Some(8.75),
        "and the second what the first left of it: {spawns:?}"
    );
}

/// One repair round, not a ladder of them.
///
/// Every round is a full harness run, and a resolver that could not make the
/// tree build with the output in front of it will not on a third read.
#[tokio::test]
async fn a_second_red_gate_refuses_rather_than_buying_a_third_turn() {
    let red =
        "Command failed (exit code: Some(101)): error[E0061]: this function takes 3 arguments";
    let runtime = Arc::new(ScriptedRuntime::default());
    let p = ports(
        happy_path().with_queue(CHECKS, &[Err(red), Err(red)]),
        vec![runtime.clone()],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);

    let outcome = run_gated(
        &p,
        &row(),
        &mut cost,
        &mut tokens,
        MergeGate {
            prepare: None,
            harness: Some(CHECKS),
        },
    )
    .await;

    let Err(ResolveSyncError::Failed(reason)) = outcome else {
        panic!("a tree that still does not build is not a resolved conflict: {outcome:?}");
    };
    assert!(
        reason.contains(CHECKS) && reason.contains("E0061"),
        "{reason}"
    );
    assert_eq!(
        runtime.prompts().len(),
        2,
        "two turns, and the third is the user's"
    );
    assert!(
        !p.scripted
            .calls()
            .iter()
            .any(|c| c == ADD_ALL || c == COMMIT),
        "and a red tree is still not staged: {:?}",
        p.scripted.calls()
    );
    assert_eq!(stored_status(&p.db), SyncSessionStatus::ResolutionFailed);
}

/// The staleness boundary: a refusal never outlives the turn that read it.
///
/// The words the harness said are true of one worktree at one moment. The node
/// path force-removes that worktree and re-merges before it asks again, so a
/// remembered refusal would send the next resolver to fix an error that no
/// longer exists — a wrong prior-failure section is worse than none, because it
/// reads as a high-confidence claim about a tree nobody looked at. Nothing here
/// carries one past the loop that built it, and this is the assertion that
/// notices if something starts to.
#[tokio::test]
async fn a_later_resolution_is_not_told_about_the_one_before_it() {
    let red =
        "Command failed (exit code: Some(101)): error[E0061]: this function takes 3 arguments";
    let runtime = Arc::new(ScriptedRuntime::default());
    let p = ports(
        happy_path().with_queue(CHECKS, &[Err(red), Err(red), Ok("all checks passed")]),
        vec![runtime.clone()],
    );
    open_conflicted(&p.db);
    let (mut cost, mut tokens) = (0.0, 0);
    let gate = MergeGate {
        prepare: None,
        harness: Some(CHECKS),
    };

    let refused = run_gated(&p, &row(), &mut cost, &mut tokens, gate).await;
    assert!(
        matches!(refused, Err(ResolveSyncError::Failed(_))),
        "{refused:?}"
    );

    run_gated(&p, &row(), &mut cost, &mut tokens, gate)
        .await
        .expect("the second ask is a resolution of its own");

    let prompts = runtime.prompts();
    assert_eq!(
        prompts.len(),
        3,
        "two turns refused, then one asked again: {prompts:?}"
    );
    assert!(
        prompts[2].contains("A merge conflict was detected"),
        "a new ask opens on the merge in front of it: {prompts:?}"
    );
    assert!(
        !prompts[2].contains("E0061") && !prompts[2].contains("Your conflict resolution in"),
        "and carries nothing of a build that was about a tree since re-merged: {prompts:?}"
    );
}

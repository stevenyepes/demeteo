// Tests extracted from `crates/demeteo-core/src/application/ask/turn.rs` (mirrored-tests convention). `super` = that module.

use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio_stream::Stream;

use super::*;
use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::local::execution::LocalSubprocessAdapter;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::application::ask::worktree;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::agent_event::{AgentEvent, StopReason, Usage};
use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId, LOCAL_MACHINE};
use crate::domain::models::{
    Availability, PathContainment, Platform, Project, Repository, SessionInfo, WindowsAgentShell,
};
use crate::ports::agent_runtime::{
    AgentCapabilities, AgentContext, AgentRuntime, AgentSession, AgentStartFuture,
    PersonalizationSupport,
};
use crate::ports::ask::AskPort;
use crate::ports::db::ProjectRepository;
use crate::ports::execution::{
    ExecutionPort, InteractiveHandle, ProgramRequest, SftpEntry, ShellOptions,
};

#[test]
fn ask_may_read_and_run_but_never_write() {
    let p = ask_permissions();
    assert_eq!(p.read_fs, Access::Allow);
    assert_eq!(p.write_fs, Access::Deny);
    assert_eq!(p.execute, Access::Allow);
    assert_eq!(p.network, Access::Allow);
}

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures: a real git checkout (so `worktree::ensure` actually provisions
// one) plus a fake `AgentRuntime`/`AgentSession` pair (so `stream_agent_turn`
// completes without a real CLI process).
// ─────────────────────────────────────────────────────────────────────────────

const FAKE_KIND: &str = "fake-agent";

/// Streams a fixed script of events, then closes — the same
/// `ScriptedSession` shape `tests/infrastructure/step_executor/sync_resolve.rs`
/// uses to drive `stream_agent_turn` without a real process.
struct FakeSession {
    events: Vec<AgentEvent>,
}

impl AgentSession for FakeSession {
    fn session_id(&self) -> &str {
        "fake-session"
    }
    fn prompt(&self, _text: &str) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
        Box::pin(tokio_stream::iter(self.events.clone()))
    }
    fn cancel(&self) -> Result<(), String> {
        Ok(())
    }
    fn set_mode(&self, _mode_id: &str) -> Result<(), String> {
        Ok(())
    }
    fn set_config_option(&self, _config_id: &str, _value: &str) -> Result<(), String> {
        Ok(())
    }
    fn session_info(&self) -> SessionInfo {
        SessionInfo::default()
    }
}

struct FakeRuntime {
    events: Vec<AgentEvent>,
}

#[async_trait]
impl AgentRuntime for FakeRuntime {
    fn kind(&self) -> &'static str {
        FAKE_KIND
    }
    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            display_label: "Fake",
            lists_models: false,
            model_listing: None,
            default_model: None,
            effort_levels: &[],
            personalization: PersonalizationSupport::Native,
            path_containment: PathContainment::UNFENCED,
            windows_agent_shell: WindowsAgentShell::Unknown,
        }
    }
    async fn availability(&self, _exec: &dyn ExecutionPort, _machine_id: &str) -> Availability {
        Availability::Installed
    }
    fn install_command(&self) -> &'static str {
        "echo fake"
    }
    fn start(&self, _ctx: AgentContext) -> AgentStartFuture<'_> {
        let events = self.events.clone();
        Box::pin(async move { Ok(Arc::new(FakeSession { events }) as Arc<dyn AgentSession>) })
    }
}

/// A real git repository at exactly the location [`fixture`] derived, so
/// `resolve`/`ensure` operate on a checkout that actually exists — mirroring
/// `tests/application/ask/worktree.rs::init_repo_at`.
async fn init_repo_at(dir: &Path) {
    let exec = LocalSubprocessAdapter::new();
    std::fs::create_dir_all(dir).expect("creates the repo directory");
    let repo = dir.to_string_lossy().to_string();
    exec.run_command("local", &format!("git -C \"{repo}\" init -b main"))
        .await
        .expect("git init succeeds");
    exec.run_command(
        "local",
        &format!("git -C \"{repo}\" config user.email \"ci@demeteo.com\""),
    )
    .await
    .expect("git config succeeds");
    exec.run_command(
        "local",
        &format!("git -C \"{repo}\" config user.name \"CI\""),
    )
    .await
    .expect("git config succeeds");
    exec.write_file("local", &format!("{repo}/README.md"), "# test")
        .await
        .expect("writes the seed file");
    exec.run_command("local", &format!("git -C \"{repo}\" add ."))
        .await
        .expect("git add succeeds");
    exec.run_command("local", &format!("git -C \"{repo}\" commit -m init"))
        .await
        .expect("git commit succeeds");
}

/// A project with a real repository and an open Ask thread against it, with
/// `ctx.registry` wired to a [`FakeRuntime`] scripted to stream `events`.
async fn fixture(tag: &str, events: Vec<AgentEvent>) -> (AppContext, AskThreadId) {
    let base = std::env::temp_dir().join(format!(
        "demeteo-ask-turn-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos()
    ));
    let mut ctx = build_core_context(
        CoreConfig {
            app_data_dir: base,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
    let project_id = ProjectId::from(format!("p-{tag}"));
    ctx.projects
        .add(Project {
            id: project_id.clone(),
            name: "ask turn fixture".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .expect("the project is stored");
    ctx.projects
        .add_repository(Repository {
            id: RepositoryId::from(format!("r-{tag}")),
            project_id: project_id.clone(),
            provider_id: ProviderId::from("provider"),
            repo_path: "repo".to_string(),
        })
        .expect("the repository is stored");
    let repo_dir = ctx
        .workspace_dir
        .join("projects")
        .join(project_id.as_str())
        .join(crate::paths::REPOS_SUBDIR)
        .join("repo");
    init_repo_at(&repo_dir).await;

    let thread = AskThread {
        id: AskThreadId::from(format!("t-{tag}")),
        project_id,
        title: "quick question".to_string(),
        status: AskStatus::Open,
        agent_kind: FAKE_KIND.to_string(),
        model: None,
        effort: None,
        machine_id: MachineId::from(LOCAL_MACHINE.to_string()),
        worktree_path: None,
        session_id: None,
        turn_count: 0,
        cost_usd: 0.0,
        tokens: 0,
        created_at: 0,
        updated_at: 0,
    };
    ctx.ask.create(&thread).expect("the thread is stored");
    ctx.registry = Arc::new(AgentRegistry::new(vec![Arc::new(FakeRuntime { events })]));

    (ctx, thread.id)
}

fn success_events(cost_usd: f64, input_tokens: u64, output_tokens: u64) -> Vec<AgentEvent> {
    vec![
        AgentEvent::Text {
            delta: "the scope fence lives in git_ops::scope".to_string(),
        },
        AgentEvent::TurnComplete {
            stop_reason: StopReason::EndOfTurn,
            usage: Some(Usage {
                input_tokens,
                output_tokens,
                cost_usd: Some(cost_usd),
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            }),
        },
    ]
}

/// A [`AskPort`] spy that passes every call through to a real store while
/// recording every patch handed to `update` — enough to tell a billing call
/// apart from the worktree-path/touch calls `worktree::ensure`/`send` also
/// make, without stubbing out the store itself.
struct SpyAskPort {
    inner: Arc<dyn AskPort>,
    updates: Mutex<Vec<AskThreadPatch>>,
}

impl SpyAskPort {
    fn wrap(inner: Arc<dyn AskPort>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            updates: Mutex::new(Vec::new()),
        })
    }
    fn updates(&self) -> Vec<AskThreadPatch> {
        self.updates.lock().expect("not poisoned").clone()
    }
}

impl AskPort for SpyAskPort {
    fn create(&self, thread: &AskThread) -> Result<(), String> {
        self.inner.create(thread)
    }
    fn get(&self, id: &AskThreadId) -> Result<Option<AskThread>, String> {
        self.inner.get(id)
    }
    fn list_for_project(&self, project_id: &ProjectId) -> Result<Vec<AskThread>, String> {
        self.inner.list_for_project(project_id)
    }
    fn update(&self, id: &AskThreadId, patch: &AskThreadPatch, now: i64) -> Result<(), String> {
        self.updates
            .lock()
            .expect("not poisoned")
            .push(patch.clone());
        self.inner.update(id, patch, now)
    }
    fn delete(&self, id: &AskThreadId) -> Result<(), String> {
        self.inner.delete(id)
    }
    fn append_message(&self, message: &AskMessage) -> Result<(), String> {
        self.inner.append_message(message)
    }
    fn list_messages(&self, id: &AskThreadId) -> Result<Vec<AskMessage>, String> {
        self.inner.list_messages(id)
    }
}

fn silent() -> impl Fn(&str, serde_json::Value) + Send + Sync + 'static {
    |_: &str, _: serde_json::Value| {}
}

type Wire = Arc<Mutex<Vec<String>>>;

/// Records every status/completion event name, coalescing a status payload
/// down to its `status` string so ordering can be asserted on plain strings.
fn recorder() -> (
    impl Fn(&str, serde_json::Value) + Send + Sync + 'static,
    Wire,
) {
    let wire: Wire = Arc::new(Mutex::new(Vec::new()));
    let written = wire.clone();
    let emit = move |event: &str, payload: serde_json::Value| {
        let mut w = written.lock().expect("not poisoned");
        match event {
            EVENT_ASK_TURN_STATUS => {
                w.push(payload["status"].as_str().unwrap_or_default().to_string());
            }
            EVENT_ASK_TURN_COMPLETED => {
                w.push(EVENT_ASK_TURN_COMPLETED.to_string());
            }
            _ => {}
        }
    };
    (emit, wire)
}

async fn wait_for(wire: &Wire, marker: &str) {
    for _ in 0..400 {
        if wire
            .lock()
            .expect("not poisoned")
            .iter()
            .any(|e| e == marker)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("'{marker}' never arrived");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_full_turn_reports_setting_up_then_running_then_completed_in_order() {
    let (ctx, id) = fixture("order", success_events(0.05, 100, 50)).await;
    let (emit, wire) = recorder();

    send(&ctx, &id, "why does this exist?", emit)
        .await
        .expect("the turn is accepted");

    wait_for(&wire, EVENT_ASK_TURN_COMPLETED).await;

    let seen = wire.lock().expect("not poisoned").clone();
    let setting_up = seen.iter().position(|e| e == STATUS_SETTING_UP);
    let running = seen.iter().position(|e| e == STATUS_RUNNING);
    let completed = seen.iter().position(|e| e == EVENT_ASK_TURN_COMPLETED);

    assert!(
        setting_up.is_some() && running.is_some() && completed.is_some(),
        "expected setting_up, running and completed all to arrive; saw {seen:?}"
    );
    assert!(
        setting_up < running,
        "setting_up must arrive before running; saw {seen:?}"
    );
    assert!(
        running < completed,
        "running must arrive before completed; saw {seen:?}"
    );

    let messages = ctx.ask.list_messages(&id).expect("the transcript reads");
    assert_eq!(
        messages.len(),
        2,
        "the user turn and the assistant turn are both stored"
    );
    assert_eq!(messages[1].role, MessageRole::Assistant);
    assert!(messages[1].canvas_paths.is_none());
    assert!(messages[1].checked_commit_sha.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_that_spent_something_bills_the_thread() {
    let (mut ctx, id) = fixture("billed", success_events(0.05, 100, 50)).await;
    let spy = SpyAskPort::wrap(ctx.ask.clone());
    ctx.ask = spy.clone();
    let (emit, wire) = recorder();

    send(&ctx, &id, "why does this exist?", emit)
        .await
        .expect("the turn is accepted");
    wait_for(&wire, EVENT_ASK_TURN_COMPLETED).await;

    let billing_calls: Vec<_> = spy
        .updates()
        .into_iter()
        .filter(|p| p.add_cost_usd != 0.0 || p.add_tokens != 0 || p.add_turns != 0)
        .collect();
    assert_eq!(billing_calls.len(), 1, "exactly one call folds the spend");
    assert_eq!(billing_calls[0].add_cost_usd, 0.05);
    assert_eq!(billing_calls[0].add_tokens, 150);
    assert_eq!(billing_calls[0].add_turns, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_that_spent_nothing_does_not_call_update_for_billing() {
    let (mut ctx, id) = fixture(
        "unbilled",
        vec![AgentEvent::TurnComplete {
            stop_reason: StopReason::EndOfTurn,
            usage: None,
        }],
    )
    .await;
    let spy = SpyAskPort::wrap(ctx.ask.clone());
    ctx.ask = spy.clone();
    let (emit, wire) = recorder();

    send(&ctx, &id, "why does this exist?", emit)
        .await
        .expect("the turn is accepted");
    wait_for(&wire, EVENT_ASK_TURN_COMPLETED).await;

    let billing_calls: Vec<_> = spy
        .updates()
        .into_iter()
        .filter(|p| p.add_cost_usd != 0.0 || p.add_tokens != 0 || p.add_turns != 0)
        .collect();
    assert!(
        billing_calls.is_empty(),
        "a turn that spent nothing must not call update for billing: {billing_calls:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_producing_no_text_persists_no_assistant_message() {
    let (ctx, id) = fixture(
        "empty",
        vec![AgentEvent::TurnComplete {
            stop_reason: StopReason::EndOfTurn,
            usage: None,
        }],
    )
    .await;
    let (emit, wire) = recorder();

    send(&ctx, &id, "why does this exist?", emit)
        .await
        .expect("the turn is accepted");
    wait_for(&wire, EVENT_ASK_TURN_COMPLETED).await;

    let messages = ctx.ask.list_messages(&id).expect("the transcript reads");
    assert_eq!(
        messages.len(),
        1,
        "only the user's own message is stored when the turn said nothing"
    );
    assert_eq!(messages[0].role, MessageRole::User);
}

// ─────────────────────────────────────────────────────────────────────────────
// `send`'s claim-then-persist-then-spawn ordering
// ─────────────────────────────────────────────────────────────────────────────

/// A `ProjectRepository` that parks the first call setup makes until the test
/// lets it through — the same technique
/// `tests/application/discovery/turn.rs::HeldProjects` uses to prove `send`
/// does not wait on setup and that a refused second turn leaves no orphan
/// row.
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
            .recv_timeout(Duration::from_secs(30));
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
    fn get_settings(
        &self,
        _: &ProjectId,
    ) -> Result<Option<crate::domain::models::ProjectSettings>, String> {
        reject_project_call!()
    }
    fn save_settings(&self, _: crate::domain::models::ProjectSettings) -> Result<(), String> {
        reject_project_call!()
    }
    fn list_workflow_overrides(
        &self,
        _: &ProjectId,
    ) -> Result<Vec<crate::domain::models::ProjectWorkflowOverride>, String> {
        reject_project_call!()
    }
    fn list_overrides_for_workflow(
        &self,
        _: &ProjectId,
        _: &crate::domain::ids::WorkflowId,
    ) -> Result<Vec<crate::domain::models::ProjectWorkflowOverride>, String> {
        reject_project_call!()
    }
    fn upsert_workflow_override(
        &self,
        _: crate::domain::models::ProjectWorkflowOverride,
    ) -> Result<(), String> {
        reject_project_call!()
    }
}

fn held() -> (Arc<HeldProjects>, std::sync::mpsc::Sender<()>) {
    let (release, gate) = std::sync::mpsc::channel();
    (
        Arc::new(HeldProjects {
            gate: std::sync::Mutex::new(gate),
        }),
        release,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_running_turn_refuses_a_second_turn_and_leaves_no_orphan_row() {
    let (mut ctx, id) = fixture("refuses", success_events(0.0, 0, 0)).await;
    let (projects, release) = held();
    ctx.projects = projects;

    send(&ctx, &id, "first", silent())
        .await
        .expect("the first turn is accepted");

    let second = send(&ctx, &id, "second", silent()).await;
    assert_eq!(second.err().as_deref(), Some(ALREADY_RUNNING));

    assert!(
        ctx.ask_turns.running(&id),
        "a refusal must not release the claim it was refused by"
    );
    assert_eq!(
        ctx.ask
            .list_messages(&id)
            .expect("the transcript reads")
            .len(),
        1,
        "a refused turn leaves no bubble behind"
    );

    release.send(()).expect("setup is still parked on the gate");
}

// ─────────────────────────────────────────────────────────────────────────────
// Canvas path verification (AC4/AC5): a turn whose streamed text carries a
// canvas block gets every path-bearing node stat'd against the mounted
// worktree, synchronously, before the turn's own message is persisted.
// ─────────────────────────────────────────────────────────────────────────────

/// A near-passthrough [`ExecutionPort`] over a real local adapter, so
/// `resolve`/`ensure`/`stream_agent_turn`'s own exec calls keep working —
/// while recording every `get_metadata` path and, once `reclaimed` flips
/// true, erroring loudly on every call instead of silently answering `Ok`,
/// per AGENTS.md §7's "prefer a double that errors on anything it was not
/// explicitly told to say."
struct WrappedExec {
    inner: LocalSubprocessAdapter,
    get_metadata_paths: Mutex<Vec<String>>,
    reclaimed: Arc<AtomicBool>,
}

impl WrappedExec {
    fn new() -> Self {
        Self {
            inner: LocalSubprocessAdapter::new(),
            get_metadata_paths: Mutex::new(Vec::new()),
            reclaimed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn get_metadata_paths(&self) -> Vec<String> {
        self.get_metadata_paths
            .lock()
            .expect("not poisoned")
            .clone()
    }

    fn reclaimed_flag(&self) -> Arc<AtomicBool> {
        self.reclaimed.clone()
    }

    fn guard(&self) -> Result<(), String> {
        if self.reclaimed.load(Ordering::SeqCst) {
            Err("WrappedExec: called after the sentinel 'reclaimed' flag was set".to_string())
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl ExecutionPort for WrappedExec {
    async fn test_connection(&self, machine_id: &str) -> Result<(), String> {
        self.guard()?;
        self.inner.test_connection(machine_id).await
    }
    async fn run_program(
        &self,
        machine_id: &str,
        request: ProgramRequest,
    ) -> Result<String, String> {
        self.guard()?;
        self.inner.run_program(machine_id, request).await
    }
    async fn run_command_with(
        &self,
        machine_id: &str,
        cmd: &str,
        opts: ShellOptions,
    ) -> Result<String, String> {
        self.guard()?;
        self.inner.run_command_with(machine_id, cmd, opts).await
    }
    async fn read_file(&self, machine_id: &str, path: &str) -> Result<String, String> {
        self.guard()?;
        self.inner.read_file(machine_id, path).await
    }
    async fn write_file(&self, machine_id: &str, path: &str, content: &str) -> Result<(), String> {
        self.guard()?;
        self.inner.write_file(machine_id, path, content).await
    }
    async fn write_file_bytes(
        &self,
        machine_id: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), String> {
        self.guard()?;
        self.inner.write_file_bytes(machine_id, path, content).await
    }
    async fn create_dir_all(&self, machine_id: &str, path: &str) -> Result<(), String> {
        self.guard()?;
        self.inner.create_dir_all(machine_id, path).await
    }
    async fn remove_dir_all(&self, machine_id: &str, path: &str) -> Result<(), String> {
        self.guard()?;
        self.inner.remove_dir_all(machine_id, path).await
    }
    async fn remove_file(&self, machine_id: &str, path: &str) -> Result<(), String> {
        self.guard()?;
        self.inner.remove_file(machine_id, path).await
    }
    async fn is_executable(&self, machine_id: &str, path: &str) -> Result<bool, String> {
        self.guard()?;
        self.inner.is_executable(machine_id, path).await
    }
    async fn get_metadata(&self, machine_id: &str, path: &str) -> Result<SftpEntry, String> {
        self.get_metadata_paths
            .lock()
            .expect("not poisoned")
            .push(path.to_string());
        self.guard()?;
        self.inner.get_metadata(machine_id, path).await
    }
    async fn list_dir(&self, machine_id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        self.guard()?;
        self.inner.list_dir(machine_id, path).await
    }
    async fn setup_worktree(
        &self,
        machine_id: &str,
        repo_path: &str,
        branch: &str,
        sandbox_path: &str,
    ) -> Result<(), String> {
        self.guard()?;
        self.inner
            .setup_worktree(machine_id, repo_path, branch, sandbox_path)
            .await
    }
    async fn resolve_home(&self, machine_id: &str) -> Result<String, String> {
        self.guard()?;
        self.inner.resolve_home(machine_id).await
    }
    async fn resolve_platform(&self, machine_id: &str) -> Result<Platform, String> {
        self.guard()?;
        self.inner.resolve_platform(machine_id).await
    }
    async fn resolve_user(&self, machine_id: &str) -> Result<String, String> {
        self.guard()?;
        self.inner.resolve_user(machine_id).await
    }
    async fn control_rpc(
        &self,
        machine_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.guard()?;
        self.inner.control_rpc(machine_id, method, params).await
    }
    fn spawn_interactive(
        &self,
        machine_id: &str,
        binary: &str,
        args: &[String],
        cwd: &str,
        env: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        self.guard()?;
        self.inner
            .spawn_interactive(machine_id, binary, args, cwd, env)
    }
}

/// A turn whose text ends in a canvas block naming one path that exists in
/// the mounted worktree (`README.md`, seeded by [`init_repo_at`]) and one
/// that does not.
fn canvas_events(cost_usd: f64, input_tokens: u64, output_tokens: u64) -> Vec<AgentEvent> {
    let canvas = serde_json::json!({
        "kind": "architecture",
        "title": "canvas",
        "stages": ["s0"],
        "lanes": ["l0"],
        "nodes": [
            {"id": "n1", "title": "Real node", "role": "boundary", "path": "README.md", "stage": 0, "lane": 0},
            {"id": "n2", "title": "Missing node", "role": "boundary", "path": "does/not/exist.rs", "stage": 0, "lane": 0}
        ],
        "edges": []
    });
    let text = format!("Here is the diagram.\n```json\n{canvas}\n```");
    vec![
        AgentEvent::Text { delta: text },
        AgentEvent::TurnComplete {
            stop_reason: StopReason::EndOfTurn,
            usage: Some(Usage {
                input_tokens,
                output_tokens,
                cost_usd: Some(cost_usd),
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            }),
        },
    ]
}

/// A turn whose canvas block fails [`crate::domain::ask_canvas::validate_canvas`]
/// — a node names stage `5` when only one stage is declared.
fn invalid_canvas_events(cost_usd: f64, input_tokens: u64, output_tokens: u64) -> Vec<AgentEvent> {
    let canvas = serde_json::json!({
        "kind": "architecture",
        "title": "canvas",
        "stages": ["s0"],
        "lanes": ["l0"],
        "nodes": [
            {"id": "n1", "title": "Bad node", "role": "boundary", "path": "README.md", "stage": 5, "lane": 0}
        ],
        "edges": []
    });
    let text = format!("Here is the diagram.\n```json\n{canvas}\n```");
    vec![
        AgentEvent::Text { delta: text },
        AgentEvent::TurnComplete {
            stop_reason: StopReason::EndOfTurn,
            usage: Some(Usage {
                input_tokens,
                output_tokens,
                cost_usd: Some(cost_usd),
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            }),
        },
    ]
}

/// Neither `resolve`'s own repo-dir check nor `ensure`'s stored-path check
/// (both pre-existing, unrelated to canvas verification) ever stat a path
/// *inside* the mounted worktree — only [`super::verify_canvas_paths`] joins
/// a node's path onto `p.agent_ctx.cwd`. So "no canvas verification ran" is
/// provable as "no recorded call names a path under the worktree", without
/// coupling the assertion to the exact, unrelated call count `resolve`/
/// `ensure` happen to make today.
async fn assert_no_worktree_scoped_get_metadata_calls(
    ctx: &AppContext,
    id: &AskThreadId,
    exec: &WrappedExec,
) {
    let thread = ctx
        .ask
        .get(id)
        .expect("thread reads")
        .expect("thread exists");
    let worktree_path = thread
        .worktree_path
        .expect("prepare provisions a worktree even for a canvas-free turn");
    let prefix = format!("{worktree_path}/");
    let calls = exec.get_metadata_paths();
    assert!(
        calls.iter().all(|p| !p.starts_with(&prefix)),
        "no canvas node path should be stat'd when there is nothing to verify: {calls:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_with_a_resolvable_and_unresolvable_canvas_path_records_both_verdicts() {
    let (mut ctx, id) = fixture("canvas", canvas_events(0.01, 10, 5)).await;
    ctx.exec = Arc::new(WrappedExec::new());
    let (emit, wire) = recorder();

    send(&ctx, &id, "sketch the architecture", emit)
        .await
        .expect("the turn is accepted");
    wait_for(&wire, EVENT_ASK_TURN_COMPLETED).await;

    let messages = ctx.ask.list_messages(&id).expect("the transcript reads");
    let assistant = &messages[1];
    let verdicts = assistant
        .canvas_paths
        .as_ref()
        .expect("a canvas with path-bearing nodes stores verdicts");
    assert_eq!(verdicts.len(), 2);
    let verdict_for = |node_id: &str| {
        verdicts
            .iter()
            .find(|v| v.node_id == node_id)
            .unwrap_or_else(|| panic!("no verdict recorded for '{node_id}'"))
    };
    assert!(
        verdict_for("n1").resolved,
        "README.md exists in the mounted worktree"
    );
    assert!(
        !verdict_for("n2").resolved,
        "does/not/exist.rs is not in the mounted worktree"
    );
    assert!(
        assistant.checked_commit_sha.is_some(),
        "a canvas with verdicts must record the commit it was checked at"
    );
}

/// A turn whose canvas names a legitimate in-worktree node alongside an
/// absolute path and a `..`-traversal that both try to walk out of the
/// mounted worktree.
fn canvas_events_with_escaping_paths(
    cost_usd: f64,
    input_tokens: u64,
    output_tokens: u64,
) -> Vec<AgentEvent> {
    let canvas = serde_json::json!({
        "kind": "architecture",
        "title": "canvas",
        "stages": ["s0"],
        "lanes": ["l0"],
        "nodes": [
            {"id": "n1", "title": "Real node", "role": "boundary", "path": "README.md", "stage": 0, "lane": 0},
            {"id": "n2", "title": "Absolute escape", "role": "boundary", "path": "/etc/hostname", "stage": 0, "lane": 0},
            {"id": "n3", "title": "Traversal escape", "role": "boundary", "path": "../../../../etc/hostname", "stage": 0, "lane": 0}
        ],
        "edges": []
    });
    let text = format!("Here is the diagram.\n```json\n{canvas}\n```");
    vec![
        AgentEvent::Text { delta: text },
        AgentEvent::TurnComplete {
            stop_reason: StopReason::EndOfTurn,
            usage: Some(Usage {
                input_tokens,
                output_tokens,
                cost_usd: Some(cost_usd),
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            }),
        },
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canvas_paths_that_escape_the_worktree_are_rejected_before_stat() {
    let (mut ctx, id) = fixture(
        "canvas-escape",
        canvas_events_with_escaping_paths(0.01, 10, 5),
    )
    .await;
    let exec = Arc::new(WrappedExec::new());
    ctx.exec = exec.clone();
    let (emit, wire) = recorder();

    send(&ctx, &id, "sketch the architecture", emit)
        .await
        .expect("the turn is accepted");
    wait_for(&wire, EVENT_ASK_TURN_COMPLETED).await;

    let messages = ctx.ask.list_messages(&id).expect("the transcript reads");
    let assistant = &messages[1];
    let verdicts = assistant
        .canvas_paths
        .as_ref()
        .expect("a canvas with path-bearing nodes stores verdicts");
    assert_eq!(verdicts.len(), 3);
    let verdict_for = |node_id: &str| {
        verdicts
            .iter()
            .find(|v| v.node_id == node_id)
            .unwrap_or_else(|| panic!("no verdict recorded for '{node_id}'"))
    };
    assert!(
        verdict_for("n1").resolved,
        "README.md exists in the mounted worktree — no regression to the legitimate case"
    );
    assert!(
        !verdict_for("n2").resolved,
        "an absolute path must be recorded unresolved, whatever exists at that host location"
    );
    assert!(
        !verdict_for("n3").resolved,
        "a `..`-traversal that walks out of the worktree must be recorded unresolved"
    );

    let calls = exec.get_metadata_paths();
    assert!(
        calls
            .iter()
            .all(|p| p != "/etc/hostname" && !p.ends_with("/etc/hostname")),
        "neither the absolute nor the traversing path may ever reach get_metadata: {calls:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_of_canvas_paths_happens_before_the_worktree_could_be_reclaimed() {
    let (mut ctx, id) = fixture("canvas-order", canvas_events(0.01, 10, 5)).await;
    let exec = Arc::new(WrappedExec::new());
    let reclaimed = exec.reclaimed_flag();
    ctx.exec = exec.clone();
    let (emit, wire) = recorder();

    send(&ctx, &id, "sketch the architecture", emit)
        .await
        .expect("the turn is accepted");
    wait_for(&wire, EVENT_ASK_TURN_COMPLETED).await;

    // Both verdicts must already be correct here — proving the stat + sha
    // calls ran to completion before the sentinel flag below is even set.
    let messages = ctx.ask.list_messages(&id).expect("the transcript reads");
    let assistant = &messages[1];
    let verdicts = assistant
        .canvas_paths
        .as_ref()
        .expect("verdicts were computed and stored");
    assert_eq!(verdicts.len(), 2);
    assert!(verdicts.iter().any(|v| v.node_id == "n1" && v.resolved));
    assert!(verdicts.iter().any(|v| v.node_id == "n2" && !v.resolved));
    assert!(assistant.checked_commit_sha.is_some());

    // Arm the gate — everything the mounted worktree depends on `ctx.exec`
    // for now errors loudly instead of silently answering `Ok`.
    reclaimed.store(true, Ordering::SeqCst);
    let after = ctx.exec.get_metadata("local", "/whatever").await;
    assert!(
        after.is_err(),
        "the gate must actually block calls made after the sentinel flag is set, \
         not silently pass them through"
    );

    // A real reclaim (the "reclaim-triggering path" this ticket has, since
    // idle-reclaim is not itself wired into the turn loop) now fails too,
    // because `resolve`'s own repo-dir check runs through the same gated
    // `ctx.exec`. That the verdicts above were already correct proves they
    // were produced strictly before this point — a turn that deferred
    // verification until after a reclaim became possible would have failed
    // exactly the way `reclaim` now does.
    let thread = ctx
        .ask
        .get(&id)
        .expect("thread reads")
        .expect("thread exists");
    let reclaim_result = worktree::reclaim(&ctx, &thread).await;
    assert!(
        reclaim_result.is_err(),
        "once the gate is armed every exec-mediated call is blocked, including reclaim's own"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_with_no_canvas_block_stats_nothing_inside_the_worktree() {
    let (mut ctx, id) = fixture("no-canvas", success_events(0.05, 10, 5)).await;
    let exec = Arc::new(WrappedExec::new());
    ctx.exec = exec.clone();
    let (emit, wire) = recorder();

    send(&ctx, &id, "why does this exist?", emit)
        .await
        .expect("the turn is accepted");
    wait_for(&wire, EVENT_ASK_TURN_COMPLETED).await;

    let messages = ctx.ask.list_messages(&id).expect("the transcript reads");
    assert!(messages[1].canvas_paths.is_none());
    assert!(messages[1].checked_commit_sha.is_none());
    assert_no_worktree_scoped_get_metadata_calls(&ctx, &id, &exec).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_whose_canvas_fails_validation_stats_nothing_inside_the_worktree() {
    let (mut ctx, id) = fixture("bad-canvas", invalid_canvas_events(0.05, 10, 5)).await;
    let exec = Arc::new(WrappedExec::new());
    ctx.exec = exec.clone();
    let (emit, wire) = recorder();

    send(&ctx, &id, "sketch it anyway", emit)
        .await
        .expect("the turn is accepted");
    wait_for(&wire, EVENT_ASK_TURN_COMPLETED).await;

    let messages = ctx.ask.list_messages(&id).expect("the transcript reads");
    assert!(
        messages[1].canvas_paths.is_none(),
        "a canvas that fails validate_canvas is treated as no canvas at all"
    );
    assert!(messages[1].checked_commit_sha.is_none());
    assert_no_worktree_scoped_get_metadata_calls(&ctx, &id, &exec).await;
}

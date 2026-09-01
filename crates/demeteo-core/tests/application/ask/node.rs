// Tests extracted from `crates/demeteo-core/src/application/ask/node.rs` (mirrored-tests convention). `super` = that module.

use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::*;
use crate::adapters::local::execution::LocalSubprocessAdapter;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{
    AskThreadId, MachineId, ProjectId, ProviderId, RepositoryId, LOCAL_MACHINE,
};
use crate::domain::models::{
    AskMessage, AskStatus, AskThread, CanvasPathVerdict, MessageRole, Platform, Project, Repository,
};
use crate::ports::execution::{ExecutionPort, InteractiveHandle, SftpEntry};

/// A project with a repository whose target checkout directory is returned
/// (not yet created on disk — a local-machine test calls [`init_repo_at`] on
/// it; a remote-machine test never touches it, since the remote path is
/// resolved entirely through a fake [`ExecutionPort`]).
async fn fixture(tag: &str) -> (AppContext, ProjectId, std::path::PathBuf) {
    let base = std::env::temp_dir().join(format!(
        "demeteo-ask-node-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos()
    ));
    let ctx = build_core_context(
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
            name: "name fixture".to_string(),
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
    (ctx, project_id, repo_dir)
}

/// A real git repository at exactly the location [`fixture`] derived, on the
/// same terms `tests/application/ask/worktree.rs::init_repo_at` uses.
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

fn thread(
    project_id: &ProjectId,
    id: &str,
    machine_id: &str,
    worktree_path: Option<&str>,
) -> AskThread {
    AskThread {
        id: AskThreadId::from(id.to_string()),
        project_id: project_id.clone(),
        title: "quick question".to_string(),
        status: AskStatus::Open,
        agent_kind: "claude-code".to_string(),
        model: None,
        effort: None,
        machine_id: MachineId::from(machine_id.to_string()),
        worktree_path: worktree_path.map(str::to_string),
        session_id: None,
        turn_count: 0,
        cost_usd: 0.0,
        tokens: 0,
        network: true,
        created_at: 0,
        updated_at: 0,
    }
}

fn message_with_verdicts(
    id: &str,
    thread_id: &AskThreadId,
    verdicts: Vec<CanvasPathVerdict>,
    checked_commit_sha: Option<&str>,
) -> AskMessage {
    AskMessage {
        id: id.to_string(),
        thread_id: thread_id.clone(),
        role: MessageRole::Assistant,
        text: "here is the canvas".to_string(),
        cost_usd: None,
        tokens: None,
        turn_activity: None,
        canvas_paths: Some(verdicts),
        checked_commit_sha: checked_commit_sha.map(str::to_string),
        created_at: 0,
    }
}

fn resolved(node_id: &str, path: &str) -> CanvasPathVerdict {
    CanvasPathVerdict {
        node_id: node_id.to_string(),
        path: path.to_string(),
        resolved: true,
    }
}

/// AC-2: a thread whose worktree was reclaimed (`worktree_path: None`) still
/// resolves against the project's own checkout — [`resolve`] never reads
/// [`AskThread::worktree_path`] at all, so there is nothing for a reclaim to
/// invalidate.
#[tokio::test]
async fn resolve_reads_the_project_checkout_when_the_worktree_was_reclaimed() {
    let (ctx, project_id, repo_dir) = fixture("reclaimed").await;
    init_repo_at(&repo_dir).await;
    let t = thread(&project_id, "t-reclaimed", LOCAL_MACHINE, None);
    ctx.ask.create(&t).expect("the thread is stored");
    let m = message_with_verdicts(
        "m-1",
        &t.id,
        vec![resolved("n1", "README.md")],
        Some("deadbeef"),
    );
    ctx.ask.append_message(&m).expect("the message is stored");

    let resolution = resolve(&ctx, &t.id, "m-1", "n1")
        .await
        .expect("resolve succeeds");

    match resolution {
        NodeResolution::Editor {
            worktree_path,
            path,
            branch,
            default_branch,
            ..
        } => {
            assert_eq!(worktree_path, repo_dir.to_string_lossy());
            assert_eq!(path, "README.md");
            assert_eq!(
                branch, default_branch,
                "a project-level checkout has no feature branch, so both must match"
            );
        }
        other => panic!("expected Editor, got {other:?}"),
    }
}

/// Baseline for AC-2: the same resolution succeeds identically when the
/// thread's worktree path is still present, proving the project's `repo_dir`
/// is used regardless of worktree presence — not only as a fallback for the
/// reclaimed case.
#[tokio::test]
async fn resolve_reads_the_project_checkout_even_when_the_worktree_is_still_present() {
    let (ctx, project_id, repo_dir) = fixture("present").await;
    init_repo_at(&repo_dir).await;
    let t = thread(
        &project_id,
        "t-present",
        LOCAL_MACHINE,
        Some("/tmp/does-not-need-to-exist"),
    );
    ctx.ask.create(&t).expect("the thread is stored");
    let m = message_with_verdicts(
        "m-1",
        &t.id,
        vec![resolved("n1", "README.md")],
        Some("deadbeef"),
    );
    ctx.ask.append_message(&m).expect("the message is stored");

    let resolution = resolve(&ctx, &t.id, "m-1", "n1")
        .await
        .expect("resolve succeeds");

    match resolution {
        NodeResolution::Editor {
            worktree_path,
            path,
            ..
        } => {
            assert_eq!(worktree_path, repo_dir.to_string_lossy());
            assert_eq!(path, "README.md");
        }
        other => panic!("expected Editor, got {other:?}"),
    }
}

/// AC-3: a verdict stored `resolved: true` whose path no longer exists in
/// the project's current checkout reports `Moved`, carrying the message's
/// own `checked_commit_sha` verbatim.
#[tokio::test]
async fn resolve_reports_moved_when_the_verified_path_no_longer_exists() {
    let (ctx, project_id, repo_dir) = fixture("moved").await;
    init_repo_at(&repo_dir).await;
    let t = thread(&project_id, "t-moved", LOCAL_MACHINE, None);
    ctx.ask.create(&t).expect("the thread is stored");
    let m = message_with_verdicts(
        "m-1",
        &t.id,
        vec![resolved("n1", "does/not/exist.rs")],
        Some("deadbeef1234"),
    );
    ctx.ask.append_message(&m).expect("the message is stored");

    let resolution = resolve(&ctx, &t.id, "m-1", "n1")
        .await
        .expect("resolve succeeds");

    match resolution {
        NodeResolution::Moved { checked_commit_sha } => {
            assert_eq!(checked_commit_sha, "deadbeef1234");
        }
        other => panic!("expected Moved, got {other:?}"),
    }
}

/// An [`ExecutionPort`] double for a single named remote machine: every
/// method but `resolve_home`/`get_metadata` panics, so a call the resolve
/// path has no business making fails loudly rather than quietly answering.
/// Records every machine id it was called with, which is what
/// [`resolve_uses_the_threads_own_machine_not_a_local_default`] checks —
/// a hard-coded local default would call this double with `"local"` instead
/// of the thread's own machine, or not call it at all.
struct FakeRemoteExec {
    home: String,
    machine_ids_called: Mutex<Vec<String>>,
}

impl FakeRemoteExec {
    fn new(home: &str) -> Self {
        Self {
            home: home.to_string(),
            machine_ids_called: Mutex::new(Vec::new()),
        }
    }

    fn machine_ids_called(&self) -> Vec<String> {
        self.machine_ids_called
            .lock()
            .expect("not poisoned")
            .clone()
    }

    fn record(&self, machine_id: &str) {
        self.machine_ids_called
            .lock()
            .expect("not poisoned")
            .push(machine_id.to_string());
    }
}

#[async_trait]
impl ExecutionPort for FakeRemoteExec {
    async fn test_connection(&self, _: &str) -> Result<(), String> {
        panic!("unexpected ExecutionPort call")
    }
    async fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
        panic!("unexpected ExecutionPort call")
    }
    async fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        panic!("unexpected ExecutionPort call")
    }
    async fn write_file_bytes(&self, _: &str, _: &str, _: &[u8]) -> Result<(), String> {
        panic!("unexpected ExecutionPort call")
    }
    async fn get_metadata(&self, machine_id: &str, path: &str) -> Result<SftpEntry, String> {
        self.record(machine_id);
        Ok(SftpEntry {
            name: "readme".to_string(),
            path: path.to_string(),
            is_dir: false,
            size: 4,
            modified: 0,
        })
    }
    async fn list_dir(&self, _: &str, _: &str) -> Result<Vec<SftpEntry>, String> {
        panic!("unexpected ExecutionPort call")
    }
    async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
        panic!("unexpected ExecutionPort call")
    }
    async fn resolve_home(&self, machine_id: &str) -> Result<String, String> {
        self.record(machine_id);
        Ok(self.home.clone())
    }
    async fn resolve_platform(&self, _: &str) -> Result<Platform, String> {
        panic!("unexpected ExecutionPort call")
    }
    async fn resolve_user(&self, _: &str) -> Result<String, String> {
        panic!("unexpected ExecutionPort call")
    }
    async fn control_rpc(
        &self,
        _: &str,
        _: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        panic!("unexpected ExecutionPort call")
    }
    fn spawn_interactive(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: &str,
        _: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        panic!("unexpected ExecutionPort call")
    }
}

/// The SSH-machine case: resolution goes through the thread's own
/// `machine_id`, proven by a fake multi-machine `ExecutionPort` that records
/// every machine id it was asked about and would fail this assertion (not
/// panic — the double answers any machine) if a hard-coded `"local"` default
/// were used instead of `thread.machine_id`.
#[tokio::test]
async fn resolve_uses_the_threads_own_machine_not_a_local_default() {
    let (mut ctx, project_id, _repo_dir) = fixture("remote").await;
    let exec = Arc::new(FakeRemoteExec::new("/home/rig"));
    ctx.exec = exec.clone();
    let t = thread(&project_id, "t-remote", "rig-1", None);
    ctx.ask.create(&t).expect("the thread is stored");
    let m = message_with_verdicts(
        "m-1",
        &t.id,
        vec![resolved("n1", "README.md")],
        Some("sha-remote"),
    );
    ctx.ask.append_message(&m).expect("the message is stored");

    let resolution = resolve(&ctx, &t.id, "m-1", "n1")
        .await
        .expect("resolve succeeds");

    match resolution {
        NodeResolution::Editor { machine_id, .. } => assert_eq!(machine_id, "rig-1"),
        other => panic!("expected Editor, got {other:?}"),
    }
    let called = exec.machine_ids_called();
    assert!(!called.is_empty(), "the fake must have been consulted");
    assert!(
        called.iter().all(|m| m == "rig-1"),
        "every call must target the thread's own machine, never a hard-coded local default: {called:?}"
    );
}

/// A near-passthrough [`ExecutionPort`] over a real local adapter, recording
/// every `get_metadata` path — enough to prove a rejected path never reaches
/// the stat call, mirroring `tests/application/ask/turn.rs`'s
/// `canvas_paths_that_escape_the_worktree_are_rejected_before_stat`.
struct RecordingExec {
    inner: LocalSubprocessAdapter,
    get_metadata_paths: Mutex<Vec<String>>,
}

impl RecordingExec {
    fn new() -> Self {
        Self {
            inner: LocalSubprocessAdapter::new(),
            get_metadata_paths: Mutex::new(Vec::new()),
        }
    }

    fn get_metadata_paths(&self) -> Vec<String> {
        self.get_metadata_paths
            .lock()
            .expect("not poisoned")
            .clone()
    }
}

#[async_trait]
impl ExecutionPort for RecordingExec {
    async fn test_connection(&self, machine_id: &str) -> Result<(), String> {
        self.inner.test_connection(machine_id).await
    }
    async fn read_file(&self, machine_id: &str, path: &str) -> Result<String, String> {
        self.inner.read_file(machine_id, path).await
    }
    async fn write_file(&self, machine_id: &str, path: &str, content: &str) -> Result<(), String> {
        self.inner.write_file(machine_id, path, content).await
    }
    async fn write_file_bytes(
        &self,
        machine_id: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), String> {
        self.inner.write_file_bytes(machine_id, path, content).await
    }
    async fn get_metadata(&self, machine_id: &str, path: &str) -> Result<SftpEntry, String> {
        self.get_metadata_paths
            .lock()
            .expect("not poisoned")
            .push(path.to_string());
        self.inner.get_metadata(machine_id, path).await
    }
    async fn list_dir(&self, machine_id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        self.inner.list_dir(machine_id, path).await
    }
    async fn setup_worktree(
        &self,
        machine_id: &str,
        repo_path: &str,
        branch: &str,
        sandbox_path: &str,
    ) -> Result<(), String> {
        self.inner
            .setup_worktree(machine_id, repo_path, branch, sandbox_path)
            .await
    }
    async fn resolve_home(&self, machine_id: &str) -> Result<String, String> {
        self.inner.resolve_home(machine_id).await
    }
    async fn resolve_platform(&self, machine_id: &str) -> Result<Platform, String> {
        self.inner.resolve_platform(machine_id).await
    }
    async fn resolve_user(&self, machine_id: &str) -> Result<String, String> {
        self.inner.resolve_user(machine_id).await
    }
    async fn control_rpc(
        &self,
        machine_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
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
        self.inner
            .spawn_interactive(machine_id, binary, args, cwd, env)
    }
}

/// An absolute path and a `..`-traversal path both resolve to `Moved`
/// (containment failure is treated the same as a stat failure) and, more to
/// the point, never reach `get_metadata` at all — proven by a call-recording
/// decorator, not just by the negative result.
#[tokio::test]
async fn resolve_rejects_absolute_and_traversal_paths_before_any_stat() {
    let (mut ctx, project_id, repo_dir) = fixture("escape").await;
    init_repo_at(&repo_dir).await;
    let exec = Arc::new(RecordingExec::new());
    ctx.exec = exec.clone();
    let t = thread(&project_id, "t-escape", LOCAL_MACHINE, None);
    ctx.ask.create(&t).expect("the thread is stored");
    let verdicts = vec![
        resolved("n-abs", "/etc/hostname"),
        resolved("n-trav", "../../../../etc/hostname"),
    ];
    let m = message_with_verdicts("m-1", &t.id, verdicts, Some("sha-escape"));
    ctx.ask.append_message(&m).expect("the message is stored");

    for node_id in ["n-abs", "n-trav"] {
        let resolution = resolve(&ctx, &t.id, "m-1", node_id)
            .await
            .unwrap_or_else(|e| panic!("resolve succeeds for {node_id}: {e}"));
        match resolution {
            NodeResolution::Moved { checked_commit_sha } => {
                assert_eq!(checked_commit_sha, "sha-escape");
            }
            other => panic!("expected Moved for {node_id}, got {other:?}"),
        }
    }

    let calls = exec.get_metadata_paths();
    assert!(
        calls
            .iter()
            .all(|p| p != "/etc/hostname" && !p.ends_with("/etc/hostname")),
        "neither the absolute nor the traversing path may ever reach get_metadata: {calls:?}"
    );
}

/// A missing thread, message, node, or an unresolved verdict is a plain
/// error, not a `Moved` result — `Moved` is reserved for "resolved at
/// turn-time, gone now", which none of these are.
#[tokio::test]
async fn resolve_rejects_a_node_whose_verdict_was_never_resolved() {
    let (ctx, project_id, repo_dir) = fixture("unresolved").await;
    init_repo_at(&repo_dir).await;
    let t = thread(&project_id, "t-unresolved", LOCAL_MACHINE, None);
    ctx.ask.create(&t).expect("the thread is stored");
    let m = AskMessage {
        canvas_paths: Some(vec![CanvasPathVerdict {
            node_id: "n1".to_string(),
            path: "does/not/exist.rs".to_string(),
            resolved: false,
        }]),
        ..message_with_verdicts("m-1", &t.id, vec![], Some("deadbeef"))
    };
    ctx.ask.append_message(&m).expect("the message is stored");

    let err = resolve(&ctx, &t.id, "m-1", "n1")
        .await
        .expect_err("an unresolved verdict must not be treated as a click-time result");
    assert!(err.contains("n1"), "{err}");
}

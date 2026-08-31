// Tests extracted from `crates/demeteo-core/src/application/ask/worktree.rs` (mirrored-tests convention). `super` = that module.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::*;
use crate::adapters::local::execution::LocalSubprocessAdapter;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{
    AskThreadId, MachineId, ProjectId, ProviderId, RepositoryId, LOCAL_MACHINE,
};
use crate::domain::models::{AskStatus, Platform, Project, Repository};
use crate::ports::execution::{ExecutionPort, InteractiveHandle, SftpEntry};
use crate::ports::worktree_ops::WorktreeOpsPort;

/// A project with a repository whose target checkout directory is returned
/// (not yet created on disk — callers that need a real repo call
/// [`init_repo_at`] on it).
async fn fixture(tag: &str) -> (AppContext, ProjectId, PathBuf) {
    let base = std::env::temp_dir().join(format!(
        "demeteo-ask-wt-{tag}-{}",
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

/// A real git repository at exactly the location [`fixture`] derived, so
/// `resolve`/`ensure` operate on a checkout that actually exists rather than
/// a fake double standing in for git itself — the same integration-test
/// convention `tests/infrastructure/worktree/git_ops/common.rs::make_repo`
/// uses.
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

fn thread(project_id: &ProjectId, id: &str, worktree_path: Option<&str>) -> AskThread {
    AskThread {
        id: AskThreadId::from(id.to_string()),
        project_id: project_id.clone(),
        title: "quick question".to_string(),
        status: AskStatus::Open,
        agent_kind: "claude-code".to_string(),
        model: None,
        effort: None,
        machine_id: MachineId::from(LOCAL_MACHINE.to_string()),
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

/// A [`WorktreeOpsPort`] spy that answers only `cleanup_subtask_worktree`,
/// recording every subtask id it was called with and failing the ones named
/// in `fail_for`. Every other method panics — `reclaim` and `reclaim_idle`
/// are the only callers of this port that `ask::worktree` makes, so a call to
/// anything else is the test catching a wrong code path, the same
/// `RecordingWorktrees` convention `tests/application/lifecycle.rs` uses.
struct SpyWorktreeOps {
    cleanup_calls: Mutex<Vec<String>>,
    fail_for: HashSet<String>,
}

impl SpyWorktreeOps {
    fn new(fail_for: &[&str]) -> Self {
        Self {
            cleanup_calls: Mutex::new(Vec::new()),
            fail_for: fail_for.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.cleanup_calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl WorktreeOpsPort for SpyWorktreeOps {
    async fn check_repo_dirty(&self, _: Option<&str>, _: &str) -> Result<(bool, bool), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn get_head_branch(&self, _: Option<&str>, _: &str) -> Option<String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn list_worktrees(
        &self,
        _: Option<&str>,
        _: &str,
    ) -> Result<Vec<crate::domain::models::WorktreeInfo>, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn create_terminal_worktree(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &crate::ports::worktree_ops::TerminalWorktreeRequest,
    ) -> Result<crate::ports::worktree_ops::TerminalWorktreeCreated, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn remove_terminal_worktree(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
        _: bool,
    ) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn list_terminal_branches(
        &self,
        _: Option<&str>,
        _: &str,
    ) -> Result<Vec<crate::domain::branch_listing::BranchOption>, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn list_terminal_worktrees(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
    ) -> Result<Vec<crate::domain::models::WorktreeInfo>, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn cleanup_legacy_terminal_worktrees(
        &self,
        _: Option<&str>,
        _: &str,
    ) -> Result<usize, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn detect_worktree_strategy(
        &self,
        _: Option<&str>,
        _: &str,
    ) -> Result<crate::domain::models::WorktreeStrategy, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn clone_repository(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn fetch_origin_refspec(
        &self,
        _: Option<&str>,
        _: &str,
        _: &crate::domain::feature_origin::Refspec,
    ) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn cut_branch_at(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn create_feature_branch(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn provision_subtask_worktree(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<String, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn cleanup_subtask_worktree(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        subtask_id: &str,
    ) -> Result<(), String> {
        self.cleanup_calls
            .lock()
            .unwrap()
            .push(subtask_id.to_string());
        if self.fail_for.contains(subtask_id) {
            return Err(format!("cleanup refused for {subtask_id}"));
        }
        Ok(())
    }
    async fn branch_delete(&self, _: Option<&str>, _: &str, _: &str) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn merge_subtask(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn sync_feature_with_upstream(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
        _: crate::ports::worktree_ops::MergeGate<'_>,
    ) -> Result<crate::ports::worktree_ops::SyncOutcome, crate::ports::worktree_ops::SyncFailure>
    {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn validate_commit_message(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
    ) -> Result<(), crate::ports::worktree_ops::CommitMessageRejected> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn squash_feature_branch(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<crate::ports::worktree_ops::SquashOutcome, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn restore_pre_squash(&self, _: Option<&str>, _: &str, _: &str) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
}

/// An [`ExecutionPort`] stub that answers only `run_command`, with a single
/// canned result — enough for [`commit_sha`], which reads nothing else off
/// the port. Every other method panics, so a change that makes `commit_sha`
/// reach for a different port method fails loudly here instead of quietly
/// passing against a double that happens to answer everything.
struct FakeExec {
    run_command_result: Result<String, String>,
    commands: Mutex<Vec<String>>,
}

impl FakeExec {
    fn new(run_command_result: Result<String, String>) -> Self {
        Self {
            run_command_result,
            commands: Mutex::new(Vec::new()),
        }
    }

    fn commands(&self) -> Vec<String> {
        self.commands
            .lock()
            .expect("the mutex is not poisoned")
            .clone()
    }
}

#[async_trait]
impl ExecutionPort for FakeExec {
    async fn test_connection(&self, _: &str) -> Result<(), String> {
        panic!("unexpected ExecutionPort call")
    }
    async fn run_command(&self, _: &str, cmd: &str) -> Result<String, String> {
        self.commands
            .lock()
            .expect("the mutex is not poisoned")
            .push(cmd.to_string());
        self.run_command_result.clone()
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
    async fn get_metadata(&self, _: &str, _: &str) -> Result<SftpEntry, String> {
        panic!("unexpected ExecutionPort call")
    }
    async fn list_dir(&self, _: &str, _: &str) -> Result<Vec<SftpEntry>, String> {
        panic!("unexpected ExecutionPort call")
    }
    async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
        panic!("unexpected ExecutionPort call")
    }
    async fn resolve_home(&self, _: &str) -> Result<String, String> {
        panic!("unexpected ExecutionPort call")
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

/// `git worktree add --detach` never leaves a symbolic ref, so a checkout
/// with no branch to report is exactly what tells the test the worktree was
/// provisioned by [`ensure`] rather than by a branch-creating variant — there
/// is no `WorktreeOpsPort` call to spy on for `ensure` itself, since
/// [`GitOpsHelper`] talks to `ctx.exec` directly.
async fn is_detached(exec: &LocalSubprocessAdapter, worktree_path: &str) -> bool {
    exec.run_command(
        "local",
        &format!("git -C \"{worktree_path}\" symbolic-ref -q HEAD"),
    )
    .await
    .is_err()
}

#[tokio::test]
async fn ensure_provisions_a_detached_worktree_denied_to_every_write() {
    let (ctx, project_id, repo_dir) = fixture("ensure").await;
    init_repo_at(&repo_dir).await;
    let t = thread(&project_id, "t-ensure", None);
    ctx.ask.create(&t).expect("the thread is stored");

    let repo = resolve(&ctx, &t).await.expect("the repo resolves");
    let path = ensure(&ctx, &t, &repo)
        .await
        .expect("the worktree provisions");

    let exec = LocalSubprocessAdapter::new();
    assert!(
        is_detached(&exec, &path).await,
        "ensure must provision a detached worktree, never a branch-creating variant"
    );

    // NONE_WRITABLE deny-all is the only shape of `apply_artifact_scope` call
    // that fences an already-tracked file: an empty slice with no
    // NONE_WRITABLE sentinel is the port's own back-compat no-op (scope.rs),
    // so a write succeeding here would mean `ensure` called it with the
    // wrong argument.
    let write = exec
        .write_file("local", &format!("{path}/README.md"), "hijacked")
        .await;
    assert!(
        write.is_err(),
        "apply_artifact_scope must have been called with exactly [NONE_WRITABLE]"
    );

    let stored = ctx
        .ask
        .get(&t.id)
        .expect("the thread reads back")
        .expect("the thread exists");
    assert_eq!(stored.worktree_path.as_deref(), Some(path.as_str()));
}

#[tokio::test]
async fn reclaim_clears_the_stored_path_even_when_cleanup_fails() {
    let (mut ctx, project_id, repo_dir) = fixture("reclaim-fail").await;
    init_repo_at(&repo_dir).await;
    let t = thread(&project_id, "t-reclaim-fail", Some("/tmp/whatever"));
    ctx.ask.create(&t).expect("the thread is stored");
    let spy = Arc::new(SpyWorktreeOps::new(&["ask-t-reclaim-fail"]));
    ctx.worktree_ops = spy.clone();

    let outcome = reclaim(&ctx, &t).await;
    assert!(outcome.is_err(), "the cleanup failure must propagate");
    assert_eq!(spy.calls(), vec!["ask-t-reclaim-fail".to_string()]);

    let stored = ctx
        .ask
        .get(&t.id)
        .expect("the thread reads back")
        .expect("the thread exists");
    assert_eq!(
        stored.worktree_path, None,
        "the stored path must be cleared even though cleanup failed"
    );
}

#[tokio::test]
async fn reclaim_idle_continues_past_one_threads_failure() {
    let (mut ctx, project_id, repo_dir) = fixture("reclaim-idle").await;
    init_repo_at(&repo_dir).await;
    let failing = thread(&project_id, "t-idle-fail", Some("/tmp/a"));
    let succeeding = thread(&project_id, "t-idle-ok", Some("/tmp/b"));
    ctx.ask.create(&failing).expect("the thread is stored");
    ctx.ask.create(&succeeding).expect("the thread is stored");
    let spy = Arc::new(SpyWorktreeOps::new(&["ask-t-idle-fail"]));
    ctx.worktree_ops = spy.clone();

    let cutoff = crate::paths::now_ms() + 1_000;
    let reclaimed = reclaim_idle(&ctx, cutoff)
        .await
        .expect("the sweep itself does not fail");

    assert_eq!(
        reclaimed,
        vec!["t-idle-ok".to_string()],
        "only the thread whose cleanup succeeded is reported reclaimed"
    );
    let mut calls = spy.calls();
    calls.sort();
    assert_eq!(
        calls,
        vec!["ask-t-idle-fail".to_string(), "ask-t-idle-ok".to_string()],
        "one thread's failure must not stop the sweep over the rest"
    );

    for id in ["t-idle-fail", "t-idle-ok"] {
        let stored = ctx
            .ask
            .get(&AskThreadId::from(id.to_string()))
            .expect("the thread reads back")
            .expect("the thread exists");
        assert_eq!(
            stored.worktree_path, None,
            "{id}'s stored path is cleared regardless of the cleanup outcome"
        );
    }
}

#[tokio::test]
async fn commit_sha_returns_the_trimmed_sha() {
    let (mut ctx, _project_id, _repo_dir) = fixture("commit-sha-ok").await;
    ctx.exec = Arc::new(FakeExec::new(Ok("deadbeef1234\n".to_string())));

    let sha = commit_sha(&ctx, "local", "/some/worktree")
        .await
        .expect("commit_sha succeeds");
    assert_eq!(sha, "deadbeef1234");
}

#[tokio::test]
async fn commit_sha_propagates_the_run_command_error() {
    let (mut ctx, _project_id, _repo_dir) = fixture("commit-sha-err").await;
    ctx.exec = Arc::new(FakeExec::new(Err("not a git repository".to_string())));

    let err = commit_sha(&ctx, "local", "/some/worktree")
        .await
        .expect_err("commit_sha propagates the underlying error");
    assert_eq!(err, "not a git repository");
}

/// A worktree path carrying a shell metacharacter must never be interpolated
/// raw into the command string `commit_sha` hands to `ExecutionPort::run_command`,
/// which runs it through a POSIX shell (ports/execution.rs). The path must
/// come out through `paths::shell_escape_posix`, matching the identical
/// `git -C <path> rev-parse HEAD` shape in `application/sync_session.rs`.
#[tokio::test]
async fn commit_sha_shell_escapes_a_metacharacter_in_the_worktree_path() {
    let (mut ctx, _project_id, _repo_dir) = fixture("commit-sha-escape").await;
    let exec = Arc::new(FakeExec::new(Ok("deadbeef\n".to_string())));
    ctx.exec = exec.clone();

    let malicious = r#"/tmp/worktree"; touch pwned; echo ""#;
    commit_sha(&ctx, "local", malicious)
        .await
        .expect("commit_sha succeeds");

    let commands = exec.commands();
    assert_eq!(commands.len(), 1);
    let expected = format!(
        "git -C {} rev-parse HEAD",
        crate::paths::shell_escape_posix(malicious)
    );
    assert_eq!(
        commands[0], expected,
        "the worktree path must be escaped via shell_escape_posix, not interpolated raw"
    );
    assert!(
        commands[0].starts_with("git -C '"),
        "an unsafe path must come out single-quoted rather than bare-interpolated: {}",
        commands[0]
    );
}

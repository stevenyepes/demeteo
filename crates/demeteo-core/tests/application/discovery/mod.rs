// Tests extracted from `crates/demeteo-core/src/application/discovery/mod.rs` (mirrored-tests convention). `super` = that module.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::branch_listing::BranchOption;
use crate::domain::ids::{MachineId, ProviderId, RepositoryId};
use crate::domain::models::ticket::{Ticket, TicketState};
use crate::domain::models::{Machine, Platform, Project, Repository, TITLE_MAX_CHARS};
use crate::ports::execution::{ExecutionPort, InteractiveHandle, SftpEntry};
use crate::ports::worktree_ops::WorktreeOpsPort;

/// A local project with nothing in it, which is as much as `create` reads.
fn fixture(tag: &str) -> (AppContext, ProjectId) {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-discovery-create-{tag}-{}",
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
    (ctx, project_id)
}

fn opening(project_id: &ProjectId, title: &str) -> NewDiscovery {
    NewDiscovery {
        project_id: project_id.as_str().to_string(),
        title: title.to_string(),
        agent_kind: "claude-code".to_string(),
        model: None,
        effort: None,
        machine_id: None,
        staged_attachments: Vec::new(),
    }
}

/// The title labels the row and is read by no prompt, so a Discovery that has
/// only been opened has had nothing said in it — and the interviewer's first
/// turn is about the first thing the user sends, not about the name they
/// filed it under.
#[tokio::test]
async fn opening_a_discovery_says_nothing_in_it() {
    let (ctx, project_id) = fixture("silent");
    let discovery =
        create(&ctx, opening(&project_id, "  chat + canvas  ")).expect("the discovery opens");

    assert_eq!(discovery.title, "chat + canvas");
    assert!(ctx
        .discoveries
        .list_messages(&discovery.id)
        .expect("the transcript reads back")
        .is_empty());
}

/// The cap is enforced where the row is written, not only where it is typed:
/// the modal's own limit is a courtesy to the user, and every other caller of
/// `discovery_create` reaches this one.
#[tokio::test]
async fn a_name_long_enough_to_be_an_idea_is_refused() {
    let (ctx, project_id) = fixture("capped");
    let idea = "I want to add a chat option so users can ask questions about the project, and \
                generate an interactive canvas for the architecture.";
    assert!(idea.chars().count() > TITLE_MAX_CHARS);

    let refusal =
        create(&ctx, opening(&project_id, idea)).expect_err("a name that long is refused");
    assert!(refusal.contains(&TITLE_MAX_CHARS.to_string()), "{refusal}");

    assert!(create(&ctx, opening(&project_id, "chat + canvas")).is_ok());
}

/// [`set_base`] needs a repository to resolve, unlike [`fixture`]'s bare
/// project — added separately so the zero-repositories case can still use
/// [`fixture`] on its own.
///
/// Also creates the local repo directory `worktree::resolve`'s
/// existing-checkout probe stats, the same convention
/// `tests/application/ask/worktree.rs`'s `fixture`/`init_repo_at` uses for it
/// — a plain directory is enough since `list_terminal_branches` stays behind
/// [`FakeBranches`].
fn add_repo(ctx: &AppContext, project_id: &ProjectId, tag: &str) {
    ctx.projects
        .add_repository(Repository {
            id: RepositoryId::from(format!("r-{tag}")),
            project_id: project_id.clone(),
            provider_id: ProviderId::from("provider"),
            repo_path: "repo".to_string(),
        })
        .expect("the repository is stored");
    let repo_dir =
        crate::paths::repo_target_dir_local(&ctx.workspace_dir, project_id.as_str(), "repo");
    std::fs::create_dir_all(&repo_dir).expect("creates the repo directory");
}

/// Registers the repository row a remote-machine [`set_base`] resolves
/// against, without [`add_repo`]'s local checkout directory — the
/// remote-machine path never touches it, since its existing-checkout probe
/// goes through [`FakeRemoteExec::get_metadata`] instead.
fn add_repo_remote(ctx: &AppContext, project_id: &ProjectId, tag: &str) {
    ctx.projects
        .add_repository(Repository {
            id: RepositoryId::from(format!("r-{tag}")),
            project_id: project_id.clone(),
            provider_id: ProviderId::from("provider"),
            repo_path: "repo".to_string(),
        })
        .expect("the repository is stored");
}

/// A machine `set_base`'s remote branch can resolve `discovery.machine_id`
/// against, the same `add_machine` convention `tests/application/ask/mod.rs`
/// uses for the same purpose.
fn add_machine(ctx: &AppContext, id: &str) {
    ctx.machines
        .add(Machine {
            id: MachineId::from(id.to_string()),
            name: id.to_string(),
            host: "example.internal".to_string(),
            port: 22,
            username: "demeteo".to_string(),
            auth_type: "key".to_string(),
            key_path: None,
            agents: None,
            auto_approved_rules: None,
            use_login_shell: Some(false),
            setup_commands: None,
            notify_webhook_url: None,
        })
        .expect("the machine is stored");
}

fn ticket(discovery_id: &DiscoveryId, id: &str, seq: i64, state: TicketState) -> Ticket {
    Ticket {
        id: crate::domain::ids::TicketId::from(id.to_string()),
        discovery_id: discovery_id.clone(),
        seq,
        title: format!("ticket {seq}"),
        description: String::new(),
        acceptance: Vec::new(),
        files: Vec::new(),
        blocked_by: Vec::new(),
        test_command: None,
        workflow_id: None,
        agent_kind: None,
        model: None,
        effort: None,
        attachments: Vec::new(),
        state,
        drop_reason: None,
        force_start_reason: None,
        force_started_at: None,
        feature_id: None,
        created_at: 0,
        updated_at: 0,
    }
}

/// A [`WorktreeOpsPort`] spy that answers only `list_terminal_branches` with
/// a fixed set, panicking on any other call — the same `SpyWorktreeOps`
/// convention `tests/application/ask/worktree.rs` uses, narrowed to the one
/// method `set_base` calls.
///
/// `calls` records the `(machine_id, repo_dir)` of every `list_terminal_branches`
/// call, unused by most tests but read by the remote-machine test to confirm
/// the resolved machine — not `None` from a wrongly-resolved local path —
/// actually reached this port.
struct FakeBranches {
    branches: Vec<BranchOption>,
    calls: Mutex<Vec<(Option<String>, String)>>,
}

impl FakeBranches {
    fn with(branches: Vec<BranchOption>) -> Arc<Self> {
        Arc::new(Self {
            branches,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<(Option<String>, String)> {
        self.calls
            .lock()
            .expect("the mutex is not poisoned")
            .clone()
    }
}

#[async_trait]
impl WorktreeOpsPort for FakeBranches {
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
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<Vec<BranchOption>, String> {
        self.calls
            .lock()
            .expect("the mutex is not poisoned")
            .push((machine_id.map(str::to_string), repo_dir.to_string()));
        Ok(self.branches.clone())
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
    async fn create_feature_branch(
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
        _: &str,
    ) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
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

fn remote_branch(name: &str) -> BranchOption {
    BranchOption {
        name: name.to_string(),
        has_local: false,
        has_remote: true,
    }
}

/// An [`ExecutionPort`] stub for `set_base`'s remote-machine branch: it
/// answers only `resolve_home` (backing `paths::project_root`'s remote
/// branch, reached through `worktree::resolve`'s repo-dir resolution) and
/// `get_metadata` (backing `worktree::resolve`'s existing-checkout probe),
/// panicking on every other call — the same panic-on-unexpected-call
/// convention `tests/application/ask/worktree.rs`'s `FakeExec` uses.
///
/// Both calls are logged with the `machine_id` they were made against, so the
/// test can assert the remote machine actually reached the port instead of a
/// wrongly-resolved local path silently taking the same happy path.
struct FakeRemoteExec {
    calls: Mutex<Vec<String>>,
}

impl FakeRemoteExec {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("the mutex is not poisoned")
            .clone()
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
        self.calls
            .lock()
            .expect("the mutex is not poisoned")
            .push(format!("get_metadata({machine_id})"));
        Ok(SftpEntry {
            name: "repo".to_string(),
            path: path.to_string(),
            is_dir: true,
            size: 0,
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
        self.calls
            .lock()
            .expect("the mutex is not poisoned")
            .push(format!("resolve_home({machine_id})"));
        Ok("/home/remote".to_string())
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

/// The remote branch of `set_base`'s machine/repo resolution
/// (`worktree::resolve`'s `else` arms) — reached when a Discovery's own
/// `machine_id` names a host other than local, per §4.5's rule that the
/// interview's host is part of the interviewer choice. Every other test in
/// this file opens through [`opening`], which hardcodes `machine_id: None`,
/// so nothing else exercises this branch: this is the one place a broken
/// remote-existence check or a machine id dropped on the way to
/// `list_terminal_branches` would hide.
#[tokio::test]
async fn set_base_adopts_a_known_remote_branch_on_a_remote_machine() {
    let (mut ctx, project_id) = fixture("remote-adopt");
    add_repo_remote(&ctx, &project_id, "remote-adopt");
    add_machine(&ctx, "m-remote");
    let branches = FakeBranches::with(vec![remote_branch("feat/payments")]);
    ctx.worktree_ops = branches.clone();
    let exec = Arc::new(FakeRemoteExec::new());
    ctx.exec = exec.clone();
    let discovery = create(
        &ctx,
        NewDiscovery {
            machine_id: Some("m-remote".to_string()),
            ..opening(&project_id, "remote-adopt")
        },
    )
    .expect("the discovery opens");
    assert_eq!(discovery.machine_id.as_str(), "m-remote");

    let updated = set_base(&ctx, &discovery.id, Some("feat/payments".to_string()))
        .await
        .expect("a known remote branch is adopted");
    assert_eq!(updated.base_branch.as_deref(), Some("feat/payments"));

    let reread = load(&ctx, &discovery.id).expect("the discovery reads back");
    assert_eq!(reread.base_branch.as_deref(), Some("feat/payments"));

    assert_eq!(
        exec.calls(),
        vec![
            "resolve_home(m-remote)".to_string(),
            "get_metadata(m-remote)".to_string(),
        ],
        "resolution must reach the remote machine, not a wrongly-resolved local path"
    );
    assert_eq!(
        branches
            .calls()
            .first()
            .map(|(machine_id, _)| machine_id.as_deref()),
        Some(Some("m-remote")),
        "the resolved machine id must reach list_terminal_branches, not None for a local checkout"
    );
}

/// Adopting a branch origin actually has stores it, and a later read shows it.
#[tokio::test]
async fn set_base_adopts_a_known_remote_branch() {
    let (mut ctx, project_id) = fixture("adopt");
    add_repo(&ctx, &project_id, "adopt");
    ctx.worktree_ops = FakeBranches::with(vec![remote_branch("feat/payments")]);
    let discovery = create(&ctx, opening(&project_id, "adopt")).expect("the discovery opens");

    let updated = set_base(&ctx, &discovery.id, Some("feat/payments".to_string()))
        .await
        .expect("a known remote branch is adopted");
    assert_eq!(updated.base_branch.as_deref(), Some("feat/payments"));

    let reread = load(&ctx, &discovery.id).expect("the discovery reads back");
    assert_eq!(reread.base_branch.as_deref(), Some("feat/payments"));
}

/// A name with no matching remote ref is refused by name, not treated as a
/// request to cut one — branch creation is out of scope here, not forever.
#[tokio::test]
async fn set_base_refuses_a_branch_with_no_remote_ref() {
    let (mut ctx, project_id) = fixture("unknown-branch");
    add_repo(&ctx, &project_id, "unknown-branch");
    ctx.worktree_ops = FakeBranches::with(vec![remote_branch("main")]);
    let discovery = create(&ctx, opening(&project_id, "unknown")).expect("the discovery opens");

    let refusal = set_base(&ctx, &discovery.id, Some("ghost-branch".to_string()))
        .await
        .expect_err("an unknown branch is refused");
    assert!(refusal.contains("ghost-branch"), "{refusal}");

    let reread = load(&ctx, &discovery.id).expect("the discovery reads back");
    assert_eq!(reread.base_branch, None);
}

/// Once any ticket has left `Unstarted`, the base is locked — clearing it is
/// refused exactly like adopting a new one.
#[tokio::test]
async fn set_base_is_locked_once_a_ticket_has_started() {
    let (mut ctx, project_id) = fixture("locked");
    add_repo(&ctx, &project_id, "locked");
    ctx.worktree_ops = FakeBranches::with(vec![remote_branch("main")]);
    let discovery = create(&ctx, opening(&project_id, "locked")).expect("the discovery opens");
    ctx.tickets
        .upsert_batch(&[ticket(&discovery.id, "t-1", 1, TicketState::Started)])
        .expect("the ticket is stored");

    let refusal = set_base(&ctx, &discovery.id, Some("main".to_string()))
        .await
        .expect_err("a started ticket locks the base branch");
    assert!(!refusal.is_empty());

    let clear_refusal = set_base(&ctx, &discovery.id, None)
        .await
        .expect_err("clearing is refused too while locked");
    assert!(!clear_refusal.is_empty());

    let reread = load(&ctx, &discovery.id).expect("the discovery reads back");
    assert_eq!(reread.base_branch, None);
}

/// Clearing after a prior adoption restores the project's default branch.
#[tokio::test]
async fn set_base_clears_back_to_the_default_branch() {
    let (mut ctx, project_id) = fixture("clear");
    add_repo(&ctx, &project_id, "clear");
    ctx.worktree_ops = FakeBranches::with(vec![remote_branch("release/v2")]);
    let discovery = create(&ctx, opening(&project_id, "clear")).expect("the discovery opens");
    set_base(&ctx, &discovery.id, Some("release/v2".to_string()))
        .await
        .expect("the branch adopts");

    let cleared = set_base(&ctx, &discovery.id, None)
        .await
        .expect("clearing an unlocked discovery succeeds");
    assert_eq!(cleared.base_branch, None);

    let reread = load(&ctx, &discovery.id).expect("the discovery reads back");
    assert_eq!(reread.base_branch, None);
}

/// A Discovery with no tickets at all has nothing to lock it — the vacuous
/// case of [`base_branch_lock_refusal`].
#[tokio::test]
async fn set_base_with_no_tickets_is_unconditionally_editable() {
    let (mut ctx, project_id) = fixture("vacuous");
    add_repo(&ctx, &project_id, "vacuous");
    ctx.worktree_ops = FakeBranches::with(vec![remote_branch("main")]);
    let discovery = create(&ctx, opening(&project_id, "vacuous")).expect("the discovery opens");

    let updated = set_base(&ctx, &discovery.id, Some("main".to_string()))
        .await
        .expect("no tickets means nothing locks the base branch");
    assert_eq!(updated.base_branch.as_deref(), Some("main"));
}

/// A project with zero configured repositories cannot resolve one to check
/// remote branches against — a clear error, not a panic on `.first()`.
#[tokio::test]
async fn set_base_with_no_repository_configured_is_an_error_not_a_panic() {
    let (ctx, project_id) = fixture("no-repo");
    let discovery = create(&ctx, opening(&project_id, "no-repo")).expect("the discovery opens");

    let refusal = set_base(&ctx, &discovery.id, Some("main".to_string()))
        .await
        .expect_err("no repository means set_base cannot resolve one");
    assert!(!refusal.is_empty());
}

/// Re-adopting the currently-stored branch succeeds like any other `Some`.
#[tokio::test]
async fn set_base_reagrees_to_the_current_branch() {
    let (mut ctx, project_id) = fixture("reagree");
    add_repo(&ctx, &project_id, "reagree");
    ctx.worktree_ops = FakeBranches::with(vec![remote_branch("main")]);
    let discovery = create(&ctx, opening(&project_id, "reagree")).expect("the discovery opens");
    set_base(&ctx, &discovery.id, Some("main".to_string()))
        .await
        .expect("the branch adopts");

    let updated = set_base(&ctx, &discovery.id, Some("main".to_string()))
        .await
        .expect("re-adopting the current branch succeeds");
    assert_eq!(updated.base_branch.as_deref(), Some("main"));
}

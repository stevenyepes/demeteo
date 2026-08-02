use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId};
use crate::domain::models::{Project, Repository, WorktreeInfo, WorktreeStrategy};
use crate::ports::worktree_ops::{
    CommitMessageRejected, SquashOutcome, SyncFailure, SyncOutcome, WorktreeOpsPort,
};
use crate::state::AppContext;
use async_trait::async_trait;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

static CONTEXT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, PartialEq)]
enum WorktreeCall {
    List {
        machine: Option<String>,
        repo_dir: String,
    },
    Create {
        machine: Option<String>,
        repo_dir: String,
        project_root: String,
        branch: String,
        name: String,
    },
}

/// Strictly records the two calls this policy is allowed to make. Every other
/// WorktreeOpsPort operation panics, making accidental Git-policy expansion
/// visible without constructing an ExecutionDriver.
struct RecordingWorktrees {
    calls: Arc<Mutex<Vec<WorktreeCall>>>,
    expected_calls: Mutex<Vec<WorktreeCall>>,
}

impl RecordingWorktrees {
    fn expect(&self, call: WorktreeCall) {
        self.expected_calls.lock().unwrap().push(call);
    }

    fn record(&self, call: WorktreeCall) -> Result<(), String> {
        let mut expected = self.expected_calls.lock().unwrap();
        match expected.first() {
            Some(expected_call) if expected_call == &call => {
                expected.remove(0);
                self.calls.lock().unwrap().push(call);
                Ok(())
            }
            Some(expected_call) => Err(format!(
                "unexpected WorktreeOpsPort call {call:?}; expected {expected_call:?}"
            )),
            None => Err(format!(
                "unexpected unconfigured WorktreeOpsPort call {call:?}"
            )),
        }
    }
}

#[async_trait]
impl WorktreeOpsPort for RecordingWorktrees {
    async fn check_repo_dirty(&self, _: Option<&str>, _: &str) -> Result<(bool, bool), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn get_head_branch(&self, _: Option<&str>, _: &str) -> Option<String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn list_worktrees(
        &self,
        machine: Option<&str>,
        repo_dir: &str,
    ) -> Result<Vec<WorktreeInfo>, String> {
        self.record(WorktreeCall::List {
            machine: machine.map(str::to_string),
            repo_dir: repo_dir.to_string(),
        })?;
        Ok(vec![
            WorktreeInfo {
                path: format!("{repo_dir}_wt_f-1-step-s-implement"),
                branch: Some("feature/one_subtask_s-implement".to_string()),
                is_locked: false,
            },
            // Deliberately shares no prefix with the workspace this context
            // resolves: git reports the path it resolved physically at add
            // time, so a filter comparing against the logical area would keep
            // nothing here.
            WorktreeInfo {
                path: format!(
                    "/physical/projects/p/{}/repo/existing",
                    crate::paths::TERMINAL_WORKTREES_SUBDIR
                ),
                branch: Some("terminal/existing".to_string()),
                is_locked: false,
            },
        ])
    }
    async fn create_terminal_worktree(
        &self,
        machine: Option<&str>,
        repo_dir: &str,
        project_root: &str,
        branch: &str,
        name: &str,
    ) -> Result<WorktreeInfo, String> {
        self.record(WorktreeCall::Create {
            machine: machine.map(str::to_string),
            repo_dir: repo_dir.to_string(),
            project_root: project_root.to_string(),
            branch: branch.to_string(),
            name: name.to_string(),
        })?;
        Ok(WorktreeInfo {
            path: format!("{repo_dir}-{name}"),
            branch: Some(branch.to_string()),
            is_locked: false,
        })
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
    ) -> Result<WorktreeStrategy, String> {
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
    ) -> Result<SyncOutcome, SyncFailure> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn validate_commit_message(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
    ) -> Result<(), CommitMessageRejected> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn squash_feature_branch(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<SquashOutcome, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn restore_pre_squash(&self, _: Option<&str>, _: &str, _: &str) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
}

fn context() -> (
    AppContext,
    Arc<RecordingWorktrees>,
    Arc<Mutex<Vec<WorktreeCall>>>,
) {
    let sequence = CONTEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let app_data_dir = std::env::temp_dir().join(format!(
        "demeteo-terminal-worktree-projects-{}-{sequence}",
        crate::paths::now_ms()
    ));
    let mut ctx = build_core_context(
        CoreConfig {
            app_data_dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
    let calls = Arc::new(Mutex::new(Vec::new()));
    let worktrees = Arc::new(RecordingWorktrees {
        calls: calls.clone(),
        expected_calls: Mutex::new(Vec::new()),
    });
    ctx.worktree_ops = worktrees.clone();
    (ctx, worktrees, calls)
}

fn add_project(ctx: &AppContext, id: &str, compute_type: &str, machine: Option<&str>) {
    ctx.projects
        .add(Project {
            id: ProjectId::from(id),
            name: id.to_string(),
            compute_type: compute_type.to_string(),
            remote_host: machine.map(MachineId::from),
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .unwrap();
}

fn add_repo(ctx: &AppContext, id: &str, project_id: &str, repo_path: &str) {
    ctx.projects
        .add_repository(Repository {
            id: RepositoryId::from(id),
            project_id: ProjectId::from(project_id),
            provider_id: ProviderId::from("provider"),
            repo_path: repo_path.to_string(),
        })
        .unwrap();
}

#[tokio::test]
async fn list_resolves_a_local_project_repository_before_calling_the_port() {
    let (ctx, worktree_port, calls) = context();
    add_project(&ctx, "p-local", "local", None);
    add_repo(&ctx, "r-local", "p-local", "org/local-repo");
    let repo_dir = ctx
        .workspace_dir
        .join("projects/p-local/repos/local-repo")
        .to_string_lossy()
        .to_string();
    worktree_port.expect(WorktreeCall::List {
        machine: None,
        repo_dir: repo_dir.clone(),
    });

    let worktrees = list_terminal_worktrees(&ctx, "p-local".to_string(), "r-local".to_string())
        .await
        .unwrap();

    assert_eq!(worktrees[0].branch.as_deref(), Some("terminal/existing"));
    assert_eq!(
        *calls.lock().unwrap(),
        [WorktreeCall::List {
            machine: None,
            repo_dir
        }]
    );
}

#[tokio::test]
async fn create_resolves_a_remote_project_machine_and_repository_before_calling_the_port() {
    let (ctx, worktree_port, calls) = context();
    add_project(&ctx, "p-remote", "remote", Some("machine-remote"));
    add_repo(&ctx, "r-remote", "p-remote", "org/remote-repo");
    let project_root = format!(
        "{}/.demeteo/projects/p-remote",
        std::env::var("HOME").unwrap()
    );
    let repo_dir = format!("{project_root}/repos/remote-repo");
    worktree_port.expect(WorktreeCall::Create {
        machine: Some("machine-remote".to_string()),
        repo_dir: repo_dir.clone(),
        project_root: project_root.clone(),
        branch: "terminal/new".to_string(),
        name: "new".to_string(),
    });

    let worktree = create_terminal_worktree(
        &ctx,
        "p-remote".to_string(),
        "r-remote".to_string(),
        "terminal/new".to_string(),
        "new".to_string(),
    )
    .await
    .unwrap();

    assert_eq!(worktree.branch.as_deref(), Some("terminal/new"));
    assert_eq!(
        *calls.lock().unwrap(),
        [WorktreeCall::Create {
            machine: Some("machine-remote".to_string()),
            repo_dir,
            project_root,
            branch: "terminal/new".to_string(),
            name: "new".to_string()
        }]
    );
}

/// A `{repo}_wt_{subtask}` checkout belongs to a running pipeline step, which
/// force-removes it when the feature ends. Offering it as a session location
/// hands the user a directory that disappears mid-edit, so the listing must
/// drop it and keep only the terminal area.
#[tokio::test]
async fn subtask_worktrees_are_not_offered_as_terminal_locations() {
    let (ctx, worktree_port, _calls) = context();
    add_project(&ctx, "p-local", "local", None);
    add_repo(&ctx, "r-local", "p-local", "org/local-repo");
    let repo_dir = ctx
        .workspace_dir
        .join("projects/p-local/repos/local-repo")
        .to_string_lossy()
        .to_string();
    worktree_port.expect(WorktreeCall::List {
        machine: None,
        repo_dir,
    });

    let worktrees = list_terminal_worktrees(&ctx, "p-local".to_string(), "r-local".to_string())
        .await
        .unwrap();

    assert_eq!(
        worktrees
            .iter()
            .map(|worktree| worktree.branch.as_deref())
            .collect::<Vec<_>>(),
        [Some("terminal/existing")],
        "only the terminal-owned worktree may be offered: {worktrees:?}"
    );
}

#[tokio::test]
async fn unknown_project_or_repository_is_rejected_before_port_io() {
    let (ctx, _worktree_port, calls) = context();
    add_project(&ctx, "p-known", "local", None);
    add_repo(&ctx, "r-known", "p-known", "org/repo");

    let missing_project =
        list_terminal_worktrees(&ctx, "p-missing".to_string(), "r-known".to_string())
            .await
            .unwrap_err();
    let missing_repo = create_terminal_worktree(
        &ctx,
        "p-known".to_string(),
        "r-missing".to_string(),
        "terminal/new".to_string(),
        "new".to_string(),
    )
    .await
    .unwrap_err();

    assert!(missing_project.contains("Project not found"));
    assert!(missing_repo.contains("does not belong"));
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn repository_owned_by_another_project_is_rejected_before_port_io() {
    let (ctx, _worktree_port, calls) = context();
    add_project(&ctx, "p-one", "local", None);
    add_project(&ctx, "p-two", "local", None);
    add_repo(&ctx, "r-two", "p-two", "org/other-repo");

    let error = list_terminal_worktrees(&ctx, "p-one".to_string(), "r-two".to_string())
        .await
        .unwrap_err();

    assert!(error.contains("does not belong to project p-one"));
    assert!(calls.lock().unwrap().is_empty());
}

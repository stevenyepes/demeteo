use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::branch_listing::BranchOption;
use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId};
use crate::domain::models::{Project, Repository, WorktreeInfo, WorktreeStrategy};
use crate::ports::worktree_ops::{
    CommitMessageRejected, SquashOutcome, SyncFailure, SyncOutcome, TerminalWorktreeCreated,
    TerminalWorktreeRequest, WorktreeOpsPort,
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
    ListTerminal {
        machine: Option<String>,
        repo_dir: String,
        project_root: String,
    },
    Create {
        machine: Option<String>,
        repo_dir: String,
        project_root: String,
        branch: String,
        base: Option<String>,
        name: String,
    },
    Remove {
        machine: Option<String>,
        repo_dir: String,
        project_root: String,
        path: String,
        force: bool,
    },
    ListBranches {
        machine: Option<String>,
        repo_dir: String,
    },
    HeadBranch {
        machine: Option<String>,
        repo_dir: String,
    },
    ListWorktrees {
        machine: Option<String>,
        repo_dir: String,
    },
    CheckDirty {
        machine: Option<String>,
        repo_dir: String,
    },
}

/// Strictly records the calls this policy is allowed to make. Every other
/// WorktreeOpsPort operation panics, making accidental Git-policy expansion
/// visible without constructing an ExecutionDriver.
struct RecordingWorktrees {
    calls: Arc<Mutex<Vec<WorktreeCall>>>,
    expected_calls: Mutex<Vec<WorktreeCall>>,
    /// What `get_head_branch` answers — raw, exactly as `rev-parse` would, so
    /// a test can hand over the detached-HEAD spelling git actually emits.
    head: Mutex<Option<String>>,
}

impl RecordingWorktrees {
    fn expect(&self, call: WorktreeCall) {
        self.expected_calls.lock().unwrap().push(call);
    }

    fn answer_head(&self, raw: Option<&str>) {
        *self.head.lock().unwrap() = raw.map(str::to_string);
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
    async fn check_repo_dirty(
        &self,
        machine: Option<&str>,
        repo_dir: &str,
    ) -> Result<(bool, bool), String> {
        self.record(WorktreeCall::CheckDirty {
            machine: machine.map(str::to_string),
            repo_dir: repo_dir.to_string(),
        })?;
        Ok((false, false))
    }
    async fn get_head_branch(&self, machine: Option<&str>, repo_dir: &str) -> Option<String> {
        // Returns an answer, never a failure, so an unexpected call has to
        // panic rather than degrade into the `None` a detached HEAD produces.
        self.record(WorktreeCall::HeadBranch {
            machine: machine.map(str::to_string),
            repo_dir: repo_dir.to_string(),
        })
        .unwrap_or_else(|error| panic!("{error}"));
        self.head.lock().unwrap().clone()
    }
    async fn list_worktrees(
        &self,
        machine: Option<&str>,
        repo_dir: &str,
    ) -> Result<Vec<WorktreeInfo>, String> {
        // Answered only where a test expects it. The terminal listing must not
        // reach for the unfiltered one: its result carries the primary
        // checkout's siblings, including the worktrees a running step owns —
        // which is why an unexpected call fails rather than returning empty.
        self.record(WorktreeCall::ListWorktrees {
            machine: machine.map(str::to_string),
            repo_dir: repo_dir.to_string(),
        })?;
        Ok(Vec::new())
    }
    async fn list_terminal_worktrees(
        &self,
        machine: Option<&str>,
        repo_dir: &str,
        project_root: &str,
    ) -> Result<Vec<WorktreeInfo>, String> {
        self.record(WorktreeCall::ListTerminal {
            machine: machine.map(str::to_string),
            repo_dir: repo_dir.to_string(),
            project_root: project_root.to_string(),
        })?;
        Ok(vec![WorktreeInfo {
            path: format!(
                "/physical/projects/p/{}/repo/existing",
                crate::paths::TERMINAL_WORKTREES_SUBDIR
            ),
            branch: Some("terminal/existing".to_string()),
            is_locked: false,
        }])
    }
    async fn create_terminal_worktree(
        &self,
        machine: Option<&str>,
        repo_dir: &str,
        project_root: &str,
        request: &TerminalWorktreeRequest,
    ) -> Result<TerminalWorktreeCreated, String> {
        self.record(WorktreeCall::Create {
            machine: machine.map(str::to_string),
            repo_dir: repo_dir.to_string(),
            project_root: project_root.to_string(),
            branch: request.branch.clone(),
            base: request.base_branch.clone(),
            name: request.worktree_name.clone(),
        })?;
        Ok(TerminalWorktreeCreated {
            worktree: WorktreeInfo {
                path: format!("{repo_dir}-{}", request.worktree_name),
                branch: Some(request.branch.clone()),
                is_locked: false,
            },
            base_ref: request
                .base_branch
                .as_ref()
                .map(|base| format!("origin/{base}"))
                .unwrap_or_else(|| "HEAD".to_string()),
        })
    }
    async fn remove_terminal_worktree(
        &self,
        machine: Option<&str>,
        repo_dir: &str,
        project_root: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<(), String> {
        self.record(WorktreeCall::Remove {
            machine: machine.map(str::to_string),
            repo_dir: repo_dir.to_string(),
            project_root: project_root.to_string(),
            path: worktree_path.to_string(),
            force,
        })
    }
    async fn list_terminal_branches(
        &self,
        machine: Option<&str>,
        repo_dir: &str,
    ) -> Result<Vec<BranchOption>, String> {
        self.record(WorktreeCall::ListBranches {
            machine: machine.map(str::to_string),
            repo_dir: repo_dir.to_string(),
        })?;
        Ok(vec![BranchOption {
            name: "main".to_string(),
            has_local: true,
            has_remote: true,
        }])
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
        head: Mutex::new(Some("chore/left-here-yesterday".to_string())),
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

/// The two strings this layer derives, built the way it derives them: one
/// `join` per component.
///
/// A literal `"projects/p-local"` is a *single* segment to `join`, which leaves
/// the separator inside it untouched — so on Windows the expectation carries a
/// `/` exactly where the production path carries a `\`, and the two describe
/// the same directory under different names.
fn local_layout(ctx: &AppContext, project_id: &str, repo_name: &str) -> (String, String) {
    let project_root = ctx.workspace_dir.join("projects").join(project_id);
    let repo_dir = project_root
        .join(crate::paths::REPOS_SUBDIR)
        .join(repo_name);
    (
        project_root.to_string_lossy().into_owned(),
        repo_dir.to_string_lossy().into_owned(),
    )
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
    let (project_root, repo_dir) = local_layout(&ctx, "p-local", "local-repo");
    worktree_port.expect(WorktreeCall::ListTerminal {
        machine: None,
        repo_dir: repo_dir.clone(),
        project_root: project_root.clone(),
    });
    worktree_port.expect(WorktreeCall::HeadBranch {
        machine: None,
        repo_dir: repo_dir.clone(),
    });

    let locations = list_terminal_locations(&ctx, "p-local".to_string(), "r-local".to_string())
        .await
        .unwrap();

    assert_eq!(
        locations.worktrees[0].branch.as_deref(),
        Some("terminal/existing")
    );
    assert_eq!(
        *calls.lock().unwrap(),
        [
            WorktreeCall::ListTerminal {
                machine: None,
                repo_dir: repo_dir.clone(),
                project_root
            },
            WorktreeCall::HeadBranch {
                machine: None,
                repo_dir
            }
        ],
        "the port needs the project root to anchor the area it classifies against"
    );
}

#[tokio::test]
async fn the_listing_names_the_branch_the_main_checkout_is_on() {
    let (ctx, worktree_port, _calls) = context();
    add_project(&ctx, "p-local", "local", None);
    add_repo(&ctx, "r-local", "p-local", "org/local-repo");
    let (project_root, repo_dir) = local_layout(&ctx, "p-local", "local-repo");
    worktree_port.expect(WorktreeCall::ListTerminal {
        machine: None,
        repo_dir: repo_dir.clone(),
        project_root,
    });
    worktree_port.expect(WorktreeCall::HeadBranch {
        machine: None,
        repo_dir,
    });
    worktree_port.answer_head(Some("chore/left-here-yesterday\n"));

    let locations = list_terminal_locations(&ctx, "p-local".to_string(), "r-local".to_string())
        .await
        .unwrap();

    assert_eq!(
        locations.main_branch.as_deref(),
        Some("chore/left-here-yesterday"),
        "a session opened at the main checkout starts on this branch, and nothing else in the listing says so"
    );
}

#[tokio::test]
async fn a_detached_main_checkout_names_no_branch() {
    let (ctx, worktree_port, _calls) = context();
    add_project(&ctx, "p-local", "local", None);
    add_repo(&ctx, "r-local", "p-local", "org/local-repo");
    let (project_root, repo_dir) = local_layout(&ctx, "p-local", "local-repo");
    worktree_port.expect(WorktreeCall::ListTerminal {
        machine: None,
        repo_dir: repo_dir.clone(),
        project_root,
    });
    worktree_port.expect(WorktreeCall::HeadBranch {
        machine: None,
        repo_dir,
    });
    worktree_port.answer_head(Some("HEAD"));

    let locations = list_terminal_locations(&ctx, "p-local".to_string(), "r-local".to_string())
        .await
        .unwrap();

    assert_eq!(
        locations.main_branch, None,
        "git's own word for a detached HEAD would render as a branch by that name"
    );
}

/// Both readers of `get_head_branch` render its answer as a branch name, so the
/// detached spelling has to be filtered in both. Settings' branch chip is the
/// second one, and it is reached from a different application function.
#[tokio::test]
async fn workspace_health_names_no_branch_for_a_detached_checkout() {
    let (mut ctx, worktree_port, _calls) = context();
    add_project(&ctx, "p-local", "local", None);
    add_repo(&ctx, "r-local", "p-local", "org/local-repo");
    let (_project_root, repo_dir) = local_layout(&ctx, "p-local", "local-repo");
    ctx.exec = Arc::new(
        crate::adapters::step_executor::scripted_exec::ScriptedExec::new(&[(
            &format!(
                "git -C {} rev-parse --is-inside-work-tree",
                crate::paths::shell_escape_posix(&repo_dir)
            ),
            Ok("true"),
        )]),
    );
    worktree_port.answer_head(Some("HEAD"));
    worktree_port.expect(WorktreeCall::HeadBranch {
        machine: None,
        repo_dir: repo_dir.clone(),
    });
    worktree_port.expect(WorktreeCall::ListWorktrees {
        machine: None,
        repo_dir: repo_dir.clone(),
    });
    worktree_port.expect(WorktreeCall::CheckDirty {
        machine: None,
        repo_dir,
    });

    let health = health_check(&ctx, "p-local".to_string()).await.unwrap();

    assert!(health[0].is_cloned, "the probe was scripted to succeed");
    assert_eq!(
        health[0].head_branch, None,
        "a chip reading this would name a branch called HEAD"
    );
}

#[tokio::test]
async fn create_resolves_a_remote_project_machine_and_repository_before_calling_the_port() {
    let (ctx, worktree_port, calls) = context();
    add_project(&ctx, "p-remote", "remote", Some("machine-remote"));
    add_repo(&ctx, "r-remote", "p-remote", "org/remote-repo");
    // Resolved rather than read from `HOME`: which variable holds it is the
    // port's business and differs per platform, and this test is about what
    // reaches the port, not about where the home directory came from.
    let home = ctx.exec.resolve_home("machine-remote").await.unwrap();
    let project_root = std::path::PathBuf::from(home)
        .join(crate::paths::DEMETEO_HOME_SUBDIR)
        .join(crate::paths::PROJECTS_SUBDIR)
        .join("p-remote");
    let repo_dir = project_root
        .join(crate::paths::REPOS_SUBDIR)
        .join("remote-repo")
        .to_string_lossy()
        .into_owned();
    let project_root = project_root.to_string_lossy().into_owned();
    worktree_port.expect(WorktreeCall::Create {
        machine: Some("machine-remote".to_string()),
        repo_dir: repo_dir.clone(),
        project_root: project_root.clone(),
        branch: "terminal/new".to_string(),
        base: Some("main".to_string()),
        name: "new".to_string(),
    });

    let created = create_terminal_worktree(
        &ctx,
        "p-remote".to_string(),
        "r-remote".to_string(),
        TerminalWorktreeRequest {
            branch: "terminal/new".to_string(),
            base_branch: Some("main".to_string()),
            worktree_name: "new".to_string(),
        },
    )
    .await
    .unwrap();

    assert_eq!(created.worktree.branch.as_deref(), Some("terminal/new"));
    assert_eq!(
        created.base_ref, "origin/main",
        "the caller learns which ref the branch was actually cut from"
    );
    assert_eq!(
        *calls.lock().unwrap(),
        [WorktreeCall::Create {
            machine: Some("machine-remote".to_string()),
            repo_dir,
            project_root,
            branch: "terminal/new".to_string(),
            base: Some("main".to_string()),
            name: "new".to_string()
        }],
        "the chosen base must reach the port unaltered"
    );
}

#[tokio::test]
async fn removal_resolves_the_same_repository_identity_the_listing_does() {
    let (ctx, worktree_port, calls) = context();
    add_project(&ctx, "p-local", "local", None);
    add_repo(&ctx, "r-local", "p-local", "org/local-repo");
    let (project_root, repo_dir) = local_layout(&ctx, "p-local", "local-repo");
    worktree_port.expect(WorktreeCall::Remove {
        machine: None,
        repo_dir: repo_dir.clone(),
        project_root: project_root.clone(),
        path: "/physical/wt/gone".to_string(),
        force: true,
    });

    remove_terminal_worktree(
        &ctx,
        "p-local".to_string(),
        "r-local".to_string(),
        "/physical/wt/gone".to_string(),
        true,
    )
    .await
    .unwrap();

    assert_eq!(
        *calls.lock().unwrap(),
        [WorktreeCall::Remove {
            machine: None,
            repo_dir,
            project_root,
            path: "/physical/wt/gone".to_string(),
            force: true
        }],
        "the port needs the project root to prove the path is one of its own"
    );
}

#[tokio::test]
async fn branch_options_carry_the_projects_configured_default() {
    let (ctx, worktree_port, _calls) = context();
    add_project(&ctx, "p-local", "local", None);
    add_repo(&ctx, "r-local", "p-local", "org/local-repo");
    let (_, repo_dir) = local_layout(&ctx, "p-local", "local-repo");
    let defaults = crate::adapters::step_executor::setup::fetch_default_settings();
    ctx.projects
        .save_settings(crate::domain::models::ProjectSettings {
            project_id: ProjectId::from("p-local"),
            worktree_strategy: WorktreeStrategy {
                default_branch: "trunk".to_string(),
                ..defaults.worktree_strategy.clone()
            },
            ..defaults
        })
        .unwrap();
    worktree_port.expect(WorktreeCall::ListBranches {
        machine: None,
        repo_dir,
    });

    let options = list_terminal_branches(&ctx, "p-local".to_string(), "r-local".to_string())
        .await
        .unwrap();

    assert_eq!(
        options.default_branch, "trunk",
        "a picker cannot read the integration branch out of refs/heads"
    );
    assert_eq!(
        options
            .branches
            .iter()
            .map(|branch| branch.name.as_str())
            .collect::<Vec<_>>(),
        ["main"],
    );
}

/// Which worktrees are terminal-owned is decided by
/// `domain::terminal_worktree`, reached through the port. This layer resolves
/// identity and forwards; a second filter here would be a rule with two homes
/// and no way to notice when they disagree.
#[tokio::test]
async fn the_listing_is_the_ports_answer_and_is_not_filtered_again() {
    let (ctx, worktree_port, _calls) = context();
    add_project(&ctx, "p-local", "local", None);
    add_repo(&ctx, "r-local", "p-local", "org/local-repo");
    let (project_root, repo_dir) = local_layout(&ctx, "p-local", "local-repo");
    worktree_port.expect(WorktreeCall::ListTerminal {
        machine: None,
        repo_dir: repo_dir.clone(),
        project_root,
    });
    worktree_port.expect(WorktreeCall::HeadBranch {
        machine: None,
        repo_dir,
    });

    let locations = list_terminal_locations(&ctx, "p-local".to_string(), "r-local".to_string())
        .await
        .unwrap();

    // The double answers with a path that shares no prefix with this context's
    // workspace, as git would after resolving symlinks. Anything comparing it
    // against a locally-derived area would drop it.
    assert_eq!(
        locations
            .worktrees
            .iter()
            .map(|worktree| worktree.branch.as_deref())
            .collect::<Vec<_>>(),
        [Some("terminal/existing")],
        "the port's answer must reach the caller intact: {:?}",
        locations.worktrees
    );
}

#[tokio::test]
async fn unknown_project_or_repository_is_rejected_before_port_io() {
    let (ctx, _worktree_port, calls) = context();
    add_project(&ctx, "p-known", "local", None);
    add_repo(&ctx, "r-known", "p-known", "org/repo");

    let missing_project =
        list_terminal_locations(&ctx, "p-missing".to_string(), "r-known".to_string())
            .await
            .unwrap_err();
    let missing_repo = create_terminal_worktree(
        &ctx,
        "p-known".to_string(),
        "r-missing".to_string(),
        TerminalWorktreeRequest {
            branch: "terminal/new".to_string(),
            base_branch: Some("main".to_string()),
            worktree_name: "new".to_string(),
        },
    )
    .await
    .unwrap_err();
    let missing_removal_repo = remove_terminal_worktree(
        &ctx,
        "p-known".to_string(),
        "r-missing".to_string(),
        "/physical/wt/anything".to_string(),
        true,
    )
    .await
    .unwrap_err();
    assert!(missing_removal_repo.contains("does not belong"));

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

    let error = list_terminal_locations(&ctx, "p-one".to_string(), "r-two".to_string())
        .await
        .unwrap_err();

    assert!(error.contains("does not belong to project p-one"));
    assert!(calls.lock().unwrap().is_empty());
}

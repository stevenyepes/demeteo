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
use crate::domain::models::EffortLevel;
use crate::domain::sync_session::SyncSessionStatus;
use crate::ports::notification::NotificationPort;
use crate::ports::sync_session::{SyncSession, SyncSessionPort};
use rusqlite::Connection;

const REPO: &str = "/repos/demeteo";
const WT: &str = "/repos/demeteo_wt_sync_feature-f-1";
const PORCELAIN: &str =
    "git -C /repos/demeteo_wt_sync_feature-f-1 status --porcelain --untracked-files=no";
const MERGE_HEAD: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse --verify MERGE_HEAD";

struct SilentNotif;
impl NotificationPort for SilentNotif {
    fn emit(&self, _event: &crate::ports::notification::DomainEvent) -> Result<(), String> {
        Ok(())
    }
}

/// Every port the turn borrows, plus the two handles a test asserts through:
/// the concrete double (for its recorder) and the database (for the row).
struct Ports {
    exec: Arc<dyn ExecutionPort>,
    scripted: Arc<ScriptedExec>,
    db: Arc<SqliteAdapter>,
    registry: Arc<AgentRegistry>,
    notif: Arc<dyn NotificationPort>,
    agent_exec: Arc<dyn AgentExecutionPort>,
    app_settings: Arc<dyn AppSettingsRepository>,
    git_ops: GitOpsHelper,
    merge_executor: Arc<dyn MergeExecutor>,
    pricing: Arc<dyn PricingTable>,
}

fn fid() -> FeatureId {
    FeatureId::from("f-1".to_string())
}

fn ports(scripted: ScriptedExec) -> Ports {
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
    Ports {
        git_ops: GitOpsHelper::new(app_settings.clone(), exec.clone()),
        exec,
        scripted,
        db,
        registry: Arc::new(AgentRegistry::new(vec![])),
        notif: Arc::new(SilentNotif),
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

/// The turn that ends before it starts, and the row that has to say so.
///
/// Both callers used to record the verdict themselves — which meant only one of
/// them did, and a resolution the user asked for left the session reading
/// `conflicted` beside a tree nothing was working on. The bundle is what proves
/// the recording is the turn's own: nothing here is a driver, a workflow or a
/// step row.
#[tokio::test]
async fn a_turn_that_found_no_merge_leaves_the_verdict_on_the_session() {
    let p = ports(ScriptedExec::new(&[]));
    open_conflicted(&p.db);
    let step_exec_id = StepExecutionId::from("se-f-1-s-sync".to_string());

    let outcome = resolve_sync_conflicts(ResolveSyncContext {
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
        step_execution_id: &step_exec_id,
        thread_id_prefix: SYNC_RESOLVER_THREAD_PREFIX,
        agent_kind: "opencode",
        override_model: None,
        effort: EffortLevel::DEFAULT,
        pricing: &p.pricing,
    })
    .await;

    assert!(
        outcome
            .as_ref()
            .err()
            .is_some_and(|reason| reason.contains("No active merge")),
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

#[test]
fn merge_markers_are_rejected_before_demeteo_stages_the_resolution() {
    assert!(has_conflict_marker(
        "const value = 1;\n<<<<<<< HEAD\nconst branch = 'feature';\n=======\nconst branch = 'main';\n>>>>>>> origin/master\n"
    ));
    assert!(!has_conflict_marker("const value = 1;\n"));
}

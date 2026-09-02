// Tests extracted from `crates/demeteo-core/src/composition/mod.rs` (mirrored-tests convention). `super` = that module.

use std::sync::Arc;

use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{FeatureId, ProjectId};
use crate::domain::sync_session::SyncSessionStatus;
use crate::paths;
use crate::ports::sync_session::SyncSession;

struct NoopNotif;

impl crate::ports::notification::NotificationPort for NoopNotif {
    fn emit(&self, _event: &crate::ports::notification::DomainEvent) -> Result<(), String> {
        Ok(())
    }
}

fn scratch(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-composition-{label}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create app data dir");
    dir
}

/// A feature whose run has finished, plus the conflicted sync a turn would be
/// holding. No worktree is named, so nothing probes and the stored status is
/// what every reader sees.
fn seed(ctx: &crate::state::AppContext) -> String {
    ctx.projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-1".to_string()),
            name: "shared-registry".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: paths::now_ms(),
        })
        .expect("add project");
    ctx.features
        .add(crate::domain::models::Feature {
            id: FeatureId::from("f-1".to_string()),
            project_id: ProjectId::from("p-1".to_string()),
            title: "shared registry".to_string(),
            status: "completed".to_string(),
            origin: FeatureOrigin::DefaultBranch,
            resolved_branch: Some("demeteo/features/f-1".to_string()),
            created_at: paths::now_ms(),
            description: String::new(),
            workflow_id: None,
            workflow_version_id: None,
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            agent_kind: None,
            model: None,
            effort: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            diff_base_branch: None,
        })
        .expect("add feature");
    let now = paths::now_ms();
    ctx.sync_sessions
        .open(&SyncSession {
            feature_id: "f-1".to_string(),
            machine_id: crate::domain::ids::LOCAL_MACHINE.to_string(),
            repo_dir: "/repos/demeteo".to_string(),
            feature_branch: "demeteo/features/f-1".to_string(),
            base_branch: "main".to_string(),
            status: SyncSessionStatus::Conflicted,
            worktree_path: None,
            head_before: Some("aaaaaaa".to_string()),
            merge_commit_sha: None,
            conflict_files: Vec::new(),
            raw_error: None,
            blocked_stage: None,
            pushed_at: None,
            attempts: 0,
            created_at: now,
            updated_at: now,
        })
        .expect("open session");
    "f-1".to_string()
}

/// The single most load-bearing wiring in the sync pane, and the one nothing
/// else can observe: the step executor, the merge executor and `AppContext`
/// must hold **one** [`SyncTurns`](crate::application::sync_turns::SyncTurns).
///
/// Give any of the three its own and every gate stays green while the app
/// breaks in the way the registry exists to prevent: the resolution claims in
/// one registry, `sync_session_get` reads another, `sync_liveness` answers
/// `Gone` for a turn that is running, `reconcile` corrects `resolving` back to
/// `conflicted`, and the pane offers Abort — `git merge --abort`,
/// `worktree remove --force`, `remove_dir_all` — at the directory an agent is
/// mid-write in.
///
/// Asserted through the two commands rather than by comparing pointers, because
/// what has to hold is that a claim taken on the context's registry is *seen*
/// by both of the others.
#[tokio::test]
async fn one_sync_registry_is_shared_by_everything_that_reads_a_live_turn() {
    let tmp = scratch("shared_registry");
    let ctx = super::build_core_context(
        super::CoreConfig {
            app_data_dir: tmp.clone(),
            execution_mode: super::ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotif),
        tokio::runtime::Handle::current(),
    );
    let feature_id = seed(&ctx);
    let held = ctx
        .sync_turns
        .claim(&feature_id, None)
        .expect("a fresh context claims nothing");

    let sync = ctx.executor.feature_sync(&feature_id).await;
    assert!(
        sync.as_ref().is_err_and(|e| e.contains("already running")),
        "the merge executor reads a different registry from the context's: {sync:?}"
    );

    let resolve = ctx
        .executor
        .feature_resolve_sync_conflicts(&feature_id, &Default::default())
        .await;
    assert!(
        resolve
            .as_ref()
            .is_err_and(|e| e.contains("already running")),
        "the step executor reads a different registry from the context's: {resolve:?}"
    );

    // And the slot really is the only thing holding them off: released, the
    // same two calls get past the refusal and fail on the repository row this
    // fixture deliberately does not have.
    drop(held);
    let after = ctx.executor.feature_sync(&feature_id).await;
    assert!(
        !format!("{after:?}").contains("already running"),
        "a released slot may not keep refusing: {after:?}"
    );

    let _ = std::fs::remove_dir_all(tmp);
}

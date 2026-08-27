// Tests extracted from `crates/demeteo-core/src/adapters/merge.rs` (mirrored-tests convention). `super` = that module.
//
// The `ExecutionPort` double errors on anything it was not scripted, so a sync
// that reached for git before it had decided whether it was allowed to start
// reddens rather than reading as an empty answer.

use super::*;

use crate::adapters::database::SqliteAdapter;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::sync_session::SyncSessionStatus;
use crate::ports::sync_session::SyncSessionPort;
use crate::ports::worktree_ops::MergeGate;
use rusqlite::{params, Connection};

const REPO: &str = "/repos/demeteo";
const WT: &str = "/repos/demeteo_wt_sync_feature-f-1";

fn fid() -> FeatureId {
    FeatureId::from("f-1".to_string())
}

/// A feature with a project but deliberately **no** `repositories` row: the
/// only sync that can get as far as needing one is a sync this refusal let
/// through, so "which error comes back" is also the assertion that the guard
/// runs before anything is resolved or fetched.
fn executor(scripted: ScriptedExec) -> (SqliteMergeExecutor, Arc<SqliteAdapter>) {
    let db = Arc::new(SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap());
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO projects (id, name, created_at) VALUES ('p-1', 'demeteo', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO features (id, project_id, title, status, created_at)
             VALUES ('f-1', 'p-1', 'sync me', 'completed', 0)",
            [],
        )
        .unwrap();
    }
    let exec: Arc<dyn ExecutionPort> = Arc::new(scripted);
    let app_settings: Arc<dyn crate::ports::db::AppSettingsRepository> = db.clone();
    (
        SqliteMergeExecutor::new(
            db.clone(),
            db.clone(),
            db.clone(),
            Arc::new(crate::application::sync_turns::SyncTurns::default()),
            GitOpsHelper::new(app_settings, exec.clone()),
            exec,
            std::path::PathBuf::from("/workspace"),
        ),
        db,
    )
}

/// The same fixture with a repository row, so the sync gets past
/// `repo_target` and issues real git. The paths are built with the production
/// helpers rather than spelled out: `-C <dir>` is what the scripted key has to
/// match, and a hand-written POSIX path matches nothing on a Windows host.
fn executor_with_repo(programs: &[(&str, Result<&str, &str>)]) -> Executor {
    executor_with(ScriptedExec::new(&[]).with_programs(programs), true)
}

struct Executor {
    executor: SqliteMergeExecutor,
    db: Arc<SqliteAdapter>,
    exec: Arc<ScriptedExec>,
    turns: Arc<crate::application::sync_turns::SyncTurns>,
    repo_dir: String,
    worktree: String,
}

const REPO_PATH: &str = "demeteo/core";

fn repo_dir_of() -> String {
    crate::paths::repo_target_dir_local(std::path::Path::new(WORKSPACE), "p-1", REPO_PATH)
        .to_string_lossy()
        .to_string()
}

const WORKSPACE: &str = "/workspace";

fn executor_with(scripted: ScriptedExec, with_repository: bool) -> Executor {
    let db = Arc::new(SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap());
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO projects (id, name, compute_type, created_at)
             VALUES ('p-1', 'demeteo', 'local', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO features (id, project_id, title, status, created_at)
             VALUES ('f-1', 'p-1', 'sync me', 'completed', 0)",
            [],
        )
        .unwrap();
        if with_repository {
            conn.execute(
                "INSERT INTO repositories (id, project_id, provider_id, repo_path)
                 VALUES ('r-1', 'p-1', 'prov', ?1)",
                params![REPO_PATH],
            )
            .unwrap();
        }
    }
    let scripted = Arc::new(scripted);
    let exec: Arc<dyn ExecutionPort> = scripted.clone();
    let app_settings: Arc<dyn crate::ports::db::AppSettingsRepository> = db.clone();
    let repo_dir = repo_dir_of();
    let worktree = crate::paths::sync_worktree_dir(
        &repo_dir,
        "feature/f-1",
        crate::paths::targets_windows_host(crate::domain::ids::LOCAL_MACHINE),
    );
    let turns = Arc::new(crate::application::sync_turns::SyncTurns::default());
    Executor {
        executor: SqliteMergeExecutor::new(
            db.clone(),
            db.clone(),
            db.clone(),
            turns.clone(),
            GitOpsHelper::new(app_settings, exec.clone()),
            exec,
            std::path::PathBuf::from(WORKSPACE),
        ),
        db,
        exec: scripted,
        turns,
        repo_dir,
        worktree,
    }
}

/// A run that fetches, merges and pushes, keyed by the paths the fixture
/// resolved. Every test below breaks or keeps exactly one entry.
fn full_run(repo: &str, wt: &str) -> Vec<(String, Result<String, String>)> {
    let ok = |s: &str| Ok(s.to_string());
    vec![
        (format!("git -C {repo} fetch origin -- master"), ok("")),
        (
            format!("git -C {repo} rev-parse --verify origin/master"),
            ok("beefbee"),
        ),
        (
            format!("git -C {repo} rev-parse refs/heads/feature/f-1"),
            ok("aaaaaaa"),
        ),
        (
            format!("git -C {repo} rev-list --count refs/heads/feature/f-1..origin/master"),
            ok("2"),
        ),
        (
            format!("git -C {repo} rev-list --count origin/master..refs/heads/feature/f-1"),
            ok("0"),
        ),
        (
            format!("git -C {repo} rev-parse --abbrev-ref HEAD"),
            ok("master"),
        ),
        (format!("git -C {repo} worktree list --porcelain"), ok("")),
        (
            format!("git -C {repo} worktree add {wt} feature/f-1"),
            ok(""),
        ),
        (
            format!(
                "git -C {wt} merge origin/master -m chore(sync): sync feature with origin/master"
            ),
            ok(""),
        ),
        (format!("git -C {wt} rev-parse HEAD"), ok("d00dd00")),
        (format!("git -C {wt} push origin feature/f-1"), ok("")),
    ]
}

fn scripted(run: &[(String, Result<String, String>)]) -> Vec<(&str, Result<&str, &str>)> {
    run.iter()
        .map(|(k, v)| {
            (
                k.as_str(),
                match v {
                    Ok(s) => Ok(s.as_str()),
                    Err(e) => Err(e.as_str()),
                },
            )
        })
        .collect()
}

fn breaking(
    run: Vec<(String, Result<String, String>)>,
    key: &str,
    err: &str,
) -> Vec<(String, Result<String, String>)> {
    run.into_iter()
        .map(|(k, v)| {
            if k == key {
                (k, Err(err.to_string()))
            } else {
                (k, v)
            }
        })
        .collect()
}

fn session(status: SyncSessionStatus, pushed_at: Option<i64>) -> SyncSession {
    SyncSession {
        feature_id: "f-1".to_string(),
        machine_id: crate::domain::ids::LOCAL_MACHINE.to_string(),
        repo_dir: REPO.to_string(),
        feature_branch: "feature/f-1".to_string(),
        base_branch: "master".to_string(),
        status,
        worktree_path: Some(WT.to_string()),
        head_before: Some("aaaaaaa".to_string()),
        merge_commit_sha: Some("c0ffeec".to_string()),
        conflict_files: Vec::new(),
        raw_error: None,
        blocked_stage: None,
        pushed_at,
        attempts: 0,
        created_at: 100,
        updated_at: 100,
    }
}

/// The reconcile probe for a tree holding a committed resolution.
fn resolved_probe() -> ScriptedExec {
    ScriptedExec::new(&[
        (
            "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse --git-dir",
            Ok(".git\n"),
        ),
        (
            "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse --verify --quiet MERGE_HEAD",
            Ok(""),
        ),
        (
            "git -C /repos/demeteo_wt_sync_feature-f-1 status --porcelain",
            Ok(""),
        ),
        (
            "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse HEAD",
            Ok("c0ffeec\n"),
        ),
    ])
}

/// Pressing "Sync with main" over a resolution nobody has read used to retire it
/// silently. `open` is an upsert on one row per feature, so the new sync took
/// `head_before`, `merge_commit_sha` and `pushed_at` with it; the merge itself
/// then changed nothing — `origin/<base>` was already in the branch — so the
/// push was skipped and the row landed on a terminal `up_to_date`, from which
/// Publish and Discard are both refused. One click, and the merge is on its way
/// to the pull request with the only affordance that could have stopped it gone.
#[tokio::test]
async fn a_sync_may_not_start_over_a_resolution_nobody_has_read() {
    let (executor, db) = executor(resolved_probe());
    let sessions: &dyn SyncSessionPort = &*db;
    sessions
        .open(&session(SyncSessionStatus::Resolved, None))
        .unwrap();

    let failure = executor
        .sync_feature_with_upstream(&fid(), "feature/f-1", "master", MergeGate::default())
        .await
        .expect_err("the held resolution is what stops it");
    match failure {
        UpstreamSyncFailure::Blocked { stage, raw_error } => {
            assert_eq!(stage, SyncBlockedStage::HeldResolution);
            assert!(
                raw_error.contains("Publish it or discard it"),
                "{raw_error}"
            );
        }
        other => panic!("{other:?}"),
    }

    let after = sessions.get(&fid()).unwrap().unwrap();
    assert_eq!(after.status, SyncSessionStatus::Resolved);
    assert_eq!(after.head_before.as_deref(), Some("aaaaaaa"));
    assert_eq!(after.merge_commit_sha.as_deref(), Some("c0ffeec"));
}

/// A resolution origin already has is nothing to protect, and the guard must
/// not stand between a feature and its next sync. This one gets as far as
/// wanting a repository row, which is exactly one step past the refusal.
#[tokio::test]
async fn a_published_resolution_does_not_stand_in_the_way_of_the_next_sync() {
    let (executor, db) = executor(resolved_probe());
    let sessions: &dyn SyncSessionPort = &*db;
    sessions
        .open(&session(SyncSessionStatus::Resolved, Some(200)))
        .unwrap();

    let failure = executor
        .sync_feature_with_upstream(&fid(), "feature/f-1", "master", MergeGate::default())
        .await
        .expect_err("this fixture configures no repository");
    match failure {
        UpstreamSyncFailure::Blocked { stage, .. } => {
            assert_eq!(stage, SyncBlockedStage::RepoContext)
        }
        other => panic!("{other:?}"),
    }
}

/// The probe `swept_worktree` runs to decide whether the row may stop naming
/// the tree — the shell form, not the argv one.
fn git_dir_probe(worktree: &str) -> String {
    format!(
        "git -C {} rev-parse --git-dir",
        crate::paths::shell_escape_posix(worktree)
    )
}

/// The second row shape holding a merge nobody published, and the one
/// `resync_refusal` did not cover.
///
/// A `push`-blocked session carries the merge on the branch plus the only copy
/// of `head_before` and `merge_commit_sha`. `open` is an upsert, so a fresh
/// sync takes all three — and because the branch already contains
/// `origin/<base>` the merge it then runs changes nothing and lands the row on
/// a terminal `up_to_date`, from which Publish is refused. The pane withholds
/// the retry for `push`, but this IPC is reachable regardless and the `merge`
/// stage produces the same row while the pane *does* offer one.
#[tokio::test]
async fn a_sync_may_not_start_over_a_merge_that_never_reached_origin() {
    let (executor, db) = executor(resolved_probe());
    let sessions: &dyn SyncSessionPort = &*db;
    let mut row = session(SyncSessionStatus::Blocked, None);
    row.blocked_stage = Some(SyncBlockedStage::Push);
    sessions.open(&row).unwrap();

    let failure = executor
        .sync_feature_with_upstream(&fid(), "feature/f-1", "master", MergeGate::default())
        .await
        .expect_err("the unpublished merge is what stops it");
    match failure {
        UpstreamSyncFailure::Blocked { stage, raw_error } => {
            assert_eq!(stage, SyncBlockedStage::HeldResolution);
            assert!(raw_error.contains("could not push it"), "{raw_error}");
        }
        other => panic!("{other:?}"),
    }

    let after = sessions.get(&fid()).unwrap().unwrap();
    assert_eq!(after.blocked_stage, Some(SyncBlockedStage::Push));
    assert_eq!(after.head_before.as_deref(), Some("aaaaaaa"));
    assert_eq!(after.merge_commit_sha.as_deref(), Some("c0ffeec"));
}

/// The press behind a sync that stopped on a divergence, over the branch only a
/// person may settle: origin rewrote it, and the reset is taken because the
/// caller asked for it and the measurement still says it drops nothing.
///
/// What this asserts past the git argv is the shape of the answer. A reconcile
/// ends in a session row like every other sync, and the row is what the pane
/// renders next — so the call has to hand back the row it just wrote, not the
/// outcome the sync returned.
#[tokio::test]
async fn a_pressed_reset_reconciles_and_answers_with_the_session() {
    let repo = repo_dir_of();
    let wt = crate::paths::sync_worktree_dir(
        &repo,
        "feature/f-1",
        crate::paths::targets_windows_host(crate::domain::ids::LOCAL_MACHINE),
    );
    let ok = |s: &str| Ok(s.to_string());
    let mut run = full_run(&repo, &wt);
    run.extend([
        (format!("git -C {repo} fetch origin -- feature/f-1"), ok("")),
        (
            format!("git -C {repo} rev-parse --verify origin/feature/f-1"),
            ok("beef2"),
        ),
        (
            format!("git -C {repo} rev-list --count refs/heads/feature/f-1..origin/feature/f-1"),
            ok("1"),
        ),
        (
            format!("git -C {repo} rev-list --count origin/feature/f-1..refs/heads/feature/f-1"),
            ok("2"),
        ),
        (
            format!("git -C {repo} cherry origin/feature/f-1 refs/heads/feature/f-1"),
            ok("- 1a2b3c\n- 4d5e6f"),
        ),
        (
            format!("git -C {wt} reset --keep origin/feature/f-1"),
            ok(""),
        ),
    ]);
    let fx = executor_with_repo(&scripted(&run));

    let view = fx
        .executor
        .reconcile_feature_with_origin(
            &fid(),
            "feature/f-1",
            "master",
            MergeGate::default(),
            crate::domain::upstream_feature::DivergenceReconcile::ResetOntoOrigin,
        )
        .await
        .expect("a reset the measurement supports is not a refusal")
        .expect("a reconcile that ran opened a session");

    assert_eq!(
        view.session.head_before.as_deref(),
        Some("d00dd00"),
        "the row has to carry the reconciled tip and not the ref this sync found: a \
         Discard against the latter rewinds the branch past origin's own commits"
    );
    assert!(
        fx.exec
            .programs()
            .contains(&format!("git -C {wt} reset --keep origin/feature/f-1")),
        "the press has to reach git: {:?}",
        fx.exec.programs()
    );
}

/// The two refusals that stop before the session is opened, over a call whose
/// answer *is* the session. Handing back the row there would answer the press
/// with whatever the last sync left on it — a blocked divergence, most often,
/// which is the row the user pressed the button on.
#[tokio::test]
async fn a_reconcile_refused_before_the_row_exists_says_so_itself() {
    let fx = executor_with_repo(&[]);
    let held = fx.turns.claim("f-1", None).expect("the registry is empty");

    let refusal = fx
        .executor
        .reconcile_feature_with_origin(
            &fid(),
            "feature/f-1",
            "master",
            MergeGate::default(),
            crate::domain::upstream_feature::DivergenceReconcile::MergeOrigin,
        )
        .await
        .expect_err("the slot is taken, and no row was written to answer with");
    assert!(refusal.contains("already running"), "{refusal}");
    assert!(
        fx.exec.programs().is_empty(),
        "no git may run against a tree another turn owns: {:?}",
        fx.exec.programs()
    );

    drop(held);
}

/// The mutual exclusion the workflow's own `sync` node did not have.
///
/// `provision_sync_worktree` sweeps every `_wt_sync` worktree checked out on
/// the branch with `worktree remove --force` plus `remove_dir_all`, and an
/// out-of-band resolution's tree is one of those. The claim was taken by the
/// *command* rather than here, so the node reached that sweep holding nothing —
/// and nothing in run control refuses starting a run while a resolution is
/// running.
#[tokio::test]
async fn a_sync_will_not_start_while_another_turn_holds_the_feature() {
    let fx = executor_with_repo(&[]);
    let held = fx.turns.claim("f-1", None).expect("the registry is empty");

    let failure = fx
        .executor
        .sync_feature_with_upstream(&fid(), "feature/f-1", "master", MergeGate::default())
        .await
        .expect_err("the slot is taken");
    match failure {
        UpstreamSyncFailure::Blocked { stage, raw_error } => {
            assert_eq!(stage, SyncBlockedStage::TurnInFlight);
            assert!(raw_error.contains("already running"), "{raw_error}");
        }
        other => panic!("{other:?}"),
    }
    assert!(
        fx.exec.programs().is_empty(),
        "no git may run against a tree another turn owns: {:?}",
        fx.exec.programs()
    );
    let sessions: &dyn SyncSessionPort = &*fx.db;
    assert!(
        sessions.get(&fid()).unwrap().is_none(),
        "and the refused sync may not open a row over the live turn's"
    );

    drop(held);
    let _ = fx
        .executor
        .sync_feature_with_upstream(&fid(), "feature/f-1", "master", MergeGate::default())
        .await;
    assert!(
        !fx.exec.programs().is_empty(),
        "the slot has to come back when the turn ends"
    );
}

/// A clean sync deletes its throwaway worktree on the way out, and the row is
/// written before that — `RecordWorktree` names the tree the instant git
/// provisions one, so an interrupted sync is probeable. Left unwritten on the
/// way out, the pane's "Sync worktree" section named a directory the sync
/// itself had just removed, on every successful sync.
#[tokio::test]
async fn a_clean_sync_stops_naming_the_worktree_it_deleted() {
    let fx = executor_with_repo(&[]);
    let run = full_run(&fx.repo_dir, &fx.worktree);
    let fx = executor_with(
        ScriptedExec::new(&[(
            git_dir_probe(&fx.worktree).as_str(),
            Err("fatal: not a git repository"),
        )])
        .with_programs(&scripted(&run)),
        true,
    );

    fx.executor
        .sync_feature_with_upstream(&fid(), "feature/f-1", "master", MergeGate::default())
        .await
        .expect("the scripted run merges and pushes cleanly");

    let after = (&*fx.db as &dyn SyncSessionPort)
        .get(&fid())
        .unwrap()
        .unwrap();
    assert_eq!(after.status, SyncSessionStatus::Merged);
    assert_eq!(
        after.worktree_path, None,
        "the row may not keep naming a tree the sync removed"
    );
    assert_eq!(after.merge_commit_sha.as_deref(), Some("d00dd00"));
}

/// The other half of the same rule: the teardown is best-effort and reports
/// nothing, so the column is cleared on a *probe* and never on the delete. A
/// tree still standing keeps its name on the row, because a row blanked here is
/// the last reader this terminal session will ever have — the directory would
/// then be named by nothing at all.
#[tokio::test]
async fn a_worktree_the_teardown_left_behind_is_still_named_on_the_row() {
    let fx = executor_with_repo(&[]);
    let run = full_run(&fx.repo_dir, &fx.worktree);
    let wt = fx.worktree.clone();
    let escaped = crate::paths::shell_escape_posix(&wt);
    let merge_head = format!("git -C {escaped} rev-parse --verify --quiet MERGE_HEAD");
    let porcelain = format!("git -C {escaped} status --porcelain");
    let fx = executor_with(
        ScriptedExec::new(&[
            (git_dir_probe(&wt).as_str(), Ok(".git\n")),
            (merge_head.as_str(), Ok("")),
            (porcelain.as_str(), Ok("")),
        ])
        .with_programs(&scripted(&run)),
        true,
    );

    fx.executor
        .sync_feature_with_upstream(&fid(), "feature/f-1", "master", MergeGate::default())
        .await
        .expect("the scripted run merges and pushes cleanly");

    let after = (&*fx.db as &dyn SyncSessionPort)
        .get(&fid())
        .unwrap()
        .unwrap();
    assert_eq!(after.worktree_path.as_deref(), Some(wt.as_str()));
}

/// The one blocked stage with work at risk, written all the way to the row.
///
/// `blocked_stage` is what the pane selects Publish on, and on a push-blocked
/// row it can only come from this `update`: `open` runs first and always
/// inserts `None`. `merge_commit_sha` is the commit Publish pushes and the only
/// thing `push_landed` can confirm against origin afterwards.
#[tokio::test]
async fn a_push_that_failed_records_the_stage_and_the_commit_it_left() {
    let fx = executor_with_repo(&[]);
    let push = format!("git -C {} push origin feature/f-1", fx.worktree);
    let run = breaking(
        full_run(&fx.repo_dir, &fx.worktree),
        &push,
        "! [rejected] feature/f-1 -> feature/f-1 (fetch first)",
    );
    let fx = executor_with(ScriptedExec::new(&[]).with_programs(&scripted(&run)), true);

    let failure = fx
        .executor
        .sync_feature_with_upstream(&fid(), "feature/f-1", "master", MergeGate::default())
        .await
        .expect_err("the push was rejected");
    assert!(matches!(
        failure,
        UpstreamSyncFailure::Blocked {
            stage: SyncBlockedStage::Push,
            ..
        }
    ));

    let after = (&*fx.db as &dyn SyncSessionPort)
        .get(&fid())
        .unwrap()
        .unwrap();
    assert_eq!(after.status, SyncSessionStatus::Blocked);
    assert_eq!(
        after.blocked_stage,
        Some(SyncBlockedStage::Push),
        "without the stage the pane offers a retry that merges nothing"
    );
    assert_eq!(
        after.merge_commit_sha.as_deref(),
        Some("d00dd00"),
        "Publish refuses without a commit, and can confirm nothing against origin"
    );
    assert_eq!(after.worktree_path.as_deref(), Some(fx.worktree.as_str()));
    assert_eq!(after.head_before.as_deref(), Some("aaaaaaa"));
}

/// A `rev-parse HEAD` that did not answer is not a commit named `""`.
///
/// Flattened with `unwrap_or_default` it was stored as this sync's merge
/// commit, and an empty sha passes every "is there one" guard between here and
/// the pane: Publish is offered, the push runs, and the confirmation
/// `git merge-base --is-ancestor '' …` is then refused by git forever — so the
/// user is told their push did not land, about one that did.
#[tokio::test]
async fn a_push_blocked_sync_that_could_not_read_its_head_records_no_commit() {
    let fx = executor_with_repo(&[]);
    let head = format!("git -C {} rev-parse HEAD", fx.worktree);
    let push = format!("git -C {} push origin feature/f-1", fx.worktree);
    let run = breaking(
        breaking(
            full_run(&fx.repo_dir, &fx.worktree),
            &head,
            "demeteo-transport: channel closed",
        ),
        &push,
        "! [rejected] feature/f-1 -> feature/f-1 (fetch first)",
    );
    let fx = executor_with(ScriptedExec::new(&[]).with_programs(&scripted(&run)), true);

    let failure = fx
        .executor
        .sync_feature_with_upstream(&fid(), "feature/f-1", "master", MergeGate::default())
        .await
        .expect_err("the push was rejected");
    assert!(matches!(
        failure,
        UpstreamSyncFailure::Blocked {
            stage: SyncBlockedStage::Push,
            ..
        }
    ));

    let after = (&*fx.db as &dyn SyncSessionPort)
        .get(&fid())
        .unwrap()
        .unwrap();
    assert_eq!(after.blocked_stage, Some(SyncBlockedStage::Push));
    assert_eq!(
        after.merge_commit_sha, None,
        "an unread tip may not be stored as the commit Publish would push"
    );
}

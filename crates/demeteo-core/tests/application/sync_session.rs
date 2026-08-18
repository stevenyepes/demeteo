// Tests extracted from `crates/demeteo-core/src/application/sync_session.rs` (mirrored-tests convention). `super` = that module.
//
// The `ExecutionPort` double errors on anything it was not scripted, so "the
// code probed something this test never anticipated" reddens rather than
// reading as a clean worktree.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::models::ConflictFile;
use rusqlite::Connection;

const REPO: &str = "/repos/demeteo";
const WT: &str = "/repos/demeteo_wt_sync_feature-f-1";

const GIT_DIR: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse --git-dir";
const MERGE_HEAD: &str =
    "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse --verify --quiet MERGE_HEAD";
const PORCELAIN: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 status --porcelain";
const HEAD_SHA: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse HEAD";

fn fid() -> FeatureId {
    FeatureId::from("f-1".to_string())
}

/// The double is kept as its own type as well as behind the port, because the
/// recorder is on the concrete side and "what git did this issue" is half of
/// what these tests assert.
fn ports(
    scripted: ScriptedExec,
) -> (
    Arc<dyn SyncSessionPort>,
    Arc<dyn ExecutionPort>,
    Arc<ScriptedExec>,
) {
    let db = Arc::new(SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap());
    {
        // Foreign keys are enforced here, and the session cascades off the
        // feature.
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
    (db, scripted.clone(), scripted)
}

fn conflicted(worktree: Option<&str>) -> SyncSession {
    SyncSession {
        feature_id: "f-1".to_string(),
        machine_id: "local".to_string(),
        repo_dir: REPO.to_string(),
        feature_branch: "feature/f-1".to_string(),
        base_branch: "master".to_string(),
        status: SyncSessionStatus::Conflicted,
        worktree_path: worktree.map(str::to_string),
        head_before: Some("aaaaaaa".to_string()),
        merge_commit_sha: None,
        conflict_files: vec![ConflictFile {
            path: "src/lib.rs".to_string(),
            kind: "both modified".to_string(),
        }],
        raw_error: Some("CONFLICT (content): Merge conflict in src/lib.rs".to_string()),
        attempts: 0,
        created_at: 100,
        updated_at: 100,
    }
}

/// The durability hole, end to end: the conflict was a React `useState` and a
/// worktree nobody named, so navigating away lost it for good. A reader that
/// comes back to a live conflicted tree must find the same conflict.
#[tokio::test]
async fn a_conflict_is_still_there_when_the_next_reader_asks() {
    let (sessions, exec, _git) = ports(ScriptedExec::new(&[
        (GIT_DIR, Ok(".git\n")),
        (MERGE_HEAD, Ok("b1b2b3b\n")),
        (PORCELAIN, Ok("UU src/lib.rs\n")),
    ]));
    sessions.open(&conflicted(Some(WT))).unwrap();

    let read = get_reconciled(&sessions, &exec, &fid())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.status, SyncSessionStatus::Conflicted);
    assert_eq!(read.conflict_files[0].path, "src/lib.rs");
    assert_eq!(read.head_before.as_deref(), Some("aaaaaaa"));
}

/// The row is a claim and git is the authority: the tree the session named is
/// gone, so nothing is left to resolve — and the correction is written, so the
/// next reader is not told the old story again.
#[tokio::test]
async fn a_conflict_whose_worktree_went_away_reconciles_to_aborted_and_stays_that_way() {
    let (sessions, exec, _git) =
        ports(ScriptedExec::new(&[(GIT_DIR, Err("not a git repository"))]));
    sessions.open(&conflicted(Some(WT))).unwrap();

    let read = get_reconciled(&sessions, &exec, &fid())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.status, SyncSessionStatus::Aborted);
    assert_eq!(
        sessions.get(&fid()).unwrap().unwrap().status,
        SyncSessionStatus::Aborted,
        "the correction must be persisted, not just returned"
    );
}

#[tokio::test]
async fn a_feature_that_never_synced_has_no_session() {
    let (sessions, exec, _git) = ports(ScriptedExec::new(&[]));
    assert!(get_reconciled(&sessions, &exec, &fid())
        .await
        .unwrap()
        .is_none());
}

/// `provision_sync_worktree` returns the clone itself when the feature branch
/// is already checked out there, so `worktree_path` can legitimately be the
/// user's repository. Aborting must undo the merge and stop — a recursive
/// delete here is the checkout.
#[tokio::test]
async fn aborting_in_the_clone_itself_undoes_the_merge_and_deletes_nothing() {
    let (sessions, exec, git) = ports(ScriptedExec::new(&[(
        "git -C /repos/demeteo merge --abort",
        Ok(""),
    )]));
    sessions.open(&conflicted(Some(REPO))).unwrap();

    let read = abort(&sessions, &exec, &fid()).await.unwrap().unwrap();
    assert_eq!(read.status, SyncSessionStatus::Aborted);
    assert_eq!(
        git.commands(),
        vec!["git -C /repos/demeteo merge --abort".to_string()],
        "nothing may remove or prune the clone itself"
    );
}

/// The common case is a worktree that is already gone — the user aborts after
/// a restart, or after cleaning up by hand — and pressing the button twice is
/// the same case. Neither may fail, and the second must issue nothing.
#[tokio::test]
async fn aborting_twice_is_the_same_as_aborting_once() {
    let (sessions, exec, git) = ports(ScriptedExec::new(&[(
        "git -C /repos/demeteo_wt_sync_feature-f-1 merge --abort",
        Err("fatal: There is no merge to abort"),
    )]));
    sessions.open(&conflicted(Some(WT))).unwrap();

    abort(&sessions, &exec, &fid()).await.unwrap();
    let issued = git.commands();
    abort(&sessions, &exec, &fid()).await.unwrap();

    let read = sessions.get(&fid()).unwrap().unwrap();
    assert_eq!(read.status, SyncSessionStatus::Aborted);
    assert_eq!(read.worktree_path, None);
    assert_eq!(
        git.commands(),
        issued,
        "a session with no worktree left must issue no git at all"
    );
}

/// The bug this file exists to keep out, and the one the first version shipped.
///
/// `aborted` is terminal, so writing it is irreversible — and every probe here
/// reaches the caller as a `Result`, which makes "the SSH channel died" and
/// "git says this is not a repository" the same shape unless the prefix is
/// read. Conflating them retires a live conflict on the first mount after a
/// network blip, leaving the worktree on disk named by nothing.
#[tokio::test]
async fn a_transport_failure_is_not_evidence_the_worktree_is_gone() {
    let (sessions, exec, _git) = ports(ScriptedExec::new(&[(
        GIT_DIR,
        Err("transport: Connection appears dead: no data and no keepalive ack"),
    )]));
    sessions.open(&conflicted(Some(WT))).unwrap();

    let read = get_reconciled(&sessions, &exec, &fid())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        read.status,
        SyncSessionStatus::Conflicted,
        "an unreadable tree may not move the stored status"
    );
    assert_eq!(
        sessions.get(&fid()).unwrap().unwrap().status,
        SyncSessionStatus::Conflicted,
        "and it must not persist a correction it did not earn"
    );
}

/// The same conflation one probe further in: a `MERGE_HEAD` read that never
/// answered, taken as "no merge open", walks a conflicted session into the
/// resolved arm and offers a review of a resolution nobody performed.
#[tokio::test]
async fn an_unreadable_merge_head_does_not_mean_the_merge_is_finished() {
    // Everything *around* the unreadable probe answers, and answers with the
    // shape that resolves: clean tree, HEAD moved off the starting sha. So the
    // only thing standing between this session and `resolved` is refusing to
    // read a dead channel as "no merge open".
    let (sessions, exec, _git) = ports(ScriptedExec::new(&[
        (GIT_DIR, Ok(".git\n")),
        (MERGE_HEAD, Err("transport: Connection appears dead")),
        (PORCELAIN, Ok("")),
        (HEAD_SHA, Ok("bbbbbbb\n")),
    ]));
    sessions.open(&conflicted(Some(WT))).unwrap();

    let read = get_reconciled(&sessions, &exec, &fid())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.status, SyncSessionStatus::Conflicted);
}

/// Teardown is best-effort because the tree is usually already gone, but the
/// verdict may not be. An unreachable host that swallows every delete would
/// otherwise be recorded as an abandoned sync — terminal, and with
/// `worktree_path` cleared, so the merge left open on disk is named by nothing
/// and revisited by no reader.
#[tokio::test]
async fn aborting_against_an_unreachable_host_leaves_the_session_open() {
    let (sessions, exec, _git) = ports(ScriptedExec::new(&[
        (
            "git -C /repos/demeteo_wt_sync_feature-f-1 merge --abort",
            Err("transport: Connection appears dead"),
        ),
        (GIT_DIR, Err("transport: Connection appears dead")),
    ]));
    sessions.open(&conflicted(Some(WT))).unwrap();

    assert!(abort(&sessions, &exec, &fid()).await.is_err());
    let read = sessions.get(&fid()).unwrap().unwrap();
    assert_eq!(read.status, SyncSessionStatus::Conflicted);
    assert_eq!(
        read.worktree_path.as_deref(),
        Some(WT),
        "the row must keep naming the tree it did not remove"
    );
}

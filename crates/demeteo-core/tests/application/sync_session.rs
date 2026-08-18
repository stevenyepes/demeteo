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
    Arc<dyn crate::ports::db::FeatureRepository>,
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
            // `completed`: the refusals these tests are about only arise once
            // no driver owns the branch, and the column defaults to `running`.
            "INSERT INTO features (id, project_id, title, status, created_at)
             VALUES ('f-1', 'p-1', 'sync me', 'completed', 0)",
            [],
        )
        .unwrap();
    }
    let scripted = Arc::new(scripted);
    (db.clone(), scripted.clone(), scripted, db)
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
        pushed_at: None,
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
    let (sessions, exec, _git, _features) = ports(ScriptedExec::new(&[
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
    let (sessions, exec, _git, _features) =
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
    let (sessions, exec, _git, _features) = ports(ScriptedExec::new(&[]));
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
    let (sessions, exec, git, features) = ports(ScriptedExec::new(&[
        ("git -C /repos/demeteo rev-parse --git-dir", Ok(".git\n")),
        (
            "git -C /repos/demeteo rev-parse --verify --quiet MERGE_HEAD",
            Ok("b1b2b3b\n"),
        ),
        (
            "git -C /repos/demeteo status --porcelain",
            Ok("UU src/lib.rs\n"),
        ),
        ("git -C /repos/demeteo merge --abort", Ok("")),
    ]));
    sessions.open(&conflicted(Some(REPO))).unwrap();

    let read = abort(&sessions, &exec, &features, &fid())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.session.status, SyncSessionStatus::Aborted);
    assert!(
        !git.commands()
            .iter()
            .any(|c| c.contains("worktree remove") || c.contains("worktree prune")),
        "nothing may remove or prune the clone itself: {:?}",
        git.commands()
    );
    assert!(
        git.commands()
            .contains(&"git -C /repos/demeteo merge --abort".to_string()),
        "{:?}",
        git.commands()
    );
}

/// The common case is a worktree that is already gone — the user aborts after
/// a restart, or after cleaning up by hand — and pressing the button twice is
/// the same case. Neither may fail, and the second must issue nothing.
#[tokio::test]
async fn aborting_twice_is_the_same_as_aborting_once() {
    let (sessions, exec, git, features) = ports(ScriptedExec::new(&[(
        "git -C /repos/demeteo_wt_sync_feature-f-1 merge --abort",
        Err("fatal: There is no merge to abort"),
    )]));
    sessions.open(&conflicted(Some(WT))).unwrap();

    abort(&sessions, &exec, &features, &fid()).await.unwrap();
    let issued = git.commands();
    abort(&sessions, &exec, &features, &fid()).await.unwrap();

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
    let (sessions, exec, _git, _features) = ports(ScriptedExec::new(&[(
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
    let (sessions, exec, _git, _features) = ports(ScriptedExec::new(&[
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
    let (sessions, exec, _git, features) = ports(ScriptedExec::new(&[
        (
            "git -C /repos/demeteo_wt_sync_feature-f-1 merge --abort",
            Err("transport: Connection appears dead"),
        ),
        (GIT_DIR, Err("transport: Connection appears dead")),
    ]));
    sessions.open(&conflicted(Some(WT))).unwrap();

    assert!(abort(&sessions, &exec, &features, &fid()).await.is_err());
    let read = sessions.get(&fid()).unwrap().unwrap();
    assert_eq!(read.status, SyncSessionStatus::Conflicted);
    assert_eq!(
        read.worktree_path.as_deref(),
        Some(WT),
        "the row must keep naming the tree it did not remove"
    );
}

const PUSH: &str = "git -C /repos/demeteo push origin feature/f-1";
const CONTAINS: &str =
    "git -C /repos/demeteo merge-base --is-ancestor c0ffeec refs/remotes/origin/feature/f-1";
const DISCARD_WT: &str =
    "git -C /repos/demeteo worktree remove --force /repos/demeteo_wt_sync_feature-f-1";
const PRUNE: &str = "git -C /repos/demeteo worktree prune";
const RESET: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 reset --hard aaaaaaa";
const MERGE_ABORT: &str = "git -C /repos/demeteo_wt_sync_feature-f-1 merge --abort";

/// A resolution committed on the branch and not yet on origin — the state the
/// review affordances act on.
fn resolved(head_before: Option<&str>) -> SyncSession {
    SyncSession {
        status: SyncSessionStatus::Resolved,
        merge_commit_sha: Some("c0ffeec".to_string()),
        head_before: head_before.map(str::to_string),
        conflict_files: Vec::new(),
        raw_error: None,
        ..conflicted(Some(WT))
    }
}

/// The reconcile probe every read runs before it answers, for a tree holding a
/// committed resolution: it exists, the merge is closed, nothing is modified,
/// and `HEAD` has moved off where the sync started.
fn resolved_probe() -> Vec<(&'static str, Result<&'static str, &'static str>)> {
    vec![
        (GIT_DIR, Ok(".git\n")),
        (MERGE_HEAD, Ok("")),
        (PORCELAIN, Ok("")),
        (HEAD_SHA, Ok("c0ffeec\n")),
    ]
}

fn script(extra: &[(&'static str, Result<&'static str, &'static str>)]) -> ScriptedExec {
    let mut all = resolved_probe();
    all.extend_from_slice(extra);
    ScriptedExec::new(&all)
}

/// Publishing twice must not push twice, and must not be an error either.
///
/// The button sits beside a diff the user is reading, so the honest reading of
/// a second press is "did the first one work" — which a refusal would answer
/// with an error dialog. The double errors on anything it was not scripted, so
/// a second `git push` here reddens rather than passing silently.
#[tokio::test]
async fn publishing_a_resolution_twice_pushes_once() {
    let (sessions, exec, git, features) = ports(script(&[
        (PUSH, Ok("")),
        (CONTAINS, Ok("")),
        (DISCARD_WT, Ok("")),
        (PRUNE, Ok("")),
    ]));
    sessions.open(&resolved(Some("aaaaaaa"))).unwrap();

    let first = publish(&sessions, &exec, &features, &fid())
        .await
        .unwrap()
        .unwrap();
    assert!(first.session.pushed_at.is_some());

    let second = publish(&sessions, &exec, &features, &fid())
        .await
        .expect("a second press is not an error")
        .unwrap();
    assert_eq!(second.session.pushed_at, first.session.pushed_at);
    assert_eq!(
        git.commands().iter().filter(|c| *c == PUSH).count(),
        1,
        "{:?}",
        git.commands()
    );
}

/// origin refused the push — the branch moved under it, most likely. The user
/// is owed git's own words and the row must not claim a publication that did
/// not happen: `pushed_at` set here is a resolution the UI stops offering to
/// publish, on a PR that never received it.
#[tokio::test]
async fn a_rejected_push_is_not_recorded_as_published() {
    let (sessions, exec, _git, features) = ports(script(&[(
        PUSH,
        Err("! [rejected] feature/f-1 -> feature/f-1 (non-fast-forward)"),
    )]));
    sessions.open(&resolved(Some("aaaaaaa"))).unwrap();

    let err = publish(&sessions, &exec, &features, &fid())
        .await
        .expect_err("origin said no");
    assert!(err.contains("non-fast-forward"), "{err}");
    assert_eq!(sessions.get(&fid()).unwrap().unwrap().pushed_at, None);
}

/// `git push` exiting zero is a verdict about the command, not about origin.
/// The commit not being reachable from the remote-tracking ref afterwards is
/// the only thing that can prove the publication, and without it nothing is
/// written — the same rule `abort` applies to the worktree it claims to have
/// deleted.
#[tokio::test]
async fn a_push_that_cannot_be_confirmed_is_not_recorded_either() {
    let (sessions, exec, _git, features) = ports(script(&[
        (PUSH, Ok("")),
        (CONTAINS, Err("transport: Connection appears dead")),
    ]));
    sessions.open(&resolved(Some("aaaaaaa"))).unwrap();

    let err = publish(&sessions, &exec, &features, &fid())
        .await
        .expect_err("an unconfirmed push is not a published one");
    assert!(err.contains("Press Publish again"), "{err}");
    assert_eq!(sessions.get(&fid()).unwrap().unwrap().pushed_at, None);
}

/// Discarding is a branch move, and the only place to move it back to is the
/// tip the sync recorded. `merge_commit^` names it only until the resolver adds
/// a follow-up commit, and nothing on the row tells the two apart — so a
/// session missing the real one is refused rather than reset to a guess.
#[tokio::test]
async fn a_resolution_with_no_recorded_base_is_not_discarded() {
    let (sessions, exec, git, features) = ports(script(&[]));
    sessions.open(&resolved(None)).unwrap();

    let err = discard_resolution(&sessions, &exec, &features, &fid())
        .await
        .expect_err("there is nowhere honest to put the branch");
    assert!(err.contains("without guessing"), "{err}");
    assert!(
        !git.commands().iter().any(|c| c.contains("reset")),
        "{:?}",
        git.commands()
    );
    assert_eq!(
        sessions.get(&fid()).unwrap().unwrap().status,
        SyncSessionStatus::Resolved
    );
}

/// What the user actually gets: the branch back where the merge found it, and
/// an abandoned sync — not the conflict. Reproducing that would mean re-running
/// the merge against an origin that has moved since, which is a different
/// operation with a different outcome.
#[tokio::test]
async fn discarding_moves_the_branch_back_and_abandons_the_sync() {
    let (sessions, exec, _git, features) = ports(
        ScriptedExec::new(&[
            (GIT_DIR, Ok(".git\n")),
            (MERGE_HEAD, Ok("")),
            (PORCELAIN, Ok("")),
            (RESET, Ok("")),
            (MERGE_ABORT, Err("fatal: There is no merge to abort")),
            (DISCARD_WT, Ok("")),
            (PRUNE, Ok("")),
        ])
        // Moved off the starting sha while the resolution stood, and back on it
        // once the reset lands — which is what the confirmation reads.
        .with_queue(HEAD_SHA, &[Ok("c0ffeec\n"), Ok("aaaaaaa\n")])
        .with_queue(GIT_DIR, &[Ok(".git\n"), Err("fatal: not a git repository")]),
    );
    sessions.open(&resolved(Some("aaaaaaa"))).unwrap();

    let after = discard_resolution(&sessions, &exec, &features, &fid())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.session.status, SyncSessionStatus::Aborted);
    assert_eq!(after.session.worktree_path, None);
}

/// `intervention_refusal` says abort is refused for a committed resolution, and
/// until this test nothing made that true: the IPC behind the hidden button
/// accepted it, removed the worktree, wrote a terminal `aborted` and left the
/// merge on the branch — with `Publish` and `Discard` both refused afterwards,
/// so the resolution was reachable by nothing.
#[tokio::test]
async fn abandoning_a_sync_is_refused_while_a_resolution_is_waiting_on_it() {
    let (sessions, exec, git, features) = ports(script(&[]));
    sessions.open(&resolved(Some("aaaaaaa"))).unwrap();

    let err = abort(&sessions, &exec, &features, &fid())
        .await
        .expect_err("abort is not how a committed resolution is undone");
    assert!(
        err.contains("Publish the resolution or discard it"),
        "{err}"
    );
    let read = sessions.get(&fid()).unwrap().unwrap();
    assert_eq!(read.status, SyncSessionStatus::Resolved);
    assert_eq!(read.worktree_path.as_deref(), Some(WT));
    assert!(
        !git.commands().iter().any(|c| c.contains("worktree remove")),
        "{:?}",
        git.commands()
    );
}

/// `reset --hard` throws away everything between the tip and the target, and
/// the checkout it runs in can be the user's own clone — `provision_sync_worktree`
/// returns `repo_dir` when the feature branch is already checked out there, and
/// the held path deliberately leaves that value on the row. So a commit added on
/// top of the resolution refuses the discard rather than being deleted by it.
#[tokio::test]
async fn a_branch_that_moved_past_the_resolution_is_not_reset_out_from_under_it() {
    let (sessions, exec, git, features) = ports(ScriptedExec::new(&[
        (GIT_DIR, Ok(".git\n")),
        (MERGE_HEAD, Ok("")),
        (PORCELAIN, Ok("")),
        (HEAD_SHA, Ok("1ate5tc\n")),
    ]));
    sessions.open(&resolved(Some("aaaaaaa"))).unwrap();

    let err = discard_resolution(&sessions, &exec, &features, &fid())
        .await
        .expect_err("something has been committed since");
    assert!(err.contains("1ate5tc"), "{err}");
    assert!(
        !git.commands().iter().any(|c| c.contains("reset")),
        "{:?}",
        git.commands()
    );
    assert_eq!(
        sessions.get(&fid()).unwrap().unwrap().status,
        SyncSessionStatus::Resolved
    );
}

/// The same rule for work that was never committed at all. Uncommitted changes
/// in the checkout holding the branch are the one thing no history can bring
/// back, and neither the button's title nor the confirm dialog mentions them.
#[tokio::test]
async fn a_dirty_checkout_is_not_reset_either() {
    let (sessions, exec, git, features) = ports(ScriptedExec::new(&[
        (GIT_DIR, Ok(".git\n")),
        (MERGE_HEAD, Ok("")),
        (PORCELAIN, Ok(" M src/lib.rs\n")),
    ]));
    sessions.open(&resolved(Some("aaaaaaa"))).unwrap();

    let err = discard_resolution(&sessions, &exec, &features, &fid())
        .await
        .expect_err("uncommitted work is not the discard's to throw away");
    assert!(err.contains("uncommitted changes"), "{err}");
    assert!(
        !git.commands().iter().any(|c| c.contains("reset")),
        "{:?}",
        git.commands()
    );
}

/// The case this module opens on: the user finished the merge in their own
/// editor, so `reconcile` promotes the session out of a moved `HEAD` and
/// nothing ever recorded the commit. Without writing it back, the review card's
/// three conditions all hold and its Publish can only answer "this sync
/// recorded no resolution commit" — while View diff is disabled blaming a base
/// that is right there.
#[tokio::test]
async fn a_resolution_nobody_recorded_still_gets_the_commit_that_proves_it() {
    let (sessions, exec, _git, _features) = ports(ScriptedExec::new(&[
        (GIT_DIR, Ok(".git\n")),
        (MERGE_HEAD, Ok("")),
        (PORCELAIN, Ok("")),
        (HEAD_SHA, Ok("byhandd\n")),
    ]));
    sessions.open(&conflicted(Some(WT))).unwrap();

    let read = get_reconciled(&sessions, &exec, &fid())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.status, SyncSessionStatus::Resolved);
    assert_eq!(read.merge_commit_sha.as_deref(), Some("byhandd"));
    assert_eq!(
        sessions
            .get(&fid())
            .unwrap()
            .unwrap()
            .merge_commit_sha
            .as_deref(),
        Some("byhandd"),
        "and the next reader must not have to re-derive it"
    );
}

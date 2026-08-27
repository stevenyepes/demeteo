use super::super::common::*;
use super::{count_divergence, measured_divergence, patch_equivalence};
use crate::ports::execution::ExecutionPort;

/// Commits on `feature/f-1` that `origin/feature/f-1` does not carry, as the
/// classification is asked to read `git cherry` against.
async fn ahead(helper: &super::super::GitOpsHelper, repo: &str) -> u64 {
    count_divergence(
        &*helper.exec,
        "local",
        repo,
        "refs/heads/feature/f-1",
        "origin/feature/f-1",
    )
    .await
    .ahead
    .expect("ahead counted")
}

/// A branch that is two commits behind upstream and one ahead of it must
/// report `behind: 2`. The two `rev-list` ranges are exact inverses of each
/// other and read the same either way round, so an asymmetric fixture is the
/// only one that can tell them apart.
#[tokio::test]
async fn behind_count_is_not_the_ahead_count() {
    let (local_dir, remote_dir, helper) = make_two_repos("divergence_behind").await;
    let local = local_dir.to_string_lossy().to_string();
    let remote = remote_dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/f-1"),
        )
        .await;
    exec.write_file("local", &format!("{local}/feature.txt"), "one")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{local}\" add ."))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" commit --no-verify -m feature-one"),
        )
        .await;

    for i in 0..2 {
        exec.write_file("local", &format!("{remote}/up{i}.txt"), "up")
            .await
            .unwrap();
        let _ = exec
            .run_command("local", &format!("git -C \"{remote}\" add ."))
            .await;
        let _ = exec
            .run_command(
                "local",
                &format!("git -C \"{remote}\" commit --no-verify -m upstream-{i}"),
            )
            .await;
    }
    let _ = exec
        .run_command("local", &format!("git -C \"{local}\" fetch origin -- main"))
        .await;

    let divergence = count_divergence(
        &*helper.exec,
        "local",
        &local,
        "refs/heads/feature/f-1",
        "origin/main",
    )
    .await;

    assert_eq!(
        divergence.behind,
        Some(2),
        "upstream is two commits ahead of the feature branch, so the feature \
         branch is two behind; got {divergence:?}"
    );
    assert_eq!(
        divergence.ahead,
        Some(1),
        "the feature branch carries one commit upstream does not have; got {divergence:?}"
    );

    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

/// A ref that does not resolve is an unmeasured branch, not a synced one.
#[tokio::test]
async fn a_count_that_could_not_be_taken_is_unknown() {
    let (dir, helper) = make_repo("divergence_unknown").await;
    let repo = dir.to_string_lossy().to_string();

    let divergence = count_divergence(
        &*helper.exec,
        "local",
        &repo,
        "refs/heads/main",
        "origin/does-not-exist",
    )
    .await;

    assert_eq!(
        divergence.behind, None,
        "an unresolvable base must read as unknown; reporting 0 would render \
         the branch as up to date with something nobody could look at"
    );
    assert_eq!(divergence.ahead, None, "and so must the other direction");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The fixture the whole classification rests on, read from git rather than
/// asserted about: a commit origin has never seen is marked `+`, which is the
/// arm a sync may merge on its own.
#[tokio::test]
async fn a_commit_origin_never_saw_is_marked_plus() {
    let (local_dir, remote_dir, helper) = make_two_repos("cherry_disjoint").await;
    let local = local_dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/f-1"),
        )
        .await;
    exec.write_file("local", &format!("{local}/pushed.txt"), "shared")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{local}\" add ."))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" commit --no-verify -m shared"),
        )
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" push origin feature/f-1"),
        )
        .await;
    exec.write_file("local", &format!("{local}/only-here.txt"), "local only")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{local}\" add ."))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" commit --no-verify -m only-here"),
        )
        .await;

    let cherry = patch_equivalence(&*helper.exec, "local", &local, "feature/f-1")
        .await
        .expect("git cherry answered");

    assert_eq!(
        crate::domain::upstream_feature::classify_divergence(
            Some(&cherry),
            ahead(&helper, &local).await
        ),
        crate::domain::upstream_feature::DivergenceMove::MergeOrigin,
        "one commit origin does not carry, and its patch is not up there \
         either: {cherry:?}"
    );

    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

/// The other half of the same fixture, and the one no count can reach: the
/// local commit is *gone* from origin's history by sha and still entirely
/// present in it by patch, which is what an amend somewhere else leaves behind.
#[tokio::test]
async fn a_commit_origin_rewrote_is_marked_minus() {
    let (local_dir, remote_dir, helper) = make_two_repos("cherry_rewritten").await;
    let local = local_dir.to_string_lossy().to_string();
    let remote = remote_dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/f-1"),
        )
        .await;
    exec.write_file("local", &format!("{local}/work.txt"), "the work")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{local}\" add ."))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" commit --no-verify -m work"),
        )
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" push origin feature/f-1"),
        )
        .await;

    // The same change, re-committed under another sha — origin's branch is
    // rewritten and its content is unchanged.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{remote}\" checkout feature/f-1"),
        )
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{remote}\" commit --amend --no-verify -m reworded"),
        )
        .await;
    let _ = exec
        .run_command("local", &format!("git -C \"{remote}\" checkout main"))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" fetch origin -- feature/f-1"),
        )
        .await;

    let cherry = patch_equivalence(&*helper.exec, "local", &local, "feature/f-1")
        .await
        .expect("git cherry answered");

    assert_eq!(
        crate::domain::upstream_feature::classify_divergence(
            Some(&cherry),
            ahead(&helper, &local).await
        ),
        crate::domain::upstream_feature::DivergenceMove::ResetOntoOrigin,
        "origin carries this commit's patch under another sha: {cherry:?}"
    );

    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

/// A read that did not happen is not a branch with nothing on it. The two are
/// one `unwrap_or_default` apart and land on opposite arms — an empty answer
/// would be read as unanimous, and unanimity is what moves the user's ref.
#[tokio::test]
async fn a_cherry_that_could_not_run_is_not_an_empty_answer() {
    let (dir, helper) = make_repo("cherry_unknown").await;
    let repo = dir.to_string_lossy().to_string();

    assert_eq!(
        patch_equivalence(&*helper.exec, "local", &repo, "never-pushed").await,
        None,
        "a branch with no upstream cannot be classified at all"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The read the pane offers its two presses off, over the one fixture where
/// the counts and the answer disagree: one commit each way, and the only safe
/// reset in the whole module.
///
/// A branch that has not diverged answers nothing at all — the pane has an arm
/// for a divergence and no arm for the absence of one, so a `Some` carrying two
/// zeroes would render an offer to reconcile a branch that is level.
#[tokio::test]
async fn the_offer_is_measured_and_not_counted() {
    let (local_dir, remote_dir, helper) = make_two_repos("divergence_offer").await;
    let local = local_dir.to_string_lossy().to_string();
    let remote = remote_dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/f-1"),
        )
        .await;
    exec.write_file("local", &format!("{local}/work.txt"), "the work")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{local}\" add ."))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" commit --no-verify -m work"),
        )
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" push origin feature/f-1"),
        )
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{remote}\" checkout feature/f-1"),
        )
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{remote}\" commit --amend --no-verify -m reworded"),
        )
        .await;
    // A second commit only origin has, so the two counts differ: they are read
    // through one struct whose fields are the same type, and a fixture with one
    // each way cannot tell a swap from the truth.
    exec.write_file("local", &format!("{remote}/theirs.txt"), "theirs")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{remote}\" add ."))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{remote}\" commit --no-verify -m theirs"),
        )
        .await;
    let _ = exec
        .run_command("local", &format!("git -C \"{remote}\" checkout main"))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" fetch origin -- feature/f-1"),
        )
        .await;

    assert_eq!(
        measured_divergence(&*helper.exec, "local", &local, "feature/f-1").await,
        Some(crate::domain::models::FeatureDivergence {
            ahead: 1,
            behind: 2,
            next_move: crate::domain::upstream_feature::DivergenceMove::ResetOntoOrigin,
        }),
        "one commit here that origin holds the patch of already, two there"
    );
    assert_eq!(
        measured_divergence(&*helper.exec, "local", &local, "main").await,
        None,
        "a branch level with its upstream has nothing to reconcile"
    );

    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

/// `git cherry` walks with `max_parents=1`, so a merge commit in the ahead set
/// is never printed and never classified. The shape is Demeteo's own: a sync or
/// a subtask merge on the branch, and a rebase in another clone that carried
/// every ordinary commit's patch across but not that merge's tree.
///
/// Fixture rather than a table, because the claim under test is git's and not
/// the domain's: the counts and the cherry lines have to come from a real
/// repository or the test only asserts what it was told.
#[tokio::test]
async fn a_merge_commit_the_cherry_never_printed_is_not_a_reset() {
    let (local_dir, remote_dir, helper) = make_two_repos("cherry_merge_commit").await;
    let local = local_dir.to_string_lossy().to_string();
    let remote = remote_dir.to_string_lossy().to_string();
    let exec = fresh_exec();
    let git = |repo: &str, args: String| format!("git -C \"{repo}\" {args}");

    let _ = exec
        .run_command("local", &git(&local, "checkout -b feature/f-1".into()))
        .await;
    exec.write_file("local", &format!("{local}/work.txt"), "the work")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &git(&local, "add .".into()))
        .await;
    let _ = exec
        .run_command("local", &git(&local, "commit --no-verify -m work".into()))
        .await;
    let _ = exec
        .run_command("local", &git(&local, "push origin feature/f-1".into()))
        .await;

    // The merge commit, and a resolution only its own tree carries.
    let _ = exec
        .run_command("local", &git(&local, "checkout -b side".into()))
        .await;
    exec.write_file("local", &format!("{local}/side.txt"), "theirs")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &git(&local, "add .".into()))
        .await;
    let _ = exec
        .run_command("local", &git(&local, "commit --no-verify -m side".into()))
        .await;
    let _ = exec
        .run_command("local", &git(&local, "checkout feature/f-1".into()))
        .await;
    let _ = exec
        .run_command(
            "local",
            &git(&local, "merge --no-ff side -m merged-side".into()),
        )
        .await;

    // Origin's branch, rewritten elsewhere: the side commit's patch is up
    // there under another sha, the merge is not up there at all.
    let _ = exec
        .run_command("local", &git(&remote, "checkout feature/f-1".into()))
        .await;
    exec.write_file("local", &format!("{remote}/side.txt"), "theirs")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &git(&remote, "add .".into()))
        .await;
    let _ = exec
        .run_command(
            "local",
            &git(&remote, "commit --no-verify -m side-rebased".into()),
        )
        .await;
    let _ = exec
        .run_command("local", &git(&remote, "checkout main".into()))
        .await;
    let _ = exec
        .run_command("local", &git(&local, "fetch origin -- feature/f-1".into()))
        .await;

    let cherry = patch_equivalence(&*helper.exec, "local", &local, "feature/f-1")
        .await
        .expect("git cherry answered");
    let counted = ahead(&helper, &local).await;
    assert_eq!(counted, 2, "the side commit and the merge: {cherry:?}");
    assert_eq!(
        cherry.lines().filter(|l| !l.trim().is_empty()).count(),
        1,
        "git cherry skipped the merge commit: {cherry:?}"
    );

    assert_eq!(
        measured_divergence(&*helper.exec, "local", &local, "feature/f-1")
            .await
            .map(|d| d.next_move),
        Some(crate::domain::upstream_feature::DivergenceMove::Refuse),
        "unanimous over one of two commits is not unanimous: {cherry:?}"
    );

    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

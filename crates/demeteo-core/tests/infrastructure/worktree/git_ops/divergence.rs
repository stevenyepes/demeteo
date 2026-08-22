use super::super::common::*;
use super::count_divergence;
use crate::ports::execution::ExecutionPort;

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

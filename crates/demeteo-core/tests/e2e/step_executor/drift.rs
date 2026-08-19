//! The staleness count, asserted as the git the header's chip pays for.
//!
//! Three decisions live above `count_divergence` and none of them left a trace
//! anywhere the divergence tests can see: which base the counts are taken
//! against, whether `origin/<base>` is fetched first, and what a fetch that
//! failed costs. The last is the whole departure from `sync_feature_with_upstream`
//! — a sync that cannot reach origin has nothing to merge and must stop, where
//! a poll that cannot reach it still has a true answer from the last time
//! anything moved that ref, and reporting an error instead spends the entire
//! signal on a flaky network.

use std::sync::Arc;

use super::harness::{build_test_executor_in, scratch_dir, FakeNotif};
use super::sync_base::{seed_feature, REPO_PATH};
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::models::FeatureDrift;
use crate::paths;

/// The git a drift read on a feature with this `origin` issues, with the two
/// `rev-list` ranges scripted and the fetch answering `fetch`.
async fn drift_git(
    label: &str,
    origin: FeatureOrigin,
    base: &str,
    refresh: bool,
    fetch: Result<&str, &str>,
) -> (Vec<String>, Result<FeatureDrift, String>) {
    let temp_dir = scratch_dir(label);
    let repo_dir = paths::repo_target_dir_local(&temp_dir, &format!("p-{label}"), REPO_PATH)
        .to_string_lossy()
        .to_string();
    let feature_branch = format!("demeteo/features/f-{label}");
    let behind =
        format!("git -C {repo_dir} rev-list --count refs/heads/{feature_branch}..origin/{base}");
    let ahead =
        format!("git -C {repo_dir} rev-list --count origin/{base}..refs/heads/{feature_branch}");
    let fetch_cmd = format!("git -C {repo_dir} fetch origin -- {base}");

    let exec = Arc::new(ScriptedExec::new(&[]).with_programs(&[
        (behind.as_str(), Ok("4")),
        (ahead.as_str(), Ok("1")),
        (fetch_cmd.as_str(), fetch),
    ]));
    let (executor, db) =
        build_test_executor_in(temp_dir.clone(), Arc::new(FakeNotif), exec.clone()).await;
    let feature_id = seed_feature(&db, label, origin, None);

    let answer = executor.feature_drift_impl(&feature_id, refresh).await;

    let ran = exec.programs();
    let _ = std::fs::remove_dir_all(temp_dir);
    (ran, answer)
}

/// A release-cut run counted against the project default reports the distance
/// to trunk — a number about a branch the sync would never merge, and one that
/// grows on its own while the release branch stays exactly where it was.
#[tokio::test]
async fn a_run_cut_from_a_release_branch_is_counted_against_that_branch() {
    let (ran, answer) = drift_git(
        "drift_release",
        FeatureOrigin::Branch {
            base: "release/2.0".to_string(),
        },
        "release/2.0",
        false,
        Ok(""),
    )
    .await;

    let drift = answer.expect("the counts were scripted");
    assert_eq!(drift.base_ref, "origin/release/2.0");
    assert_eq!(drift.divergence.behind, Some(4));
    assert_eq!(drift.divergence.ahead, Some(1));
    assert!(
        ran.iter().all(|argv| !argv.contains("origin/main")),
        "nothing may be counted against the project default here: {ran:?}"
    );
}

/// The read is two local `git` calls and no network unless the user asked for
/// one. Nothing else moves `origin/<base>` for a finished feature, so the flag
/// the chip renders has to be the flag this returns.
#[tokio::test]
async fn a_fetch_happens_only_when_it_was_asked_for() {
    let (quiet, answer) = drift_git(
        "drift_quiet",
        FeatureOrigin::DefaultBranch,
        "main",
        false,
        Ok(""),
    )
    .await;
    assert!(
        !quiet.iter().any(|argv| argv.contains(" fetch ")),
        "an unasked-for fetch is a network round trip per row on the project view: {quiet:?}"
    );
    assert!(
        !answer.expect("the counts were scripted").fetched,
        "counts taken over a ref nobody refreshed must not claim to be current"
    );

    let (asked, answer) = drift_git(
        "drift_asked",
        FeatureOrigin::DefaultBranch,
        "main",
        true,
        Ok(""),
    )
    .await;
    assert!(
        asked
            .iter()
            .any(|argv| argv.ends_with("fetch origin -- main")),
        "the press exists to move origin/main; without the fetch the count cannot change: {asked:?}"
    );
    assert!(answer.expect("the counts were scripted").fetched);
}

/// A sync stops on a fetch it could not make. This must not: the previous
/// `origin/<base>` is still a real ref, and an error here would replace a
/// slightly old number with no number at all.
#[tokio::test]
async fn a_fetch_that_failed_still_answers_with_the_counts_it_has() {
    let (_, answer) = drift_git(
        "drift_offline",
        FeatureOrigin::DefaultBranch,
        "main",
        true,
        Err("fatal: unable to access 'https://origin/': Could not resolve host"),
    )
    .await;

    let drift = answer.expect("an unreachable origin is not the end of the signal");
    assert_eq!(drift.divergence.behind, Some(4));
    assert!(
        !drift.fetched,
        "a fetch that failed must not be reported as one that happened"
    );
}

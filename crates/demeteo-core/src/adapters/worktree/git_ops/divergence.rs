use super::git_request;
use crate::domain::upstream_feature::FeatureUpstream;
use crate::ports::execution::ExecutionPort;
use crate::ports::worktree_ops::BranchDivergence;

/// Count how `feature_ref` and `tracking` have diverged, from local refs only.
///
/// Two `git rev-list --count` reads and no network: the answer is only as fresh
/// as whatever last moved `tracking`, and a caller that needs it current has to
/// fetch before asking. Nothing here does that, because the one caller that
/// must fail on a bad fetch and the one that must survive it disagree about
/// what a fetch error means.
///
/// A read that did not answer is `None`, never `0` —
/// [`BranchDivergence`] carries why that distinction is load-bearing.
pub(crate) async fn count_divergence(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
    feature_ref: &str,
    tracking: &str,
) -> BranchDivergence {
    BranchDivergence {
        behind: count_range(exec, machine_id, repo_dir, feature_ref, tracking).await,
        ahead: count_range(exec, machine_id, repo_dir, tracking, feature_ref).await,
    }
}

/// Commits reachable from `head` and not from `base`.
async fn count_range(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
    base: &str,
    head: &str,
) -> Option<u64> {
    exec.run_program(
        machine_id,
        git_request(
            repo_dir,
            ["rev-list", "--count", &format!("{base}..{head}")],
        ),
    )
    .await
    .ok()?
    .trim()
    .parse::<u64>()
    .ok()
}

/// Where `origin/<feature_branch>` stands relative to the local branch of the
/// same name, or `None` when origin carries no such branch.
///
/// `None` is the first sync of every feature and says nothing is wrong: the
/// branch was cut here and has never been pushed, so there is no upstream half
/// to reconcile. It is deliberately not distinguished from an unreadable
/// `rev-parse` — both leave the caller with no upstream to compare against, and
/// the sync that follows behaves the same way in either case.
///
/// The counts are read against `refs/heads/<feature>` rather than `HEAD`: this
/// runs before any worktree exists, and the shared checkout may be on anything.
pub(crate) async fn feature_upstream(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
    feature_branch: &str,
) -> Option<FeatureUpstream> {
    let tracking = format!("origin/{feature_branch}");
    exec.run_program(
        machine_id,
        git_request(repo_dir, ["rev-parse", "--verify", &tracking]),
    )
    .await
    .ok()?;
    Some(crate::domain::upstream_feature::reconcile(
        count_divergence(
            exec,
            machine_id,
            repo_dir,
            &format!("refs/heads/{feature_branch}"),
            &tracking,
        )
        .await,
    ))
}

/// Bring `refs/remotes/origin/<base_branch>` up to date, answering whether it
/// worked.
///
/// A `bool` rather than a `Result` because the only caller must carry on
/// either way: an unreachable origin leaves the previous ref in place, which is
/// still a real answer as long as nothing presents it as a current one.
pub(crate) async fn refresh_base_ref(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
    base_branch: &str,
) -> bool {
    exec.run_program(
        machine_id,
        git_request(repo_dir, ["fetch", "origin", "--", base_branch]),
    )
    .await
    .is_ok()
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/divergence.rs"]
mod tests;

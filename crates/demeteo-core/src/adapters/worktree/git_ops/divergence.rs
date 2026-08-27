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

/// The raw `git cherry origin/<feature_branch> refs/heads/<feature_branch>`
/// output, for
/// [`classify_divergence`](crate::domain::upstream_feature::classify_divergence)
/// to read a [`DivergenceMove`](crate::domain::upstream_feature::DivergenceMove)
/// off.
///
/// Local refs only, exactly like [`count_divergence`]: a divergence is only
/// worth classifying once the counts have been taken, and those are taken
/// after the fetch that made `origin/<feature>` current. Fetching again here
/// would put a second network failure between a measured divergence and the
/// one read that can tell a rewritten branch from disjoint work.
///
/// `None` is every way the answer did not arrive — a `git` that could not run,
/// a ref that does not resolve, a transport that dropped — and it is the same
/// `None` the domain refuses on. Flattening it to an empty string would make
/// a failed command read as a branch with nothing on it, and the arm that
/// answer lands on resets the user's ref.
pub(crate) async fn patch_equivalence(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
    feature_branch: &str,
) -> Option<String> {
    let tracking = format!("origin/{feature_branch}");
    let feature_ref = format!("refs/heads/{feature_branch}");
    exec.run_program(
        machine_id,
        git_request(repo_dir, ["cherry", &tracking, &feature_ref]),
    )
    .await
    .ok()
}

/// What the branch and `origin/<feature_branch>` each hold that the other does
/// not, and what may be done about it — or `None` when they do not disagree.
///
/// The read behind the pane's offer, and it deliberately re-measures rather
/// than reading the counts a blocked sync recorded
/// ([`crate::domain::upstream_feature`]). Local refs only, like everything
/// else here — the sync's own fetch is what makes `origin/<feature>` current,
/// and a read that fetched would answer a different question than the one the
/// sync will act on.
///
/// `None` for a branch with no upstream, for one that is level or merely ahead,
/// and for a read that did not answer. They differ, but not in what is on
/// offer: there is nothing to reconcile in any of them.
pub(crate) async fn measured_divergence(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
    feature_branch: &str,
) -> Option<crate::domain::models::FeatureDivergence> {
    let FeatureUpstream::Diverged { ahead, behind } =
        feature_upstream(exec, machine_id, repo_dir, feature_branch).await?
    else {
        return None;
    };
    let cherry = patch_equivalence(exec, machine_id, repo_dir, feature_branch).await;
    Some(crate::domain::models::FeatureDivergence {
        ahead,
        behind,
        next_move: crate::domain::upstream_feature::classify_divergence(cherry.as_deref(), ahead),
    })
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

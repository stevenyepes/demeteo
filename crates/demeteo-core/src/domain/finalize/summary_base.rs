//! Where the range finalize summarises starts.

use crate::domain::models::Feature;

/// The left side of the range the finalize agent is shown: the revision the
/// squash collapses the run onto, [`FeatureOrigin::squash_base`].
///
/// The summary becomes the message of the one commit that is published, and
/// the title and body of the PR that carries it, so it has to describe that
/// commit and nothing below it. [`diff_base::resolve`] answers a different
/// question — where a *review* diff starts — and on a run launched to fix a
/// pull request that is the request's target, a range that also holds every
/// commit of the request itself.
///
/// Takes the whole [`Feature`] so that reading `diff_base_branch` here is a
/// regression a test can catch rather than a signature that rules it out.
///
/// [`FeatureOrigin::squash_base`]: crate::domain::feature_origin::FeatureOrigin::squash_base
/// [`diff_base::resolve`]: crate::domain::diff_base::resolve
pub(crate) fn summary_base(feature: &Feature, default_branch: &str) -> String {
    feature.origin.squash_base(default_branch)
}

#[cfg(test)]
#[path = "../../../tests/domain/finalize/summary_base.rs"]
mod tests;

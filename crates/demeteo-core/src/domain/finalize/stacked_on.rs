//! What a stacked pull request must disclose about inherited commits.

use crate::domain::feature_origin::FeatureOrigin;

/// The notice prepended to the published PR body when it carries commits
/// outside the finalize summary range, or `None` when it does not. The same
/// notice is included in the authoring prompt so the agent can explain it.
///
/// Only [`FeatureOrigin::Ref`] gets one. Its summary range starts at the
/// fetched head of the reviewed request ([`FeatureOrigin::squash_base`]), but
/// its PR opens against that request's target
/// ([`FeatureOrigin::publish_target`]), so the pushed branch carries every
/// commit of the request as ancestors of the one fix commit, and merging it
/// lands them all. Widening the range instead would make the agent summarise
/// the contributor's work as its own and squash it into the fix. Without the
/// note, the PR is titled after a 2-line fix and a squash-merge records the
/// whole request under that title.
///
/// `Branch` publishes against the head it was cut from, and `DefaultBranch`
/// cuts from and publishes against the same branch. In both cases the PR
/// carries exactly the summarised range.
pub(crate) fn stacked_on_note(origin: &FeatureOrigin, lands_on: &str) -> Option<String> {
    match origin {
        FeatureOrigin::Ref { fetch_spec, .. } => {
            let request = reviewed_request(fetch_spec);
            Some(format!(
                "> ⚠️ This pull request is stacked on the commits of {request}. Merging it into \
                 `{lands_on}` also merges that reviewed request's commits."
            ))
        }
        FeatureOrigin::DefaultBranch | FeatureOrigin::Branch { .. } => None,
    }
}

fn reviewed_request(fetch_spec: &str) -> String {
    for (prefix, marker) in [("refs/pull/", "#"), ("refs/merge-requests/", "!")] {
        if let Some(number) = fetch_spec
            .strip_prefix(prefix)
            .and_then(|tail| tail.strip_suffix("/head"))
            .filter(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
        {
            return format!("reviewed request {marker}{number}");
        }
    }
    "the reviewed request".to_string()
}

#[cfg(test)]
#[path = "../../../tests/domain/finalize/stacked_on.rs"]
mod tests;

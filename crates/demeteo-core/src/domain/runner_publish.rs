//! Whether a detached run's terminal push may be driven again from the desktop.
//!
//! A detached run's feature branch exists only in the runner's clone, so its
//! push and PR can only be made there. The desktop's Publish MR on such a run
//! asks the runner to repeat the tail it ran at the end — push, then open the
//! PR — rather than pushing from a desktop clone that never had the branch.
//!
//! What may be repeated is the publish and nothing before it. A run whose
//! feature did not finish has nothing worth publishing, and a feature that
//! already has a PR would only be re-pushed under a PR someone may be
//! reviewing.

/// The runner RPC this module decides for.
pub const METHOD: &str = "publish_run";

/// Feature statuses a run reaches when every step succeeded and only the
/// publish is left — or was already attempted.
const PUBLISHABLE: [&str; 2] = ["awaiting_mr", "completed"];

/// Run-row statuses with a task still driving the run, which the publish would
/// race.
const IN_FLIGHT: [&str; 2] = ["pending", "running"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishRefusal {
    InFlight { status: String },
    Unfinished { feature_status: String },
    AlreadyOpen { url: String },
}

impl std::fmt::Display for PublishRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InFlight { status } => write!(
                f,
                "the run is still {status} on the runner; it publishes when it finishes"
            ),
            Self::Unfinished { feature_status } => write!(
                f,
                "the feature is {feature_status}, not finished, so there is nothing to publish"
            ),
            Self::AlreadyOpen { url } => write!(f, "the feature already has a PR: {url}"),
        }
    }
}

/// `None` when the runner may push and open the PR for a run in `run_status`
/// whose feature is in `feature_status` with `mr_url`.
pub fn publish_refusal(
    run_status: &str,
    feature_status: &str,
    mr_url: Option<&str>,
) -> Option<PublishRefusal> {
    if IN_FLIGHT.contains(&run_status) {
        return Some(PublishRefusal::InFlight {
            status: run_status.to_string(),
        });
    }
    if let Some(url) = mr_url.filter(|url| !url.is_empty()) {
        return Some(PublishRefusal::AlreadyOpen {
            url: url.to_string(),
        });
    }
    (!PUBLISHABLE.contains(&feature_status)).then(|| PublishRefusal::Unfinished {
        feature_status: feature_status.to_string(),
    })
}

#[cfg(test)]
#[path = "../../tests/domain/runner_publish.rs"]
mod tests;

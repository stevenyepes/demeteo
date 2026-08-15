//! Where a run starts, as one value instead of three derivations. See
//! [`crate::domain`].
//!
//! The starting point used to be `ProjectSettings.worktree_strategy.default_branch`
//! read separately at each place that needed it: the branch cut in
//! `adapters/worktree/git_ops/worktree.rs`, the squash base in
//! `adapters/worktree/git_ops/squash.rs`, and the PR target in
//! `adapters/mr_publisher/mod.rs`. Three readers of one field is fine while the
//! answer is always the same field; it stops being fine the moment a run may
//! start somewhere else, because "somewhere else" is four separate answers —
//! what to fetch, what to cut from, what to name the branch, and what the run
//! is measured against (its squash base, its review diff, its PR target) — and
//! nothing forces three call sites to agree on them. Those four answers are
//! the methods here, and they are the reason the type exists rather than an
//! extra `Option<String>` beside `default_branch`.
//!
//! An earlier draft carried a fourth arm that adopted an existing worktree. It
//! was cut after review: an adopted worktree's uncommitted changes are
//! reachable by no step in the pipeline, so the arm could only ever start a run
//! from a tree whose visible state it silently discarded.

use serde::{Deserialize, Serialize};

/// The refs `git fetch origin <refspec>` must bring down before the cut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchPlan {
    /// The single argument after the remote name.
    pub refspec: String,
    /// The ref that argument lands in, and the one
    /// [`FeatureOrigin::start_point`] then names.
    pub local_ref: String,
}

/// Where a run's feature branch is cut from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeatureOrigin {
    #[default]
    DefaultBranch,
    Branch {
        base: String,
    },
    /// The only arm that reaches a head living in a fork, which is where a
    /// pull request's branch usually is: `refs/pull/<n>/head` and
    /// `refs/merge-requests/<iid>/head` resolve against the *upstream*
    /// remote whether or not the contributor's remote is reachable at all.
    /// `label` is what a person called it; nothing derives from it.
    Ref {
        fetch_spec: String,
        label: String,
    },
}

/// How the bootstrap brings a run's start point down and points the run's
/// branch at it.
///
/// A value the adapter executes rather than two calls it chooses between,
/// because the arms differ in what a failed fetch *means* and that is a
/// decision, not an argument. `FromDefaultBranch` keeps the pre-V41
/// behaviour: an unreachable origin leaves the local `<default>` ref alone
/// and the cut falls back to it, because a slightly stale default branch is
/// still the branch the user asked for. `FromFetchedRef` has no such
/// fallback in either half — the ref it names exists only because this fetch
/// created it, so swallowing the failure would cut the run's branch from
/// whatever else `start_point` happened to resolve to and hand a reviewer a
/// diff against a tree nobody chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchCut {
    FromDefaultBranch,
    FromFetchedRef {
        refspec: String,
        start_point: String,
    },
}

/// Refs fetched for a [`FeatureOrigin::Ref`] land under this namespace.
///
/// Not `refs/heads/`: a fetched PR head is a snapshot to cut from, and a local
/// branch of the same name would be offered as a base by
/// [`crate::domain::branch_listing`] and pushed by anything that pushes
/// matching branches. A private namespace is invisible to both.
const FETCHED_REF_PREFIX: &str = "refs/demeteo/origins/";

impl FeatureOrigin {
    /// The branch this run works on, which no arm varies — the origin decides
    /// where the branch starts, not what it is called.
    pub fn branch_to_cut(&self, prefix: &str, feature_id: &str) -> String {
        format!("{prefix}{feature_id}")
    }

    /// What `git fetch origin …` must bring down before the cut. Every arm has
    /// something to fetch, so this is total: [`FeatureOrigin::start_point`]
    /// names a ref that only exists once this has run.
    pub fn fetch_plan(&self, default_branch: &str) -> FetchPlan {
        let branch_fetch = |branch: &str| FetchPlan {
            refspec: branch.to_string(),
            local_ref: format!("origin/{branch}"),
        };
        match self {
            Self::DefaultBranch => branch_fetch(default_branch),
            Self::Branch { base } => branch_fetch(base),
            Self::Ref { fetch_spec, .. } => {
                let local_ref = fetched_ref(fetch_spec);
                FetchPlan {
                    refspec: format!("{fetch_spec}:{local_ref}"),
                    local_ref,
                }
            }
        }
    }

    /// What `git branch -f <branch_to_cut> <start_point>` is given.
    pub fn start_point(&self, default_branch: &str) -> String {
        match self {
            Self::DefaultBranch => format!("origin/{default_branch}"),
            Self::Branch { base } => format!("origin/{base}"),
            Self::Ref { fetch_spec, .. } => fetched_ref(fetch_spec),
        }
    }

    /// The [`BranchCut`] the bootstrap performs for this origin, combining
    /// [`FeatureOrigin::fetch_plan`] and [`FeatureOrigin::start_point`] with
    /// the strictness each arm is owed.
    pub fn branch_cut(&self, default_branch: &str) -> BranchCut {
        match self {
            Self::DefaultBranch => BranchCut::FromDefaultBranch,
            Self::Branch { .. } | Self::Ref { .. } => BranchCut::FromFetchedRef {
                refspec: self.fetch_plan(default_branch).refspec,
                start_point: self.start_point(default_branch),
            },
        }
    }

    /// Decode `features.origin_json` (V41). NULL, empty, or a document this
    /// build cannot parse all answer [`FeatureOrigin::DefaultBranch`]: a run
    /// that started from nowhere is not a state, so there is no third answer
    /// to give and no caller has to handle one.
    pub fn from_column(raw: Option<&str>) -> Self {
        let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Self::DefaultBranch;
        };
        serde_json::from_str(raw).unwrap_or_default()
    }

    /// Encode for `features.origin_json`. The inverse of
    /// [`FeatureOrigin::from_column`], so the default stores as SQL NULL and
    /// the column carries one spelling of it rather than two.
    pub fn to_column(&self) -> Option<String> {
        match self {
            Self::DefaultBranch => None,
            other => serde_json::to_string(other).ok(),
        }
    }

    /// The branch a run started here treats as its base: what a PR targets,
    /// what the review diff is measured from, and what `finalize` squashes
    /// onto. `None` means the run named no base of its own and the project's
    /// default branch stands.
    ///
    /// `review_base` is the launcher's explicit answer
    /// ([`crate::domain::run_spec::RunSpec::diff_base_branch`]) and wins where
    /// it is given, because [`FeatureOrigin::Ref`] has no base to offer: a PR
    /// head is not a target any host accepts — often not even a ref in this
    /// repository — so what such a run merges into is a choice only its
    /// launcher can make.
    ///
    /// [`FeatureOrigin::DefaultBranch`] answers `None` whatever `review_base`
    /// says. That arm's fetch and cut are *defined* in terms of the default
    /// branch, so letting a review base through here would move the start
    /// point of a run that asked to start from the default branch.
    pub fn base_branch<'a>(&'a self, review_base: Option<&'a str>) -> Option<&'a str> {
        match self {
            Self::DefaultBranch => None,
            Self::Branch { base } => review_base.or(Some(base.as_str())),
            Self::Ref { .. } => review_base,
        }
    }

    /// What a PR opened by this run targets.
    pub fn publish_target(&self, default_branch: &str) -> String {
        self.base_branch(None).unwrap_or(default_branch).to_string()
    }
}

fn fetched_ref(fetch_spec: &str) -> String {
    let tail = fetch_spec.trim_start_matches("refs/");
    format!("{FETCHED_REF_PREFIX}{tail}")
}

#[cfg(test)]
#[path = "../../tests/domain/feature_origin.rs"]
mod tests;

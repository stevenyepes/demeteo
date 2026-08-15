//! Where a run starts, as one value instead of three derivations. See
//! [`crate::domain`].
//!
//! Three call sites would otherwise each read
//! `ProjectSettings.worktree_strategy.default_branch` for themselves: the
//! branch cut in `adapters/worktree/git_ops/worktree.rs`, the squash base in
//! `adapters/worktree/git_ops/squash.rs`, and the PR target in
//! `adapters/mr_publisher/mod.rs`. Three readers of one field is fine while the
//! answer is always that field; it stops being fine the moment a run may
//! start somewhere else, because "somewhere else" is several separate answers
//! — what to fetch, what to cut from, what to name the branch, what the squash
//! parents onto, and what the run is measured against (its review diff, its PR
//! target) — and nothing forces three call sites to agree on them. Those
//! answers are the methods here, and they are the reason the type exists rather
//! than an extra `Option<String>` beside `default_branch`.
//!
//! An earlier draft carried a fourth arm that adopted an existing worktree. It
//! was cut after review: an adopted worktree's uncommitted changes are
//! reachable by no step in the pipeline, so the arm could only ever start a run
//! from a tree whose visible state it silently discarded.

use serde::{Deserialize, Serialize};

/// A refspec `git fetch` cannot mistake for an option.
///
/// git parses argv before it parses refspecs, so an argument beginning with
/// `-` is an option whatever it was meant to be, and `--upload-pack=<cmd>`
/// makes git run `<cmd>`: `git fetch origin --upload-pack='touch X' main`
/// creates X. [`FeatureOrigin::Ref`] is the arm a later ticket fills from a
/// provider's API — a PR number becoming `refs/pull/<n>/head` — so the value
/// reaching that argv stops being one this repository wrote.
///
/// Two defences, because either alone is one edit away from gone: this type,
/// which no hostile string can be built into or deserialised into, and the
/// `--` separator that
/// [`WorktreeOpsPort::fetch_origin_refspec`](crate::ports::worktree_ops::WorktreeOpsPort::fetch_origin_refspec)
/// passes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Refspec(String);

impl Refspec {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Refspec {
    type Error = String;

    fn try_from(spec: String) -> Result<Self, String> {
        let named = spec.trim_start_matches('+');
        if named.is_empty() {
            return Err("a refspec names no ref".to_string());
        }
        if named.starts_with('-') {
            return Err(format!("git would read the refspec '{spec}' as an option"));
        }
        if spec.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(format!(
                "the refspec '{spec}' carries whitespace, so it names no single ref"
            ));
        }
        Ok(Self(spec))
    }
}

impl From<Refspec> for String {
    fn from(spec: Refspec) -> Self {
        spec.0
    }
}

/// The refs `git fetch origin -- <refspec>` must bring down before the cut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchPlan {
    /// The single argument after the remote name.
    pub refspec: Refspec,
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
/// A value the adapter executes rather than three calls it chooses between,
/// because the arms differ in what a failed fetch *means* and that is a
/// decision, not an argument.
///
/// `FromDefaultBranch` and `FromRemoteBranch` both cut from a
/// remote-tracking ref a normal clone already carries, so an unreachable
/// origin leaves that ref alone and the cut proceeds from a possibly stale
/// copy of the branch the user named — the best-effort fetch every pre-V41
/// path here used, and the reason it exists. `FromFetchedRef` has no such
/// predecessor: the ref it names exists only because this fetch created it,
/// so swallowing the failure would cut the run's branch from whatever else
/// `start_point` happened to resolve to and hand a reviewer a diff against a
/// tree nobody chose.
///
/// The cut is strict in the two arms that name a ref outright. Only
/// `FromDefaultBranch` falls back, and to the local `<default>`, which is
/// [`create_feature_branch`](crate::ports::worktree_ops::WorktreeOpsPort::create_feature_branch)'s
/// own offline path rather than anything decided here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchCut {
    FromDefaultBranch,
    FromRemoteBranch {
        refspec: Refspec,
        start_point: String,
    },
    FromFetchedRef {
        refspec: Refspec,
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
    /// something to fetch, so every arm answers: [`FeatureOrigin::start_point`]
    /// names a ref that only exists once this has run.
    ///
    /// `Err` is a refspec git would not read as this origin meant it — see
    /// [`Refspec`], and the `refs/` requirement below. It is the gate every
    /// other `Ref` derivation stands behind: [`FeatureOrigin::start_point`]
    /// and [`FeatureOrigin::squash_base`] name the ref *this* fetch created,
    /// so they cannot be reached for an origin whose plan was refused.
    pub fn fetch_plan(&self, default_branch: &str) -> Result<FetchPlan, String> {
        let branch_fetch = |branch: &str| {
            Ok(FetchPlan {
                refspec: Refspec::try_from(branch.to_string())?,
                local_ref: format!("origin/{branch}"),
            })
        };
        match self {
            Self::DefaultBranch => branch_fetch(default_branch),
            Self::Branch { base } => branch_fetch(base),
            Self::Ref { fetch_spec, .. } => {
                // A `Ref` that is not a full ref path lands under
                // `FETCHED_REF_PREFIX` anyway, where it reads as a PR head
                // this repository never fetched.
                if !fetch_spec.starts_with("refs/") {
                    return Err(format!(
                        "a fetched origin must name a ref under refs/, not '{fetch_spec}'"
                    ));
                }
                let local_ref = fetched_ref(fetch_spec);
                Ok(FetchPlan {
                    // `+` because the destination is a disposable snapshot in
                    // a namespace nothing else reads: re-running a review
                    // against a force-pushed PR head has to retarget it, and
                    // without the `+` git rejects that fetch as a
                    // non-fast-forward and the whole run fails.
                    refspec: Refspec::try_from(format!("+{fetch_spec}:{local_ref}"))?,
                    local_ref,
                })
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
    pub fn branch_cut(&self, default_branch: &str) -> Result<BranchCut, String> {
        Ok(match self {
            Self::DefaultBranch => BranchCut::FromDefaultBranch,
            Self::Branch { .. } => BranchCut::FromRemoteBranch {
                refspec: self.fetch_plan(default_branch)?.refspec,
                start_point: self.start_point(default_branch),
            },
            Self::Ref { .. } => BranchCut::FromFetchedRef {
                refspec: self.fetch_plan(default_branch)?.refspec,
                start_point: self.start_point(default_branch),
            },
        })
    }

    /// Decode `features.origin_json` (V41). NULL and empty answer
    /// [`FeatureOrigin::DefaultBranch`]: a run that started from nowhere is
    /// not a state, and every row written before the column existed cut from
    /// the default branch.
    ///
    /// A document that is present and unreadable is a different fact and gets
    /// a different answer. It says a run started somewhere this build cannot
    /// name, and calling that the default branch would resume it on the wrong
    /// branch, diff it against the wrong tree and squash it onto the wrong
    /// parent — three silent wrongs from one unreadable column.
    pub fn from_column(raw: Option<&str>) -> Result<Self, serde_json::Error> {
        let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(Self::DefaultBranch);
        };
        serde_json::from_str(raw)
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

    /// The branch a run started here treats as its base: what a PR targets and
    /// what the review diff is measured from. `None` means the run named no
    /// base of its own and the project's default branch stands.
    ///
    /// Not what `finalize` squashes onto — that is
    /// [`FeatureOrigin::squash_base`], and the two answers separate exactly
    /// where it matters.
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

    /// The revision `finalize` collapses the run's commits onto — the point
    /// the branch was cut from, and so the parent of the one commit that is
    /// published.
    ///
    /// Deliberately not [`FeatureOrigin::base_branch`], which answers where
    /// the run's *diff* starts. For [`FeatureOrigin::Ref`] the two differ by
    /// the whole of the pull request the run was launched on: squashing onto
    /// the branch that PR targets would give the published commit a parent
    /// below the PR author's commits, so the squash swallows their work into
    /// the run's single commit — and nothing shows it, because a stacked PR
    /// built on that commit still applies and simply replays the original diff.
    ///
    /// Spelled as the bare name rather than [`FeatureOrigin::start_point`]'s
    /// `origin/<branch>` because the squash tries `refs/remotes/origin/<base>`
    /// first and the local ref second, which keeps a clone with no reachable
    /// origin resolving. A fetched ref has no such pair — it exists only
    /// locally — and is named outright.
    pub fn squash_base(&self, default_branch: &str) -> String {
        match self {
            Self::DefaultBranch => default_branch.to_string(),
            Self::Branch { base } => base.clone(),
            Self::Ref { fetch_spec, .. } => fetched_ref(fetch_spec),
        }
    }
}

fn fetched_ref(fetch_spec: &str) -> String {
    let tail = fetch_spec.trim_start_matches("refs/");
    format!("{FETCHED_REF_PREFIX}{tail}")
}

#[cfg(test)]
#[path = "../../tests/domain/feature_origin.rs"]
mod tests;

//! Which harness resolves a merge conflict, at which model and effort.
//!
//! A chain of its own, and not the step executor's `resolve_agent_model`,
//! because the resolver is a *role* rather than a step: the harness a run was
//! launched with was chosen for the coding work, and a project may want merge
//! conflicts handled by something cheaper — or something stronger — without
//! that following the run around. Hence `project_sync` above `run`, on the
//! reasoning `VerifierConfig::model` outranks a run's model override.
//!
//! Policy only; see [`crate::domain`] for why that means synchronous and
//! port-free.
//!
//! What the choice does **not** reach: the profile the resolver spawns under
//! ([`PermissionProfile::all_allow`](crate::domain::permission::PermissionProfile::all_allow)),
//! how that profile becomes argv or env, and the worktree it is fenced to.
//! Those are the same for every harness a user can pick here — but *what the
//! fence is worth* is not, and that asymmetry is recorded where the spawn
//! happens rather than here.

use crate::domain::models::{AgentKind, EffortLevel, Feature, ProjectSettings};

/// One tier's opinion. Every field `None` means "no opinion, ask the tier
/// below", so an empty choice costs a caller nothing to send.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncResolverChoice {
    pub agent_kind: Option<String>,
    pub model: Option<String>,
    pub effort: Option<EffortLevel>,
}

impl SyncResolverChoice {
    /// This choice, with each dimension it has no opinion on taken from
    /// `lower`. Per-dimension rather than whole-choice: pinning the harness at
    /// one tier and the model at another is legal, matching how
    /// `resolve_agent_model` reads its own tiers.
    pub fn or(self, lower: &SyncResolverChoice) -> Self {
        SyncResolverChoice {
            agent_kind: self.agent_kind.or_else(|| lower.agent_kind.clone()),
            model: self.model.or_else(|| lower.model.clone()),
            effort: self.effort.or(lower.effort),
        }
    }

    /// The harness this choice names that Demeteo has no runtime for, if any.
    ///
    /// The only thing that can be wrong with a choice, and it has to be
    /// answered by whoever *receives* it rather than by the turn: the
    /// resolution kills its agent session on each of its many exit paths, so a
    /// refusal added below the spawn is how the process leaks instead. A kind
    /// nobody named is not a problem — that is inheritance.
    pub fn unsupported_agent_kind(&self) -> Option<&str> {
        self.agent_kind
            .as_deref()
            .filter(|kind| !AgentKind::is_supported(kind))
    }

    /// The project's resolver-specific default (migration V44).
    pub fn from_project_sync(settings: &ProjectSettings) -> Self {
        SyncResolverChoice {
            agent_kind: settings.sync_resolver_agent_kind.clone(),
            model: settings.sync_resolver_model.clone(),
            effort: settings.sync_resolver_effort,
        }
    }

    /// The project's run-wide default, the lowest tier of every other chain.
    pub fn from_project_default(settings: &ProjectSettings) -> Self {
        SyncResolverChoice {
            agent_kind: settings.default_agent_kind.clone(),
            model: settings.default_model.clone(),
            effort: settings.default_effort,
        }
    }

    /// What the run was launched with.
    pub fn from_run(feature: &Feature) -> Self {
        SyncResolverChoice {
            agent_kind: feature.agent_kind.clone(),
            model: feature.model.clone(),
            effort: feature.effort,
        }
    }
}

/// The tiers, named rather than positional: four `&SyncResolverChoice`
/// arguments in a row is an argument-order bug that no type would catch.
pub struct SyncResolverChain<'a> {
    /// What *this* attempt asked for — the conflict banner's picker on the
    /// button's path, the `sync` node's own config (under any per-step run
    /// override) on the workflow's.
    pub asked: &'a SyncResolverChoice,
    /// [`SyncResolverChoice::from_project_sync`].
    pub project_sync: &'a SyncResolverChoice,
    /// [`SyncResolverChoice::from_run`].
    pub run: &'a SyncResolverChoice,
    /// [`SyncResolverChoice::from_project_default`].
    pub project_default: &'a SyncResolverChoice,
}

/// A resolved launch identity.
///
/// `effort` is not optional: every turn runs at *some* effort, and the adapter
/// clamps it to what its harness accepts — never this type's job.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncResolver {
    pub agent_kind: String,
    pub model: Option<String>,
    pub effort: EffortLevel,
}

/// The harness a chain that names none anywhere terminates at, matching
/// `resolve_agent_model`'s own terminal fallback.
const FALLBACK_AGENT_KIND: &str = "opencode";

impl SyncResolverChain<'_> {
    pub fn resolve(&self) -> SyncResolver {
        let folded = self
            .asked
            .clone()
            .or(self.project_sync)
            .or(self.run)
            .or(self.project_default);
        SyncResolver {
            agent_kind: folded
                .agent_kind
                .unwrap_or_else(|| FALLBACK_AGENT_KIND.to_string()),
            model: folded.model,
            effort: folded.effort.unwrap_or(EffortLevel::DEFAULT),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/domain/sync_resolver.rs"]
mod tests;

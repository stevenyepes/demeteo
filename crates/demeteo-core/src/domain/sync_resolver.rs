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

use crate::domain::models::{
    AgentKind, EffortLevel, Feature, ProjectSettings, StepConfig, StepOverride,
};

/// One tier's opinion. Every field `None` means "no opinion, ask the tier
/// below", so an empty choice costs a caller nothing to send.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncResolverChoice {
    pub agent_kind: Option<String>,
    pub model: Option<String>,
    pub effort: Option<EffortLevel>,
}

impl SyncResolverChoice {
    /// This choice over `lower` — everything it has no opinion on taken from
    /// there, except that **the model travels with the harness**.
    ///
    /// A model name is one harness's namespace, so a choice naming a harness
    /// `lower` does not name cannot borrow `lower`'s model: `sonnet` inherited
    /// past a `codex` pin spawns `codex --model sonnet`. Dropped instead, the
    /// model stays unset and the chosen harness resolves its own default —
    /// what `impl_traits/replay.rs` does when an operator re-pins the
    /// feature-wide harness without naming a model. Effort has no such
    /// namespace — the ladder is canonical and each adapter clamps it — so it
    /// crosses freely.
    ///
    /// The harness and the model may still be pinned at different tiers; what
    /// they may not do is disagree about which harness the model is for. That
    /// comparison is only sound against an already-folded `lower` — see
    /// [`SyncResolverChain::resolve`].
    pub fn or(self, lower: &SyncResolverChoice) -> Self {
        let model_travels = self.agent_kind.is_none() || self.agent_kind == lower.agent_kind;
        let inherited_model = if model_travels {
            lower.model.clone()
        } else {
            None
        };
        SyncResolverChoice {
            agent_kind: self.agent_kind.or_else(|| lower.agent_kind.clone()),
            model: self.model.or(inherited_model),
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
    /// Folded from the bottom up, which for the harness and the effort is the
    /// same answer either way and for the model is not: a tier that names no
    /// harness of its own runs under the one the tiers *below* it resolve to,
    /// and only a stack already folded that far can say what that is. Folding
    /// downwards would compare a pinned harness against a bare `None` and drop
    /// a model the two tiers actually agreed on.
    pub fn resolve(&self) -> SyncResolver {
        let below_run = self.run.clone().or(self.project_default);
        let below_sync = self.project_sync.clone().or(&below_run);
        let folded = self.asked.clone().or(&below_sync);
        SyncResolver {
            agent_kind: folded
                .agent_kind
                .unwrap_or_else(|| FALLBACK_AGENT_KIND.to_string()),
            model: folded.model,
            effort: folded.effort.unwrap_or(EffortLevel::DEFAULT),
        }
    }
}

/// The chain behind the "Resolve with agent" button, and behind the harness
/// name the conflict banner shows for an untouched picker: no driver is running
/// on that path, so every tier under `asked` is read straight off the two rows
/// that hold it.
///
/// One function so the button and the banner's label cannot disagree about who
/// would run — a label naming a harness other than the one that spawns is the
/// whole failure this replaces.
pub fn resolve_stored(
    asked: &SyncResolverChoice,
    feature: &Feature,
    settings: &ProjectSettings,
) -> SyncResolver {
    let project_sync = SyncResolverChoice::from_project_sync(settings);
    let run = SyncResolverChoice::from_run(feature);
    let project_default = SyncResolverChoice::from_project_default(settings);
    SyncResolverChain {
        asked,
        project_sync: &project_sync,
        run: &run,
        project_default: &project_default,
    }
    .resolve()
}

/// What a `sync` node inside a running workflow resolves through.
///
/// The tiers below `project_sync` reach this as choices rather than rows: a
/// driver has already folded the project's workflow overrides into them, and
/// re-reading the feature here would undo that.
pub struct SyncNodeTiers<'a> {
    /// The node's own `agent_kind` / `model` / `effort`.
    pub step_conf: &'a StepConfig,
    /// This run's override for *this* node, if the launch pinned one.
    pub step_override: Option<&'a StepOverride>,
    pub settings: &'a ProjectSettings,
    /// [`SyncResolverChoice::from_run`], as the driver holds it.
    pub run: &'a SyncResolverChoice,
    /// [`SyncResolverChoice::from_project_default`], as the driver holds it.
    pub project_default: &'a SyncResolverChoice,
}

impl SyncNodeTiers<'_> {
    /// The node's config and this run's override for it are one tier, not two:
    /// both name the resolution turn specifically, so they fold together into
    /// `asked` before the chain the button walks resumes underneath.
    pub fn resolve(&self) -> SyncResolver {
        let node = SyncResolverChoice {
            agent_kind: self.step_conf.agent_kind.clone(),
            model: self.step_conf.model.clone(),
            effort: self.step_conf.effort,
        };
        let asked = match self.step_override {
            Some(ov) => SyncResolverChoice {
                agent_kind: ov.agent_kind.clone(),
                model: ov.model.clone(),
                effort: ov.effort,
            }
            .or(&node),
            None => node,
        };
        let project_sync = SyncResolverChoice::from_project_sync(self.settings);
        SyncResolverChain {
            asked: &asked,
            project_sync: &project_sync,
            run: self.run,
            project_default: self.project_default,
        }
        .resolve()
    }
}

#[cfg(test)]
#[path = "../../tests/domain/sync_resolver.rs"]
mod tests;

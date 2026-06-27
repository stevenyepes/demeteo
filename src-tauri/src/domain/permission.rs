//! Step capabilities and the agent-agnostic permission model.
//!
//! The orchestrator never speaks an agent's native permission dialect
//! (opencode's `OPENCODE_PERMISSION` JSON, claude-code's `--disallowedTools`,
//! …). Instead every agent step resolves to a [`StepCapability`], which
//! compiles to an abstract [`PermissionProfile`] of four orthogonal
//! intents. Each agent adapter then translates the profile into its own
//! enforcement (see `AgentRuntime::permission_env` and the per-agent
//! `build_args`). This keeps "what a step is allowed to do" in one place
//! and lets every coding agent work the same way.
//!
//! Two invariants make this safe to run fully autonomously (no permission
//! prompts, the property the pipeline depends on):
//!
//! 1. The compiled policy only ever uses **allow** or **deny**, never
//!    "ask". A denied tool is rejected instantly; the agent gets a
//!    tool-result error and keeps going. Nothing blocks waiting on a human.
//! 2. The *artifacts-vs-source* line is path-shaped, which no agent's
//!    tool model expresses. That distinction is enforced uniformly by the
//!    OS-level chmod fence (`worktree::git_ops::scope`), driven by
//!    [`StepCapability::write_scope`]. `write_fs: Allow` + the fence means
//!    "can write `artifacts/`, cannot touch source" identically across
//!    every agent.

use serde::{Deserialize, Serialize};

/// The role a step plays, and the permission posture that follows from it.
///
/// A step author picks the capability; the engine derives the tool policy
/// and the writable path scope from it. Authors can widen a capability
/// without changing its role via the orthogonal `allow_network` /
/// `allow_shell` toggles on `StepConfig` (see [`resolve_profile`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepCapability {
    /// Pure review/inspection. Reads the repo; writes nothing; runs no
    /// shell. E.g. an adversarial critic or a coverage-baseline check.
    ReadOnly,
    /// Analysis / specification. Reads the repo and writes *only* under
    /// `artifacts/`; no shell. E.g. research, spec drafting, diagnosis.
    /// The chmod fence makes source read-only; the missing shell removes
    /// the `chmod u+w` escape hatch.
    Artifacts,
    /// Validation. Reads the repo, runs build/test/lint/audit commands,
    /// and writes *only* under `artifacts/` (its report). Must NOT modify
    /// source — fixes are routed back to an implementation step. E.g. the
    /// final validate/QA gate.
    Verify,
    /// Implementation. Full read/write/shell within the worktree. E.g.
    /// the parallel implement workers, a bugfix step, a prototype.
    Implement,
}

impl StepCapability {
    /// The base permission profile for this capability, before any
    /// per-step `allow_network` / `allow_shell` overrides.
    pub fn base_profile(&self) -> PermissionProfile {
        match self {
            StepCapability::ReadOnly => PermissionProfile {
                read_fs: Access::Allow,
                write_fs: Access::Deny,
                execute: Access::Deny,
                network: Access::Deny,
            },
            StepCapability::Artifacts => PermissionProfile {
                read_fs: Access::Allow,
                write_fs: Access::Allow, // path-scoped to artifacts/ by the fence
                execute: Access::Deny,
                network: Access::Deny,
            },
            StepCapability::Verify => PermissionProfile {
                read_fs: Access::Allow,
                write_fs: Access::Allow, // path-scoped to artifacts/ by the fence
                execute: Access::Allow,
                network: Access::Deny,
            },
            StepCapability::Implement => PermissionProfile {
                read_fs: Access::Allow,
                write_fs: Access::Allow,
                execute: Access::Allow,
                network: Access::Deny,
            },
        }
    }

    /// Which worktree paths the step may write to. Consumed by
    /// `worktree::git_ops::scope` to set the chmod fence and the
    /// post-step diff guard.
    pub fn write_scope(&self) -> WriteScope {
        match self {
            StepCapability::ReadOnly => WriteScope::None,
            StepCapability::Artifacts | StepCapability::Verify => WriteScope::ArtifactsOnly,
            StepCapability::Implement => WriteScope::All,
        }
    }
}

/// The path-scope intent of a capability. The scope adapter turns this
/// into concrete writable paths (`ArtifactsOnly` resolves to the step's
/// declared artifact paths, falling back to `artifacts/`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteScope {
    /// No writes anywhere in the worktree.
    None,
    /// Writes only under the step's declared artifact paths (or
    /// `artifacts/` when none are declared).
    ArtifactsOnly,
    /// Writes anywhere in the worktree (no fence).
    All,
}

/// Allow or deny — the only two states the compiled policy ever uses.
/// There is deliberately no "ask": prompting would break the autonomous
/// pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Allow,
    Deny,
}

impl Access {
    pub fn is_allow(&self) -> bool {
        matches!(self, Access::Allow)
    }
    /// The opencode permission string for this access level.
    pub fn opencode_str(&self) -> &'static str {
        match self {
            Access::Allow => "allow",
            Access::Deny => "deny",
        }
    }
}

/// The abstract, agent-agnostic permission posture for a single step.
///
/// Four orthogonal intents. Each agent adapter maps these to its native
/// tool model:
/// - `read_fs`  → Read/Grep/Glob/LS (cat/grep/find are tools, *not* shell)
/// - `write_fs` → Edit/Write/Create (path-scoped by the chmod fence)
/// - `execute`  → the shell / Bash tool
/// - `network`  → WebSearch / WebFetch
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionProfile {
    pub read_fs: Access,
    pub write_fs: Access,
    pub execute: Access,
    pub network: Access,
}

impl PermissionProfile {
    /// The unrestricted profile — everything allowed. Used for the
    /// interactive/probe agent sessions that aren't pipeline steps, and
    /// as the back-compat default on [`AgentContext`].
    pub fn all_allow() -> Self {
        Self {
            read_fs: Access::Allow,
            write_fs: Access::Allow,
            execute: Access::Allow,
            network: Access::Allow,
        }
    }
}

impl Default for PermissionProfile {
    fn default() -> Self {
        Self::all_allow()
    }
}

/// Compile a capability plus its per-step overrides into a concrete
/// profile. `allow_network` opts the step into web search/fetch (e.g. a
/// research step consulting live docs); `allow_shell` opts a non-shell
/// capability into the shell (e.g. an Artifacts step that wants `git
/// log`). Overrides can only *widen*, never narrow — a capability that
/// already denies a dimension can be granted it, but an allowed
/// dimension stays allowed.
pub fn resolve_profile(
    cap: StepCapability,
    allow_network: bool,
    allow_shell: bool,
) -> PermissionProfile {
    let mut p = cap.base_profile();
    if allow_network {
        p.network = Access::Allow;
    }
    if allow_shell {
        p.execute = Access::Allow;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_denies_writes_shell_and_net() {
        let p = StepCapability::ReadOnly.base_profile();
        assert_eq!(p.read_fs, Access::Allow);
        assert_eq!(p.write_fs, Access::Deny);
        assert_eq!(p.execute, Access::Deny);
        assert_eq!(p.network, Access::Deny);
        assert_eq!(StepCapability::ReadOnly.write_scope(), WriteScope::None);
    }

    #[test]
    fn artifacts_allows_write_denies_shell() {
        let p = StepCapability::Artifacts.base_profile();
        assert_eq!(p.write_fs, Access::Allow);
        assert_eq!(p.execute, Access::Deny);
        assert_eq!(
            StepCapability::Artifacts.write_scope(),
            WriteScope::ArtifactsOnly
        );
    }

    #[test]
    fn verify_allows_shell_but_scopes_writes_to_artifacts() {
        let p = StepCapability::Verify.base_profile();
        assert_eq!(p.execute, Access::Allow);
        assert_eq!(p.write_fs, Access::Allow);
        assert_eq!(
            StepCapability::Verify.write_scope(),
            WriteScope::ArtifactsOnly
        );
    }

    #[test]
    fn implement_allows_everything_in_worktree() {
        let p = StepCapability::Implement.base_profile();
        assert_eq!(p.write_fs, Access::Allow);
        assert_eq!(p.execute, Access::Allow);
        assert_eq!(StepCapability::Implement.write_scope(), WriteScope::All);
    }

    #[test]
    fn network_default_off_for_every_capability() {
        for cap in [
            StepCapability::ReadOnly,
            StepCapability::Artifacts,
            StepCapability::Verify,
            StepCapability::Implement,
        ] {
            assert_eq!(cap.base_profile().network, Access::Deny);
        }
    }

    #[test]
    fn overrides_widen_only() {
        // Artifacts gains shell + network when toggled on.
        let p = resolve_profile(StepCapability::Artifacts, true, true);
        assert_eq!(p.network, Access::Allow);
        assert_eq!(p.execute, Access::Allow);
        assert_eq!(p.write_fs, Access::Allow);

        // Toggling off leaves the base posture untouched.
        let p = resolve_profile(StepCapability::Artifacts, false, false);
        assert_eq!(p.network, Access::Deny);
        assert_eq!(p.execute, Access::Deny);
    }

    #[test]
    fn capability_round_trips_through_serde_snake_case() {
        let json = serde_json::to_string(&StepCapability::ReadOnly).unwrap();
        assert_eq!(json, "\"read_only\"");
        let back: StepCapability = serde_json::from_str("\"artifacts\"").unwrap();
        assert_eq!(back, StepCapability::Artifacts);
    }
}

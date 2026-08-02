// `AgentConfig` is declared in `thread.rs` because that is where the DB row it
// is parsed out of lives, but the rules for *defaulting* one belong with the
// agent vocabulary they are expressed in, so they are written here.
use super::thread::AgentConfig;
use serde::{Deserialize, Serialize};

/// Canonical identifier for a supported coding agent.
///
/// The wire/DB form is the kebab string (`opencode`, `hermes`,
/// `claude-code`). [`AgentConfig::kind`](super::AgentConfig) and the
/// [`AgentRuntime`](crate::ports::agent_runtime::AgentRuntime) port stay
/// stringly-typed for serde/DB compatibility and to key live sessions, but
/// every *validation* boundary routes a raw string through [`AgentKind::parse`]
/// and every *behavior* decision routes through the runtime's declared
/// [`AgentCapabilities`](crate::ports::agent_runtime::AgentCapabilities) —
/// never a bare `match` on the string. `kind()` on the runtime is documented
/// to equal [`AgentKind::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentKind {
    Opencode,
    Hermes,
    ClaudeCode,
    Codex,
    Pi,
}

impl AgentKind {
    /// Every supported kind, in canonical display order.
    pub const ALL: [AgentKind; 5] = [
        AgentKind::Opencode,
        AgentKind::Hermes,
        AgentKind::ClaudeCode,
        AgentKind::Codex,
        AgentKind::Pi,
    ];

    /// The stable kebab identifier used on the wire, in the DB, and as the
    /// registry `kind()` key.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentKind::Opencode => "opencode",
            AgentKind::Hermes => "hermes",
            AgentKind::ClaudeCode => "claude-code",
            AgentKind::Codex => "codex",
            AgentKind::Pi => "pi",
        }
    }

    /// Parse a stored/wire kind string. Returns `None` for unknown kinds
    /// (e.g. the removed `antigravity`) so legacy stored configs degrade to
    /// "unsupported" rather than panicking or being silently accepted.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "opencode" => Some(AgentKind::Opencode),
            "hermes" => Some(AgentKind::Hermes),
            "claude-code" => Some(AgentKind::ClaudeCode),
            "codex" => Some(AgentKind::Codex),
            "pi" => Some(AgentKind::Pi),
            _ => None,
        }
    }

    /// Whether `s` names a currently-supported agent kind.
    pub fn is_supported(s: &str) -> bool {
        Self::parse(s).is_some()
    }
}

impl std::fmt::Display for AgentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The answer an availability probe gives for one agent kind on one machine.
///
/// [`Unknown`](Self::Unknown) is the whole reason this is not a `bool`. The
/// probe is a command run on the target host, so "the binary is not there"
/// and "the host did not answer" arrive through the same channel; collapsing
/// them loses the only distinction a *default* may be derived from. A remote
/// machine that is briefly unreachable would otherwise report every agent as
/// uninstalled, and because Project Settings persists the whole list on any
/// save, that momentary answer would become the user's stored intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Availability {
    Installed,
    Missing,
    /// The probe could not be completed — a transport error, not an answer.
    Unknown,
}

impl Availability {
    /// Whether the binary is known to be there. `Unknown` answers `false`:
    /// callers asking this are about to *run* the agent, where the
    /// conservative reading is the safe one. Seeding a default is the one
    /// caller that must not use this — see [`AgentConfig::default_for`].
    pub fn is_installed(self) -> bool {
        matches!(self, Availability::Installed)
    }

    /// Whether the probe produced an answer worth remembering. The registry's
    /// session cache consults this so a single transport failure is retried on
    /// the next look rather than pinned for the life of the app.
    pub fn is_conclusive(self) -> bool {
        !matches!(self, Availability::Unknown)
    }

    /// Read what an `ExecutionPort` said about a `command -v` probe.
    ///
    /// The three fates are already in the port's D3 contract, so no adapter
    /// has to invent them: a non-zero exit is `command -v` answering "no",
    /// while [`TRANSPORT_ERROR_PREFIX`] and [`TIMEOUT_ERROR_PREFIX`] mark a
    /// probe that never produced an answer at all. `ok_marker` is the token
    /// the probe echoes on success.
    ///
    /// [`TRANSPORT_ERROR_PREFIX`]: crate::ports::execution::TRANSPORT_ERROR_PREFIX
    /// [`TIMEOUT_ERROR_PREFIX`]: crate::ports::execution::TIMEOUT_ERROR_PREFIX
    pub fn from_probe(result: Result<String, String>, ok_marker: &str) -> Availability {
        use crate::ports::execution::{TIMEOUT_ERROR_PREFIX, TRANSPORT_ERROR_PREFIX};
        match result {
            Ok(out) if out.trim() == ok_marker => Availability::Installed,
            Ok(_) => Availability::Missing,
            Err(e)
                if e.starts_with(TRANSPORT_ERROR_PREFIX) || e.starts_with(TIMEOUT_ERROR_PREFIX) =>
            {
                Availability::Unknown
            }
            Err(_) => Availability::Missing,
        }
    }
}

impl AgentConfig {
    /// Default enablement for a kind not yet in the stored config: disabled
    /// only when the CLI is known to be absent, so an uninstalled agent never
    /// appears pre-checked.
    ///
    /// [`Availability::Unknown`] defaults to *enabled* — the opposite of
    /// what [`Availability::is_installed`] would give. An unanswered probe is
    /// not evidence of absence, and the two mistakes are not symmetric: a
    /// pre-checked agent that turns out to be missing is still filtered out of
    /// every picker by its `available` flag, whereas a pre-unchecked agent
    /// that was installed all along becomes stored user intent the moment
    /// anything in Project Settings is saved.
    pub fn default_for(kind: &str, availability: Availability) -> AgentConfig {
        AgentConfig {
            kind: kind.to_string(),
            enabled: availability != Availability::Missing,
        }
    }

    /// Append any `(kind, availability)` pair in `known` that isn't already in
    /// `existing`, defaulted via [`Self::default_for`]. Entries already
    /// present in `existing` pass through untouched — a saved enable/disable
    /// is never overridden by a probe.
    pub fn seed_missing(
        mut existing: Vec<AgentConfig>,
        known: &[(&str, Availability)],
    ) -> Vec<AgentConfig> {
        for (kind, availability) in known {
            if existing.iter().any(|c| c.kind == *kind) {
                continue;
            }
            existing.push(AgentConfig::default_for(kind, *availability));
        }
        existing
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: Option<String>,
    pub is_locked: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoHealthStatus {
    pub repo_path: String, // logical path e.g. "org/repo"
    pub is_cloned: bool,
    pub head_branch: Option<String>,
    pub worktrees: Vec<WorktreeInfo>,
    pub has_uncommitted: bool,
    pub has_unpushed: bool,
}

#[cfg(test)]
#[path = "../../../tests/domain/models/agent_config.rs"]
mod tests;

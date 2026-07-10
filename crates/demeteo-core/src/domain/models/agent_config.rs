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
}

impl AgentKind {
    /// Every supported kind, in canonical display order.
    pub const ALL: [AgentKind; 4] = [
        AgentKind::Opencode,
        AgentKind::Hermes,
        AgentKind::ClaudeCode,
        AgentKind::Codex,
    ];

    /// The stable kebab identifier used on the wire, in the DB, and as the
    /// registry `kind()` key.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentKind::Opencode => "opencode",
            AgentKind::Hermes => "hermes",
            AgentKind::ClaudeCode => "claude-code",
            AgentKind::Codex => "codex",
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

#[derive(Debug, Serialize, Deserialize, Clone)]
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

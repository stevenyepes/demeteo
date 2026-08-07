use super::agent_config::AgentKind;
use super::platform::Platform;

/// What Demeteo knows about the sandbox an agent enforces on a given platform.
///
/// A peer of [`EffortLevel::supported_for`](super::EffortLevel::supported_for):
/// one declared per-agent capability, so an adapter asks instead of deriving
/// and no caller carries agent-specific knowledge. It differs in being keyed on
/// the platform as well, because a sandbox is an OS mechanism rather than a
/// model setting — codex enforces through Seatbelt on macOS and Landlock/seccomp
/// on Linux, so the same flag is backed by two different kernels and, on a host
/// with neither, by nothing.
///
/// The platform is an [`Option`] because it reaches the caller from
/// `ExecutionPort::resolve_platform`, which a transport is allowed to decline
/// to answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SandboxSupport {
    /// The agent has an enforcement backend on this platform, and Demeteo picks
    /// which posture it runs in.
    Enforced,
    /// Demeteo's adapter for this agent puts no sandbox selection on the wire,
    /// on any platform. A statement about the adapter and not about the agent:
    /// a harness that grows a sandbox flag Demeteo decides to drive leaves this
    /// arm the day the adapter emits one, not the day upstream ships it.
    Undriven,
    /// No evidence either way — the agent is driven, but whether this platform
    /// backs the flag with anything has never been observed.
    ///
    /// Resolves to the same bytes as [`Enforced`](Self::Enforced): an open
    /// question must not change what ships, so this variant is a record rather
    /// than a behaviour, and [`selects_sandbox`](Self::selects_sandbox) is where
    /// that decision is spelled once.
    Unknown,
}

impl SandboxSupport {
    /// The sandbox posture Demeteo can expect from `kind` running on `platform`.
    ///
    /// Codex on Windows is [`Unknown`](Self::Unknown), which is not
    /// [`Undriven`](Self::Undriven): its two published backends are POSIX
    /// kernel facilities, and whether its Windows build carries a third has not
    /// been observed here. A capture from `DEMETEO_AGENT_TRACE` of a codex turn
    /// on a Windows desktop settles it — the turn either runs with the mode
    /// accepted, or codex names the unsupported sandbox on its own event stream.
    /// A capture showing nothing enforces it moves this arm to a variant whose
    /// [`selects_sandbox`](Self::selects_sandbox) is false, and the codex arg
    /// builder then emits no sandbox selection on Windows. Until such a capture
    /// exists the arm is inert and the wire bytes are the POSIX ones.
    ///
    /// An unresolved platform reads as `Unknown` for the same reason: the
    /// question was not answered, and the answer is not the desktop's own OS.
    pub fn for_agent(kind: AgentKind, platform: Option<Platform>) -> Self {
        match kind {
            AgentKind::Codex => match platform {
                Some(Platform::Linux) | Some(Platform::MacOS) => Self::Enforced,
                Some(Platform::Windows) | None => Self::Unknown,
            },
            AgentKind::ClaudeCode | AgentKind::Hermes | AgentKind::Opencode | AgentKind::Pi => {
                Self::Undriven
            }
        }
    }

    /// Whether the agent's arg builder should put a sandbox selection on the
    /// wire at all.
    pub const fn selects_sandbox(self) -> bool {
        match self {
            Self::Enforced | Self::Unknown => true,
            Self::Undriven => false,
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/models/sandbox.rs"]
mod tests;

use super::agent_config::AgentKind;
use super::platform::Platform;
use serde::{Deserialize, Serialize};

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

/// What refuses a turn's access to paths outside the directory it was given,
/// one answer per class of access.
///
/// A claim about *Demeteo's adapter*, in the shape [`SandboxSupport`] already
/// uses: the value moves the day Demeteo's own spawn puts a fence on the wire,
/// not the day a harness ships one upstream. It answers for the
/// capability-scoped turn — [`AgentContext::bare_mode`](crate::ports::agent_runtime::AgentContext::bare_mode)
/// — which is the turn a per-attempt harness picker is choosing between, and
/// the turn whose profile may be `all_allow`, leaving the cwd as the only thing
/// confining it.
///
/// Three dimensions and not one rank, because every harness measured refuses
/// one class of access and serves another, and no ordering of those answers is
/// truthful. Codex's kernel sandbox refuses a write anywhere outside its
/// writable roots and serves a read of the entire filesystem; opencode's check
/// covers both classes through the file tools and then loses the shell. A
/// single value has to pick one class to report, and whichever it picks it
/// overstates the rest for somebody — which is the whole failure this type
/// exists to prevent, because what reads it is a sentence shown to a user
/// deciding whether their other repositories are safe.
///
/// The evidence for each value sits on the adapter that declares it, beside the
/// argv and env builders that are the whole of it. Nothing is restated here:
/// the day a builder changes is the day the reader is in that file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PathContainment {
    pub reads: Enforcement,
    pub writes: Enforcement,
    pub shell: Enforcement,
}

/// What stands between one class of access and a path outside the turn's cwd.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Enforcement {
    /// The kernel refuses. The agent cannot talk its way past it.
    Os,
    /// The harness refuses before it dispatches the tool.
    Harness,
    /// The harness refuses part of this dimension and not the rest. Carries no
    /// detail — the adapter's own declaration records what escapes.
    HarnessPartial,
    /// Nothing refuses.
    None,
}

impl PathContainment {
    /// The weakest claim, and the one an unmeasured harness gets: the directory
    /// is where the turn starts, not a boundary it is held to.
    pub const UNFENCED: Self = Self {
        reads: Enforcement::None,
        writes: Enforcement::None,
        shell: Enforcement::None,
    };

    /// What confines `kind` to its cwd on `platform`.
    ///
    /// Codex reads [`UNFENCED`](Self::UNFENCED) on Windows and on an unresolved
    /// platform, where [`SandboxSupport::for_agent`] reads
    /// [`Unknown`](SandboxSupport::Unknown) and still ships the selection. One
    /// open question, resolved opposite ways on purpose: there it decides what
    /// Demeteo sends, and an unobserved backend is no reason to change the
    /// bytes; here it decides what a user is told is protecting them, and an
    /// unobserved backend protects nobody. The capture named there settles both.
    pub fn for_agent(kind: AgentKind, platform: Option<Platform>) -> Self {
        match kind {
            AgentKind::Codex => match platform {
                Some(Platform::Linux) | Some(Platform::MacOS) => Self {
                    reads: Enforcement::None,
                    writes: Enforcement::Os,
                    shell: Enforcement::Os,
                },
                Some(Platform::Windows) | None => Self::UNFENCED,
            },
            // `shell` is the dimension no gate in this tree can reach, for
            // the reason and with the settling capture recorded beside the
            // declaration in `adapters/agent/opencode/mod.rs`.
            AgentKind::Opencode => Self {
                reads: Enforcement::Harness,
                writes: Enforcement::Harness,
                shell: Enforcement::HarnessPartial,
            },
            AgentKind::ClaudeCode | AgentKind::Hermes | AgentKind::Pi => Self::UNFENCED,
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/models/sandbox.rs"]
mod tests;

//! Which of the desktop's own environment variables an agent may inherit,
//! decided by the OS the agent lands on.
//!
//! Forwarding is not neutral. A coding agent reads its environment as evidence
//! about the machine it is on, and `SHELL=/usr/bin/bash` is the loudest
//! available claim that the machine speaks POSIX. Demeteo's desktop is
//! routinely started from a Git Bash terminal on Windows, which exports exactly
//! that — so the orchestrator was handing a Windows agent a POSIX identity and
//! then being surprised when the agent reached for POSIX tooling. Nothing in
//! the harness was wrong; the block it was spawned with was.
//!
//! The desktop and the machine the agent lands on are two different questions
//! and both are asked: the values are the desktop's, and a Windows desktop
//! driving a Linux remote is a supported topology.
//!
//! The rule lives here rather than in
//! [`agent_base_env`](crate::ports::agent_runtime::agent_base_env) because that
//! function is `async` and resolves identity over the execution port — see
//! `domain/mod.rs`. The injected reader is the other half of the same move: a
//! rule about `std::env` that could only be tested by mutating `std::env` is a
//! rule nothing tests, because that mutation is process-global and races every
//! other test in the binary.

use crate::domain::models::Platform;

/// Forwarded verbatim between two POSIX platforms and in no other case.
///
/// Both are per-*platform*, never per-machine, which is what makes it sound to
/// take them from the desktop's own environment at all: a Linux desktop and a
/// Linux remote agree on what `SHELL` means even when they disagree on its
/// value, and either way the value is the user's stated preference. Windows
/// defines neither — it names its temp directory `TEMP`/`TMP` and has no
/// `SHELL` at all — so an inherited pair there contributes two false claims
/// and one path that resolves to nothing.
const POSIX_ONLY: [&str; 2] = ["SHELL", "TMPDIR"];

/// The desktop-inherited environment an agent running on `target` may see.
///
/// Both platforms are asked because the *values* come from the desktop and the
/// *agent* is on the target, so either half being non-POSIX makes the pair a
/// false claim — pointed one way, a Git Bash desktop tells a Windows agent it
/// has `/usr/bin/bash`; pointed the other, that same desktop tells a Linux
/// remote it has a `$TMPDIR` on a drive letter. The agreement above is between
/// two POSIX platforms, and this is where it is enforced rather than assumed.
///
/// `None` is treated as "not POSIX", not as "probably Linux": it means the
/// execution port declined to name the machine, and
/// [`AgentContext::platform`](crate::ports::agent_runtime::AgentContext::platform)
/// already fixes what an adapter does with that — emit nothing rather than
/// guess. Guessing POSIX here would restore the exact leak on precisely the
/// hosts least able to answer for themselves.
pub fn inherited_agent_env(
    desktop: Option<Platform>,
    target: Option<Platform>,
    read: impl Fn(&str) -> Option<String>,
) -> Vec<(String, String)> {
    let posix = |platform: Option<Platform>| platform.is_some_and(Platform::is_posix);
    if !posix(desktop) || !posix(target) {
        return Vec::new();
    }
    POSIX_ONLY
        .iter()
        .filter_map(|name| read(name).map(|value| ((*name).to_string(), value)))
        .collect()
}

#[cfg(test)]
#[path = "../../tests/domain/agent_env.rs"]
mod tests;

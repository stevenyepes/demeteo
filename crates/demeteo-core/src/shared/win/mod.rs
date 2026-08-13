//! Windows-specific host facts, resolved once and shared.
//!
//! Only `discovery.rs` and `dacl_sys.rs` are `#[cfg(windows)]` — the two files
//! that hold nothing but calls. Everything else here compiles and
//! is unit-tested on every platform on purpose: no Windows cross-compiler runs
//! on the development host, so a decision reachable only from a Windows build
//! is a decision whose first observation costs a CI round trip. The rule is
//! AGENTS.md §3's — a policy decision does not live inside an I/O path — with
//! an extra edge here, because the usual escape (run it locally and see) is
//! unavailable.

pub mod dacl;
pub mod exe;
pub mod npm_shim;
pub mod posix_shell;

#[cfg(windows)]
pub mod dacl_sys;
#[cfg(windows)]
pub mod discovery;

use std::path::PathBuf;

/// Parse a Windows path from any of the sources that hand one over, all of
/// which spell it differently. Backslashes become forward slashes because
/// Windows accepts either everywhere and `\` is not a legal filename
/// character, so the rewrite is lossless — and it leaves one [`std::path::Path`]
/// implementation that behaves the same on the Linux host these functions are
/// tested on.
fn win_path(raw: &str) -> PathBuf {
    PathBuf::from(raw.trim().trim_matches('"').replace('\\', "/"))
}

/// The absolute `bash.exe` a prompt can tell an agent to invoke, when this
/// process is one that can resolve it.
///
/// The process-local resolver answers for the *host*, which is the right answer
/// here for a reason outside this module: a remote resolves to Linux or macOS
/// and nothing else (`Platform::from_uname` in `adapters/ssh/platform.rs`), so
/// a Windows worktree is always this machine's. If a remote ever learns to be
/// Windows, this is wrong and silently so — it would hand the agent a path off
/// the wrong filesystem, and the prompt has no way to notice.
///
/// `None` when there is no such path to quote: off Windows, and on a Windows
/// box where resolution failed. A caller must degrade rather than substitute a
/// literal — the whole point of the resolver is that the install location is
/// not guessable.
pub fn quotable_bash_path() -> Option<String> {
    #[cfg(windows)]
    {
        posix_shell::posix_shell().ok().map(quotable_path)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// How a resolved shell is spelled into a prompt.
///
/// Split out of [`quotable_bash_path`] so the `cfg(windows)` arm above is one
/// call and nothing else. This is the part with a decision in it, and behind the
/// `cfg` it would be unreachable from any test on a non-Windows host — where,
/// per this module's own header, no Windows compiler runs either.
#[cfg_attr(not(windows), allow(dead_code))]
fn quotable_path(shell: &posix_shell::PosixShell) -> String {
    shell.bash.display().to_string()
}

#[cfg(test)]
#[path = "../../../tests/shared/win/mod_paths.rs"]
mod tests;

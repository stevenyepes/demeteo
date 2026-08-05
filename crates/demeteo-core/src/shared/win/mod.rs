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

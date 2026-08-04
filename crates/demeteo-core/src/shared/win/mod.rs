//! Windows-specific host facts, resolved once and shared.
//!
//! Only `discovery.rs` is `#[cfg(windows)]`. Everything else here compiles and
//! is unit-tested on every platform on purpose: no Windows cross-compiler runs
//! on the development host, so a decision reachable only from a Windows build
//! is a decision whose first observation costs a CI round trip. The rule is
//! AGENTS.md §3's — a policy decision does not live inside an I/O path — with
//! an extra edge here, because the usual escape (run it locally and see) is
//! unavailable.

pub mod posix_shell;

#[cfg(windows)]
pub mod discovery;

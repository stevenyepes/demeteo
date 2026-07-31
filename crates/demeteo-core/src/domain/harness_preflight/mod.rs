//! What a project's configured commands are, what a probe of them establishes,
//! and how that answer is attributed back to the settings that produced it
//! (HB1, HB4, HB6).
//!
//! Every decision here is pure. The probing itself — two `async fn`s spawning
//! `command -v` under an interactive login shell — stays in
//! `adapters::step_executor::preflight`, which is where the ceiling, the launch
//! phase and the settings entry point live.
//!
//! # What "the configured commands" means (HB4)
//!
//! Every command the project configured, not just `test_command`:
//! `prepare_command`, `test_command`, and every value in the `harnesses` map. A
//! `prepare_command` naming a missing binary is as fatal as a `test_command`
//! doing so, and a step selecting a named harness runs *that* string — probing
//! only `test_command` would check something the run never executes. Probing is
//! `command -v` either way, so covering all of them costs nothing extra beyond
//! the probes themselves.
//!
//! # The bias, stated once
//!
//! **A false negative is cheap; a false positive blocks a legitimate launch.**
//! Everything below is built around that asymmetry: anything this module cannot
//! confidently resolve to a plain binary name is skipped rather than guessed at.
//! Missing a broken command means the user lands in today's behaviour. Wrongly
//! flagging a working one means they cannot start work at all.

pub mod commands;
pub mod report;
pub mod verdict;

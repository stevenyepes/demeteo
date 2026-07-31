//! The `command` node's policy, with no I/O in it.
//!
//! A command node's hard parts are two decisions, and neither of them needs a
//! process: whether the authored config describes something runnable at all,
//! and what the run's `Result` *meant* — a verdict the harness delivered, or a
//! machine that never delivered one. Both used to be spelled inside
//! `adapters/step_executor/steps/command.rs`, in the middle of an `async fn`
//! that was also provisioning worktrees and storing artifacts, so nothing
//! outside the Docker conformance suite could assert that a transport failure
//! is `Environmental` rather than a verdict.
//!
//! Everything here is synchronous and total: it takes what the adapter
//! observed and returns what should happen. `domain/` has no `async fn`
//! anywhere in it, which is what keeps that boundary honest.
//!
//! The adapter keeps the choreography — provisioning, running, storing,
//! emitting — and owns the mapping from [`outcome::CommandRun`] onto the
//! executor's own `StepOutcome`.

pub(crate) mod outcome;
pub(crate) mod spec;

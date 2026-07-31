//! The `sequence` step's policy, with no I/O in it.
//!
//! A `sequence` step's hard parts are decisions, not calls: which tasks a
//! resumed attempt may skip, what a rollback owes the checkpoint, whether a
//! task list is executable at all. Those decisions used to live inside
//! `adapters/step_executor/steps/sequence`, spelled out in the middle of
//! `async fn`s that were also provisioning worktrees and running git — so
//! reaching them from a test meant standing up an `ExecutionDriver` and its
//! twenty-odd port doubles, and most of them were never reached at all.
//!
//! This module is where they go instead — synchronous and total, per the
//! [`domain`](crate::domain) rule, which reduces the adapter's job to
//! observing and obeying.
//!
//! The adapter side keeps the choreography: provisioning, probing, merging,
//! persisting, emitting.

pub mod checkpoint;
pub mod outcome;
pub mod progress;
pub mod sha;
pub mod tasks;

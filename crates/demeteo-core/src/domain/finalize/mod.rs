//! The finalize step's policy, with no I/O in it.
//!
//! Finalize is mostly choreography — run git reads, ask an agent, validate
//! against the repo's own hook, squash, record — but three things it does are
//! decisions: what counts as a usable answer from the agent, what to write
//! when there was none, and which commits on the branch actually describe the
//! work. All three used to be spelled inside `async fn`s in
//! `adapters/step_executor/steps/finalize`, one of them 250 lines long, which
//! is why the ⚠️ hook-bypass warning the PR body carries had never been
//! asserted anywhere.
//!
//! Everything here is synchronous and total: it takes what the adapter
//! observed and returns what should happen. `domain/` has no `async fn`
//! anywhere in it, which is what keeps that boundary honest.
//!
//! The adapter keeps the choreography: the git reads, the agent turn, the
//! hook validation, the squash and the row write.

pub(crate) mod authored;
pub(crate) mod commit_log;

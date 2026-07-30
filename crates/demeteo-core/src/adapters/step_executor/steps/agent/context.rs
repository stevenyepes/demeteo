//! The parameter bundles the agent step's stages pass among themselves.
//!
//! `handle_agent_step` threaded nine parameters, and
//! `process_agent_artifacts` seven, which is what put
//! `#[allow(clippy::too_many_arguments)]` on both. `AGENTS.md` §3 calls that
//! attribute a review trigger rather than a fix, and the fix is to notice
//! that those values never travel *individually* — they travel in groups no
//! call site has ever split:
//!
//! * [`AgentStepCtx`] — where the step sits in its feature's plan. Immutable
//!   for the whole run. Nothing has ever wanted `step_execs` without
//!   `step_index`, because reading an upstream step's artifact needs both to
//!   know which steps are upstream.
//! * [`AgentSpend`] — the feature-wide totals plus the two cache slots, all
//!   `&mut` because they are *not* step-scoped: the driver carries them
//!   across every step, so a stage that spends has to report it here or the
//!   step's cost is wrong. `start` rides along because every reader of the
//!   totals reports wall-clock beside them.
//! * [`AgentRunTarget`] — the agent's launch identity. Derived once at the
//!   top of the step through the five-tier precedence chain and never
//!   re-derived: the spawn, both `stream_agent_turn` calls and the
//!   merge-conflict pass are all the *same* agent, and a stage that resolved
//!   its own would be able to disagree with the session it is resuming.
//! * [`AgentWorktree`] — the ephemeral subtask worktree this step runs in.
//!   Its id and its path are two names for one thing (the id derives the
//!   branch and the directory name, the path is where git and the agent
//!   run), and the machine is the third: addressing the path without saying
//!   which host it is on is how a remote worktree gets written to locally.
//!   Eleven call sites take all three; none takes a subset.
//! * [`TurnBaseline`] — what the turn's output is measured against. Only
//!   knowable once the worktree exists, which is why it is separate from
//!   [`AgentWorktree`] rather than a field of it.
//!
//! All of them are borrows or `Copy` scalars, so passing one costs nothing
//! and none owns anything a stage could mutate behind the caller's back —
//! except [`AgentSpend`], which is the point of it.

use std::time::Instant;

use crate::adapters::step_executor::artifacts::WorktreeSnapshot;
use crate::domain::models::{EffortLevel, StepConfig, StepExecution};

/// Where this step sits in its feature's plan.
#[derive(Clone, Copy)]
pub(crate) struct AgentStepCtx<'a> {
    /// The persisted execution row for this step.
    pub step_exec: &'a StepExecution,
    /// The step's definition.
    pub step_conf: &'a StepConfig,
    /// Index of this step in the ordered plan.
    pub step_index: usize,
    /// Every step-execution row for the feature, in plan order.
    pub step_execs: &'a [StepExecution],
}

/// The feature-wide spend totals, plus when this step started.
///
/// `cost` and `tokens` are `&mut` all the way down because they are not the
/// step's: the driver accumulates them across every step of the feature.
///
/// `cache_read` and `cache_creation` are **overwritten, not accumulated**,
/// and they are written from two different places with two different
/// meanings: the main agent turn reports its own cache counts, and the
/// merge-conflict pass then replaces them because conflict resolution is
/// always an agent step's last turn, so its counts are the ones the UI's
/// cache chip should show. Neither slot is write-once — a stage that
/// treated them that way would pin the chip to the first turn of a step
/// that went on to run two more.
pub(crate) struct AgentSpend<'a> {
    pub cost: &'a mut f64,
    pub tokens: &'a mut i64,
    /// When the driver started this step — the wall-clock every progress
    /// event and every terminal row reports beside the totals above.
    pub start: Instant,
    pub cache_read: &'a mut Option<u64>,
    pub cache_creation: &'a mut Option<u64>,
}

/// The agent's launch identity, resolved once for the whole step.
///
/// Resolved at the top of `handle_agent_step` so the spawn, the main turn,
/// the strict-JSON verdict correction and the merge-conflict pass all
/// address the same agent. `session_key` belongs here rather than being
/// re-derived per stage: it is a fingerprint over exactly the other three
/// fields plus the step's permission profile, so a stage that recomputed it
/// from a locally-resolved model could silently address a different
/// session than the one it is talking to.
#[derive(Clone, Copy)]
pub(crate) struct AgentRunTarget<'a> {
    /// The harness (`opencode`, `claude-code`, …).
    pub agent_kind: &'a str,
    /// The model override, extended to the runtime default so the pricing
    /// table can compute a cost when the wire format omits one.
    pub override_model: Option<&'a str>,
    /// The reasoning effort this step's turns inherit.
    pub effort: EffortLevel,
    /// The registry key this step's session lives under.
    pub session_key: &'a str,
}

/// The ephemeral subtask worktree one agent step runs in.
///
/// The three fields are not interchangeable and are never useful apart:
/// `git_ops` addresses the worktree by `subtask_id` (it derives both the
/// branch and the directory name from it), every git command and the agent
/// itself run against `path`, and `machine` says which host either of those
/// is true on.
#[derive(Clone, Copy)]
pub(crate) struct AgentWorktree<'a> {
    pub machine: &'a str,
    pub subtask_id: &'a str,
    pub path: &'a str,
}

/// What this step's output is measured against.
///
/// `snapshot` is the file listing taken before the agent ran, so the delta
/// is what the agent wrote; `base_ref` is the feature branch's tip at the
/// same moment, so the no-op guard can ask whether anything was committed.
/// Both are only knowable once the worktree exists, which is why they are
/// not fields of [`AgentWorktree`].
#[derive(Clone, Copy)]
pub(crate) struct TurnBaseline<'a> {
    pub snapshot: &'a WorktreeSnapshot,
    /// `None` when `rev-parse` could not resolve the branch — the diff
    /// falls back to `HEAD` and the no-op guard to the worktree's own tip.
    pub base_ref: Option<&'a str>,
}

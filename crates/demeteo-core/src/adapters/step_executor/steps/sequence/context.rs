//! The parameter bundles the stages pass among themselves.
//!
//! Every stage of a `sequence` step needs some subset of the same dozen
//! values, and threading them one-by-one is what put
//! `#[allow(clippy::too_many_arguments)]` on eight functions in this module.
//! The allow silenced the symptom; `AGENTS.md` §3 calls it a review trigger
//! rather than a fix, and the fix is to notice that those values do not
//! travel *individually* — they travel in four groups that no call site ever
//! splits:
//!
//! * [`StepCtx`] — where the step sits in its feature's plan. Immutable for
//!   the whole run, and always passed whole; no stage has ever wanted
//!   `step_execs` without `step_index`, because reading an upstream step's
//!   artifacts needs both.
//! * [`RunTarget`] — who runs the agent turns and where. Resolved once in
//!   `handle_sequence_step` and never re-derived; the three stages that spawn
//!   an agent (planner, task, merge-conflict) each need all four fields.
//! * [`StepSpend`] — the feature-wide totals, which are `&mut` because they
//!   are *not* step-scoped: the driver carries them across every step, and a
//!   stage that spends has to report it here or the step's cost is wrong.
//!   `start` rides along because every reader of the totals is also
//!   reporting wall-clock beside them.
//! * [`StepWorktree`] — the one worktree the whole list runs in. Its id and
//!   its path are two names for the same thing (the id derives the branch,
//!   the path is where git runs), and passing one without the other is
//!   always a mistake.
//! * [`TaskRun`] — one task's position in the list. Only the innermost two
//!   stages see it, and it is the bundle that keeps `run_one_task` from
//!   needing seventeen arguments.
//!
//! All of them are borrows or `Copy` scalars, so the bundles cost nothing to
//! pass and none of them owns anything a stage could mutate behind the
//! caller's back — except [`StepSpend`], which is the point of it.

use std::time::Instant;

use crate::domain::models::{EffortLevel, StepConfig, StepExecution};
use crate::domain::sequence::tasks::{PlanKind, PlannedTask};

use super::prompt::CompletedTask;

/// Where this step sits in its feature's plan.
#[derive(Clone, Copy)]
pub(crate) struct StepCtx<'a> {
    /// The persisted execution row for this step.
    pub step_exec: &'a StepExecution,
    /// The step's definition.
    pub step_conf: &'a StepConfig,
    /// Index of this step in the ordered plan.
    pub step_index: usize,
    /// Every step-execution row for the feature, in plan order.
    pub step_execs: &'a [StepExecution],
}

impl StepCtx<'_> {
    /// The workflow-level step id — what every log line and checkpoint row
    /// in this module keys on.
    pub(crate) fn step_id(&self) -> &str {
        &self.step_exec.step_id.0
    }
}

/// Who runs this step's agent turns, and where.
///
/// Resolved once, at the top of `handle_sequence_step`, so every session the
/// step opens — planner, task, merge-conflict — is spawned against the same
/// answer. `effort` is part of it rather than re-derived per stage: it comes
/// from the same precedence chain as `agent_kind`, and three call sites used
/// to spell `self.resolve_step_effort(step_conf)` separately.
#[derive(Clone, Copy)]
pub(crate) struct RunTarget<'a> {
    /// The machine every command and every agent runs on.
    pub machine: &'a str,
    /// The harness (`opencode`, `claude-code`, …).
    pub agent_kind: &'a str,
    /// The model override, when the step or feature declares one.
    pub override_model: Option<&'a str>,
    /// The reasoning effort a task turn inherits.
    pub effort: EffortLevel,
}

/// The feature-wide spend totals, plus when this step started.
///
/// `cost` and `tokens` are `&mut` all the way down because they are not the
/// step's: the driver accumulates them across every step of the feature, and
/// a task's own spend is only knowable as the difference across its run.
pub(crate) struct StepSpend<'a> {
    pub cost: &'a mut f64,
    pub tokens: &'a mut i64,
    /// When the driver started this step — the wall-clock every progress
    /// event and every terminal row reports beside the totals above.
    pub start: Instant,
}

/// The one worktree the whole task list runs in.
///
/// The id and the path are not interchangeable and are never useful apart:
/// `git_ops` addresses the worktree by id (it derives the step branch from
/// it), while every git command runs against the path.
#[derive(Clone, Copy)]
pub(crate) struct StepWorktree<'a> {
    pub id: &'a str,
    pub path: &'a str,
}

/// One task's place in the list, as its own stage sees it.
///
/// A fresh session knows nothing, so most of this exists to be spelled into
/// the prompt: which task, how far through, what already landed, and whether
/// the tree it opens is pristine.
#[derive(Clone, Copy)]
pub(crate) struct TaskRun<'a> {
    pub task: &'a PlannedTask,
    /// Zero-based; the prompt renders `index + 1`.
    pub index: usize,
    /// How many tasks this attempt is running — not how many the plan had
    /// before the checkpoint dropped the landed ones.
    pub total: usize,
    /// What the agent is told is already on the branch: this attempt's
    /// finished tasks, seeded with the ones it is skipping.
    pub completed: &'a [CompletedTask],
    /// Whether the worktree was cut from a branch that already carries a
    /// previous attempt's work. Saying "this is the first task" when it does
    /// sends the agent to reimplement code it is looking at.
    pub resumes_landed_work: bool,
    /// Greenfield list, or a delta closing a downstream verdict.
    ///
    /// Both run against a branch that already carries work, so both need
    /// `resumes_landed_work` — but what the agent should *do* about it is
    /// opposite. A re-run task is looking at an earlier version of itself
    /// and must revise it in place; a rework task is new work whose earlier
    /// version does not exist, and telling it to "revise in place" sends it
    /// hunting for code nobody wrote.
    pub plan_kind: PlanKind,
    /// This task's session key — unique per (feature, step, task), so the
    /// runtime can never hand back the previous task's conversation.
    pub thread_id: &'a str,
}

//! Read model for a `sequence` node's task list (task P2.5, PRD §6.2).
//!
//! A sequence step runs an ordered task list, each task in a fresh agent
//! session but the *same* worktree, committing before the next starts. The
//! "landed prefix" — tasks whose commit is already on the feature branch — is
//! the load-bearing idea of Decision 13: on a crash or targeted retry only the
//! remainder re-runs. Nothing surfaced that split to the user, so the panel
//! drill-down (P2.5) renders it, joining three durable sources the engine
//! already writes:
//!
//!  - `sequence_plan_cache` (V32) — the full ordered task plan (id + title).
//!  - `sequence_checkpoints` (V32) — the landed task ids, the committed prefix.
//!  - `subtask_runs` (V4) — per-task status / cost / tokens / error.
//!
//! Assembled in [`RunView::sequence_state`](crate::application::run_view). The
//! join is per (feature, node) for plan/checkpoint and per step-execution for
//! the subtask rows, because a feature may hold several sequence nodes.

use serde::{Deserialize, Serialize};

/// What the landed tasks emitted, carried on the checkpoint so an attempt
/// that runs none of them can still answer the two questions the task loop
/// would otherwise have answered in memory: *was the declared deliverable
/// produced*, and *what does this step hand downstream*.
///
/// Scoped to the tasks the checkpoint names, which is what separates it
/// from re-reading the artifact store: the store is keyed by (feature,
/// step) and would also return an earlier, rolled-back attempt's files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointProduced {
    /// Artifact-store references the landed tasks wrote, in landed order.
    pub artifact_refs: Vec<String>,
    /// Names of the `StepConfig::artifacts` declarations those tasks
    /// satisfied. Judged across the whole list, never per task — only one
    /// task in a sequence may be the one that writes the report.
    pub satisfied_decls: Vec<String>,
}

/// A sequence step's durable resume point: which tasks are done, where
/// their work ends, and what it produced.
///
/// The fields answer different questions and only the first is always
/// available. `landed_task_ids` says *what not to re-run*. `anchor_sha`
/// says *where that work is*, and is `None` for a checkpoint written
/// before V35 or one whose `rev-parse` failed — in which case the resume
/// falls back to V32's assumption that the prefix is already merged to the
/// feature branch, which is what the only V32 writer guaranteed.
/// `produced` (V36) says *what came out of it*, and is `None` for a row
/// written before V36 — which means **unknown**, not "produced nothing".
///
/// Note that an anchor does **not** by itself mean "unmerged": the graceful
/// mid-list failure path records both, having merged the prefix. Only a
/// git query at resume time can tell the two apart — see
/// `resolve_checkpoint_resume`, which asks it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceCheckpoint {
    /// Task ids already done, in landed order. Empty when the step never
    /// checkpointed (or completed and cleared).
    pub landed_task_ids: Vec<String>,
    /// The commit the landed prefix ends at, pinned by
    /// `refs/demeteo/seq/<feature>/<step>` so `git gc` cannot reclaim it.
    pub anchor_sha: Option<String>,
    /// What those tasks emitted, or `None` for a pre-V36 row.
    pub produced: Option<CheckpointProduced>,
}

impl SequenceCheckpoint {
    /// Nothing to resume from.
    pub fn is_empty(&self) -> bool {
        self.landed_task_ids.is_empty()
    }
}

/// One `subtask_runs` row, projected to the fields the task drill-down reads.
/// Keyed to a task by [`subtask_id`](Self::subtask_id) (== `PlannedTask::id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskRunRow {
    pub subtask_id: String,
    /// `pending | running | completed | failed | skipped | interrupted`.
    pub status: String,
    /// This task's own spend (not the step's running total).
    pub cost_usd: f64,
    pub tokens: i64,
    pub error_message: Option<String>,
}

/// Full-fidelity `subtask_runs` row for mirroring a detached run's per-task
/// telemetry onto the laptop in one write (the `get_sequence_state` runner
/// RPC and
/// [`FeatureRepository::subtask_runs_replace_for_step`](crate::ports::db::FeatureRepository::subtask_runs_replace_for_step)).
/// Unlike [`SubtaskRunRow`], which the drill-down reads, this carries every
/// column `subtask_run_start`/`subtask_run_finish` populate — the mirror
/// write replaces the whole row set for a step in one shot, so it needs
/// each row whole rather than the read-projected subset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskRunMirrorRow {
    pub id: String,
    pub subtask_id: String,
    pub agent_id: Option<String>,
    pub worktree_path: String,
    pub branch: String,
    /// `pending | running | completed | failed | skipped | interrupted`.
    pub status: String,
    pub cost_usd: f64,
    pub tokens: i64,
    pub error_message: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

/// Wire shape of the runner's `get_sequence_state` RPC (C4.1): one
/// `sequence` node's whole resume state, bundled into a single response so
/// the laptop's mirror write (`hydrate_shadow_feature`) is never torn
/// across polls — it either gets this node's whole state or none of it.
/// Shared by `demeteo-runner` (produces it) and `demeteo-core`
/// (`hydrate_shadow_feature` consumes it) rather than duplicated, the same
/// way [`Feature`](crate::domain::models::Feature) and
/// [`StepExecution`](crate::domain::models::StepExecution) back the other
/// C4 read RPCs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceStateMirror {
    pub plan_json: Option<String>,
    pub checkpoint: SequenceCheckpoint,
    pub subtask_runs: Vec<SubtaskRunMirrorRow>,
}

/// One task in a sequence node, merged from its plan entry and (if it has run)
/// its `subtask_runs` row, with the landed flag from the checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceTaskView {
    pub id: String,
    pub title: String,
    /// Derived per-task status: `landed | running | completed | failed |
    /// interrupted | skipped | pending`. `landed` wins when the task's commit
    /// is on the feature branch (the checkpoint), regardless of the subtask
    /// row — a landed task is done-and-committed, the Decision-13 prefix.
    pub status: String,
    /// True when the task id is in the checkpoint: its work is committed to the
    /// feature branch and a resume/retry will *not* re-run it.
    pub landed: bool,
    /// Which decomposition cycle planned this task: `0` for the original
    /// list, incrementing once per rework cycle.
    ///
    /// A rework cycle does not replace the decomposition it is a delta
    /// against — both are on the branch and both cost money — so the
    /// drill-down groups by this rather than showing whichever list ran
    /// last and silently dropping the other.
    #[serde(default)]
    pub cycle: u32,
    /// True for tasks planned by an earlier cycle. They are shown for
    /// context and are not part of what the current cycle runs.
    #[serde(default)]
    pub prior_cycle: bool,
    pub cost_usd: Option<f64>,
    pub tokens: Option<i64>,
    pub error_message: Option<String>,
}

/// A sequence node's whole task list for the drill-down accordion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceState {
    /// False when the node has not resolved a task plan yet (no plan-cache
    /// row) — distinct from a plan that legitimately holds zero tasks.
    pub planned: bool,
    pub tasks: Vec<SequenceTaskView>,
}

impl SequenceState {
    /// The empty, not-yet-planned state — what a sequence node reads before it
    /// has run, and what a non-sequence node would read (it never plans).
    pub fn unplanned() -> Self {
        Self {
            planned: false,
            tasks: Vec::new(),
        }
    }
}

/// One task as the plan cache records it, tagged with the cycle that planned
/// it. The input `assemble_tasks` merges the durable run state onto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedTaskRef {
    pub id: String,
    pub title: String,
    pub cycle: u32,
    /// Planned by a cycle earlier than the current one.
    pub prior_cycle: bool,
}

/// Merge the ordered plan with the landed-task set and the per-task run rows
/// into the drill-down view. Pure so it can be tested without a DB;
/// `RunView::sequence_state` supplies the three durable sources.
///
/// The status precedence is deliberate: **landed wins**. A task whose commit
/// is on the feature branch is done-and-committed (the Decision-13 prefix)
/// regardless of its `subtask_runs` row — a rev-parse hiccup can leave a
/// `completed` row uncheckpointed, but the checkpoint is the resume authority.
/// A task with no run row and no checkpoint is `pending`.
///
/// One exception, and it is what makes a multi-cycle view readable: a task
/// from a **prior cycle** reads `landed` whatever the checkpoint says. The
/// checkpoint is cleared the moment a step completes, so by the time a rework
/// cycle exists at all, the cycle it is a delta against has no checkpoint
/// left — yet its commits are demonstrably on the branch, because the rework
/// cycle is running against them. Reading those rows as `pending` would show
/// twenty-five never-started tickets beside four running ones.
pub fn assemble_tasks(
    plan: &[PlannedTaskRef],
    landed: &std::collections::HashSet<String>,
    runs: &std::collections::HashMap<String, SubtaskRunRow>,
) -> Vec<SequenceTaskView> {
    plan.iter()
        .map(|planned| {
            let is_landed = planned.prior_cycle || landed.contains(&planned.id);
            let run = runs.get(&planned.id);
            let status = if is_landed {
                "landed".to_string()
            } else {
                run.map(|r| r.status.clone())
                    .unwrap_or_else(|| "pending".to_string())
            };
            SequenceTaskView {
                id: planned.id.clone(),
                title: planned.title.clone(),
                status,
                landed: is_landed,
                cycle: planned.cycle,
                prior_cycle: planned.prior_cycle,
                cost_usd: run.map(|r| r.cost_usd),
                tokens: run.map(|r| r.tokens),
                error_message: run.and_then(|r| r.error_message.clone()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// A task planned by the current cycle.
    fn planned(id: &str, title: &str) -> PlannedTaskRef {
        PlannedTaskRef {
            id: id.into(),
            title: title.into(),
            cycle: 0,
            prior_cycle: false,
        }
    }

    fn run(id: &str, status: &str, cost: f64) -> SubtaskRunRow {
        SubtaskRunRow {
            subtask_id: id.into(),
            status: status.into(),
            cost_usd: cost,
            tokens: 10,
            error_message: None,
        }
    }

    #[test]
    fn landed_wins_over_run_row_and_pending_fills_the_rest() {
        let plan = vec![
            planned("t1", "First"),
            planned("t2", "Second"),
            planned("t3", "Third"),
        ];
        // t1 committed (landed) though its run row still says completed; t2 is
        // the live task; t3 never started.
        let landed: HashSet<String> = ["t1".to_string()].into_iter().collect();
        let runs: HashMap<String, SubtaskRunRow> = [
            ("t1".to_string(), run("t1", "completed", 0.5)),
            ("t2".to_string(), run("t2", "running", 0.2)),
        ]
        .into_iter()
        .collect();

        let out = assemble_tasks(&plan, &landed, &runs);
        assert_eq!(out.len(), 3);

        assert_eq!(out[0].status, "landed");
        assert!(out[0].landed);
        assert_eq!(out[0].cost_usd, Some(0.5));

        assert_eq!(out[1].status, "running");
        assert!(!out[1].landed);
        assert_eq!(out[1].cost_usd, Some(0.2));

        assert_eq!(out[2].status, "pending");
        assert!(!out[2].landed);
        assert_eq!(out[2].cost_usd, None);
        assert_eq!(out[2].title, "Third");
    }

    #[test]
    fn a_failed_uncheckpointed_task_keeps_its_failure() {
        let plan = vec![planned("t1", "Only")];
        let landed = HashSet::new();
        let mut row = run("t1", "failed", 0.9);
        row.error_message = Some("boom".into());
        let runs: HashMap<String, SubtaskRunRow> = [("t1".to_string(), row)].into_iter().collect();

        let out = assemble_tasks(&plan, &landed, &runs);
        assert_eq!(out[0].status, "failed");
        assert!(!out[0].landed);
        assert_eq!(out[0].error_message.as_deref(), Some("boom"));
    }

    /// A prior cycle's tasks are on the branch — the current cycle is
    /// running against them — but the checkpoint that named them was
    /// cleared when their step completed. Reading them off the checkpoint
    /// alone would render twenty-five finished tickets as never-started.
    #[test]
    fn a_prior_cycles_tasks_read_landed_without_a_checkpoint() {
        let plan = vec![
            PlannedTaskRef {
                id: "ticket-01".into(),
                title: "Original".into(),
                cycle: 0,
                prior_cycle: true,
            },
            planned("fix-1", "Rework"),
        ];
        // Deliberately empty: the step completed, so the checkpoint is gone.
        let landed = HashSet::new();
        let runs: HashMap<String, SubtaskRunRow> =
            [("fix-1".to_string(), run("fix-1", "running", 0.1))]
                .into_iter()
                .collect();

        let out = assemble_tasks(&plan, &landed, &runs);
        assert_eq!(out[0].status, "landed");
        assert!(out[0].landed);
        assert!(out[0].prior_cycle);
        assert_eq!(out[0].cycle, 0);

        assert_eq!(out[1].status, "running");
        assert!(!out[1].landed);
        assert!(!out[1].prior_cycle);
    }

    /// The cycle tag is what the accordion groups by, so it has to survive
    /// onto every row rather than being inferred from list position.
    #[test]
    fn every_row_carries_the_cycle_that_planned_it() {
        let plan = vec![
            PlannedTaskRef {
                id: "a".into(),
                title: "A".into(),
                cycle: 0,
                prior_cycle: true,
            },
            PlannedTaskRef {
                id: "b".into(),
                title: "B".into(),
                cycle: 1,
                prior_cycle: true,
            },
            PlannedTaskRef {
                id: "c".into(),
                title: "C".into(),
                cycle: 2,
                prior_cycle: false,
            },
        ];
        let out = assemble_tasks(&plan, &HashSet::new(), &HashMap::new());
        let cycles: Vec<u32> = out.iter().map(|t| t.cycle).collect();
        assert_eq!(cycles, [0, 1, 2]);
    }
}

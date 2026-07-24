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

/// Merge the ordered plan (`(id, title)` pairs) with the landed-task set and
/// the per-task run rows into the drill-down view. Pure so it can be tested
/// without a DB; `RunView::sequence_state` supplies the three durable sources.
///
/// The status precedence is deliberate: **landed wins**. A task whose commit
/// is on the feature branch is done-and-committed (the Decision-13 prefix)
/// regardless of its `subtask_runs` row — a rev-parse hiccup can leave a
/// `completed` row uncheckpointed, but the checkpoint is the resume authority.
/// A task with no run row and no checkpoint is `pending`.
pub fn assemble_tasks(
    plan: &[(String, String)],
    landed: &std::collections::HashSet<String>,
    runs: &std::collections::HashMap<String, SubtaskRunRow>,
) -> Vec<SequenceTaskView> {
    plan.iter()
        .map(|(id, title)| {
            let is_landed = landed.contains(id);
            let run = runs.get(id);
            let status = if is_landed {
                "landed".to_string()
            } else {
                run.map(|r| r.status.clone())
                    .unwrap_or_else(|| "pending".to_string())
            };
            SequenceTaskView {
                id: id.clone(),
                title: title.clone(),
                status,
                landed: is_landed,
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
            ("t1".to_string(), "First".to_string()),
            ("t2".to_string(), "Second".to_string()),
            ("t3".to_string(), "Third".to_string()),
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
        let plan = vec![("t1".to_string(), "Only".to_string())];
        let landed = HashSet::new();
        let mut row = run("t1", "failed", 0.9);
        row.error_message = Some("boom".into());
        let runs: HashMap<String, SubtaskRunRow> = [("t1".to_string(), row)].into_iter().collect();

        let out = assemble_tasks(&plan, &landed, &runs);
        assert_eq!(out[0].status, "failed");
        assert!(!out[0].landed);
        assert_eq!(out[0].error_message.as_deref(), Some("boom"));
    }
}

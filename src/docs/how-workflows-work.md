# How Workflows Work

A **Workflow** is a directed acyclic graph (DAG) of steps that defines how Demeteo processes a feature request.

## Step Types

Each step has a **kind** that determines how it executes:

### Agent Step
Runs a prompt against a coding agent (e.g. opencode, Claude). The agent receives context from previous steps and produces artifacts (code, docs, analysis).

### Sequence Step
Executes an ordered **task list**, one task at a time. Each task gets its own fresh agent session — so no single context window has to carry the whole feature — but they all share one worktree, and each task commits before the next begins. That means task N opens a worktree that already contains task N-1's work, so later tasks can build on earlier ones. When every task is done, the whole thing merges back to the feature branch in one go.

The task list normally comes from an earlier step, named in the step's `task_list_from` field: that step writes `artifacts/task-list.json`, which puts the decomposition in front of the human gate — you approve the task breakdown *before* any code is written. If `task_list_from` is unset, the step plans the work itself with a planner turn.

> **`parallel` is the old name for this step.** It used to run its subtasks *concurrently*, each on its own worktree, merging each back independently. That was removed: concurrent worktrees on a shared repo could delete each other, every subtask merge was another chance to conflict, and it forced the planner to pretend that work partitions into disjoint sets of files. Steps still saved with `kind: "parallel"` keep working — they now run sequentially.

### Gate Step
Pauses execution and requests human input. The user can:
- **Approve** — continue to the next step
- **Redirect** — send feedback to re-run a previous step
- **Cancel** — stop the entire feature pipeline

## Conditional Edges

Steps can define a `goto` target in their `on_failure` configuration. If a step fails, execution redirects to the specified step instead of halting. This enables retry loops and error-recovery flows.

## Iteration Budget

Each step has a `max_iterations` setting. If a step exceeds its iteration budget, it transitions to `failed` with a `budget_exhausted` error. This prevents runaway retry loops.

## Built-in Workflows

Demeteo ships with starter workflows:

- **Standard Feature Pipeline**: Research → Spec → Plan → Tasks → Implement → Validate
- **Bug Fix**: Reproduce → Diagnose → Fix → Verify
- **Refactor**: Analyze → Plan → Execute → Validate
- **Documentation**: Audit → Draft → Review → Publish

You can customize any workflow in the **Workflow Editor** or create new ones from scratch.

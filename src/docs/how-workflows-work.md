# How Workflows Work

A **Workflow** is a directed acyclic graph (DAG) of steps that defines how Demeteo processes a feature request.

## Step Types

Each step has a **kind** that determines how it executes:

### Agent Step
Runs a prompt against a coding agent (e.g. opencode, Claude). The agent receives context from previous steps and produces artifacts (code, docs, analysis).

### Parallel Step
Spawns multiple sub-tasks concurrently. A planner agent decomposes the objective into sub-tasks, each executed by a separate agent session in parallel.

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

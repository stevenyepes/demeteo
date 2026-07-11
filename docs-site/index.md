# Demeteo

**Fleet-style multi-agent orchestrator.** Describe a feature in plain language; Demeteo decomposes it into a workflow, delegates steps to coding agents, manages Git worktrees per step, and gates merges behind human approval.

[Get started](getting-started.md){ .md-button }
[View on GitHub](https://github.com/stevenyepes/demeteo){ .md-button .md-button--primary }

## Why Demeteo

Most AI coding tools help you write code one session at a time. Demeteo plans the work as a directed acyclic graph of steps, runs multiple agents in parallel worktrees, and keeps you in control of what gets merged, and when.

## Status

**V1 — core fleet-style multi-agent orchestrator** (fully implemented).

See the [project repository](https://github.com/stevenyepes/demeteo) for build instructions, contribution guidelines, and architectural notes.

## Core concepts

| Term      | What it is                                                                            |
|-----------|----------------------------------------------------------------------------------------|
| Project   | A local or remote Git repository tracked by Demeteo.                                   |
| Feature   | A user-described piece of work that gets decomposed into a workflow.                   |
| Workflow  | A reusable, versioned DAG of steps.                                                    |
| Step      | One node in the DAG: an `agent`, `parallel`, or `gate`.                                |
| Gate      | A human-approval checkpoint before the orchestrator continues.                         |
| Subtask   | Work assigned to one agent in one worktree.                                            |

For the full domain glossary, see the [architecture documentation](https://github.com/stevenyepes/demeteo/tree/master/docs) in the repository.

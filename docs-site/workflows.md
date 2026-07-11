# Workflows

A **Workflow** is a reusable, versioned DAG of Steps. Demeteo ships seven **starter workflows** — every new workspace can run any of them, and you can edit copies in **Project Settings** for your own conventions.

The workflow catalog lives at `src-tauri/workflows/*.json` and is compiled into the binary; see [`docs/ARCHITECTURE.md`](https://github.com/stevenyepes/demeteo/blob/master/docs/ARCHITECTURE.md) for the schema.

## The starters

| Workflow | When to use it |
|----------|---------------|
| **Standard Feature Pipeline** | Default. Moderate-to-high complexity features: research, spec, gated implementation, validation, critic, gated merge. |
| **Bugfix Pipeline** | Isolated, well-scoped bugs. Reproduce → confirm root cause → fix → smoke test → regression → review. |
| **CI Fix Pipeline** | Red CI from test failures, lint errors, or build regressions. Diagnose → confirm → fix → verify → review. |
| **Documentation Update** | API docs, READMEs, guides, inline comments. Survey → scope → draft → validate links → review → polish. |
| **Experiment & Prototype** | Spikes and proof-of-concept work. Hypothesis → approve → build → evaluate → decide keep-or-discard → harden/archive. |
| **Refactor Pipeline** | Behaviour-preserving refactors. Baseline → plan → approve → parallel execute → regression → public-API drift review → review diff. |
| **Simple Task Pipeline** | Well-scoped, straightforward tasks. Plan → implement → validate. No gates. |

## Anatomy of a step

Every step declares:

- A **kind**: `agent` (run a coding agent in a worktree), `parallel` (fan out subtasks across multiple agents), or `gate` (pause for human approval).
- A **prompt template** — the instructions the orchestrator renders for the agent, with placeholders like `{{feature_description}}`, `{{project_conventions}}`, and step-artifact references.
- An **artifact contract** — files the agent must produce (e.g. `artifacts/research-report.md`). The orchestrator captures them and attaches them to the next step's prompt.
- **Failure routing** — `on_failure: <step-id>` declares which step to bounce to when this one fails.
- **Max iterations** — how many times the orchestrator will retry before giving up.

`gate` steps have no prompt template or artifact contract. They are a structured pause that surfaces three actions to the human: **Approve**, **Redirect** (with feedback that re-enters the prior step), or **Cancel**.

## The Standard Feature Pipeline

The default workflow — and the one in the screenshot below — has seven steps:

![The Standard Feature Pipeline running](assets/screenshots/feature-pipeline.png)

| # | Step | Kind | What it does |
|---|------|------|--------------|
| 1 | **Research Codebase** | `agent` | A senior-architect pass: every file likely to change, the patterns the implementation must follow, a risk register, and any external dependencies. Output: `artifacts/research-report.md`. |
| 2 | **Draft Implementation Spec** | `agent` | Turns the research report into a binding spec — acceptance criteria, data model changes, files to modify, public API changes, testing strategy, constraints, open questions. Output: `artifacts/implementation-spec.md`. |
| 3 | **Review Spec Before Implementation** | `gate` | **You** read the spec and either approve (implementation starts), redirect (your feedback re-enters step 2), or cancel. |
| 4 | **Implement Subtasks** | `parallel` | The spec is sliced into subtasks; each runs in its own worktree with a strict ownership list. Worker agents commit `feat(<slug>): <what>` messages. |
| 5 | **Validate, Test & Security Scan** | `agent` | A QA pass that interprets the project's test harness output (already executed by the orchestrator), checks each acceptance criterion, scans for hardcoded secrets / TODOs / unhandled errors, and emits an overall **READY TO SHIP / BLOCKED** verdict. |
| 6 | **Critic Review** | `agent` | An adversarial review across correctness, spec compliance, security, performance, test coverage, and code quality. Emits **Critical / Major / Minor** issues and a final **PASS / PASS_WITH_NOTES / FAIL** verdict. |
| 7 | **Approve Merge / Publish** | `gate` *(dangerous)* | **You** review the validation and critic reports and approve to merge and open an MR, redirect (re-opens a prior step with feedback), or cancel. Marked *dangerous* because this step pushes to your remote. |

The orchestrator handles all transitions: when step 1 finishes, it renders step 2's prompt with the research artifact attached; when step 4 finishes and the spec is approved, it fans out subtasks and waits for all of them; and so on. You intervene only at the two `gate` steps.

## Running a non-default workflow

When you click **Launch feature**, the workflow dropdown in the Start a feature modal pre-selects your workspace's default (Standard for new workspaces). Switch to any other starter per-feature — for example, *Bugfix Pipeline* for an isolated regression, *Refactor Pipeline* for a behaviour-preserving change, *Simple Task* for a one-line fix.
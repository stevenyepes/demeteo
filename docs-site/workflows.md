# Workflows

A **Workflow** is a reusable, versioned DAG of Steps. Demeteo ships nine **starter workflows** — every project can run any of them — and you can edit them, or build your own, on the canvas in the **Workflow Library** (*Workflows* in the top bar).

![Workflow Library listing the nine starter pipelines, with the Standard Feature Pipeline selected: its description, shape and step configuration](assets/screenshots/workflow-library.png)

The starters live in `src-tauri/workflows/*.json` and are compiled into the binary; see [`docs/PRD_DAG_WORKFLOWS.md`](https://github.com/stevenyepes/demeteo/blob/master/docs/PRD_DAG_WORKFLOWS.md) for the model.

## The starters

| Workflow | When to use it | Shape |
|----------|----------------|-------|
| **Standard Feature Pipeline** | New features of moderate-to-high complexity. | Baseline → research → spec → tickets → **gate** → implement tickets → validate → critic → **ship gate** → open PR. |
| **Bugfix Pipeline** | Isolated, well-scoped bugs. | Baseline → reproduce & diagnose → **confirm root cause** → decompose the fix → apply fix → full regression check → **ship gate** → open PR. |
| **CI Fix Pipeline** | Red CI from test failures, lint errors, or build regressions. | Baseline → diagnose from the CI log → **confirm diagnosis** → apply fix → verify locally → **ship gate** → open PR. |
| **Documentation Update** | API docs, READMEs, guides, inline comments. | Baseline → survey docs → **approve scope** → draft → validate links & code examples → **review draft** → polish → open PR. |
| **Experiment & Prototype** | Spikes and proof-of-concept work. | Hypothesis → **approve hypothesis** → build prototype → evaluate → **commit or discard?** → harden or archive → open PR. |
| **Refactor Pipeline** | Behaviour-preserving refactors. | Baseline → analyse & plan → **approve plan** → refactor tasks → regression check → public-API drift review → **review diff** → open PR. |
| **Simple Task Pipeline** | Well-scoped, straightforward tasks. | Baseline → plan → implement → validate → open PR. No gates. |
| **Code Review** | Reviewing a change on a branch or pull request. | Review the change → run the project's own gates on the branch. Uses whatever review method the agent brings; nothing is committed, pushed or published — the report is the only output. |
| **Address Review Findings** | Acting on a review that already happened. | Work through the findings on the reviewed branch → confirm every finding is closed or answered (up to two attempts) → open PR. |

*Baseline* is a `command` step that measures the project's own test harness before any change, so a check that was already failing is reported as pre-existing instead of being blamed on the feature. *Open PR* is the `finalize` step (**Summarize & open PR**).

Code Review and Address Review Findings are usually launched from the **Code Review** button on a project's home, which lists the project's open pull requests.

## Step kinds

| Kind | What it does |
|------|--------------|
| `agent` | One agent turn against the feature worktree: writes the declared artifacts, optionally checked by a verifier. |
| `gate` | Pauses for a human decision: approve, redirect the run back to an earlier step with feedback, or abort. |
| `sequence` | Fans a task list into one agent turn per task, checkpointing each task as it lands. (`parallel` is its retired name, still accepted in old definitions.) |
| `command` | Runs a deterministic shell command (harness, build, script) at zero token cost. |
| `sync` | Merges the branch the run is based on into the feature branch, resolving any conflict with an agent turn. |
| `finalize` | Squashes the feature branch and publishes it. Ends the run — nothing may follow, and a workflow has at most one. |

## Anatomy of a step

Every step declares:

- A **kind** (above) and a **title**.
- A **prompt template** (agent and sequence steps) — the instructions the orchestrator renders for the agent, with placeholders like `{{feature_description}}`, `{{project_conventions}}`, and references to earlier steps' artifacts.
- An **artifact contract** — files the agent must produce (e.g. `artifacts/research-report.md`). The orchestrator captures them and attaches them to later steps' prompts.
- Optionally a **verifier** — a check that judges the step's output and returns a verdict, as the Standard pipeline's validation step does.
- **Failure routing** — `on_failure: <step-id>` declares which step to go back to when this one fails.
- **Max iterations** — how many times the step may run before the orchestrator gives up. The loop budget for validation going back to implementation defaults to 3 and is overridable per project and per run.

### Gate actions

A gate shows every artifact the pipeline has produced so far, and three actions:

- **Approve step** — the run continues.
- **Redirect / Loop** — type feedback and send it. The run goes back to the step you name in the feedback (e.g. `redo s-tickets, the split is too coarse`); if you name none, to the gate's `on_failure` step, else to the nearest earlier step that can change code (or the step that writes its ticket list), else to the step just before the gate.
- **Abort feature** — ends the whole feature.

![Manual Approval Gate on the ship step: the prior steps' artifacts, the critic review open, and Abort feature / Redirect / Loop / Approve step](assets/screenshots/gate-critic-review.png)

Whether a gate waits for you at all is a per-project setting, **gate autonomy**:

| Level | Behaviour |
|-------|-----------|
| `attended` *(default)* | Every gate waits for a person. |
| `review` | Review gates approve themselves; a gate the workflow marks *dangerous* still waits. Among the starters, that is the ship gate of the Standard, Bugfix and CI Fix pipelines. |
| `full` | Every gate approves itself, the ship gate included. |

An automatic approval never redirects or rejects. A run detached to a remote runner always runs with at least `review`, because nobody is attached to answer a review gate.

## What every agent prompt carries

The template is only the middle of what an agent reads. The orchestrator wraps it, in this order, so that a template stored long ago still gets the current engine contract:

- An **Operating Boundary** naming what the step's capability may not do (write outside `artifacts/`, run a shell, reach the network), mirrored by a real filesystem fence.
- A **conduct block**: the turn is unattended, so a reply that ends on a plan or a question has done nothing; claims in the report are audited against tool results from the session; an implement turn holds the task's scope and reports anything else as a follow-up.
- The **artifact contract** — the files the step is expected to produce and where.
- For a verifying step, the **Harness Results** the orchestrator already captured and the **verdict** menu (`pass`, `fail`, `environment`, `evidence`). A verifier's `when_nothing_ran` field says whether an absent harness is terminal (the default) or a pass for a step that judges nothing the project must configure.

Prompts state goals, contracts and the reasons for constraints rather than step-by-step methods: current models plan better than a hand-written script, and read pressure language literally.

## The Standard Feature Pipeline

The default workflow has ten steps:

![A remote, detached feature pipeline with a "1 gate needs you" banner and the Standard pipeline's step graph](assets/screenshots/pipeline-gate-needs-you.png)

| # | Step | Kind | What it does |
|---|------|------|--------------|
| 1 | **Measure Harness Baseline** | `command` | Runs the project's harness on the base commit, so later results read as *before this feature vs. now*. |
| 2 | **Research Codebase** | `agent` | A senior-architect pass: every file likely to change, the patterns the implementation must follow, a risk register, and any external dependencies. Output: `artifacts/research-report.md`. |
| 3 | **Draft Implementation Spec** | `agent` | Turns the research report into a binding spec — 3–7 testable acceptance criteria, each tagged by how it is proved (`[code]`, `[harness]`, `[process]`), plus the changes, testing strategy, constraints and open questions. Everything downstream is judged against it. Output: `artifacts/implementation-spec.md`. |
| 4 | **Decompose Into Tickets** | `agent` | Breaks the spec into an ordered list of tickets sized for one agent session each, every acceptance criterion covered, with its own acceptance, test command and `blocked_by` dependencies. Output: `artifacts/task-list.json`. |
| 5 | **Review Tickets & Spec Before Implementation** | `gate` | **You** read the spec and the ticket list and approve (implementation starts), redirect (your feedback re-enters the spec or ticket step), or abort. This is the moment to fix a bad decomposition — before any code is written. |
| 6 | **Implement Tickets** | `sequence` | Executes the approved ticket list in order: each ticket gets a fresh agent session in the same worktree, sees the spec, the research report, and the record of already-committed tickets — including the `## Handoff` note each earlier ticket's agent left for whoever runs next — and is committed by the orchestrator before the next starts. |
| 7 | **Validate, Test & Security Scan** | `agent` | A QA pass that interprets the project's harness output (already executed by the orchestrator), checks each acceptance criterion, scans for hardcoded secrets / TODOs / unhandled errors, and emits a **READY TO SHIP / BLOCKED** verdict. A failure goes back to the ticket step, which writes a short *rework* list — one ticket per defect — rather than re-implementing the feature. |
| 8 | **Critic Review** | `agent` | An adversarial review across correctness, spec compliance, security, performance, test coverage, and code quality. Emits **Critical / Major / Minor** issues and a **PASS / PASS_WITH_NOTES / FAIL** verdict. |
| 9 | **Approve Merge / Publish** | `gate` *(dangerous)* | **You** review the validation and critic reports and approve, redirect, or abort. Marked *dangerous* because approving lets the run publish to your remote. |
| 10 | **Summarize & open PR** | `finalize` | Squashes the feature branch and opens the pull request; its title and description are written by the agent. |

| | |
|---|---|
| ![Implement Tickets panel: the original decomposition and two rework rounds, every ticket landed with its cost](assets/screenshots/pipeline-implement-tickets.png) | ![critic-review.md: each acceptance criterion with its status and the evidence in the code](assets/screenshots/critic-review.png) |
| **Implement** — one fresh agent per ticket, each with its cost; validation failures come back as rework rounds. | **Critic** — reads the landed code, not just the reports, and checks every acceptance criterion against evidence. |

## Running a non-default workflow

The workflow picker in the **Start a feature** modal pre-selects the project's default (set under **Project Settings → Agent Strategy & Policies → Default Workflow**). Switch it per feature — for example, *Bugfix Pipeline* for an isolated regression, *Refactor Pipeline* for a behaviour-preserving change, *Simple Task* for a small fix. The **Run Workflow** button in the Workflow Library starts a feature with the selected workflow.

## Editing workflows

Open a workflow from the Workflow Library and choose **Edit Workflow** to change it on the canvas: add node types from the palette, connect them, and configure each node in its side panel. The editor validates the graph as you go (*Valid* in the toolbar) and every save is a new immutable version. A starter you have edited can be put back with **Revert to Default**. **New** starts a workflow by cloning an existing pipeline or from a blank shape; **Import** and **Export** move workflows between machines as JSON.

![Workflow canvas editor: the node palette (Agent, Gate, Sequence, Sync, Finalize, Command) beside a copy of the Standard pipeline's DAG](assets/screenshots/workflow-canvas.png)

To pin a different agent or model for a workflow, or for one of its steps, in a single project without editing the shared workflow, use **Project Settings → Workflow Overrides**.

## Definition schema

Workflow definitions are data with a versioned, machine-checkable schema. **Schema v2** (nodes + edges — a true DAG) is what the engine and the canvas work with, and what **Export** writes; it is published as JSON Schema at [`workflow-schema-v2.json`](workflow-schema-v2.json), generated from the engine's own types and checked by a test so it cannot drift from the code. The bundled starters are written as schema v1 (an ordered step list) and migrate to v2 mechanically when read — list order becomes a chain of edges, `on_failure` becomes a retry policy, and `task_list_from` becomes a typed edge.

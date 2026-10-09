# Starting a feature

The fastest way to give work to Demeteo: describe it in plain language on a project's home and launch a pipeline. For work too big or too fuzzy to describe in one go, [run a Discovery](run-a-discovery.md) instead — it ends in tickets that each start a feature like this one.

![Project home on the Pipelines tab: published feature pipelines with their workflow, where they ran, cost, tokens and duration](../assets/screenshots/project-pipelines.png)

## Entry points

A project's home has three tabs — **Pipelines**, **Discovery** and **Ask** — and a **Start session** button above them. You can start a feature:

1. **From the composer** on the *Pipelines* tab — type the feature into *Draft and delegate a new feature pipeline…* and press <kbd>Enter</kbd> (or click *Continue*). You can paste an image into it to attach it.
2. **With the keyboard** — <kbd>⌘</kbd>/<kbd>Ctrl</kbd> + <kbd>T</kbd> opens the same modal with nothing filled in.
3. **From the Workflow Library** — *Run Workflow* on the selected workflow.

**Start session** is not a pipeline: it opens a terminal at the selected location, or (from its dropdown) an interactive agent. No workflow, no gates.

## The Start a feature modal

It collects everything the orchestrator needs before the first agent turn:

| Field | Required | Notes |
|-------|----------|-------|
| **Attachments** | no | PNG, JPG, WebP, GIF, PDF, TXT, Markdown or JSON — up to 100 MB each, 10 per feature. Referenced as `[attachment -- <name>]` in prompts. |
| **Title** | yes | Short label — shown in the pipeline list and at the top of the run. |
| **Describe the feature** | yes | The full intent: what it should do, who it's for, any constraints. Be specific. |
| **Workflow** | yes | Pre-selected from the project's default. Switch per feature (e.g. *Bugfix Pipeline* for an isolated bug, *Simple Task* for small work). |
| **Where to run** | yes | *This machine* (or, for a remote project, the project's machine over SSH — *attached*, so keep the app open), or a machine with `demeteo-runner` — *detached*: you can close Demeteo and the run continues, with optional cost and wall-clock caps. A detached run is always unattended, so its review gates approve themselves. |
| **Target repositories** | yes | Auto-detected from the description; toggle to choose explicitly. A detached run clones only the first selected repository. |
| **Start point** | no | *Start from* — the branch the run cuts its branch from (default: the project's default branch). *Diff against* — the branch its changes are measured against. |

**Customize…** expands the rest: the default agent, model and effort for all steps; a preview of the pipeline's shape; **per-step overrides** (a different harness, model and effort for any step); loop iterations; a per-turn budget; and whether step reports are committed to the PR.

![Start a feature, customized: per-step harness, model and effort overrides with the Critic Review step on Claude Code / Claude Opus / High, then loop iterations, per-turn budget and commit artifacts](../assets/screenshots/start-feature-overrides.png)

Press <kbd>⌘</kbd>/<kbd>Ctrl</kbd> + <kbd>Enter</kbd> (or click **Launch feature**) to start the pipeline.

## What you see next

The feature opens in its run view:

![A remote, detached feature pipeline: header metrics, a "1 gate needs you" banner, the step graph, the selected Approve Merge / Publish gate, and the runner's Approve / Reject in Activity](../assets/screenshots/pipeline-gate-needs-you.png)

- **Header** — status, where it runs, and live **elapsed time, cost, tokens and cache reads**; **Code with Agent** (an interactive agent session in the feature's worktree) and **Browse Code** (the feature branch's diff, read-only).
- **Initial prompt** and any **attachments**.
- **Graph / Timeline** — the run as the workflow graph or as a step list. Select a step to inspect it: *Overview* (attempts, cost, duration, the task list for a sequence step), *Live*, *Output* (its artifacts), and *Actions*.
- **Activity** — the run's live event feed, and for pipelines that measure a baseline, the project's **harness gates before this feature vs. now**.

You do **not** chat with the agents. The orchestrator drives them and surfaces the artifacts they write. You step in at **Gates**: when one is waiting, a *GATE NEEDS YOU* banner offers **Decide Gate**, and the gate lets you **Approve step**, **Redirect / Loop** with feedback, or **Abort feature**. See [Gate actions](../workflows.md#gate-actions).

![Browse Code: the feature branch's changed-file tree and a side-by-side diff](../assets/screenshots/browse-code-diff.png)

## During and after the run

- **Cancel Feature** in the header while the run is live.
- A step's *Actions* tab offers **Retry node**, **Replay from node** (re-runs it and everything downstream, replacing their artifacts), and **Stop node**. Retry is disabled while an earlier step is still running.
- When the run ends, **Publish MR** opens the pull request if the run didn't already, and **View PR** opens it in your browser; the PR's state shows in the pipeline list.
- **Sync** shows how far the feature branch is behind its base and merges it in, resolving conflicts with an agent turn if needed.
- The **⋯** menu has *Refresh PR state*, *Cleanup* (apply the project's completed-feature lifecycle), and *Copy feature ID*.

See [Workflows](../workflows.md) for each starter step by step, and [Settings](../settings.md) for the policies that change how a pipeline behaves.

# Starting a feature

The fastest way to give work to Demeteo: a single sentence in plain language on the workspace home view.

![Workspace home view with the feature input expanded](../assets/screenshots/home.png)

## The three entry points

From the **Project Home** you can start a feature three ways:

1. **The hero input** — type the feature into the box at the top of the workspace and press <kbd>Enter</kbd> (or click *Continue*).
2. **The command palette** — <kbd>⌘</kbd> / <kbd>Ctrl</kbd> + <kbd>K</kbd> opens the palette and lets you fuzzy-search workspaces.
3. **The "Start a coding session" tile** — if you don't want a full pipeline, this opens an interactive agent in the repo directly. No workflow, no gates.

## What happens after you press Continue

The **Start a feature** modal opens. It collects everything the orchestrator needs before the first agent turn:

| Field | Required | Notes |
|-------|----------|-------|
| **Attachments** | no | Drag PNG, JPG, WebP, GIF, PDF or TXT into the dropzone (max 100 MB each). Referenced as `[attachment -- <name>]` in prompts. |
| **Title** | yes | Short label — appears in the sidebar list and at the top of the workflow timeline. |
| **Describe the feature** | yes | The full intent: what it should do, who it's for, any constraints. Be specific. |
| **Workflow** | yes | Pre-selected from the workspace's default. Switch per-feature (e.g. *Bugfix Pipeline* for isolated bugs, *Simple Task* for trivial work). |
| **Where to run** | yes | The host machine: *This machine*, a connected remote, or local-only. |
| **Target repositories** | yes | Auto-detected from the description. Toggle to add or remove repos, or click *Customize...* for per-step agent overrides, conflict policies, and budget caps. |

Hit <kbd>⌘</kbd> / <kbd>Ctrl</kbd> + <kbd>Enter</kbd> (or click *Launch feature*) to spawn the pipeline.

## What you see next

The Project Home transitions to **Feature Detail** — a list-style timeline of the workflow steps:

![Feature pipeline running the Standard Feature Pipeline](../assets/screenshots/feature-pipeline.png)

Each row is one *Step*: an `agent`, a `parallel` fan-out, or a `gate`. The top bar shows live telemetry — elapsed duration, accrued cost, total tokens — and exposes *Code with Agent*, *Browse Code*, and *Cancel Feature* controls.

You do **not** chat with the agents. The orchestrator drives them and surfaces only the artifacts they write. You intervene at **Gates** — the two `gate` steps in the Standard pipeline (after spec, before merge) — to approve, redirect with feedback, or abort.

## During the run

- **Pause / Resume / Cancel** from the top bar.
- **Sync / Resolve Sync Conflicts** if the orchestrator rebases against `origin/<default>` and a conflict appears.
- **Retry Step** on any failed or interrupted card — disabled with a rose-bordered banner if an earlier step is still `pending | running | verifying | awaiting_gate`.

See the [Workflows](../workflows.md) page for the full step-by-step of each starter, and [Settings](../settings.md) for the policy controls that change how the pipeline behaves.
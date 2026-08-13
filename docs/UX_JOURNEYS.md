# Demeteo: UX Specifications & User Journeys

> **Purpose:** This document is intended for UI/UX designers to translate Demeteo's multi-agent orchestrator architecture into concrete UI mockups. It maps the locked architectural decisions into end-to-end user journeys and defines the exact screens to be designed.

## 1. Product Philosophy & Core Mechanics

Demeteo has pivoted from a single-agent chat interface into a **fleet-style multi-agent orchestrator**. The user is no longer chatting with an LLM; instead, the user acts as a product manager who describes a feature, delegates the execution to automated workflows, and intervenes only when explicit approval is required (Gates).

**Core Vocabulary:**
- **Project:** The workspace, mapped to repositories and a host machine.
- **Workflow:** A versioned, reusable template defining the steps to build a feature.
- **Feature:** A running instance of a workflow. This is what the user tracks.
- **Step:** An individual node in a workflow (`agent`, `parallel`, or `gate`).
- **Gate:** A pause in execution where the system waits for human approval, feedback, or conflict resolution.

## 2. Design Language: Dark Neon Glassmorphism

To ensure Demeteo feels like a premium desktop control center, all mockups must strictly adhere to the following visual design rules:

- **Backgrounds:** Obsidian and deep carbon gradients (`#08090c` and `#0d0f14`), with subtle radial violet and cyan gradients for depth.
- **Surfaces & Blurs:** Translucent cards (`rgba(18, 22, 30, 0.75)`) using `backdrop-filter: blur(12px)` and thin border glows (`rgba(255,255,255,0.05)`).
- **Typography:**
  - Headings: **Outfit** (sharp, geometric)
  - General UI: **Inter** (clean, readable)
  - Terminals/Code: **Fira Code** or **JetBrains Mono**
- **Status Accents:**
  - **Violet (`#8b5cf6`):** Active connection tunnels and core operations.
  - **Cyan (`#06b6d4`):** Real-time data streams and interactive sessions.
  - **Emerald (`#10b981`):** Running processes and healthy statuses.
  - **Ruby (`#ef4444`):** Inactive servers, stopped tasks, failed states.
- **Motion:** Micro-animations, subtle pulsing glows for status dots, and smooth view transitions.

## 3. Global Shell & Navigation Structure

The application functions within a single unified shell (no multi-window popouts for standard operations).

- **Top Bar:** three tracks — logo, a centred Command Palette trigger
  (`Cmd/Ctrl+K`), and a nav cluster (Workflows · Providers · Runs · Terminals ·
  Notification Bell · account avatar). Global Settings lives in the menu the
  avatar opens; the docs panel has no header control and is reached with `F1`
  or `?`. See [`ARCHITECTURE.md`](ARCHITECTURE.md) §5 for why neither has an
  icon of its own.
- **Left Rail (Project Navigation):**
  - Search/filter input.
  - List of Projects (with active status dots).
  - `+ New` button for creating projects.
  - `⚙ Mng` button for project list management.
- **Main Pane:** Context-sensitive area that updates based on the active project or rail selection.

## 4. Key User Journeys

### Journey 1: First-Run & Onboarding
*Getting a new user to their 'Aha!' moment without needing API keys.*
- **UI State:** `EmptyStateCard`.
- **Content:** Welcome greeting, brief explanation of the orchestrator concept.
- **CTA:** Prominent "Try a sample project" button. This runs a bundled starter workflow using a real LLM on a dummy local repo.
- **Secondary CTA:** "Connect a Provider" (GitHub/GitLab) and "Create New Project".

### Journey 2: Connecting a Provider
*Wiring up external systems to allow Demeteo to clone and publish.*
- **UI State:** `ProviderSettings` Modal or View.
- **Inputs:** Provider Type dropdown (GitHub/GitLab), Host URL, Encrypted Personal Access Token (PAT).
- **Action:** System validates the PAT on connect (calls `/user`), fetches user avatar, and displays it upon success.

### Journey 3: Project Bootstrap
*Creating a new workspace from remote repositories.*
- **UI State:** `NewProjectView` (slim modal).
- **Inputs:** Project Name, Environment (Local/Remote SSH), select Repositories via connected Provider.
- **System Action:** Clones repos, detects default branch, PR template, CI setup.
- **Next Step:** Shows the user a "Proposed Worktree Strategy" (branch naming conventions, merge flow). The user can approve or edit.

### Journey 4: The Project Home
*The control center for a specific project.*
- **UI State:** `ProjectHome` (Main Pane).
- **Hero Element:** "Start a Feature" slim input modal.
- **Hero Element:** `StartSessionButton` split-button — one click opens a plain shell scoped to the project's repo path and navigates to the Terminals view; the caret lets the user pick a coding agent instead. Available for both local and remote projects.
- **Active Area:** Shows the currently running Feature (progress bar, current step, cost telemetry).
- **Queue/History:** A list of pending or completed (archived) features.
- **Repo Map:** A lazy-loaded list of connected repositories and active feature branches (driven by `RepoHealthStatus`; not a visual map).

### Journey 5: Starting a Feature
*Taking a user requirement and kicking off an automated workflow.*
- **UI State:** Slim Modal expands from Project Home (`StartFeatureModal`).
- **Inputs:** User describes the feature in a textarea.
- **Auto-Inference:** The system locally matches keywords to suggest Repository Chips and detect conflicts (no LLM call in the modal).
- **Customization:** User clicks "Customize..." to expand the form and override default Workflows, Target Repos, Conflict Policies, per-step agent/model overrides, commit-artifacts toggle, or set Budget limits.
- **Pre-flight Validation:** Static checks display the workflow step list, potential risks, and repo fit. No cost is estimated here.
- **Attachments:** User may drop files / images into the attachment dropzone; they are persisted to the feature row before the driver spawns so the agent's first turn sees them.
- **Submit:** Kicks off the Feature (`start_feature`) and transitions the view to `FeatureDetail`.

### Journey 6: Orchestration Monitoring (Feature Detail)
*Watching the fleet of agents work without chat.*
- **UI State:** `FeatureDetail` (Main Pane).
- **Visualization:** `WorkflowCanvas` in run mode — a node graph with live status overlay, node drill-down panel (Overview / Output / Live / Actions), sequence-node task expansion, and remote runs on the same surface. A `Graph | Timeline` toggle keeps the original list-style timeline available. (Superseded the "DAG visualization is deferred" position — see `PRD_DAG_WORKFLOWS.md` §6.)
- **Telemetry:** Per-step cost ($) and duration (time) metrics. *(Note: No pre-launch cost estimates, only real-time accrued cost).*
- **Status Indicators:** Steps use the color language (Emerald=Running, Ruby=Failed, Violet=Active).
- **Actions:** Pause, Resume, Cancel, Sync, Resolve Sync Conflicts. Each failed/interrupted step card exposes "Retry Step" — disabled when a predecessor is in `pending | running | verifying | awaiting_gate`, with a rose-bordered banner naming the blocker.

### Journey 7: The Gate (Approval Workflow)
*Where the orchestrator pauses for human intervention.*
- **UI State:** `GateView` — **full-screen takeover** of the main pane (no slide-up overlay in v1).
- **Content:**
  - **Planner Summary Card:** What the agents did and why.
  - **Artifacts:** Code diffs, written specs, or merge request summaries.
- **Actions:**
  - **Approve:** Continue execution.
  - **Redirect:** Open an input field to send feedback/corrections to the planner.
  - **Cancel:** Abort the current feature run.
- **Predecessor-running guard:** when an earlier step is `pending | running | verifying | awaiting_gate`, the Approve and Redirect buttons are disabled and a rose-bordered banner names the blocker. The "Abort feature" button stays enabled.

### Journey 8: Handling Subtasks & Parallel Execution
*Breaking down work across multiple agents simultaneously.*
- **Trigger:** Workflow reaches a `parallel` step.
- **UI State:** Subtask execution list inside the `FeatureDetail` timeline.
- **Visuals:** A list of parallel tasks (one host, one agent per worktree).
- **Feedback:** Continue-and-report semantics for failures. Shows error chips on failed subtasks, with an opt-in "Retry" button (with cost cap).

### Journey 9: Resolving Merge Conflicts
*Handling overlapping changes smartly.*
- **Trigger:** A subtask merge back into `feature/<slug>` fails, or `feature_sync` against `origin/<default>` leaves conflicts.
- **System Flow:** A conflicting *step* merge costs one automatic agent turn in the step's own worktree (`steps/conflict_pass`); only if that fails does the step fail. A conflicting *sync* surfaces the file list to the user, who triggers `feature_resolve_sync_conflicts` ("Resolve with agent") — it spawns a resolution agent, commits the fix, and replays the validation step. (The "Conflict Resolution Policy" project setting is currently decorative — nothing reads it; see decision 20's history.)
- **UI State:** Conflict UX lives inside the existing `GateView` (no dedicated Monaco 3-way merge component ships in v1) — the file list is shown, the user can retry, abort, or hand-roll a manual edit via the in-app terminal.
- **Actions:** Approve (re-run after manual edit), Retry (re-spawn the auto-agent), Abort feature.

### Journey 10: Workflow Authoring
*Creating the templates that agents follow.*
- **UI State:** `WorkflowCanvas` design mode. (Replaced the form-first `WorkflowEditor`, which was deleted — Decision 19 is superseded; see `DECISIONS.md` §2.)
- **Content:** Visual DAG builder — palette driven by the node-type registry, connect-time validation, schema-driven config panel, live lint surface, dirty guard, undo/redo, templates and v2 import/export, read-only Monaco source tab (Decision 42).
- **Configuration:** Node config from the registry's JSON Schema; conditional `when` edges exist in the engine but are **not yet exposed in the builder** (task P4.3).
- **Versioning:** Every save creates a new `WorkflowVersion` row; `workflow_versions` and `workflow_revert_to_default` round-trip the history.

## 5. UI Views & Screens to Design

Based on the journeys above, designers must deliver the following discrete screen mockups:

1. **App Shell & Project Rail:** Base layout with cross-project navigation and the Command Palette (`Cmd+K`) active state.
2. **First Run / Empty State Card:** Onboarding flow centered around running a sample project.
3. **Project Creation & Worktree Strategy Forms:** Bootstrap flow with repo selection and strategy proposal.
4. **Project Home:** The main dashboard featuring the "Start a Feature" slim modal, active run status, and repository list.
5. **Feature Detail (Orchestration View):** The step timeline showing step progress, cost/duration telemetry, subtask fan-outs, and the predecessor-running guard banner.
6. **Gate View:** Full-screen takeover showing the planner's summary, diffs, and Approve/Redirect buttons (with predecessor-running guard).
7. **Conflict UX (inside Gate View):** Conflict file list with retry / abort affordances. A dedicated Monaco 3-way merge editor is not in v1.
8. **Workflow Editor:** The form-based UI for creating/editing workflow steps with version history.
9. **Settings & Preferences:** Global settings (theme, memory agent, pricing tables) and Provider instances setup.
10. **Machines & Agent Profiles:** Per-host agent configuration for legacy shell / custom-http agent kinds (ollama, openai, cli, custom_http).
# Demeteo: User Stories & Agent Tasks

> **Purpose:** Detailed user stories and actionable tasks for agent execution. Each story is mapped to the multi-agent orchestrator architecture, UX journeys, and UI areas.
>
> **Status (2026):** All v1 stories are implemented and shipped in the current multi-agent orchestrator. The task lists below are retained for historical traceability and marked `[x]` end-to-end; new work belongs in the active plan, not here.

## Story 1: First-Run & Onboarding
**Description:** As a new user, I want to see an empty state that explains the orchestrator and lets me run a sample project so I can understand the value without setting up API keys.
**References:**
- **UX Journey:** [Journey 1](UX_JOURNEYS.md#journey-1-first-run--onboarding)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (UiStateRepository / `AppSettingsRepository`)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Identity & Fleet)
- **UI Areas:** `EmptyStateCard`, `TopBar`

**Status:** Implemented.

**Tasks:**
- [x] Implement `EmptyStateCard` UI component based on dark neon glassmorphism guidelines.
- [x] Wire "Try a sample project" button to seed a dummy project and starter workflow.
- [x] Add application shell (`TopBar`, `Sidebar` empty state).

## Story 2: Connecting a Provider
**Description:** As a user, I want to connect my GitHub/GitLab account using a PAT so Demeteo can clone repositories and publish MRs.
**References:**
- **UX Journey:** [Journey 2](UX_JOURNEYS.md#journey-2-connecting-a-provider)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (`ProviderHttpPort`, `AppSettingsRepository`)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Identity & Fleet: `ProviderInstance`)
- **UI Areas:** `ProviderSettings`, `TopBar` avatars

**Status:** Implemented.

**Tasks:**
- [x] Create `ProviderSettings` modal/view.
- [x] Implement form to capture Provider Type, Host URL, and PAT.
- [x] Wire UI to Tauri command for `/user` PAT validation (`validate_provider_pat`).
- [x] Display connected provider avatar in `TopBar`.

## Story 3: Project Bootstrap
**Description:** As a user, I want to create a new workspace by selecting remote repositories, so I can start running feature workflows against them.
**References:**
- **UX Journey:** [Journey 3](UX_JOURNEYS.md#journey-3-project-bootstrap)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (`ProjectRepository`, `WorktreeOpsPort`)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Project Management)
- **UI Areas:** `NewProjectView`

**Status:** Implemented.

**Tasks:**
- [x] Implement `NewProjectView` with form for Name, Compute Type, and Repositories.
- [x] Build Repo Selection Modal with fuzzy search.
- [x] Display "Proposed Worktree Strategy" UI post-selection (`get_proposed_strategy`).
- [x] Wire `create_project` backend invocation.

## Story 4: The Project Home & Starting a Feature
**Description:** As a user, I want a control center where I can describe a feature, see active pipelines, and monitor accumulated costs.
**References:**
- **UX Journey:** [Journey 4 — The Project Home](UX_JOURNEYS.md#journey-4-the-project-home), [Journey 5 — Starting a Feature](UX_JOURNEYS.md#journey-5-starting-a-feature)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (`StepExecutor::feature_start`)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Feature Orchestration)
- **UI Areas:** `ProjectHome`, `StartFeatureModal`

**Status:** Implemented.

**Tasks:**
- [x] Implement `ProjectHome` layout including header block with telemetry (spend/nodes).
- [x] Build the "Start Feature Expanded Card" text area with auto-inference visual simulation.
- [x] Add the "Active Running Pipelines" list rendering active features with status/cost indicators.
- [x] Hook up "Delegate Workspace" button to launch the workflow (`start_feature`).

## Story 5: Orchestration Monitoring (Feature Detail)
**Description:** As a user, I want to see the execution timeline of a feature to monitor agent progress, subtask fan-outs, and per-step telemetry.
**References:**
- **UX Journey:** [Journey 6](UX_JOURNEYS.md#journey-6-orchestration-monitoring-feature-detail)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (`StepExecutor`)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Feature Orchestration: `StepExecution`)
- **UI Areas:** `FeatureDetail`

**Status:** Implemented.

**Tasks:**
- [x] Implement `FeatureDetail` view with sticky header and total cost/duration.
- [x] Render the Orchestration Timeline view of the step DAG (list-style step rows with per-step telemetry; circular-node DAG visualization is deferred).
- [x] Implement `parallel` step subtask rendering (expandable/nested lists for parallel workers).
- [x] Wire real-time status updates (`running`, `done`, `gated`) and pulsing micro-animations.

## Story 6: The Gate (Approval Workflow)
**Description:** As a user, I want to review an agent's proposed changes at a Gate so I can approve, reject, or provide redirect instructions before code is merged.
**References:**
- **UX Journey:** [Journey 7](UX_JOURNEYS.md#journey-7-the-gate-approval-workflow)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (`GatePresenter::gate_decide`)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Feature Orchestration: `GateDecision`)
- **UI Areas:** `GateView`

**Status:** Implemented.

**Tasks:**
- [x] Build `GateView` as a full-screen takeover of the main pane.
- [x] Render the "Orchestrator Synthesis" summary card.
- [x] Implement the Unified Code Diff Viewer to show `+`/`-` changes (`ArtifactViewer` / `CodeEditorView`).
- [x] Add Radio inputs for Action selection (Approve vs Redirect).
- [x] Wire the "Resume Pipeline" button to send the gate decision to the Rust backend (`gate_decide`).
- [x] Surface the predecessor-running guard: the Approve / Redirect buttons are disabled when an earlier step is in `pending | running | verifying | awaiting_gate`, with a rose-bordered banner naming the blocker.

## Story 7: Resolving Merge Conflicts
**Description:** As a user, I want merge conflicts handled where they happen — an agent turn for step merges, a "Resolve with agent" action for sync conflicts — to ensure branch integrity. (The original "smart cascade" framing was superseded 2026-07-12; see decision 20's history in [DECISIONS.md](DECISIONS.md#2-superseded-decisions).)
**References:**
- **UX Journey:** [Journey 9](UX_JOURNEYS.md#journey-9-resolving-merge-conflicts)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (`feature_sync`, `feature_resolve_sync_conflicts`)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Worktree & Git: `ConflictReport`)

**Status:** Partially implemented.

**Tasks:**
- [x] Surface a structured `ConflictReport` (conflicting files with `kind`, raw stderr, `detected_at`) from `feature_sync`.
- [x] Resolve a conflicting step merge inline: `steps/conflict_pass` spends one agent turn in the step's own worktree, then retries the merge.
- [x] Resolve a sync conflict on demand: `feature_resolve_sync_conflicts` spawns a fresh resolution agent, commits the resolution, and (optionally) replays the named step so validation re-runs on the merged tree.
- [x] Surface the conflict UI inside the existing `GateView` — there is **no dedicated Monaco 3-way component** in v1. The gate shows the file list, lets the user retry, abort, or hand-roll a manual edit via the in-app terminal.
- [ ] Make the "Conflict Resolution Policy" project setting real or remove it: the dropdown is stored but nothing reads it (see decision 20's "known loose end").

## Story 8: Workflow Authoring
**Description:** As a user, I want to create and edit workflow templates to define custom execution steps, conditions, and agent assignments.
**References:**
- **UX Journey:** [Journey 10](UX_JOURNEYS.md#journey-10-workflow-authoring)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (`WorkflowRepository`)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Workflow Catalog)
- **UI Areas:** `WorkflowList`, `WorkflowEditor`

**Status:** Implemented.

**Tasks:**
- [x] Create `WorkflowList` view displaying bundled starter packs.
- [x] Build `WorkflowEditor` form for adding/reordering `agent`, `parallel`, and `gate` steps.
- [x] Implement export/import functionality via JSON (`workflow_export` / `workflow_import`).
- [x] Implement version history (immutable `WorkflowVersion` rows, `workflow_versions`, `workflow_revert_to_default`).

## Story 9: Global Shell & Project Rail
**Description:** As a user, I want to easily switch between projects and access global settings via a command palette and left rail.
**References:**
- **UX Journey:** [Journey 1 — Global Shell](UX_JOURNEYS.md#journey-1-first-run--onboarding) (sidebar / top bar); the project-rail specifically is described in [Journey 4 — The Project Home](UX_JOURNEYS.md#journey-4-the-project-home).
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (`AppSettingsRepository`)
- **UI Areas:** `Sidebar`, `ProjectRail`, `TopBar`, `CommandPalette`, `DocsPanel`

**Status:** Implemented.

**Tasks:**
- [x] Implement `ProjectRail` rendering active projects with status dots (`emerald`, `ruby`).
- [x] Implement `Sidebar` shell that hosts the project rail plus global actions.
- [x] Add Command Palette (`Cmd+K`) triggering a fuzzy search overlay for navigation.
- [x] Wire the `?` icon to open the markdown `DocsPanel`.
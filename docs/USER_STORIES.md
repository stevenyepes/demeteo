# Demeteo: User Stories & Agent Tasks

> **Purpose:** Detailed user stories and actionable tasks for agent execution. Each story is mapped to the multi-agent orchestrator architecture, UX journeys, and UI areas.

## Story 1: First-Run & Onboarding
**Description:** As a new user, I want to see an empty state that explains the orchestrator and lets me run a sample project so I can understand the value without setting up API keys.
**References:**
- **UX Journey:** [Journey 1](UX_JOURNEYS.md#journey-1-first-run--onboarding)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (UiStateRepository)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Identity & Fleet)
- **UI Areas:** EmptyStateCard, TopBar

**Tasks:**
- [ ] Implement `EmptyStateCard` UI component based on dark neon glassmorphism guidelines.
- [ ] Wire "Try a sample project" button to seed a dummy project and starter workflow.
- [ ] Add application shell (`TopBar`, `Sidebar` empty state).

## Story 2: Connecting a Provider
**Description:** As a user, I want to connect my GitHub/GitLab account using a PAT so Demeteo can clone repositories and publish MRs.
**References:**
- **UX Journey:** [Journey 2](UX_JOURNEYS.md#journey-2-connecting-a-provider)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (ProviderInstanceRepository)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Identity & Fleet: ProviderInstance)
- **UI Areas:** ProviderSettings, TopBar avatars

**Tasks:**
- [x] Create `ProviderSettings` modal/view.
- [x] Implement form to capture Provider Type, Host URL, and PAT.
- [x] Wire UI to Tauri command for `/user` PAT validation.
- [x] Display connected provider avatar in `TopBar`.

## Story 3: Project Bootstrap
**Description:** As a user, I want to create a new workspace by selecting remote repositories, so I can start running feature workflows against them.
**References:**
- **UX Journey:** [Journey 3](UX_JOURNEYS.md#journey-3-project-bootstrap)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (ProjectRepository, WorktreeManager)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Project Management)
- **UI Areas:** NewProjectView

**Tasks:**
- [x] Implement `NewProjectView` with form for Name, Compute Type, and Repositories.
- [x] Build Repo Selection Modal with fuzzy search.
- [x] Display "Proposed Worktree Strategy" UI post-selection.
- [x] Wire `Project.create` backend invocation.

## Story 4: The Project Home & Starting a Feature
**Description:** As a user, I want a control center where I can describe a feature, see active pipelines, and monitor accumulated costs.
**References:**
- **UX Journey:** [Journey 4 & 5](UX_JOURNEYS.md#journey-4-the-project-home)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (FeatureOrchestrator)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Feature Orchestration)
- **UI Areas:** ProjectHome, Feature input block

**Tasks:**
- [x] Implement `ProjectHome` layout including header block with telemetry (spend/nodes).
- [x] Build the "Start Feature Expanded Card" text area with auto-inference visual simulation.
- [x] Add the "Active Running Pipelines" list rendering active features with status/cost indicators.
- [x] Hook up "Delegate Workspace" button to launch the workflow.

## Story 5: Orchestration Monitoring (Feature Detail)
**Description:** As a user, I want to see the execution DAG of a feature to monitor agent progress, subtask fan-outs, and per-step telemetry.
**References:**
- **UX Journey:** [Journey 6](UX_JOURNEYS.md#journey-6-orchestration-monitoring-feature-detail)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (StepExecutor)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Feature Orchestration: StepExecution)
- **UI Areas:** FeatureDetail

**Tasks:**
- [ ] Implement `FeatureDetail` view with sticky header and total cost/duration.
- [ ] Render the Orchestration DAG Execution Graph using absolute lines and circular step nodes.
- [ ] Implement `parallel` step subtask rendering (expandable/nested lists for parallel workers).
- [ ] Wire real-time status updates (`running`, `done`, `gated`) and pulsing micro-animations.

## Story 6: The Gate (Approval Workflow)
**Description:** As a user, I want to review an agent's proposed changes at a Gate so I can approve, reject, or provide redirect instructions before code is merged.
**References:**
- **UX Journey:** [Journey 7](UX_JOURNEYS.md#journey-7-the-gate-approval-workflow)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (FeatureOrchestrator: gate_decide)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Feature Orchestration: GateDecision)
- **UI Areas:** GateView

**Tasks:**
- [x] Build `GateView` overlay sliding in from the bottom.
- [x] Render the "Orchestrator Synthesis" summary card.
- [x] Implement the Unified Code Diff Viewer to show `+`/`-` changes.
- [x] Add Radio inputs for Action selection (Approve vs Redirect).
- [x] Wire the "Resume Pipeline" button to send the gate decision to the Rust backend.

## Story 7: Resolving Merge Conflicts
**Description:** As a user, I want to handle subtask merge conflicts using a smart cascade (agent first, then manual 3-way merge) to ensure branch integrity.
**References:**
- **UX Journey:** [Journey 9](UX_JOURNEYS.md#journey-9-resolving-merge-conflicts)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (ConflictResolver)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Worktree & Git: ConflictReport)

**Tasks:**
- [x] Implement `ConflictResolver` component using Monaco editor's 3-way merge mode.
- [x] Add action buttons for "Skip/Abort Subtask" or "Save Manual Resolution".
- [x] Integrate conflict state rendering into `FeatureDetail` gate block.

## Story 8: Workflow Authoring
**Description:** As a user, I want to create and edit workflow templates to define custom execution steps, conditions, and agent assignments.
**References:**
- **UX Journey:** [Journey 10](UX_JOURNEYS.md#journey-10-workflow-authoring)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (WorkflowRepository)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Workflow Catalog)
- **UI Areas:** TopBar workflows button

**Tasks:**
- [ ] Create `WorkflowList` view displaying bundled starter packs.
- [ ] Build `WorkflowEditor` form for adding/reordering `agent`, `parallel`, and `gate` steps.
- [ ] Implement export/import functionality via JSON.

## Story 9: Global Shell & Project Rail
**Description:** As a user, I want to easily switch between projects and access global settings via a command palette and left rail.
**References:**
- **UX Journey:** [Journey 4](UX_JOURNEYS.md#journey-4-the-project-home) (Sidebar)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (UiStateRepository)
- **UI Areas:** Sidebar, TopBar

**Tasks:**
- [x] Implement `Sidebar` rendering active projects with status dots (`emerald`, `ruby`).
- [x] Add Command Palette (`Cmd+K`) triggering a fuzzy search overlay for navigation.
- [x] Wire the `?` icon to open the markdown `DocsPanel`.

## Story 10: Single Escape / Overlay-Priority Stack (UX1.6)
**Description:** As a user, I want exactly one Escape keypress to dismiss only the topmost overlay, and I want the dismissal order to follow a predictable priority — so that layered modals (Gate → palette → modal → toast → drawer) behave predictably instead of closing the wrong surface.
**Feature IDs:** F35 (single Escape dismisses topmost overlay only), F40 (priority order: `GateView > CommandPalette > modal > toast > sheet`).
**References:**
- **UX Journey:** [Journey 11: Keyboard & Overlay Behavior](UX_JOURNEYS.md#journey-11-keyboard--overlay-behavior)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (UiStateRepository — new `OverlayStackContext` reducer)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (UI overlay aggregate)
- **Decision:** [docs/DECISIONS.md](DECISIONS.md) — Decision 37
- **Spec:** `artifacts/_context/implementation-spec.md` (UX1.6)
- **UI Areas:** `Modal`, `GateView`, `StartFeatureModal`, `CommandPalette`, `DocsPanel`, `AgentTerminalDrawer`, `ErrorToast`, `NotificationBell`, `ProjectSettingsShell`, `EnvModal`, `ProviderSettings`, `NewProjectView`, `ProvidersPage`, `FeatureDetail`, `ConflictResolver`

**Tasks:**
- [x] Create `src/context/OverlayStackContext.tsx` exposing `OverlayStackProvider`, `useOverlayStack`, `useOverlay`, the `OverlayStackEntry` / `OverlayPriorityTier` types, and a pure `overlayStackReducer` ordering by `(tierRank, priority, -createdAt)`.
- [x] Create `src/hooks/useOverlayEscape.ts` — the single global `keydown` listener (registered with `{ capture: true }`) that consults `top()` and dispatches to its `onEscape`.
- [x] Re-export the new symbols from `src/context/index.tsx`.
- [x] Add optional `stackId`, `stackTier`, `stackPriority`, `onStackEscape` props to `src/components/ui/Modal.tsx`; call `useOverlay(stackId, …)` when supplied and keep backdrop-click dismissal unchanged.
- [x] Remove every per-component `window.addEventListener('keydown', …)` Escape branch (the single listener survives only in `useOverlayEscape`); enforce via repo-wide ripgrep.
- [x] Migrate overlay call-sites to register through `useOverlay` with explicit tiers: `gate` (`GateView`), `modal` (`StartFeatureModal`, `PromptDialog`, `EnvModal`, `ProviderSettings`, `ProjectSettingsShell` inner modals, `NewProjectView` repo picker, `ProvidersPage` connect modal, `FeatureDetail` replay-confirm), `palette` (`CommandPalette`, `DocsPanel`), `drawer` (`AgentTerminalDrawer`, `FeatureDetail` agent drawer), `toast` (`ErrorToast` per-instance, `NotificationBell`).
- [x] In `src/App.tsx`, wrap the tree in `<OverlayStackProvider>`, delete the legacy cascade at `src/App.tsx:119-123`, call `useOverlayEscape()` once at the shell, and inside the `gate_required` handler dispatch `CLOSE_OVERLAY('feature.start')` before navigating so a Gate overrides an open feature modal.
- [x] Strip the `onEscape` field from `ShortcutMap` and the Escape branch from `src/hooks/useKeyboardShortcuts.ts`; non-Escape shortcuts (`Cmd+K`, `?`, `Cmd+,`) remain.
- [x] Add `OPEN_OVERLAY` / `CLOSE_OVERLAY` additive reducer variants to `src/context/UIStateContext.tsx` so future call-sites can opt into stack semantics.
- [x] Add reducer unit tests (`T1`–`T6`): tier ordering, within-tier priority ordering, `POP` by id, `POP` no-op, `REPLACE` in-place, `createdAt` tie-break.
- [x] Append Journey 11 (Keyboard & Overlay Behavior) plus the UX1.6 acceptance checklist to `docs/UX_JOURNEYS.md`.
- [x] Bump `docs/DECISIONS.md` to "37 locked decisions" and record Decision 37 (overlay priority tiers and single Escape listener).
- [x] Rename the `docs/BACKEND_REFACTOR_TASKS.md` block at lines 509–514 to "Frontend UX Hardening (U1–U5) — COMPLETED" and append the U5 / F40 entry.

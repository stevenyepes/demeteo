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
- [ ] Create `ProviderSettings` modal/view.
- [ ] Implement form to capture Provider Type, Host URL, and PAT.
- [ ] Wire UI to Tauri command for `/user` PAT validation.
- [ ] Display connected provider avatar in `TopBar`.

## Story 3: Project Bootstrap
**Description:** As a user, I want to create a new workspace by selecting remote repositories, so I can start running feature workflows against them.
**References:**
- **UX Journey:** [Journey 3](UX_JOURNEYS.md#journey-3-project-bootstrap)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (ProjectRepository, WorktreeManager)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Project Management)
- **UI Areas:** NewProjectView

**Tasks:**
- [ ] Implement `NewProjectView` with form for Name, Compute Type, and Repositories.
- [ ] Build Repo Selection Modal with fuzzy search.
- [ ] Display "Proposed Worktree Strategy" UI post-selection.
- [ ] Wire `Project.create` backend invocation.

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
- [ ] Build `GateView` overlay sliding in from the bottom.
- [ ] Render the "Orchestrator Synthesis" summary card.
- [ ] Implement the Unified Code Diff Viewer to show `+`/`-` changes.
- [ ] Add Radio inputs for Action selection (Approve vs Redirect).
- [ ] Wire the "Resume Pipeline" button to send the gate decision to the Rust backend.

## Story 7: Resolving Merge Conflicts
**Description:** As a user, I want to handle subtask merge conflicts using a smart cascade (agent first, then manual 3-way merge) to ensure branch integrity.
**References:**
- **UX Journey:** [Journey 9](UX_JOURNEYS.md#journey-9-resolving-merge-conflicts)
- **Architecture:** [docs/ARCHITECTURE.md](ARCHITECTURE.md) (ConflictResolver)
- **DDD Domain:** [docs/DDD_MODEL.md](DDD_MODEL.md) (Worktree & Git: ConflictReport)

**Tasks:**
- [ ] Implement `ConflictResolver` component using Monaco editor's 3-way merge mode.
- [ ] Add action buttons for "Skip/Abort Subtask" or "Save Manual Resolution".
- [ ] Integrate conflict state rendering into `FeatureDetail` gate block.

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
- [ ] Implement `Sidebar` rendering active projects with status dots (`emerald`, `ruby`).
- [ ] Add Command Palette (`Cmd+K`) triggering a fuzzy search overlay for navigation.
- [ ] Wire the `?` icon to open the markdown `DocsPanel`.

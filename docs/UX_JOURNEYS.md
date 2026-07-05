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

- **Top Bar:** 
  - Application Logo.
  - Command Palette trigger hint (`Cmd/Ctrl+K`).
  - Global Settings (`⚙` icon).
  - Documentation/Help (`?` icon).
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
- **UI State:** `ProjectCreation` Form (Slim Modal).
- **Inputs:** Project Name, Environment (Local/Remote SSH), select Repositories via connected Provider, assign default Planner (Agent).
- **System Action:** Clones repos, detects default branch, PR template, CI setup.
- **Next Step:** Shows the user a "Proposed Worktree Strategy" (branch naming conventions, merge flow). The user can approve or edit.

### Journey 4: The Project Home
*The control center for a specific project.*
- **UI State:** `ProjectHome` (Main Pane).
- **Hero Element:** "Start a Feature" slim input modal.
- **Active Area:** Shows the currently running Feature (progress bar, current step, cost telemetry).
- **Queue/History:** A list of pending or completed (archived) features.
- **Repo Map:** A lazy-loaded, visual representation of the connected repositories and active feature branches.

### Journey 5: Starting a Feature
*Taking a user requirement and kicking off an automated workflow.*
- **UI State:** Slim Modal expands from Project Home.
- **Inputs:** User describes the feature in a textarea.
- **Auto-Inference:** The system locally matches keywords to suggest Repository Chips and detect conflicts (no LLM call in the modal).
- **Customization:** User clicks "Customize..." to expand the form and override default Workflows, Target Repos, Conflict Policies, or set Budget limits.
- **Pre-flight Validation:** Static checks display the workflow step list, potential risks, and repo fit. No cost is estimated here.
- **Submit:** Kicks off the Feature and transitions the view to `FeatureDetail`.

### Journey 6: Orchestration Monitoring (Feature Detail)
*Watching the fleet of agents work without chat.*
- **UI State:** `FeatureDetail` (Main Pane).
- **Visualization:** A DAG/Timeline view of the steps (e.g., `research` → `spec` → `plan` → `tasks` → `implement-stub`).
- **Telemetry:** Per-step cost ($) and duration (time) metrics. *(Note: No pre-launch cost estimates, only real-time accrued cost).*
- **Status Indicators:** Steps use the color language (Emerald=Running, Ruby=Failed, Violet=Active).
- **Actions:** Pause, Resume, or Cancel the feature.

### Journey 7: The Gate (Approval Workflow)
*Where the orchestrator pauses for human intervention.*
- **UI State:** `GateView` (Takes over the main pane or overlaps as a prominent card).
- **Content:**
  - **Planner Summary Card:** What the agents did and why.
  - **Artifacts:** Code diffs, written specs, or merge request summaries.
- **Actions:**
  - **Approve:** Continue execution.
  - **Redirect:** Open an input field to send feedback/corrections to the planner.
  - **Cancel:** Abort the current feature run.

### Journey 8: Handling Subtasks & Parallel Execution
*Breaking down work across multiple agents simultaneously.*
- **Trigger:** Workflow reaches a `parallel` step.
- **UI State:** Subtask execution list inside the `FeatureDetail` timeline.
- **Visuals:** A DAG or list of parallel tasks (one host, one agent per worktree). 
- **Feedback:** Continue-and-report semantics for failures. Shows error chips on failed subtasks, with an opt-in "Retry" button (with cost cap).

### Journey 9: Resolving Merge Conflicts
*Handling overlapping changes smartly.*
- **Trigger:** A subtask merge back into `feature/<slug>` fails.
- **System Flow:** Auto-agent tries to resolve first. If it fails, the workflow hits a Gate.
- **UI State:** `ConflictResolver` View.
- **Content:** A Monaco-based 3-way merge editor.
- **Actions:** User manually resolves code, saves, or chooses to Skip/Abort the subtask entirely.

### Journey 10: Workflow Authoring
*Creating the templates that agents follow.*
- **UI State:** `WorkflowEditor` View.
- **Content:** Form-first builder (v1.0) allowing users to piece together `agent`, `parallel`, and `gate` steps. 
- **Configuration:** Users set conditional edges (e.g., `on_failure -> goto`), max iterations, and artifact outputs (`full`, `summary_only`, `none`).

### Journey 11: Keyboard & Overlay Behavior (UX1.6)
*Predictable Escape behavior across stacked modals, palette, toasts, and drawers.*
- **Trigger:** Any keypress of `Escape`, regardless of which overlay (if any) currently holds focus.
- **Feature IDs:** F35 (single Escape dismisses topmost overlay only), F40 (priority order: `GateView > CommandPalette > modal > toast > sheet`).
- **Architecture:** A single `useOverlayEscape` listener (registered with `{ capture: true }`) consults `OverlayStackContext.top()` and dispatches to its `onEscape`. Every overlay registers itself through `useOverlay(stackId, opts)` with an explicit `tier` and `priority`.
- **Priority tiers (top → bottom):**
  1. `gate` — `GateView` (priority 100) — absolute top; never eclipsed.
  2. `palette` — `CommandPalette`, `DocsPanel` (priority 10) — outranks ordinary modals.
  3. `modal` — `StartFeatureModal`, `PromptDialog`, `ProviderSettings`, `EnvModal`, `ProjectSettingsShell` inner modals, `NewProjectView` repo picker, `ProvidersPage` connect modal, `FeatureDetail` replay-confirm (priority 50).
  4. `toast` — per-instance `ErrorToast`, `NotificationBell` (priority −10) — never blocks an Escape meant for a modal.
  5. `drawer` — `AgentTerminalDrawer`, `FeatureDetail` agent drawer (priority 0).
- **Gate-over-feature-modal precedence:** When `gate_required` fires while `StartFeatureModal` is open, the shell dispatches `CLOSE_OVERLAY('feature.start')` before navigating to the Gate. The Gate then renders alone; one Escape dismisses the Gate and returns to non-gated detail.
- **Cascade example:** With `GateView`, `CommandPalette`, `StartFeatureModal`, `DocsPanel`, and a visible `ErrorToast` all open, pressing Escape three times closes, in order: `GateView → CommandPalette → StartFeatureModal`. A fourth press closes `DocsPanel`; a fifth closes the `ErrorToast`.

#### UX1.6 Acceptance Checklist
Use this list as the binary pass/fail gate for every UX1.6 PR. Each item is verifiable mechanically (grep, dev-tools) or with a short manual keypress sequence.

- **Escape behavior**
  - [ ] Exactly one `keydown` listener for Escape survives in `src/` — `rg -nE "e\.key\s*===?\s*['\"]Escape['\"]" src/` returns only `src/hooks/useOverlayEscape.ts`.
  - [ ] No component file owns a `window.addEventListener('keydown', …)` for Escape — `rg -n "addEventListener\(['\"]keydown['\"]" src/` matches only `useOverlayEscape.ts` (and `useKeyboardShortcuts.ts` for non-Escape shortcuts).
  - [ ] Pressing Escape with an empty stack is a no-op (no `preventDefault`, no listener side effect).
  - [ ] Pressing Escape on a non-top entry does nothing — only `top().onEscape` fires.
  - [ ] Rapid Escape mashing (≥10 presses/sec) never throws and never pops more than one entry per keypress.
- **Focus restore**
  - [ ] When an overlay opens, focus moves to the first focusable element inside it (or the dialog root) — no focus leaks to the body.
  - [ ] When an overlay is dismissed (Escape or otherwise), focus returns to the element that opened it (or the most recent sensible focus target if the opener is gone).
  - [ ] Dismissing a lower-priority overlay while a higher-priority one is open restores focus **inside the higher-priority overlay**, not to the opener.
  - [ ] Focus restoration is not interrupted when the Escape handler is registered with `{ capture: true }`.
- **Tab trap**
  - [ ] `Tab` / `Shift+Tab` cycle inside the topmost overlay and do not escape to background controls.
  - [ ] Tab trap is re-established whenever a new overlay becomes `top()`; torn down on pop.
  - [ ] `Shift+Tab` from the first focusable element lands on the last, and `Tab` from the last lands on the first.
  - [ ] Tab trap survives the `gate_required` → `CLOSE_OVERLAY('feature.start')` handoff: focus is in the Gate, not the closed feature modal.
- **Z-index free**
  - [ ] No `z-index` literal is required to make the dismissal order work — overlays may render in any DOM order; correctness comes from the reducer's `(tierRank, priority, -createdAt)` sort.
  - [ ] Migrating a new overlay is a matter of adding a `useOverlay(stackId, { tier, priority, onEscape })` call — no styling changes.
  - [ ] Removing a higher-priority overlay while a lower one is open does not visually re-order the remaining overlays (render order is independent of stack top).

## 5. UI Views & Screens to Design

Based on the journeys above, designers must deliver the following discrete screen mockups:

1. **App Shell & Project Rail:** Base layout with cross-project navigation and the Command Palette (`Cmd+K`) active state.
2. **First Run / Empty State Card:** Onboarding flow centered around running a sample project.
3. **Project Creation & Worktree Strategy Forms:** Bootstrap flow with repo selection and strategy proposal.
4. **Project Home:** The main dashboard featuring the "Start a Feature" slim modal, active run status, and repository map.
5. **Feature Detail (Orchestration View):** The DAG timeline showing step progress, cost/duration telemetry, and subtask fan-outs.
6. **Gate View:** The critical human-in-the-loop screen showing the planner's summary, diffs, and Approve/Redirect buttons.
7. **Conflict Resolver:** An integrated Monaco 3-way merge editor within the app shell.
8. **Workflow Editor:** The form-based UI for creating/editing workflow steps.
9. **Settings & Preferences:** Global settings (theme, pricing tables) and Provider instances setup.

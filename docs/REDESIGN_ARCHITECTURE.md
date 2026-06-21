# Demeteo Redesign: Architecture

> **Source of truth for the multi-agent orchestrator's structure.** See
> [`REDESIGN_PLAN.md`](../REDESIGN_PLAN.md) for the master plan and
> [`REDESIGN_DDD_MODEL.md`](REDESIGN_DDD_MODEL.md) for the domain entities
> referenced here. This doc covers the hexagonal layout, the port surface,
> the file layout, the Tauri command surface, and the frontend state model.

## 1. The Hexagon (unchanged pattern, new ports)

```
+-----------------+     +-----------------------------------------------+     +------------------+
|  DRIVERS (UI)   |     |                  PORTS                        |     | DRIVEN ADAPTERS  |
|                 |     |                                               |     |                  |
|  React (Tauri   | ==> |  WorkflowRepository                           | <== |  SqliteAdapter   |
|  webview)       |     |  ProjectRepository                            |     |  SshClientAdapt. |
|  - ProjectRail  |     |  ProviderInstanceRepository                   |     |  LocalFsAdapter  |
|  - ProjectHome  |     |  FeatureOrchestrator / StepExecutor           |     |  CliRuntime       |
|  - FeatureDetail|     |  WorktreeManager / MergeExecutor / MrPublisher|     |  ArtifactStore   |
|  - WorkflowEdit |     |  AgentRuntime / PermissionPolicyPort          |     |  PricingTable    |
|  - Gates        |     |  UiStateRepository / DiskUsageCalculator      |     |                  |
|  - Settings     |     |                                               |     |                  |
+-----------------+     +-----------------------^-----------------------+
                                                 |
                                                 v
                   +-------------------------------------------------------+
                   |                       CORE DOMAIN                     |
                   |  - StepExecutor (small DAG engine)                    |
                   |  - FeatureOrchestrator (lifecycle + checkpoints)      |
                   |  - WorktreeStrategy + merge ordering                  |
                   |  - ConflictPolicy cascade                             |
                   |  - Pricing (model → cost)                             |
                   +-------------------------------------------------------+
```

The hexagonal pattern is preserved exactly. The big change is in the *driver* side: the React frontend is no longer a chat-style supervisor with per-turn event streams. It's a small set of focused views (ProjectRail, ProjectHome, FeatureDetail, GateView, WorkflowEditor, Settings) that consume step transitions, not agent transcripts.

## 2. Port Catalogue

### Carried from v1 (with simplifications)

- **`DatabasePort`** (`ports/db.rs`) — extends with the new tables; loses `thread_sessions` complexity. The legacy `thread_sessions` table is preserved in v1 for migration safety, marked deprecated, and removed in v2.
- **`AgentRuntime`** (`ports/agent_runtime.rs`) — `CliRuntime` (one-shot CLI + JSON-lines) for all agents. `opencode` and `hermes` use `opencode run --format json` / `hermes run --format json`. `claude-code` and `antigravity` use their existing `--print --output-format stream-json` / `--print -` modes. ACP is deleted. The trait surface stays the same (same callers in `StepExecutor`); only the implementation changes.
- **`PermissionPolicyPort`** (`ports/permission_policy.rs`) — renders a `PermissionPolicy` struct to a JSON string for the `OPENCODE_PERMISSION` env var. Default impl: `WorktreeScopedPolicy` injects `external_directory: "deny"` so the agent cannot touch paths outside its worktree.
- **`ExecutionPort`** (`ports/execution.rs`) — `spawn_interactive` used only for remote agent processes (local agents use `tokio::process::Command` directly).
- **`NotificationPort`** (`ports/notification.rs`) — slimmed. Per-turn streams removed; telemetry events only.

### New ports (v1)

- **`WorkflowRepository`** (`ports/workflow_repo.rs`)
  - `workflow_create(workflow) -> Result<WorkflowId>`
  - `workflow_update(workflow) -> Result<()>`
  - `workflow_delete(workflow_id) -> Result<()>`
  - `workflow_get(workflow_id) -> Result<Workflow>`
  - `workflow_list() -> Result<Vec<WorkflowSummary>>`
  - `workflow_save_version(workflow_id, version, json_blob, note) -> Result<VersionId>`
  - `workflow_versions(workflow_id) -> Result<Vec<WorkflowVersion>>`
  - `workflow_revert_to_version(workflow_id, version_id) -> Result<()>`
- **`ProjectRepository`** (`ports/project_repo.rs`)
  - `project_create(project) -> Result<ProjectId>`
  - `project_update(project) -> Result<()>`
  - `project_delete(project_id) -> Result<()>`
  - `project_get(project_id) -> Result<Project>`
  - `project_list() -> Result<Vec<ProjectSummary>>`
  - `repository_add(project_id, repo) -> Result<RepoId>`
  - `repository_remove(project_id, repo_id) -> Result<()>`
  - `repository_list(project_id) -> Result<Vec<Repository>>`
- **`ProviderInstanceRepository`** (`ports/provider_repo.rs`)
  - `provider_connect(kind, host, pat) -> Result<ProviderInstanceId>` (validates the PAT first)
  - `provider_disconnect(id) -> Result<()>`
  - `provider_list() -> Result<Vec<ProviderInstance>>`
  - `provider_validate(id) -> Result<ProviderUserInfo>` (re-runs the `/user` call; updates display name)
- **`FeatureOrchestrator`** (`ports/feature_orchestrator.rs`)
  - `feature_start(project_id, workflow_id, spec) -> Result<FeatureId>`
  - `feature_pause(feature_id) -> Result<()>`
  - `feature_resume(feature_id) -> Result<()>`
  - `feature_cancel(feature_id) -> Result<()>`
  - `feature_get(feature_id) -> Result<Feature>`
  - `feature_list(project_id) -> Result<Vec<FeatureSummary>>`
  - `feature_archive(feature_id) -> Result<()>`
  - `feature_restore(feature_id) -> Result<()>` (archive → completed)
  - `feature_rerun(feature_id) -> Result<FeatureId>` (creates a new FeatureRun)
- **`StepExecutor`** (`ports/step_executor.rs`)
  - `step_get(execution_id) -> Result<StepExecution>`
  - `step_retry(execution_id) -> Result<()>` (opt-in per-step retry, planner-driven)
  - `step_list_for_run(run_id) -> Result<Vec<StepExecution>>`
- **`GatePresenter`** (lives in `StepExecutor`'s surface but has its own port for testability)
  - `gate_pending_for_run(run_id) -> Result<Option<GateDecision>>`
  - `gate_decide(execution_id, decision, feedback) -> Result<()>`
- **`WorktreeManager`** (`ports/worktree_mgr.rs`)
  - `worktree_create_feature_branch(project_id, slug) -> Result<BranchName>`
  - `worktree_provision_subtask(run_id, subtask_id) -> Result<SubtaskWorktree>`
  - `worktree_cleanup_subtask(subtask_id) -> Result<()>`
  - `worktree_list_for_feature(feature_id) -> Result<Vec<SubtaskWorktree>>`
- **`MergeExecutor`** (`ports/worktree_mgr.rs`)
  - `merge_subtask_into_feature(subtask_id) -> Result<MergeResult>` (may produce ConflictReport)
  - `merge_rebase_subtask(subtask_id) -> Result<RebaseResult>` (rebase before merge)
  - `merge_topological_order(run_id) -> Result<Vec<SubtaskId>>` (DAG-derived order)
- **`MrPublisher`** (`ports/worktree_mgr.rs`)
  - `mr_publish(feature_id, draft, auto_merge) -> Result<MrUrl>`
  - `mr_get_status(feature_id) -> Result<MrStatus>`
- **`ConflictResolver`** (`ports/worktree_mgr.rs`)
  - `conflict_resolve_agent(subtask_id) -> Result<Resolution>` (spawns resolution subtask)
  - `conflict_resolve_manual(subtask_id, resolution) -> Result<()>`
  - `conflict_get_report(subtask_id) -> Result<ConflictReport>`
- **`ArtifactStore`** (`ports/artifact_store.rs`)
  - `artifact_write(feature_id, step, content) -> Result<Path>`
  - `artifact_read(feature_id, step) -> Result<String>`
  - `artifact_list(feature_id) -> Result<Vec<ArtifactRef>>`
  - `artifact_glob(feature_id, pattern) -> Result<Vec<ArtifactRef>>`
- **`PricingTable`** (`ports/pricing.rs`)
  - `cost_for(model, input_tokens, output_tokens) -> Result<Option<Cost>>`
  - `models_known() -> Result<Vec<ModelPricing>>`
  - `pricing_set(model, cost) -> Result<()>` (user override)
- **`UiStateRepository`** (`ports/ui_state.rs`)
  - `ui_pref_get(key) -> Result<Option<JsonValue>>`
  - `ui_pref_set(key, value) -> Result<()>`
  - `ui_pref_list() -> Result<Vec<(String, JsonValue)>>`
- **`DiskUsageCalculator`** (`ports/ui_state.rs`)
  - `disk_usage_global() -> Result<DiskUsageReport>`
  - `disk_usage_project(project_id) -> Result<DiskUsageReport>`
- **`DocsRepository`** (`ports/ui_state.rs`)
  - `docs_list() -> Result<Vec<DocsEntry>>`
  - `docs_get(slug) -> Result<String>` (markdown content)

## 3. Directory Layout

```
src-tauri/src/
├── main.rs
├── lib.rs
├── domain/
│   ├── mod.rs
│   ├── models.rs                 # all entities + value objects
│   ├── provider.rs               # ProviderInstance, ProviderKind
│   ├── project.rs                # Project, Repository, WorktreeStrategy
│   ├── workflow.rs               # Workflow, WorkflowVersion, StepConfig
│   ├── feature.rs                # Feature, FeatureRun, StepExecution, GateDecision
│   ├── worktree.rs               # SubtaskRun, SubtaskMerge, MergeStrategy
│   ├── conflict.rs               # ConflictReport, ConflictPolicy
│   └── pricing.rs                # PricingTable, model cost
├── ports/
│   ├── mod.rs
│   ├── db.rs                     # DatabasePort (extends with new tables)
│   ├── execution.rs              # ExecutionPort (carries spawn_interactive)
│   ├── agent_runtime.rs          # AgentRuntime (carried from v1)
│   ├── workflow_repo.rs          # NEW: WorkflowRepository, WorkflowVersionRepository
│   ├── project_repo.rs           # NEW: ProjectRepository
│   ├── provider_repo.rs          # NEW: ProviderInstanceRepository
│   ├── feature_orchestrator.rs   # NEW: FeatureOrchestrator
│   ├── step_executor.rs          # NEW: StepExecutor + GatePresenter
│   ├── worktree_mgr.rs           # NEW: WorktreeManager, MergeExecutor, MrPublisher, ConflictResolver
│   ├── artifact_store.rs         # NEW: ArtifactStore
│   ├── pricing.rs                # NEW: PricingTable
│   ├── notification.rs           # NotificationPort (slimmed)
│   └── ui_state.rs               # NEW: UiStateRepository, DiskUsageCalculator, DocsRepository
├── adapters/
│   ├── mod.rs
│   ├── database/
│   │   ├── mod.rs
│   │   └── sqlite.rs             # all new tables; carries legacy
│   ├── ssh/
│   │   ├── mod.rs
│   │   └── client.rs             # carries; adds per-feature worktree helpers
│   ├── local/                    # NEW: local FS + subprocess adapters
│   │   ├── mod.rs
│   │   ├── fs.rs
│   │   └── pty.rs                # (existing)
│   ├── agent/                    # carried from v1, scoped to feature step
│   │   ├── mod.rs
│   │   ├── registry.rs            # simplified: no session dedup
│   │   ├── cli_runtime.rs         # (existing) one-shot CLI + JSON-lines
│   │   ├── permission_policy.rs   # NEW: PermissionPolicyPort impl
│   │   ├── opencode/mod.rs         # CliAgentRuntime + parse_opencode_event
│   │   ├── hermes/mod.rs           # CliAgentRuntime + parse_hermes_event
│   │   ├── claude_code/mod.rs      # NEW: CliAgentRuntime + parse_claude_code_event
│   │   └── antigravity/mod.rs     # NEW: CliAgentRuntime + parse_antigravity_event
│   ├── workflow/                 # NEW: workflow catalog adapters
│   │   ├── mod.rs
│   │   ├── json_format.rs        # import/export
│   │   └── starter_pack.rs       # bundled JSON files
│   ├── worktree/                 # NEW: worktree + merge + publish
│   │   ├── mod.rs
│   │   ├── git_ops.rs
│   │   ├── merge.rs
│   │   ├── conflict.rs
│   │   └── publish.rs
│   ├── pricing/                  # NEW: hard-coded + editable pricing
│   │   ├── mod.rs
│   │   └── table.rs
│   └── tauri_ui/                 # carried from v1
│       ├── mod.rs
│       ├── commands.rs           # new Tauri commands for the new ports
│       └── events.rs             # slimmed event set
└── plugins/                      # deferred (kept empty)

src/
├── main.tsx
├── App.tsx                       # rewritten
├── App.css
├── types.ts                      # new types
├── commandPalette.ts             # NEW
├── uiPrefs.ts                    # NEW
└── components/
    ├── ProjectRail.tsx           # NEW (cross-project nav)
    ├── ProjectHome.tsx           # NEW (current feature + queue + repo map)
    ├── ProjectSettings.tsx       # NEW (per-project config)
    ├── FeatureDetail.tsx         # NEW (step timeline + telemetry)
    ├── GateView.tsx              # NEW (planner summary + artifact list)
    ├── WorkflowEditor.tsx        # NEW (form-based step editor)
    ├── WorkflowList.tsx          # NEW
    ├── StartFeatureModal.tsx     # NEW (slim modal w/ inferred chips)
    ├── PreFlightPanel.tsx        # NEW (step list + risks + repo fit)
    ├── ProviderSettings.tsx      # NEW (per-provider-instance config)
    ├── PreferencesScreen.tsx     # NEW (global Preferences)
    ├── EmptyStateCard.tsx        # NEW (state-driven first-run UX)
    ├── DocsPanel.tsx             # NEW (bundled markdown viewer)
    ├── ConflictResolver.tsx      # NEW (Monaco 3-way merge)
    ├── CommandPalette.tsx        # NEW (Cmd/Ctrl+K)
    └── ... (carries: Sidebar, TerminalTabs, SSHTerminal, EnvModal → PreferencesScreen)

src/docs/                         # NEW: bundled markdown
├── index.md
├── first-project.md
├── how-workflows-work.md
├── connecting-providers.md
├── feature-branch-model.md
└── conflict-resolution.md
```

## 4. Tauri Command Surface

### New commands (one per port method the UI calls)

- `project_create`, `project_update`, `project_delete`, `project_list`, `project_get`
- `repository_add`, `repository_remove`, `repository_list`
- `provider_connect`, `provider_disconnect`, `provider_list`, `provider_validate`
- `workflow_create`, `workflow_update`, `workflow_save_version`, `workflow_export`, `workflow_import`, `workflow_list`, `workflow_get`, `workflow_versions`, `workflow_revert_to_version`
- `feature_start`, `feature_pause`, `feature_resume`, `feature_cancel`, `feature_get`, `feature_list`, `feature_archive`, `feature_restore`, `feature_rerun`
- `step_get`, `step_retry`, `step_list_for_run`
- `gate_pending_for_run`, `gate_decide`
- `worktree_list_for_feature`
- `merge_topological_order`
- `conflict_get_report`, `conflict_resolve_agent`, `conflict_resolve_manual`
- `mr_publish`, `mr_get_status`
- `artifact_get`, `artifact_list`, `artifact_glob`
- `disk_usage` (project + global)
- `migration_log` (read)
- `preflight_validate` (static checks)
- `ui_pref_get`, `ui_pref_set`
- `docs_list`, `docs_get`

### Modified commands (carried with reduced payload)

- `add_thread_session` → REMOVED (replaced by `feature_start`)
- `request_action` → REMOVED from UI path; kept as an internal port for the tool bridge if needed
- All SFTP/SSH commands → kept (read/write files for Monaco editor and worktree ops)

### Removed events

- `permission_requested` (no intercept UX in v1; conflict resolution is the new gate)
- `command_executed` (no chat stream)

### New events

- `feature_status_changed` (per-feature state transitions)
- `step_progress` (heartbeat, optional, throttled to 1Hz)
- `gate_required` (a gate needs user attention; the UI navigates to the gate view)
- `conflict_detected` (a merge conflict needs resolution)
- `migration_progress` (visible migration indicator for additive v1.x updates)

## 5. Frontend State Model (React, simplified)

The frontend is one stateful app, not a multi-pane chat UI:

- `currentProjectId` (drives the main pane)
- `featureDetail` (the active feature, if any; loaded on demand)
- `gateView` (the current gate's data, when a gate is active)
- `uiPrefs` (theme, accent, collapse state — persisted)
- `commandPaletteOpen` (boolean)

No per-thread session registry. No per-turn `Channel<AgentEvent>` stream. The agent session is now scoped to a step execution; the UI gets step transitions as events, not streams.

### Top-level navigation (one shell)

```
┌────────────────────────────────────────────────────────────┐
│  [≡] Demeteo                  [⌘K]  [⚙]  [?]               │  ← top bar
├──────────┬─────────────────────────────────────────────────┤
│          │                                                  │
│ Project  │   <main pane: current project>                  │
│ Rail     │   - ProjectHome (default)                       │
│          │   - FeatureDetail (when a feature is active)    │
│ [search] │   - GateView (when a gate is active)            │
│ • Proj A │   - WorkflowEditor (when editing a workflow)    │
│ • Proj B │   - PreferencesScreen (when opened)             │
│ • Proj C │                                                  │
│          │                                                  │
│ [+ New]  │                                                  │
│ [⚙ Mng]  │                                                  │
└──────────┴─────────────────────────────────────────────────┘
```

The "Mng" button at the bottom of the rail opens a project list / create / delete view (a full-page Preferences screen for project management). The "⚙" at the top opens global Preferences. The "?" opens the docs panel.

## 6. Migration Strategy (Q30)

- **v1.0 ships greenfield**: single init script `migrations/0001_initial.sql`. The legacy `thread_sessions` table is *not* created — we start clean.
- **v1.x (additive)**: silent auto-migration. New tables, new nullable columns, new indexes. No user prompt.
- **v2.0+ (breaking)**: schema version check on launch. If behind on a breaking migration, demeteo offers "wipe and re-init" with a confirmation prompt. The old DB is moved to `demeteo.db.wiped.<timestamp>`. The user can pre-export workflows + projects to JSON to re-import after the wipe.
- **Pre-migration backup**: `cp demeteo.db demeteo.db.bak.<timestamp>` before any migration runs. 7-day retention, auto-pruned.
- **Migration log**: `~/.local/share/demeteo/migrations.log`, always written, viewable from Preferences → Storage.

See `REDESIGN_PLAN.md` §4 (Phase R8) for the hardening phase.

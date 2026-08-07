# Demeteo: Architecture

> **Source of truth for the multi-agent orchestrator's structure.** See
> [`DECISIONS.md`](DECISIONS.md) for the master decisions table and
> [`DDD_MODEL.md`](DDD_MODEL.md) for the domain entities
> referenced here. This doc covers the hexagonal layout, the port surface,
> the file layout, the Tauri command surface, and the frontend state model.

## 1. The Hexagon

```
+-----------------+     +-----------------------------------------------+     +------------------+
|  DRIVERS (UI)   |     |                  PORTS                        |     | DRIVEN ADAPTERS  |
|                 |     |                                               |     |                  |
|  React (Tauri   | ==> |  DatabasePort (split into 11 sub-traits)     | <== |  SqliteAdapter   |
|  webview)       |     |  StepExecutor / GatePresenter                 |     |  SshClientAdapt. |
|  - ProjectRail  |     |  WorktreeOps / Merge / Conflict / MrPublisher |     |  LocalFsAdapter  |
|  - ProjectHome  |     |  AgentRuntime / AgentExecutionPort            |     |  CliRuntime      |
|  - FeatureDetail|     |  ExecutionPort / NotificationPort             |     |  ArtifactStore   |
|  - GateView     |     |  PricingTable / MemoryLlmPort                  |     |  FsAttachment    |
|  - WorkflowEdit |     |  ProviderHttpPort / ArtifactStore              |     |                  |
|  - Settings     |     |                                               |     |                  |
+-----------------+     +-----------------------^-----------------------+
                                                  |
                                                  v
                    +-------------------------------------------------------+
                    |                       CORE DOMAIN                     |
                    |  - StepExecutor driver (adapters/step_executor/driver)|
                    |  - AgentSession lifecycle (one-shot CLI + JSON-lines) |
                    |  - Worktree ops (clone / branch / merge / sync)       |
                    |  - Inline conflict resolution (steps/conflict_pass)   |
                    |  - StepCapability -> PermissionProfile + WriteScope   |
                    |  - Pricing (model -> cost)                            |
                    +-------------------------------------------------------+
```

The hexagonal pattern is preserved. The driver side is a small set of
focused React views (`ProjectRail`, `ProjectHome`, `FeatureDetail`,
`GateView`, `WorkflowBuilderScreen`, `Settings`) that consume step transitions,
not agent transcripts. The port side is split into narrow,
bounded-context-aligned traits (see §2). The core domain holds the
step executor driver, the agent session lifecycle, and the worktree
merge/sync machinery.

## 2. Port Catalogue

### Database port (split into sub-traits)

The original `DatabasePort` trait carried 66 methods spanning 13 domains.
It is split in [`src-tauri/src/ports/db.rs`](../../src-tauri/src/ports/db.rs)
into eleven narrow sub-ports aligned with the bounded contexts in
[`DDD_MODEL.md`](DDD_MODEL.md):

| Sub-port                  | Bounded context | Owns                                         |
|---------------------------|-----------------|----------------------------------------------|
| `MachineRepository`       | machines        | `Machine`, `AgentProfile`                    |
| `ThreadRepository`        | threads         | `ThreadSession`, `Message`, `AgentConfig`, `WorkingMemoryEntry` |
| `ProjectRepository`       | projects        | `Project`, `Repository`, `ProjectSettings`, `ProjectWorkflowOverride` |
| `FeatureRepository`       | features        | `Feature`, `StepExecution`                   |
| `SequenceResumeRepository`| sequence resume | `SequenceCheckpoint` + the sequence plan cache — a `sequence` step's durable resume point, keyed per (feature, node) |
| `WorkflowRepository`      | workflows       | `Workflow`, `WorkflowVersion`, `WorkflowSchedule` |
| `GateRepository`          | gates           | `GateDecision`                               |
| `AppSettingsRepository`   | app settings    | `ProviderInstance`, app-session KV, first-launch flags |
| `MergeAuditRepository`    | merge audit     | `record_merge_outcome`, `record_sync_outcome`, worktree/repo context lookup |
| `SubtaskRunRepository`    | subtask runs    | `SubtaskRunRow` — per-task run telemetry for a `sequence` step (`subtask_run_start` / `_finish` / `_interrupt_stale`) |
| `NotificationRepository`  | notifications   | `Notification` (bell cache)                  |

Each sub-port is small (≤ 12 methods), cohesive, and takes strongly-typed
ID newtypes (`MachineId`, `ProjectId`, `FeatureId`, `StepExecutionId`,
…). `AppContext` holds one `Arc<dyn …Repository>` per sub-port, and
Tauri commands extract only the sub-port they need.

### Mutation through Patch value objects

`ThreadPatch`, `FeaturePatch`, `StepExecutionPatch` (`ports/db.rs:48-104`)
replace the previous multi-argument `step_execution_update_status`,
`update_thread_status`, etc. Each field is `Option<Option<T>>` so callers
distinguish "leave alone" from "set to NULL".

### Carried ports

- **`AgentRuntime`** (`ports/agent_runtime.rs`) — `CliRuntime` (one-shot
  CLI + JSON-lines) for all agents, each declaring its `AgentCapabilities`.
  `opencode` and `hermes` use
  `opencode run --format json` / `hermes run --format json`; `claude-code`
  uses `claude --print --output-format stream-json`. ACP is removed. The
  trait surface stays dyn-safe (`start` returns a boxed `AgentStartFuture`).
- **`AgentExecutionPort`** (`ports/agent_execution.rs`) — `submit` /
  `submit_agent` / `approve` / `reject` for hand-rolled and
  agent-originated tool actions. `submit_agent` returns a typed
  `ActionError { Network, NotFound, Internal }` (three variants).
- **`ExecutionPort`** (`ports/execution.rs`) — `spawn_interactive` used
  only for remote agent processes (local agents use
  `tokio::process::Command` directly).
- **`WorktreeOpsPort`** (`ports/worktree_ops.rs`) — worktree primitives
  (`clone_repository`, `create_feature_branch`,
  `provision_subtask_worktree`, `cleanup_subtask_worktree`,
  `branch_delete`, `merge_subtask`, `sync_feature_with_upstream`).
- **`TrustedWorktreePort`** (`ports/worktree_ops.rs`) — the future narrow
  boundary for terminal-worktree creation/removal and dependency-cache
  materialization beneath Demeteo-owned roots. Its no-follow and transport
  contract is in [`TRUSTED_WORKTREE.md`](TRUSTED_WORKTREE.md); no adapter or
  caller implements it yet.
- **`MergePort`** (`ports/merge.rs`) — the feature ↔ upstream sync flow
  (`sync_feature_with_upstream`, `feature_syncs` audit rows). Task-branch
  merges are *not* a port: the steps that own the worktree merge inline
  via `GitOpsHelper::merge_subtask` and resolve conflicts with
  `steps/conflict_pass` (see decision 20's history in `DECISIONS.md` —
  the R6 cascade port was deleted as never-called).
- **`MrPublisher`** (`ports/mr_publisher.rs`) — publish a draft MR/PR
  via the project's provider instance; returns `MrInfo`.
- **`ProviderHttpPort`** (`ports/provider_http.rs`) — typed wrapper
  over `reqwest` + keyring for `validate_provider_pat` /
  `fetch_provider_repos`.
- **`ArtifactStore`** (`ports/artifact_store.rs`) and
  **`AttachmentStore`** (`ports/attachment_store.rs`) — durable artifact
  and attachment storage (filesystem-backed adapters).
- **`PricingTable`** (`ports/pricing.rs`) — `cost_for(model, in, out)`,
  `models_known`, `pricing_set`, `context_window`.
- **`MemoryPort`** (`ports/memory.rs`),
  **`MemoryLlmPort`** (`ports/memory_llm.rs`),
  **`MemorySignalsPort`** (`ports/memory_signals.rs`) — typed inputs for
  the opt-in Memory Agent. The LLM port points at a user-configured
  OpenAI-compatible endpoint; it is the one place Demeteo calls a model
  provider directly.
- **`NotificationPort`** (`ports/notification.rs`) — telemetry events
  to the React bell and step timeline; the per-feature event stream is
  not consumed by the UI.

### Step execution and gate ports

The orchestrator's command surface for features is one trait:

- **`StepExecutor`** (`ports/step_executor.rs`) — `feature_start`,
  `feature_pause`, `feature_resume`, `feature_cancel`, `feature_sync`,
  `feature_resolve_sync_conflicts`, `step_get`, `step_retry`,
  `replay_from_step`, `step_list_for_run`. `feature_start` accepts
  per-feature overrides (`agent_kind`, `model`, `commit_artifacts`,
  `loop_iterations`, `step_overrides`, `staged_attachments`) and an
  optional `FeaturePatch`-driven override for the feature row.
  `feature_sync` and `feature_resolve_sync_conflicts` return
  `SyncOutcomeView::{Ok, Conflict, Resolved, ResolutionFailed}` so the
  React side can render the outcome without re-parsing the database.
- **`GatePresenter`** (`ports/step_executor.rs`) — `gate_pending_for_run`,
  `gate_decide`. Both ports are async (Tauri v2 supports async commands
  natively); the previous `block_in_place` wrappers are removed.

### Permission policy ports

`PermissionPolicyPort` (no separate file) is folded into the agent
runtime. The compiled policy is a four-axis
[`PermissionProfile`](../../src-tauri/src/domain/permission.rs) —
`read_fs`, `write_fs`, `execute`, `network`, each `Access::{Allow,
Deny}` — plus a path-shaped `WriteScope::{None, ArtifactsOnly, All}`
that the OS-level chmod fence (`adapters/worktree/git_ops/scope.rs`)
turns into concrete writable paths. The scope and per-step
`allow_network` / `allow_shell` overrides flow from
`StepCapability::{ReadOnly, Artifacts, Verify, Implement}`; the
runtime translates the abstract profile to its native dialect
(opencode → `OPENCODE_PERMISSION` env; claude-code →
`--disallowedTools`; hermes → `OPENCODE_PERMISSION` env).

## 3. Directory Layout

```
src-tauri/src/
├── main.rs
├── lib.rs
├── state.rs                       # AppContext / AppState (one Arc<dyn ...Repository> per sub-port)
├── db.rs                          # SQLite connection + query helpers
├── domain/
│   ├── ids.rs                     # ID newtypes (MachineId, ProjectId, FeatureId, …)
│   ├── models/                    # Split by bounded context (was domain/models.rs)
│   │   ├── mod.rs
│   │   ├── agent_config.rs
│   │   ├── feature.rs             # Feature, StepExecution, SubtaskRun, GateDecision, StepOverride
│   │   ├── machine.rs             # Machine, AgentProfile
│   │   ├── merge.rs               # FeatureSync, ConflictReport, UpstreamSyncOutcome/Failure
│   │   ├── notification.rs
│   │   ├── project.rs             # Project, Repository, WorktreeStrategy, ProjectSettings, ProjectWorkflowOverride
│   │   ├── provider.rs
│   │   ├── thread.rs              # ThreadSession, Message, WorkingMemoryEntry
│   │   ├── timeouts.rs            # AgentTimeouts
│   │   └── workflow.rs            # Workflow, WorkflowVersion, WorkflowSchedule
│   ├── permission.rs              # StepCapability, PermissionProfile, WriteScope, Access, resolve_profile
│   ├── agent_event.rs             # AgentEvent enum (Text, ToolCall, ToolCallUpdate, Plan, Usage, Error, TurnComplete, ModeChanged, ConfigChanged, ArtifactProduced)
│   ├── artifact.rs                # Artifact
│   ├── attachment.rs              # AttachedFile
│   ├── action.rs                  # ActionKind, AgentAction
│   ├── intercept.rs               # ExecutionResult, InterceptPayload
│   ├── prompt_context.rs
│   ├── text.rs
│   ├── usage.rs                   # UsageAccumulator
│   └── verifier.rs
├── ports/
│   ├── mod.rs
│   ├── db.rs                      # Eleven sub-ports + Patch value objects
│   ├── execution.rs
│   ├── agent_runtime.rs           # AgentRuntime + AgentSession + opencode_permission_env / no_permission_env
│   ├── agent_execution.rs         # AgentExecutionPort + CommandOutcome + ActionError
│   ├── step_executor.rs           # StepExecutor + GatePresenter + SyncOutcomeView
│   ├── worktree_ops.rs            # WorktreeOpsPort + MergePreCheck
│   ├── merge.rs
│   ├── conflict.rs
│   ├── mr_publisher.rs
│   ├── artifact_store.rs
│   ├── attachment_store.rs
│   ├── provider_http.rs
│   ├── pricing.rs
│   ├── memory.rs
│   ├── memory_llm.rs
│   ├── memory_signals.rs
│   └── notification.rs
├── adapters/
│   ├── mod.rs
│   ├── database/                  # SQLite-backed implementations of the eleven sub-ports
│   ├── ssh/                       # SSH transport (keyring, ssh2)
│   ├── local/                     # Local FS + subprocess adapters
│   ├── agent/
│   │   ├── mod.rs
│   │   ├── cli_runtime.rs         # UnifiedCliRuntime (binary, install_cmd, parse_event, build_args, perm_env)
│   │   ├── registry.rs            # AgentRegistry (simplified, no session dedup)
│   │   ├── opencode/mod.rs        # CliRuntime + parse_opencode_event
│   │   ├── hermes/mod.rs          # CliRuntime + parse_hermes_event
│   │   ├── claude_code/mod.rs     # CliRuntime + parse_claude_code_event + disallowed_tools_for
│   │   ├── install.rs             # agent_install_and_start
│   │   ├── direct_execution.rs
│   │   ├── noop.rs
│   │   ├── test_stubs.rs
│   │   └── event_stream/
│   │       ├── mod.rs
│   │       ├── turn.rs            # stream_agent_turn (tokio::select! body)
│   │       └── cleanup.rs
│   ├── worktree/
│   │   ├── mod.rs
│   │   └── git_ops/
│   │       ├── mod.rs
│   │       ├── clone.rs
│   │       ├── strategy.rs
│   │       ├── worktree.rs
│   │       ├── merge.rs
│   │       ├── sync.rs
│   │       ├── health.rs
│   │       ├── scope.rs           # chmod fence driven by WriteScope
│   │       └── tests.rs
│   ├── artifact_store/
│   ├── attachment_store/
│   ├── memory_worker.rs
│   ├── memory_llm.rs
│   ├── mr_monitor.rs              # background MR-state poller
│   ├── mr_publisher/
│   │   ├── mod.rs
│   │   ├── github.rs
│   │   ├── gitlab.rs
│   │   ├── http.rs                # HttpClient + ReqwestHttp
│   │   └── push.rs                # token-free origin + GIT_ASKPASS branch push
│   ├── pricing.rs
│   ├── provider_http.rs
│   ├── conflict.rs
│   ├── merge.rs
│   ├── router.rs                  # RouterExecutionPort (local/remote dispatch)
│   ├── scheduler.rs
│   ├── step_executor/
│   │   ├── mod.rs
│   │   ├── driver.rs              # ExecutionDriver (re-entry, terminal status, watchdog)
│   │   ├── driver/
│   │   │   ├── failure.rs         # fail_step_and_feature, StepOutcome variants
│   │   │   └── verifier.rs        # on_failure -> goto, max_iterations
│   │   ├── driver_registry.rs
│   │   ├── steps/
│   │   │   ├── mod.rs
│   │   │   ├── agent/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── spawn.rs
│   │   │   │   ├── artifacts.rs
│   │   │   │   └── error_message.rs
│   │   │   ├── gate.rs
│   │   │   ├── parallel/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── planner.rs
│   │   │   │   ├── subtask.rs
│   │   │   │   └── list_unmerged.rs
│   │   │   └── sync.rs
│   │   ├── artifacts/
│   │   ├── gate_waiter.rs
│   │   ├── impl_traits/
│   │   ├── setup.rs
│   │   ├── sync.rs
│   │   ├── tests/
│   │   └── updates.rs
│   ├── tauri_ui/                  # Tauri commands + event adapters
│   │   ├── mod.rs
│   │   ├── commands.rs            # thin IPC handlers grouped by bounded context
│   │   └── events.rs              # slim event set
│   └── … (sftp, forward, terminal, ssh client adapters)
├── application/                   # Use cases / application services
│   ├── mod.rs
│   ├── agents.rs
│   ├── agent_probe.rs
│   ├── bootstrap.rs
│   ├── lifecycle.rs
│   ├── memory.rs
│   ├── projects.rs
│   ├── providers.rs
│   └── timeouts.rs
├── composition/                   # Composition root (wires ports to adapters, builds AppContext)
│   └── mod.rs
├── error/                         # AppError + IPC error envelope
│   ├── mod.rs
│   └── ipc.rs
├── shared/                        # Cross-cutting utilities (no domain logic)
│   ├── mod.rs
│   ├── ids.rs
│   ├── proc.rs
│   ├── shell.rs
│   └── time.rs
├── infrastructure/                # OS-level infrastructure (worktree helper, etc.)
│   ├── mod.rs
│   └── worktree/
├── commands/                      # Thin Tauri command handlers
│   ├── mod.rs
│   ├── agent_config.rs
│   ├── agent_config_probe.rs
│   ├── agent_exec.rs
│   ├── agent_lifecycle.rs
│   ├── agent_profile.rs
│   ├── app_session.rs
│   ├── app_version.rs
│   ├── attachments.rs
│   ├── bootstrap.rs
│   ├── feature_lifecycle.rs
│   ├── features.rs
│   ├── git.rs
│   ├── machine.rs
│   ├── memory.rs
│   ├── messages.rs
│   ├── mr_publisher.rs
│   ├── notifications.rs
│   ├── pricing.rs
│   ├── project.rs
│   ├── providers.rs
│   ├── ssh.rs
│   ├── thread.rs
│   ├── timeouts.rs
│   └── workflows.rs
├── sftp.rs
├── ssh_util.rs
├── terminal.rs
├── forward.rs
└── paths.rs

src-tauri/
└── migrations/                    # SQL migrations (refinery), V1–V19+

src/                              # React frontend
├── main.tsx
├── App.tsx
├── types.ts
├── components/
│   ├── AgentTerminalDrawer.tsx
│   ├── ArtifactViewer.tsx
│   ├── AttachmentChip.tsx
│   ├── AttachmentDropzone.tsx
│   ├── CodeEditorView.tsx
│   ├── CommandPalette.tsx
│   ├── CommandSelector.tsx
│   ├── DocsPanel.tsx
│   ├── EmptyStateCard.tsx
│   ├── EnvModal.tsx
│   ├── ErrorToast.tsx
│   ├── FeatureDetail.tsx
│   ├── GateView.tsx
│   ├── MachinesView.tsx
│   ├── MemoryAgentSettings.tsx
│   ├── NewProjectView.tsx
│   ├── NotificationBell.tsx
│   ├── PreferencesScreen.tsx
│   ├── ProjectHome.tsx
│   ├── ProjectRail.tsx
│   ├── ProjectSettings.tsx
│   ├── PromptDialog.tsx
│   ├── ProviderSettings.tsx
│   ├── ProvidersPage.tsx
│   ├── settings/
│   ├── Sidebar.tsx
│   ├── StartFeatureModal.tsx
│   ├── TerminalStatusOverlay.tsx
│   ├── TerminalWindow.tsx
│   ├── TopBar.tsx
│   ├── ui/
│   └── WorkflowList.tsx
├── lib/                           # Typed Tauri IPC wrappers (no raw invoke() in components)
│   ├── agentModels.ts
│   ├── appInfo.ts
│   ├── appVersion.ts
│   ├── attachments.ts
│   ├── errorBus.tsx
│   ├── errors.ts
│   ├── features.ts                # findActivePredecessor, listBlockingPredecessor
│   ├── featureSync.ts
│   ├── modelImageSupport.ts
│   ├── notifications.ts
│   ├── project.ts
│   ├── terminal.ts
│   ├── timeouts.ts
│   └── utils.ts
├── hooks/
└── docs/                          # Bundled user-facing help markdown (out of scope for engineering docs)
```

## 4. Tauri Command Surface

The commands registered in [`src-tauri/src/lib.rs:461-585`](../../src-tauri/src/lib.rs)
are grouped by bounded context. All return `Result<T, String>` (or
`AppError` for the step executor where validation messages matter).

### Machines / agent profiles

- `get_machines`, `add_machine`, `delete_machine`, `update_machine`,
  `test_machine_connection`
- `get_agent_profiles`, `add_agent_profile`, `delete_agent_profile`
- `get_agent_configs`, `set_agent_configs`, `get_working_memory`,
  `clear_working_memory`
- `get_agent_models` (probe), `set_agent_timeouts`, `get_agent_timeouts`

### Threads, messages, attachments, sessions

- `get_thread_sessions`, `add_thread_session`, `update_thread_status`,
  `delete_thread_session`
- `get_messages`, `append_message`
- `feature_add_attachment`, `feature_list_attachments`,
  `attachment_read`, `feature_remove_attachment`,
  `attachment_stage_metadata`
- `get_app_session`, `set_app_session`, `delete_app_session`,
  `get_app_info`, `get_workspace_dir`, `get_workspace_dir_setting`,
  `set_workspace_dir_setting`, `get_app_version`

### Agent lifecycle

- `agent_start`, `agent_install_and_start`, `agent_prompt`,
  `agent_cancel`, `agent_restart`, `agent_get_session_info`,
  `agent_set_mode`, `agent_set_config_option`
- `request_action`, `approve_intercept`, `reject_intercept`

### Providers

- `validate_provider_pat`, `fetch_provider_repos`,
  `connect_provider_instance`, `list_provider_instances`,
  `delete_provider_instance`

### Projects + bootstrap

- `create_project`, `get_projects`, `get_project_by_id`, `update_project`,
  `delete_project`, `seed_sample_project`
- `check_repos_dirty`, `get_repositories_for_project`,
  `get_workspace_health`, `resolve_repo_dir`
- `bootstrap_project`, `get_proposed_strategy`, `save_project_settings`
- `project_memory_list`, `project_memory_upsert`,
  `project_memory_delete`
- `get_workflow_overrides`, `set_workflow_override`

### Memory agent

- `memory_agent_config_get`, `memory_agent_config_set`,
  `memory_agent_test_connection`, `memory_agent_list_models`

### Features + steps + gates

- `start_feature`, `fetch_active_features`, `feature_get`,
  `feature_pause`, `feature_resume`, `feature_cancel`,
  `feature_sync`, `feature_resolve_sync_conflicts`,
  `feature_get_worktree`, `feature_cleanup`
- `step_get`, `step_list_for_run`, `step_retry`, `replay_from_step`
- `gate_pending_for_run`, `gate_decide`

### Git + worktrees

- `git_changed_files`, `git_file_at_ref`

### Workflows

- `workflow_list`, `workflow_get`, `workflow_save`, `workflow_delete`,
  `workflow_versions`, `workflow_version_graph`, `workflow_lint`,
  `workflow_export`, `workflow_import`, `workflow_revert_to_default`,
  `workflow_save_schedule`, `node_types_list`, `feature_workflow_graph`

`workflow_save` replaced the separate `workflow_create` / `workflow_update`
pair in P3.6: both minted a version row from a v1 step list, and the builder
needs one write that stores the schema-v2 document (V34 `definition_json`)
alongside its v1 projection.

### Pricing + MR publisher + notifications

- `pricing_list`, `pricing_for`
- `publish_mr`, `fetch_mr_state`
- `notifications_list`, `notification_mark_read`,
  `notification_unread_count`

### Terminal, SFTP, forwarding (carried)

- `set_machine_secret`, `delete_machine_secret`,
  `start_terminal_session`, `write_terminal_session`,
  `resize_terminal_session`, `close_terminal_session`,
  `list_terminal_sessions`, `close_machine_sessions`,
  `attach_terminal_session`, `detach_terminal_session`
- `sftp_list_dir`, `sftp_read_file`, `sftp_write_file`,
  `sftp_get_metadata`
- `start_port_forward`, `stop_port_forward`
- `test_ssh_connection`

### Removed events

- `permission_requested` (no intercept UX in v1; the gate is the
  human-in-the-loop surface)
- `command_executed` (no chat stream)

### New events

- `feature_status_changed` (per-feature state transitions)
- `step_progress` (heartbeat, throttled)
- `gate_required` (a gate needs user attention; the UI navigates to
  the gate view)
- `conflict_detected` (a merge conflict needs resolution)

## 5. Frontend State Model (React, simplified)

The frontend is one stateful app, not a multi-pane chat UI:

- `currentProjectId` (drives the main pane)
- `featureDetail` (the active feature, if any; loaded on demand)
- `gateView` (the current gate's data, when a gate is active)
- `uiPrefs` (theme, accent, collapse state — persisted)
- `commandPaletteOpen` (boolean)

No per-thread session registry. No per-turn `Channel<AgentEvent>` stream.
The agent session is scoped to a step execution; the UI gets step
transitions as events, not streams.

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
│ • Proj A │   - WorkflowBuilderScreen (editing a workflow)  │
│ • Proj B │   - PreferencesScreen (when opened)             │
│ • Proj C │                                                  │
│          │                                                  │
│ [+ New]  │                                                  │
│ [⚙ Mng]  │                                                  │
└──────────┴─────────────────────────────────────────────────┘
```

The "Mng" button at the bottom of the rail opens a project list /
create / delete view. The "⚙" at the top opens global Preferences.
The "?" opens the docs panel.

## 6. Migration Strategy

The migration runner is `refinery`-based (`crates/demeteo-core/migrations/`).
The schema is at V19+; v1 is no longer greenfield.

- **Additive migrations** (new tables, new nullable columns, new
  indexes) apply silently on launch. No user prompt.
- **Pre-migration backup:** the runner copies `demeteo.db` to
  `demeteo.db.bak.<timestamp>` before any migration runs. 7-day
  retention, auto-pruned.
- **Migration log:** every migration writes one line to
  `migrations.log` (next to `demeteo.db`), always viewable from
  Preferences → Storage.
- **Breaking changes:** for any breaking migration (drop/rename
  column) the runner prompts "wipe and re-init" with a confirmation;
  the old DB is moved to `demeteo.db.wiped.<timestamp>`. The user can
  pre-export workflows + projects to JSON to re-import after the
  wipe. Schema-version checks at launch enforce the rule.

See [`DECISIONS.md`](DECISIONS.md) for the decisions that govern this
plan.

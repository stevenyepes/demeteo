# Demeteo: Domain Model & Bounded Contexts

> **Source of truth for the multi-agent orchestrator.** See [`DECISIONS.md`](DECISIONS.md)
> for the locked decision table. This document covers the ubiquitous language and core domain bounded contexts.

## Ubiquitous Language

- **Project** — the top-level container the user creates. Has exactly one host (local or remote SSH) and one or more Repositories.
- **Repository** — a git repo on the Project's host, tracked for workflow purposes.
- **Provider Instance** — a (kind, host) tuple with credentials for GitHub/GitLab/etc. Project's repos are bound to a provider instance at creation.
- **Workflow** — a reusable, versioned template. A Workflow has Steps.
- **Step** — a node in a Workflow. Type: `agent` | `parallel` | `gate`. Has a Step Config (tool, model, mode, prompt, artifact path, conditional edges, retry policy).
- **Step Capability** — the role a step plays; derives the permission posture and the writable path scope. `ReadOnly | Artifacts | Verify | Implement`.
- **Permission Profile** — the abstract, agent-agnostic permission posture for a step. Four orthogonal axes: `read_fs`, `write_fs`, `execute`, `network`. Each `Allow` or `Deny`; never `ask`.
- **Write Scope** — the path-shaped intent of a capability (`None | ArtifactsOnly | All`). Enforced uniformly by the OS-level chmod fence across every agent.
- **Feature** — a running instance of a Workflow on a Project. The user starts a feature; demeteo orchestrates it.
- **Step Execution** — one execution of one step in one feature. Has status, timings, cost, artifact paths, gate decision (if applicable).
- **Subtask** — a unit of work inside a `parallel` step. One (host, agent) pair on a worktree, branched off the feature branch.
- **Subtask Merge** — the act of merging a subtask's worktree branch into the feature branch. May conflict.
- **Worktree Strategy** — the project-level settings for how subtask branches are named and merged (default branch, branch prefix, default test command, PR template, `extra_writable_paths`).
- **Artifact** — a file produced by a Step. Stored under `artifacts/<feature_id>/<step_id>/` (per `ProjectSettings::artifact_subdir`). May be a markdown transcript, a JSON manifest, an MR URL, etc.
- **Gate** — a Step that pauses the feature and surfaces a UI for the user to Approve / Redirect / Cancel.
- **Gate Decision** — the user's choice at a gate (decision, optional feedback). Persisted with the step execution.
- **Conflict Policy** — per-project setting for how merge conflicts are handled (`auto_agent` / `auto_human` / `always_gate`).
- **Conflict Report** — structured output of a failed merge or sync: source/target branch, conflicting files with `kind`, raw stderr, detection timestamp.
- **Feature Lifecycle** — per-project setting for what happens to a completed feature (`keep` / `archive` / `auto_delete`).
- **Artifact Mode** — per-workflow setting for how much step output to persist (`full` / `summary_only` / `none`).
- **Workflow Schedule** — optional schedule attached to a workflow (`workflow_save_schedule`); the scheduler adapter (`adapters/scheduler.rs`) fires it on cadence.
- **Project Workflow Override** — project-scoped override of agent/model for a workflow or step (`step_id = None` for workflow-level, `Some(...)` for step-level). Persisted in `project_workflow_overrides`.
- **Step Override** — per-step agent/model override chosen when launching a feature; snapshotted on the feature row.
- **Memory** — typed project-level knowledge captured by the Memory Agent (`conventions | lessons | decisions | preferences | facts`).
- **Notification** — UI-side cache row for the in-app notification bell.

## Bounded Contexts

### 1. Identity & Fleet (Core Subdomain)

The cross-cutting context for app-level config and external identity.

- **Aggregates:** `AppSettings` (singleton KV), `ProviderInstance`
- **Value Objects:** `ProviderKind` (`github` | `gitlab` | future), `Host` (string), `EncryptedPat` (opaque, keyring-backed)
- **Ports:** `ProviderHttpPort`, `AppSettingsRepository`
- **Adapters:** `SqliteAppSettingsRepository`, `HttpProviderAdapter` (GitHub / GitLab)
- **Key invariants:**
  - A provider instance's PAT is encrypted at rest with a key from the OS keyring (`keyring` crate).
  - A provider instance is uniquely keyed by `(kind, host)`. Two `github.com` instances with the same kind+host are an error; user must disconnect the first to add a second with the same key.
  - `AppSettings` is a singleton row of KV (`app_setting_get` / `app_setting_set`); updates are atomic.
  - The OS keyring key is generated on first launch and persisted as `demeteo.db_key`; loss of the key = loss of PATs (user must reconnect providers).

### 2. Project Management (Core Subdomain)

The user's "workspace" — what they're working on and where it lives.

- **Aggregates:** `Project`, `Repository`, `ProjectSettings`, `ProjectWorkflowOverride`
- **Value Objects:** `ProjectType` (`local` | `remote`), `WorktreeStrategy`, `SshConnection`, `PublishOptions`, `MrInfo`
- **Ports:** `ProjectRepository`, `WorktreeOpsPort` (clone / branch / worktree helpers), `ProviderHttpPort` (repos)
- **Adapters:** `SqliteProjectRepository`, `SshRepositoryCloner`, `LocalFsRepositoryCloner`, `GitWorkflowDetector`
- **Key invariants:**
  - A Project has exactly one host (either a local folder or a remote SSH target).
  - `ProjectSettings::default_agent_kind` / `default_model` define the per-project planner. Per-workflow overrides (`ProjectWorkflowOverride` with `step_id = None`) win at workflow scope; per-step overrides win at step scope. Both lose to a run-time override chosen in `StartFeatureModal`.
  - A Project's repos are bound to a Provider Instance at creation; PAT lookup is by `(kind, host)`.
  - `WorktreeStrategy` is detected at bootstrap and stored; user can edit.
  - `WorktreeStrategy::extra_writable_paths` adds repo-relative paths to the chmod fence for tool side-effects (`target/`, `node_modules/`, `.venv/`); each entry must be relative and `..` is rejected.
  - Strict serial per project: at most one running feature per project at a time.

### 3. Workflow Catalog (Core Subdomain)

The reusable templates that drive feature execution.

- **Aggregates:** `Workflow`, `WorkflowVersion`, `WorkflowSchedule`
- **Value Objects:** `StepType`, `StepConfig`, `ConditionalEdge`, `RetryPolicy`, `ArtifactMode`, `WorkflowDigest`, `ProjectWorkflowOverride`
- **Ports:** `WorkflowRepository`
- **Adapters:** `SqliteWorkflowRepository` (handles export/import as JSON via `workflow_export` / `workflow_import` commands)
- **Key invariants:**
  - A Workflow has a unique name; a `WorkflowVersion` is unique per `(workflow_id, version)`.
  - The starter pack workflows are seeded at first launch (`count()` drives the first-launch seed step); user can edit (creates a new version) but not delete (revert to default via `workflow_revert_to_default`).
  - Import creates a new Workflow + initial Version; if the imported JSON has multiple versions, all are preserved.
  - JSON export includes the workflow's full version history as an array of version blobs.
  - `WorkflowSchedule` is optional and is persisted on the workflow row; `workflow_save_schedule` and `list_scheduled` are the in/out commands.

### 4. Feature Orchestration (Core Subdomain)

The runtime — features in motion, steps executing, gates waiting.

- **Aggregates:** `Feature`, `StepExecution`, `SubtaskRun`, `GateDecision`, `FeatureSync`
- **Value Objects:** `FeatureStatus` (`draft` | `running` | `paused` | `completed` | `archived` | `aborted`), `StepStatus` (`pending` | `running` | `verifying` | `awaiting_gate` | `completed` | `failed` | `skipped` | `interrupted`), `Cost`, `Duration`, `SyncOutcomeView` (the typed result of `feature_sync` / `feature_resolve_sync_conflicts`)
- **Ports:** `StepExecutor` (the only orchestrator port — `feature_start` / `_pause` / `_resume` / `_cancel` / `_sync` / `_resolve_sync_conflicts`, `step_get` / `step_retry` / `replay_from_step` / `step_list_for_run`), `GatePresenter` (`gate_pending_for_run`, `gate_decide`)
- **Adapters:** `StepExecutorAdapter` (`adapters/step_executor/`), `DagDriver` (`adapters/step_executor/driver.rs`)
- **Key invariants:**
  - A Feature has exactly one active run at a time (strict serial per project).
  - The current step is the source of truth for the orchestrator's state; everything else is derived.
  - Per-step checkpoints are atomic: a step is "complete" only when its artifact is written and (if it's a gate) its decision is recorded.
  - On re-entry (launch), mid-step interruptions surface a synthetic gate; completed steps are not re-run.
  - Cost and duration are computed at step completion, not estimated mid-step.
  - `paused` is a valid `FeatureStatus` value (transient; `feature_pause` flips `running` → `paused`).
  - `interrupted` is a valid `StepStatus` value; surfaced by the shutdown watchdog.
  - `verifying` is a valid `StepStatus` value (per `StepExecutor` precondition docstring); a step in `verifying` is treated as non-terminal for predecessor-running guards.
  - `gate_decide` and `step_retry` both refuse to apply when an earlier step (`step_index < target.step_index`) is in any of `pending | running | verifying | awaiting_gate`. The check surfaces as `AppError::validation` so the UI can both disable the Retry Step / Approve buttons and surface the blocking predecessor by name. The frontend mirrors the rule in pure TypeScript via `findActivePredecessor` (`src/lib/features.ts`).

### 5. Worktree & Git (Supporting Subdomain)

The git mechanics that make the feature-branch model work.

- **Aggregates:** `SubtaskRun`, `SubtaskMerge`, `FeatureSync`
- **Value Objects:** `MergePreCheck` (`AlreadyMerged` | `CleanMerge` | `WouldConflict`), `MergeOutcome`, `ConflictReport`, `ConflictFile`, `ConflictPolicy` (`AlwaysGate` | `AutoAgent` | `AutoHuman`), `WorktreeBranchName`, `CommitSha`, `UpstreamSyncOutcome`, `UpstreamSyncFailure`
- **Ports:** `WorktreeOpsPort`, `MergePort`, `ConflictPort`, `MrPublisher`
- **Adapters:** `GitOpsHelper` (`adapters/worktree/git_ops/`), `TopologicalMergeExecutor`, `ProviderMrPublisher`, `AgentConflictResolver`
- **Key invariants:**
  - Each `SubtaskRun` has exactly one worktree branch.
  - Subtask branches are rebased onto the latest feature branch before merge, in topological order from the DAG.
  - Conflicts surface as a structured `ConflictReport` (source/target branch, `ConflictFile`s with `kind`, raw stderr, `detected_at`); the per-project `ConflictPolicy` decides the next step.
  - `MrPublisher` is the only port that calls the provider instance's PAT for write operations (clone uses the same PAT but via a different code path; the boundary is "read vs write").
  - Merge preflight runs `precheck_merge` and returns `MergePreCheck` before any working tree is touched.
  - `MergeStrategy` (legacy) is gone; the per-project `ConflictPolicy` replaces it.
  - `WriteScope` (`None | ArtifactsOnly | All`) drives the chmod fence in `adapters/worktree/git_ops/scope.rs`; the path-shaped artifacts-vs-source line is enforced uniformly across every agent.

### 6. Agent Runtime (Supporting Subdomain)

The layer that talks to coding agents.

- **Aggregates:** `AgentRegistry`, `AgentSession`
- **Value Objects:** `AgentKind` (`opencode` | `hermes` | `claude-code`) — a real enum (`domain/models/agent_config.rs`) whose `as_str` equals the runtime `kind()` key; `AgentCapabilities` (`display_label`, `lists_models`, `default_model`) declared once per runtime so no downstream site matches on the kind string; `AgentConfig`, `AgentContext`, `AgentEvent`, `StepCapability`, `PermissionProfile`, `WriteScope`, `Access`, `Usage`, `StopReason`
- **Ports:** `AgentRuntime` (`kind` / `capabilities` / `binary` / `is_available` / `install_command` / `start`), `AgentExecutionPort` (`submit` / `submit_agent` / `approve` / `reject`), `AgentSession`, `ExecutionPort`
- **Adapters:** `UnifiedCliRuntime` (`adapters/agent/cli_runtime.rs`) — one impl, configured per agent. `opencode` and `hermes` use `OPENCODE_PERMISSION` env; `claude-code` uses `--disallowedTools` + `--exclude-dynamic-system-prompt-sections` + `--setting-sources user,project` + `--strict-mcp-config` and lets Claude own its own credentials.
- **Key invariants:**
  - Every supported agent is a one-shot CLI runtime that takes its model via a `--model` flag built from `AgentContext.model`; there is no config-env/ACP model path.
  - One `UnifiedCliRuntime` impl serves all agents (binary + args + install_command + parse_event + capabilities differ; everything else is shared).
  - Agent sessions are scoped to a step execution — no global session reuse.
  - The planner is just an agent session with a planning prompt; no special planner port.
  - `AgentEvent` is an internal contract (consumed by `StepExecutor`), not a UI contract. The UI sees step transitions, not agent transcripts. Variants: `Text`, `ToolCall`, `ToolCallUpdate`, `Plan`, `Usage` (input / output / cache_read / cache_creation / cost_usd), `Error`, `TurnComplete`, `ModeChanged`, `ConfigChanged`, `ArtifactProduced`.
  - `PermissionProfile` is *complete* and only uses `allow` / `deny`, never `ask`. `external_directory: "deny"` (opencode only) is the worktree scope fence; the OS-level chmod fence plus `--disallowedTools` enforce path-shape on agents that don't speak `external_directory`.
  - `ACTIONError` (the typed envelope for agent-originated actions) has three variants: `Network`, `NotFound`, `Internal`.
  - Install commands (current; subject to drift — see `README.md` for upstream-status notes):
    - `opencode` → `curl -fsSL https://opencode.ai/install | bash`
    - `hermes` → `curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash`
    - `claude-code` → `npm install -g @anthropic-ai/claude-code`

### 7. Memory (Supporting Subdomain)

The opt-in project-memory layer.

- **Aggregates:** `Memory` (typed project knowledge: `conventions | lessons | decisions | preferences | facts`)
- **Value Objects:** `MemoryKind`, `WorkingMemoryEntry`
- **Ports:** `MemoryPort`, `MemoryLlmPort`, `MemorySignalsPort`
- **Adapters:** `FsMemoryStore` (SQLite-backed), `MemoryWorker` (`adapters/memory_worker.rs`), `OpenAiCompatLlmClient` (`adapters/memory_llm.rs`)
- **Key invariants:**
  - The Memory Agent is the **one** place Demeteo calls a model provider directly (user-configured OpenAI-compatible endpoint, e.g. Ollama).
  - The Memory Agent is disabled by default; its API key lives in the OS keyring.
  - It runs in the background and never drives a feature run.
  - Memories are surfaced at `Project Settings → Project Memory` (read/edit) and injected into future agent prompts via semantic search.

### 8. UI & Telemetry (Supporting Subdomain)

The presentation layer's persistent state, the docs surface, and the on-disk observability.

- **Aggregates:** `UiPreferences` (KV in `app_session`), `DocsRepository` (bundled markdown in the binary), `MigrationLog` (append-only text file)
- **Value Objects:** `Notification` (typed payload for the in-app bell)
- **Ports:** `NotificationPort`, `NotificationRepository`
- **Adapters:** `SqliteNotificationRepository`, `BundledDocsRepository`
- **Key invariants:**
  - `UiPreferences` is per-project (collapse state, sort order) and per-user (theme, accent), stored as KV in `app_session`.
  - Notifications are a UI-side cache; nothing in the orchestrator's correctness path depends on them.
  - `DocsRepository` serves markdown from the bundled binary; no network calls.
  - `MigrationLog` is an append-only text file at `~/.local/share/demeteo/migrations.log`; readable from Preferences → Storage.

### 9. Attachments (Supporting Subdomain)

Per-feature user attachments (images, files) carried into the agent's prompt context.

- **Aggregates:** `Attachment` (per-feature, keyed by feature id + sha256)
- **Value Objects:** `AttachedFile`, `StagedAttachmentInput`
- **Ports:** `AttachmentStore`
- **Adapters:** `FsAttachmentStore` (filesystem at `<app_local_data_dir>/attachments/<feature_id>/<sha256>.<ext>`)
- **Key invariants:**
  - Attachment metadata lives on the feature row (`features.attachments_json`, migration V19) so feature cleanup auto-releases the attachment lifetime.
  - The on-disk file content is dropped by `FsAttachmentStore::clear_feature` when the feature is purged.
  - Staged attachments are persisted to the freshly-created feature row BEFORE the driver is spawned, so the agent's first turn sees them.
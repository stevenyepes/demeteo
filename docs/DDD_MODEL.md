# Demeteo: Domain Model & Bounded Contexts

> **Source of truth for the multi-agent orchestrator.** See [`DECISIONS.md`](DECISIONS.md)
> for the locked decision table. This document covers the ubiquitous language and core domain bounded contexts.

## Ubiquitous Language

- **Project** — the top-level container the user creates. Has exactly one host (local or remote SSH) and one or more Repositories.
- **Repository** — a git repo on the Project's host, tracked for workflow purposes.
- **Provider Instance** — a (kind, host) tuple with credentials for GitHub/GitLab/etc. Project's repos are bound to a provider instance at creation.
- **Planner** — the (machine, agent) pair that runs research, spec, plan, and subtask-decomposition steps.
- **Workflow** — a reusable, versioned template. A Workflow has Steps.
- **Step** — a node in a Workflow. Type: `agent` | `parallel` | `gate`. Has a Step Config (tool, model, mode, prompt, artifact path, conditional edges, retry policy).
- **Feature** — a running instance of a Workflow on a Project. The user starts a feature; demeteo orchestrates it.
- **Feature Run** — one execution attempt of a feature. A feature can be re-run; each run preserves the prior run's state for diff.
- **Step Execution** — one execution of one step in one feature run. Has status, timings, cost, artifact paths, gate decision (if applicable).
- **Subtask** — a unit of work inside a `parallel` step. One (host, agent) pair on a worktree, branched off the feature branch.
- **Subtask Merge** — the act of merging a subtask's worktree branch into the feature branch. May conflict.
- **Worktree Strategy** — the project-level settings for how subtask branches are named and merged (default branch, branch prefix, default test command, PR template).
- **Artifact** — a file produced by a Step. Stored under `features/<id>/artifacts/`. May be a markdown transcript, a JSON manifest, an MR URL, etc.
- **Gate** — a Step that pauses the feature and surfaces a UI for the user to Approve / Redirect / Cancel.
- **Gate Decision** — the user's choice at a gate (decision, optional feedback). Persisted with the step execution.
- **Conflict Policy** — per-project setting for how merge conflicts are handled (`auto_agent` / `auto_human` / `always_gate`).
- **Feature Lifecycle** — per-project setting for what happens to a completed feature (`keep` / `archive` / `auto_delete`).
- **Artifact Mode** — per-workflow setting for how much step output to persist (`full` / `summary_only` / `none`).
- **Retry Policy** — per-step opt-in setting for planner-driven retry on failure, with a cost cap.

## Bounded Contexts

### 1. Identity & Fleet (Core Subdomain)

The cross-cutting context for app-level config and external identity.

- **Aggregates:** `AppSettings` (singleton), `ProviderInstance`
- **Value Objects:** `ProviderKind` (`github` | `gitlab` | future), `Host` (string), `EncryptedPat` (opaque)
- **Ports:** `ProviderInstanceRepository`, `AppSettingsRepository`
- **Adapters:** `SqliteProviderInstanceRepository`, `SqliteAppSettingsRepository`
- **Key invariants:**
  - A provider instance's PAT is encrypted at rest with a key from the OS keyring (`keyring` crate).
  - A provider instance is uniquely keyed by `(kind, host)`. Two `github.com` instances with the same kind+host are an error; user must disconnect the first to add a second with the same key.
  - `AppSettings` is a singleton row; updates are atomic. No field-level locking needed at v1 scale.
  - The OS keyring key is generated on first launch and persisted as `demeteo.db_key`; loss of the key = loss of PATs (user must reconnect providers).

### 2. Project Management (Core Subdomain)

The user's "workspace" — what they're working on and where it lives.

- **Aggregates:** `Project`, `Repository`
- **Value Objects:** `ProjectType` (`local` | `remote`), `WorktreeStrategy`, `SshConnection`, `ProjectSettings`, `PlannerAssignment`
- **Ports:** `ProjectRepository`, `RepositoryCatalog`, `WorktreeStrategyDetector`
- **Adapters:** `SqliteProjectRepository`, `SshRepositoryCloner`, `LocalFsRepositoryCloner`, `GitWorkflowDetector`
- **Key invariants:**
  - A Project has exactly one host (either a local folder or a remote SSH target).
  - A Project's planner is a `(machine_id, agent_kind)` pair that must exist on the host. Validation runs at feature-start time, not at project-create time.
  - A Project's repos are bound to a Provider Instance at creation; PAT lookup is by `(kind, host)`.
  - The `WorktreeStrategy` is detected at bootstrap and stored; user can edit.
  - Strict serial per project: at most one running feature per project at any time (deferred-but-field exists for v1.x).

### 3. Workflow Catalog (Core Subdomain)

The reusable templates that drive feature execution.

- **Aggregates:** `Workflow`, `WorkflowVersion`
- **Value Objects:** `StepType`, `StepConfig`, `ConditionalEdge`, `RetryPolicy`, `ArtifactMode`, `WorkflowDigest`
- **Ports:** `WorkflowRepository`, `WorkflowVersionRepository`, `WorkflowImporter`, `WorkflowExporter`
- **Adapters:** `SqliteWorkflowRepository`, `JsonWorkflowImporter`, `JsonWorkflowExporter`, `BundledStarterPackProvider`
- **Key invariants:**
  - A Workflow has a unique name; a WorkflowVersion is unique per `(workflow_id, version)`.
  - The starter pack workflows are seeded at first launch; user can edit (creates a new version) but not delete (revert to default instead).
  - Import creates a new Workflow + initial Version; if the imported JSON has multiple versions, all are preserved.
  - JSON export includes the workflow's full version history as an array of version blobs.

### 4. Feature Orchestration (Core Subdomain)

The runtime — features in motion, steps executing, gates waiting.

- **Aggregates:** `Feature`, `FeatureRun`, `StepExecution`, `GateDecision`
- **Value Objects:** `FeatureStatus` (`draft` | `running` | `paused` | `completed` | `archived` | `aborted`), `StepStatus` (`pending` | `running` | `awaiting_gate` | `completed` | `failed` | `skipped`), `Cost`, `Duration`
- **Ports:** `FeatureOrchestrator`, `StepExecutor`, `GatePresenter`, `CheckpointStore`
- **Adapters:** `SqliteFeatureOrchestrator`, `DagStepExecutor`, `TauriGatePresenter`, `SqliteCheckpointStore`
- **Key invariants:**
  - A Feature has exactly one active `FeatureRun` at a time (strict serial per project).
  - A `FeatureRun`'s current step is the source of truth for the orchestrator's state; everything else is derived.
  - Per-step checkpoints are atomic: a step is "complete" only when its artifact is written and (if it's a gate) its decision is recorded.
  - On re-entry (launch), mid-step interruptions surface a synthetic gate; completed steps are not re-run.
  - Cost and duration are computed at step completion, not estimated mid-step.

### 5. Worktree & Git (Supporting Subdomain)

The git mechanics that make the feature-branch model work.

- **Aggregates:** `Worktree`, `SubtaskRun`, `SubtaskMerge`
- **Value Objects:** `MergeStrategy` (`fast_forward_only` | `merge_commit` | `squash`), `ConflictReport`, `WorktreeBranchName`, `CommitSha`
- **Ports:** `WorktreeManager`, `MergeExecutor`, `MrPublisher`, `ConflictResolver`
- **Adapters:** `Git2WorktreeManager`, `TopologicalMergeExecutor`, `ProviderMrPublisher`, `AgentConflictResolver`
- **Key invariants:**
  - Each `SubtaskRun` has exactly one worktree branch.
  - Subtask branches are rebased onto the latest feature branch before merge, in topological order from the DAG.
  - Conflicts surface as a structured `ConflictReport`; the conflict policy decides the next step.
  - `MrPublisher` is the only port that calls the provider instance's PAT for write operations (clone uses the same PAT but via a different code path; the boundary is "read vs write" not "clone vs publish").
  - Merge strategy is per-project (from the bootstrap-detected `WorktreeStrategy`); the project default can be overridden per-step.

### 6. Agent Runtime (Supporting Subdomain)

The layer that talks to coding agents. Carried from v1 with one big change in the *caller* (now `StepExecutor`, not a per-thread UI stream).

- **Aggregates:** `AgentRegistry`, `AgentSession`
- **Value Objects:** `AgentKind` (`opencode` | `hermes` | `claude-code` | `antigravity`), `AgentConfig`, `AgentContext`, `AgentEvent`, `PermissionPolicy`
- **Ports:** `AgentRuntime`, `PermissionPolicyPort`
- **Adapters:** `CliRuntime` (one impl, configured per agent), `WorktreeScopedPolicy`
- **Key invariants:**
  - One `CliRuntime` impl serves all four agents (binary + args + install_command + parse_event differ).
  - Agent sessions are scoped to `(feature_run_id, step_execution_id)` — no global session reuse.
  - The planner is just an agent session with a planning prompt; no special planner port.
  - `AgentEvent` is an internal contract (consumed by `StepExecutor`), not a UI contract. The UI sees step transitions, not agent transcripts.
  - `OPENCODE_PERMISSION` is injected at spawn time; `external_directory: "deny"` enforces worktree scope.

### 7. UI & Telemetry (Supporting Subdomain)

The presentation layer's persistent state, the docs surface, and the on-disk observability.

- **Aggregates:** `UiPreferences`, `DocsRepository`, `DiskUsageReport`, `MigrationLog`
- **Value Objects:** `CommandPaletteEntry`, `ShortcutBinding`, `DiskUsageBreakdown`
- **Ports:** `UiStateRepository`, `DocsRepository`, `DiskUsageCalculator`
- **Adapters:** `SqliteUiStateRepository`, `BundledDocsRepository`, `FsDiskUsageCalculator`
- **Key invariants:**
  - `UiPreferences` is per-project (collapse state, sort order) and per-user (theme, accent).
  - `DiskUsageReport` is computed on demand from the artifact store + git worktree directories; never cached.
  - `DocsRepository` serves markdown from the bundled binary; no network calls.
  - `MigrationLog` is an append-only text file at `~/.local/share/demeteo/migrations.log`; readable from Preferences → Storage.

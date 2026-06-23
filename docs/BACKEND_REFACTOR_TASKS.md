# Demeteo — Rust Backend Refactor: Remaining Tasks

> Generated after Phase A (foundation: `error/`, `shared/`, deps) and Phase B (async port traits) shipped clean. Library compiles, 155 tests pass, `cargo clippy --lib -- -D warnings` is clean, `npx tsc --noEmit` is clean.
>
> This document is the work backlog for Phases C, D, E, F from the original phased plan. Each task lists **what**, **where**, **why**, and a **definition of done** so an agent can pick it up and execute without further context.

---

## Phase C — Missing Ports (Hexagon Closure)

### C1. [COMPLETED] `WorktreeOpsPort` — replace concrete `GitOpsHelper` imports in commands

**Files to change:**
- `src-tauri/src/ports/worktree_ops.rs` (NEW, ~50 LOC)
- `src-tauri/src/lib.rs` (register module)
- `src-tauri/src/adapters/worktree/git_ops.rs` (add `impl WorktreeOpsPort for GitOpsHelper`)
- `src-tauri/src/state.rs` (add `worktree_ops: Arc<dyn WorktreeOpsPort>` field)
- `src-tauri/src/lib.rs::run` (wire it up — already constructs `GitOpsHelper`)
- `src-tauri/src/commands/project.rs` (replace `GitOpsHelper::new(...)` at lines 246, 295 with `ctx.worktree_ops.X(...)`)
- `src-tauri/src/commands/bootstrap.rs` (replace `use crate::adapters::worktree::git_ops::GitOpsHelper;` and constructor)
- `src-tauri/src/commands/feature_lifecycle.rs` (replace inline `git -C ...` with `ctx.worktree_ops.branch_delete(...)`)

**Trait surface:**
```rust
#[async_trait]
pub trait WorktreeOpsPort: Send + Sync {
    async fn check_repo_dirty(&self, machine_id: Option<&str>, repo_dir: &str) -> Result<(bool, bool), String>;
    async fn get_head_branch(&self, machine_id: Option<&str>, repo_dir: &str) -> Option<String>;
    async fn list_worktrees(&self, machine_id: Option<&str>, repo_dir: &str) -> Vec<WorktreeInfo>;
    async fn detect_worktree_strategy(&self, machine_id: Option<&str>, repo_dir: &str) -> Result<WorktreeStrategy, String>;
    async fn clone_repository(&self, machine_id: Option<&str>, provider_id: &str, repo_path: &str, target_dir: &str) -> Result<(), String>;
    async fn create_feature_branch(&self, machine_id: Option<&str>, repo_dir: &str, branch: &str, base_ref: Option<&str>) -> Result<(), String>;
    async fn provision_subtask_worktree(&self, machine_id: Option<&str>, repo_dir: &str, branch: &str, subtask_id: &str) -> Result<String, String>;
    async fn cleanup_subtask_worktree(&self, machine_id: Option<&str>, repo_dir: &str, branch: &str, subtask_id: &str) -> Result<(), String>;
    async fn branch_delete(&self, machine_id: Option<&str>, repo_dir: &str, branch: &str) -> Result<(), String>;
    async fn merge_subtask(&self, machine_id: Option<&str>, worktree_dir: &str, branch: &str, subtask_id: &str) -> Result<(), String>;
    async fn sync_feature_with_upstream(&self, feature_id: &FeatureId, feature_branch: &str, default_branch: &str) -> Result<UpstreamSyncOutcome, UpstreamSyncFailure>;
}
```

**Definition of done:**
- `grep -RnE 'use crate::adapters::worktree::git_ops' src/commands src/application` returns 0 lines
- All callers use `ctx.worktree_ops.X(...)` instead
- `cargo clippy --lib -- -D warnings` clean
- Existing 155 tests still pass

---

### C2. [COMPLETED] `ProviderHttpPort` — replace hand-rolled `reqwest`/`keyring` in `commands/providers.rs`

**Files to change:**
- `src-tauri/src/ports/provider_http.rs` (NEW)
- `src-tauri/src/adapters/provider_http/reqwest_impl.rs` (NEW)
- `src-tauri/src/state.rs` (add `provider_http: Arc<dyn ProviderHttpPort>` field)
- `src-tauri/src/lib.rs::run` (wire it up)
- `src-tauri/src/commands/providers.rs` (rewrite `validate_provider_pat`, `fetch_provider_repos`, `connect_provider_instance` to use the port)

**Trait surface:**
```rust
#[async_trait]
pub trait ProviderHttpPort: Send + Sync {
    async fn validate_pat(&self, host: &str, kind: ProviderKind, pat: &str) -> Result<ProviderUserInfo, AppError>;
    async fn list_repos(&self, host: &str, kind: ProviderKind, pat: &str) -> Result<Vec<RepoSummary>, AppError>;
}
```

**Side effect:** DELETE the 6 `std::fs::write("/tmp/demeteo_fetch.log", ...)` calls in `commands/providers.rs:106-192` (audit smell #4). Replace with `tracing::warn!` macros. The `/tmp/demeteo_fetch.log` file leaks keyring error messages and HTTP response bodies — it's debug-time instrumentation in production paths.

**Definition of done:**
- `/tmp/demeteo_fetch.log` is never written to in production code
- `grep -RnE 'std::fs::write.*demeteo' src/` returns 0 lines
- All HTTP calls in `commands/providers.rs` go through the port
- `commands/providers.rs` < 100 LOC

---

### C3. [COMPLETED] `MergeAuditRepository` — replace raw SQL in `adapters/merge.rs`

**Files to change:**
- `src-tauri/src/ports/db.rs` (add `MergeAuditRepository` sub-trait)
- `src-tauri/src/adapters/database/repos/merge_audit.rs` (NEW, ~120 LOC — extracts the raw SQL from `adapters/merge.rs:405-468`)
- `src-tauri/src/adapters/database/mod.rs` (re-export `MergeAuditRepository`)
- `src-tauri/src/state.rs` (add `merge_audit: Arc<dyn MergeAuditRepository>` field)
- `src-tauri/src/lib.rs::run` (wire it up — `db_adapter.clone()` for the 8th sub-port)
- `src-tauri/src/adapters/merge.rs` (replace raw `SqliteConnection` calls with `ctx.merge_audit.X(...)`)
- `src-tauri/src/adapters/step_executor/sync.rs` and `steps/sync.rs` (no longer need to reach into `merge_executor.get_last_sync_worktree_path` for raw DB queries)

**Trait surface:**
```rust
pub trait MergeAuditRepository: Send + Sync {
    fn record_merge_outcome(&self, row: MergeAuditRow) -> Result<(), DbError>;
    fn record_sync_outcome(&self, row: SyncAuditRow) -> Result<(), DbError>;
    fn list_unmerged_files(&self, repo_dir: &str) -> Result<Vec<UnmergedFile>, DbError>;
    fn lookup_worktree_context(&self, subtask_run_id: &str) -> Result<WorktreeContext, DbError>;
    fn lookup_repo_context(&self, repo_id: &str) -> Result<RepoContext, DbError>;
}
```

**Side effect:** `SqliteMergeExecutor` no longer holds `SqliteConnection` directly — it composes the audit repo with the worktree ops port.

**Definition of done:**
- `grep -RnE 'db_adapter.conn' src/adapters/merge.rs` returns 0 lines
- All SQL queries that touch `subtask_merges` / `upstream_syncs` tables go through the new port
- `adapters/merge.rs` < 500 LOC

---

### C4. [COMPLETED] Consolidate POSIX shell escape — eliminate 3 duplicates

**Files to change:**
- `src-tauri/src/shared/shell.rs` (already has canonical `escape_posix`; mark as the only version)
- `src-tauri/src/adapters/merge.rs:595-637` (private `shell_escape` — DELETE, route through `shared::shell::escape_posix`)
- `src-tauri/src/commands/feature_lifecycle.rs:173-193` (private `shell_escape` — DELETE, route through `shared::shell::escape_posix`)
- Update all call sites (`adapters/merge.rs:114, 157, 318, 346, 358, 398, 404, 410`; `commands/feature_lifecycle.rs:107-135`)

**Decision:** `shared::shell::escape_posix` and `paths::shell_escape_posix` have slightly different semantics (the former always wraps in single quotes; the latter has a "safe chars" fast path). Audit and pick one behavior. **Recommendation:** keep the legacy `paths::shell_escape_posix` semantics (the "safe chars" fast path) and have `shared::shell::escape_posix` delegate to it. This avoids changing the on-wire shell command format and breaking existing tests.

**Definition of done:**
- `grep -RnE 'fn shell_escape|fn shell_escape_posix' src/` shows exactly one definition (in `shared/shell.rs`)
- `cargo clippy --lib -- -D warnings` clean

---

### C5. [COMPLETED] Consolidate SSH connect — eliminate 5 duplicates

**Files to change:**
- `src-tauri/src/infrastructure/ssh/connect.rs` (NEW, ~80 LOC — single source of TCP+handshake+auth)
- `src-tauri/src/infrastructure/ssh/mod.rs` (add `connect(machine_id, secret) -> Session` helper and `with_session(machine_id, |session| async { ... })` wrapper)
- Delete the inline SSH-connect dance in:
  - `adapters/ssh/client.rs::get_sftp` (115 LOC at lines 72-189)
  - `adapters/ssh/client.rs::test_connection` (lines 257-345)
  - `terminal.rs::start_terminal_session` (lines 96-128)
  - `forward.rs::start_port_forward` (lines 60-76)

**Definition of done:**
- SSH connect logic exists in exactly one place
- All 5 callers use the same helper
- Existing tests still pass

---

### C6. [COMPLETED] Machine-id 4-way fallback resolver — eliminate 5 duplicates

**Files to change:**
- `src-tauri/src/infrastructure/worktree/machine_resolver.rs` (NEW, ~50 LOC)
- `src-tauri/src/state.rs` (add `machine_resolver: Arc<MachineResolver>` field — or make it a free function in `ports`)
- Replace the 5 sites:
  - `adapters/ssh/client.rs:96-103`, `client.rs:263-269`, `client.rs:588-594`
  - `forward.rs:46-51`
  - `router.rs:33-41`
  - `commands/agent_config.rs:11-25` (6-way variant)

**Trait surface:**
```rust
pub fn resolve_machine(machines: &dyn MachineRepository, id: &str) -> Result<Machine, AppError>;
```

**Definition of done:**
- All 5 sites call the same `resolve_machine(...)` function
- Behavior is identical to current (verify by snapshot test)

---

## Phase D — Deep Module Extraction

### D1. [COMPLETED] `infrastructure/agent/event_stream/{turn, cleanup}.rs` — kill the 5× duplication

The agent-event-stream loop with `tokio::select!` over `fast_sleep`/`normal_sleep`/`wall_sleep`/`cancel_watch` is duplicated **5 times** across the executor:
- `adapters/step_executor/steps/agent.rs:204-309` (primary turn)
- `adapters/step_executor/steps/parallel.rs:343-418` (planner)
- `adapters/step_executor/steps/parallel.rs:628-742` (per-worker)
- `adapters/step_executor/steps/parallel.rs:901-993` (per-worker conflict-resolution)
- `adapters/step_executor/sync.rs:140-194` (resolver)
- `adapters/step_executor/driver.rs:622-699` (verifier)

**New files:**
- `src-tauri/src/infrastructure/agent/event_stream/mod.rs` (~30 LOC)
- `src-tauri/src/infrastructure/agent/event_stream/turn.rs` (~150 LOC — `stream_agent_turn(session, prompt, timeouts, cancel_watch) -> Result<TurnOutcome, AgentError>`)
- `src-tauri/src/infrastructure/agent/event_stream/cleanup.rs` (~80 LOC — `cleanup_subtask_after_failure(registry, git_ops, worktree, subtask_id, cancellation_reason)`)

**Definition of done:**
- The 75-line `tokio::select!` body exists in exactly one place
- All 5 call sites reduce to one line: `let outcome = stream_agent_turn(...).await?;`
- The 5× duplication of the "kill session + sleep 200ms + cleanup worktree" teardown reduces to one helper
- `cargo clippy --lib -- -D warnings` clean
- All 155 tests still pass

**Estimated effort:** 2-3 days. This is the single most impactful extraction in the codebase.

---

### D2. [COMPLETED] Split `parallel.rs` (1,361 LOC → 5 files, each ≤ 500)

**Current structure:** `adapters/step_executor/steps/parallel.rs` contains:
- `extract_subtask_dag`, `find_top_level_object`, `PlannedSubtask`, `SubtaskDag` (~200 LOC planner logic)
- `handle_parallel_step` (~1,090 LOC — planner pass → per-subtask fan-out → per-subtask merge → step artifact summary)
- `list_unmerged_files` (already moved to async in Phase B)
- 7 tests (~210 LOC)

**Proposed split:**
- `infrastructure/step_executor/parallel/planner.rs` (~250 LOC — `extract_subtask_dag`, `find_top_level_object`, `PlannedSubtask`, `SubtaskDag`)
- `infrastructure/step_executor/parallel/handler.rs` (~500 LOC — `handle_parallel_step`, now thin)
- `infrastructure/step_executor/parallel/subtask.rs` (~200 LOC — per-subtask fan-out logic, conflict resolution)
- `infrastructure/step_executor/parallel/list_unmerged.rs` (50 LOC)
- `tests/infrastructure/step_executor/parallel.rs` (200 LOC — co-located tests)

**Definition of done:**
- Each file ≤ 500 LOC
- All 155 tests still pass
- `cargo clippy --lib -- -D warnings` clean

---

### D3. [COMPLETED] Split `agent.rs` (815 LOC → 4 files)

**Current structure:** `adapters/step_executor/steps/agent.rs` contains:
- `handle_agent_step` (~720 LOC — spawn → prompt → artifacts → verifier → commit → merge → conflict-resolution → cleanup)
- `format_agent_error_message` (35 LOC, already async)
- `list_unmerged_files` (already async)
- 1 inline test

**Proposed split:**
- `infrastructure/step_executor/agent/spawn.rs` (~200 LOC — worktree provisioning, prompt construction, spawn)
- `infrastructure/step_executor/agent/artifacts.rs` (~200 LOC — artifact contract injection, worktree snapshot)
- `infrastructure/step_executor/agent/verifier.rs` (~150 LOC — `run_verifier_logic` extraction)
- `infrastructure/step_executor/agent/handler.rs` (~250 LOC — thin `handle_agent_step` that orchestrates the above)
- `infrastructure/step_executor/agent/error_message.rs` (50 LOC — `format_agent_error_message`)

**Definition of done:**
- Each file ≤ 500 LOC
- `handle_agent_step` body shrinks from 720 LOC to ≤ 250 LOC
- All tests still pass

---

### D4. [COMPLETED] Split `artifacts.rs` (1,209 LOC → 4 files)

**Current structure:** `adapters/step_executor/artifacts.rs` contains:
- `WorktreeSnapshot` and its capture/delta logic (~150 LOC)
- `resolve_attached_artifacts`, `inject_artifact_contract` (~250 LOC)
- `resolve_declared_artifacts`, `read_worktree_file`, `compute_git_diff`, `commit_worktree_changes` (~250 LOC)
- Internal helpers `parse_status_porcelain`, `git_status_porcelain`, `unquote_git_path`, `is_excluded`, `is_likely_binary`, `strip_extension` (~150 LOC)
- 16 tests (~600 LOC, half the file)

**Proposed split:**
- `infrastructure/step_executor/artifacts/snapshot.rs` (~300 LOC — `WorktreeSnapshot`, `WorktreeSnapshot::capture`, `WorktreeSnapshot::delta`, `parse_status_porcelain`, `git_status_porcelain`)
- `infrastructure/step_executor/artifacts/attached.rs` (~200 LOC — `resolve_attached_artifacts`, `inject_artifact_contract`)
- `infrastructure/step_executor/artifacts/declared.rs` (~250 LOC — `resolve_declared_artifacts`, `read_worktree_file`, `compute_git_diff`, `commit_worktree_changes`, `unquote_git_path`, `is_excluded`, `is_likely_binary`, `strip_extension`)
- `tests/infrastructure/step_executor/artifacts.rs` (~600 LOC — all tests)

**Side effect:** Wrap `std::fs::read_to_string` inside `read_worktree_file` (audit smell — port-bypass) with a `WorktreeFileReader` port trait. The `FsArtifactStore` already abstracts file I/O for the artifact store; extend it with `read_local_file(path)`.

**Definition of done:**
- Each file ≤ 500 LOC
- No `std::fs::*` calls outside the new `WorktreeFileReader` adapter
- All 155 tests still pass

---

### D5. [COMPLETED] Split `git_ops.rs` (1,584 LOC → 6 files)

**Current structure:** `adapters/worktree/git_ops.rs` contains every git operation Demeteo issues, in one file.

**Proposed split:**
- `infrastructure/worktree/git_ops.rs` (NEW — facade, ≤ 100 LOC, delegates to modules below)
- `infrastructure/worktree/clone.rs` (~200 LOC — `clone_repository`, `get_provider_pat`)
- `infrastructure/worktree/strategy.rs` (~200 LOC — `detect_worktree_strategy`, `fallback_default_branch`, `LanguageGuesser`)
- `infrastructure/worktree/worktree.rs` (~250 LOC — `provision_subtask_worktree`, `cleanup_subtask_worktree`, `list_worktrees`, `get_head_branch`, `create_feature_branch`)
- `infrastructure/worktree/merge.rs` (~250 LOC — `precheck_merge`, `merge_subtask`)
- `infrastructure/worktree/sync.rs` (~300 LOC — `ensure_default_branch_updated`, `sync_feature_with_upstream`, `provision_sync_worktree`)
- `infrastructure/worktree/health.rs` (~80 LOC — `check_repo_dirty`)
- `tests/infrastructure/worktree/git_ops.rs` (~500 LOC — moves the inline integration tests)

**Side effect:** `GitOpsHelper` becomes a thin facade (`pub struct GitOpsHelper` re-exporting methods from each submodule). After Phase C1 it also implements `WorktreeOpsPort`.

**Definition of done:**
- No file > 500 LOC
- `GitOpsHelper` body ≤ 100 LOC (delegation only)
- All 155 tests still pass

---

### D6. [COMPLETED] Split `driver.rs` (749 LOC → 3 files)

**Current structure:** `adapters/step_executor/driver.rs` contains:
- `ExecutionDriver::run` (~250 LOC — main loop)
- `ExecutionDriver::fail_step_and_feature` (~50 LOC)
- `ExecutionDriver::cancel_feature` (~30 LOC)
- `ExecutionDriver::evaluate_on_failure` (~80 LOC)
- `ExecutionDriver::run_verifier_logic` (~280 LOC)
- 6 tests (~120 LOC)

**Proposed split:**
- `infrastructure/step_executor/driver/mod.rs` (~250 LOC — the main loop, now thin)
- `infrastructure/step_executor/driver/failure.rs` (~150 LOC — `fail_step_and_feature`, `evaluate_on_failure`, retry budget)
- `infrastructure/step_executor/driver/verifier.rs` (~300 LOC — `run_verifier_logic`; or move to `step_executor/agent/verifier.rs` from D3)
- `infrastructure/step_executor/step_writer.rs` (NEW, ~200 LOC — the `StepStatusWriter` helper that replaces the 12 hand-coded `StepExecutionPatch + DomainEvent::StepProgress` pairs at `driver.rs:100-113, 124-125, 184-235, 283-333, 358-455` — audit smell #8)
- `tests/infrastructure/step_executor/driver.rs` (~120 LOC)

**Definition of done:**
- Each file ≤ 500 LOC
- The 12 hand-coded `StepExecutionPatch + DomainEvent::StepProgress` pairs reduce to one helper call site each
- All tests pass

---

### D7. [COMPLETED] Split `impl_traits.rs` (840 LOC → 3 files)

**Current structure:** `adapters/step_executor/impl_traits.rs` contains:
- `impl DagStepExecutor::start_execution_loop` (~190 LOC)
- `impl StepExecutor for DagStepExecutor`: `feature_start` (165), `step_retry` (135), `replay_from_step` (140), and 6 others
- `impl GatePresenter for DagStepExecutor`
- `startup_watchdog` (~80 LOC)

**Proposed split:**
- `infrastructure/step_executor/impl_traits/mod.rs` (~200 LOC — re-exports + the trait impl glue)
- `infrastructure/step_executor/impl_traits/execution_context.rs` (~150 LOC — `resolve_execution_context` + `start_execution_loop` prelude; eliminates the 6-DB-hit prelude duplicated between `start_execution_loop` and `feature_start`)
- `infrastructure/step_executor/impl_traits/replay.rs` (~200 LOC — `replay_steps_from`; both `step_retry` and `replay_from_step` become 5-line wrappers — audit smell #9)

**Definition of done:**
- `step_retry` and `replay_from_step` differ only by `include_target: bool`
- The 6-DB-hit prelude in `feature_start` is `ExecutionContext::resolve(...)` (single call)
- All tests pass

---

### D8. [COMPLETED] Merge the two `sync.rs` files + split

**Current state:** TWO `sync.rs` files in `adapters/step_executor/`:
- `sync.rs` (550 LOC) — `feature_sync_impl`, `feature_resolve_sync_conflicts_impl`, `resolve_sync_conflicts_shared` (200 LOC, 14 params)
- `steps/sync.rs` (272 LOC) — `handle_sync_step`, `resolve_sync_conflicts_in_step`

**Proposed split:**
- `infrastructure/step_executor/feature_sync.rs` (~300 LOC — feature-level orchestration)
- `infrastructure/step_executor/steps/sync.rs` (~200 LOC — step handler)

**Side effect:** The `resolve_sync_conflicts_shared` function takes 14 params (audit smell #9) — extract a `ResolveSyncContext` struct that bundles them.

**Definition of done:**
- Two distinct files, no ambiguity
- `ResolveSyncContext` struct replaces the 14-param signature
- All tests pass

---

### D9. [COMPLETED] Split domain/models.rs (592 LOC → 8 files)

**Current structure:** `domain/models.rs` contains 34 entity structs in one file.

**Proposed split (one file per aggregate):**
- `domain/models/mod.rs` (re-exports)
- `domain/models/machine.rs` (Machine, AgentProfile)
- `domain/models/project.rs` (Project, Repository, ProjectSettings)
- `domain/models/feature.rs` (Feature, StepExecution)
- `domain/models/workflow.rs` (Workflow, WorkflowVersion, WorkflowSchedule, StepConfig)
- `domain/models/thread.rs` (ThreadSession, Message, AgentConfig, WorkingMemoryEntry, SessionInfo, ConfigOptionValue)
- `domain/models/provider.rs` (ProviderInstance)
- `domain/models/merge.rs` (MergeOutcome, ConflictReport, MergePreCheck, ConflictFile, ConflictPolicy, UpstreamSyncOutcome, UpstreamSyncFailure)
- `domain/models/agent_config.rs` (RepoHealthStatus, WorktreeInfo)

**Definition of done:**
- No file > 250 LOC
- All `use domain::models::*` import paths still resolve via the `mod.rs` re-exports
- All tests pass

---

### D10. [COMPLETED] Split `adapters/database/repos/feature.rs` (815 LOC → 2 files)

**Current structure:** Owns both `FeatureRepository` and `StepExecutionRepository` SQL.

**Proposed split:**
- `infrastructure/database/repos/feature.rs` (~350 LOC — feature CRUD only)
- `infrastructure/database/repos/feature_steps.rs` (~450 LOC — step-execution CRUD extracted)

**Definition of done:**
- Each file ≤ 500 LOC
- The 7 sub-port traits still resolve correctly
- All tests pass

---

### D11. [COMPLETED] Move `#[cfg(test)] mod tests` blocks to `src-tauri/tests/`

**Migration targets** (17 sites identified in the audit):
- `domain/{ids,agent_event,artifact,intercept,prompt_context}.rs` → `tests/domain/{ids,agent_event,artifact,intercept,prompt_context}.rs`
- `adapters/database/tests.rs` → `tests/infrastructure/database/mod.rs`
- `adapters/database/repos/feature.rs` (~450 LOC of tests) → `tests/infrastructure/database/feature_steps.rs`
- `adapters/artifact_store/fs.rs` (~100 LOC) → `tests/infrastructure/artifact_store.rs`
- `adapters/agent/{cli_runtime,install,registry,opencode}.rs` → `tests/infrastructure/agent/...`
- `adapters/merge.rs` (~110 LOC) → `tests/infrastructure/merge.rs`
- `adapters/mr_publisher.rs` (~30 LOC) → `tests/infrastructure/mr_publisher.rs`
- `adapters/pricing.rs` (~70 LOC) → `tests/infrastructure/pricing.rs`
- `adapters/scheduler.rs` (~50 LOC) → `tests/infrastructure/scheduler.rs`
- `adapters/ssh/client.rs` (~80 LOC) → `tests/infrastructure/ssh.rs`
- `adapters/step_executor/steps/parallel.rs` (~210 LOC) → `tests/infrastructure/step_executor/parallel.rs`
- `adapters/step_executor/artifacts.rs` (~600 LOC) → `tests/infrastructure/step_executor/artifacts.rs`
- `adapters/step_executor/tests/mod.rs` → `tests/e2e/step_executor.rs`
- `adapters/worktree/git_ops.rs` (~500 LOC) → `tests/infrastructure/worktree/git_ops.rs`

**Approach:** Each `#[cfg(test)] mod tests` block is renamed `#[cfg(test)] pub(crate) mod tests_for_X` and re-exported. Then `tests/X.rs` re-exports those internals and re-wraps them. **OR** simpler: use `#[path = ...]` to move the test bodies into `tests/` while keeping the `#[cfg(test)]` gating in the source files. **RECOMMENDED:** start with a `tests/common/mod.rs` for shared fakes (`FakeExec`, `FakeNotif`, `FakeAgentSession`, in-mem Sqlite), then incrementally move each test module.

**Definition of done:**
- Zero `#[cfg(test)] mod tests` blocks in `src/` source files
- All 155 tests still pass under `cargo test --tests`
- A `tests/common/` directory provides shared test fakes

---

## Phase E — Application Layer Extraction

### E1. [COMPLETED] Create `application/` module

The 14 commands flagged in the audit as containing business logic (audit §4) get extracted into use cases in `application/`. Each command file then becomes ≤ 30 LOC of pure IPC glue.

**New use cases:**
- `application::projects::health_check(ctx, project_id)` — extracts the 85-LOC body of `get_workspace_health` from `commands/project.rs:289-373`
- `application::projects::delete_workspace(ctx, project_id)` — extracts the 50-LOC body of `delete_project` from `commands/project.rs:178-230`
- `application::projects::update(ctx, project_id, patch)` — extracts the 40-LOC body of `update_project`
- `application::projects::check_repos_dirty(ctx, project_id)` — extracts `check_repos_dirty`
- `application::bootstrap::bootstrap_project(ctx, input)` — extracts the 135-LOC body of `do_bootstrap_inner` from `commands/bootstrap.rs`
- `application::providers::validate_pat(ctx, host, kind, pat)` — extracts the 60-LOC body from `commands/providers.rs`
- `application::providers::fetch_repos(ctx, host, kind, pat)` — extracts the 100-LOC body from `commands/providers.rs`
- `application::providers::connect_instance(ctx, input)` — extracts `connect_provider_instance`
- `application::lifecycle::feature_cleanup(ctx, feature_id, lifecycle)` — extracts the 120-LOC body of `feature_cleanup` from `commands/feature_lifecycle.rs`
- `application::agents::prompt(ctx, thread_id, prompt)` — extracts the 130-LOC `tokio::spawn` closure from `commands/agent_lifecycle.rs::agent_prompt`
- `application::agents::start_with_install(ctx, thread_id, kind)` — extracts `agent_install_and_start`
- `application::agent_probe::discover_models(ctx, machine_id, kind)` — extracts the 60-LOC body from `commands/agent_config_probe.rs`

**Use-case signature pattern:**
```rust
pub async fn use_case_name(
    ctx: &AppContext,
    input: InputStruct,
) -> Result<OutputStruct, AppError> {
    // ... business logic ...
}
```

**Definition of done:**
- `commands/*.rs` files collectively ≤ 800 LOC (down from current ~2,500 LOC)
- Each use case is independently unit-testable without a Tauri `AppHandle`
- `grep -RnE 'tauri::AppHandle' src/application/` returns 0 lines
- All 155 tests still pass

---

## Phase F — Polish & Hardening

### F1. [COMPLETED] Replace remaining `.unwrap()` / `.expect()` in production paths

All `.unwrap()` / `.expect()` in `commands/`, `application/`, and `infrastructure/` (excluding tests) eliminated. Final verification (`grep -RnE '\.expect\(|\.unwrap\(\)' src/{commands,application,infrastructure} | grep -v 'cfg(test)' | grep -v '/tests/'`) returns 0 lines.

---

### F2. [COMPLETED] Migrate `std::sync::Mutex` to `parking_lot::Mutex` everywhere

`parking_lot` is now used for all sync mutexes that don't need to be held across `.await` points. `AgentRegistry::availability_cache` migrated to `tokio::sync::Mutex` (it's used inside async methods, so `parking_lot` was the wrong choice — it was std::sync::Mutex which poisoned).

---

### F3. [COMPLETED] Replace `tokio::task::block_in_place` and `tokio::runtime::Builder`

**Three sites eliminated:**
- `cli_runtime.rs::is_available` — was using `block_in_place + block_on` from inside a sync method. Now `AgentRuntime::is_available` is `async_trait`, and the implementation just `.await`s the port call directly.
- `mr_publisher.rs::ReqwestHttp::post_json` + `get_json` — was constructing a fresh `tokio::runtime::Builder::new_current_thread()` inside `std::thread::spawn` for each HTTP call. Now `HttpClient` is `#[async_trait]`, and `ReqwestHttp` uses `reqwest` directly.

**Files touched:** `ports/agent_runtime.rs`, `adapters/agent/{cli_runtime,noop,opencode/mod,registry}.rs`, `adapters/mr_publisher.rs`, `commands/agent_config.rs`, `application/agents.rs`, `tests/infrastructure/agent/registry.rs`.

**Final verification:** `grep -RnE 'tokio::task::block_in_place|tokio::runtime::Builder' src/` returns 0 lines. `grep -RnE 'std::thread::spawn' src/{application,commands}/` returns 0 lines.

---

### F4. [COMPLETED] Strip redundant `async` from commands

After F3's async-trait migration, audited every `pub async fn` in `commands/`. Stripped `async` from commands that don't actually `.await`:

- `commands/features.rs` — 2 sites (`fetch_active_features`, `feature_get`)
- `commands/workflows.rs` — 10 sites (all commands; none await)
- `commands/machine.rs` — already done previously (4 of 5)

`commands/agent_config.rs::get_agent_configs` was promoted to `async` (it now awaits `registry.is_available`).

**Final verification:** `grep -nE 'pub async fn' src/commands/{features,workflows,machine}.rs` shows only commands that genuinely `.await`.

---

### F5. [COMPLETED] Migrate `Result<T, String>` → `Result<T, AppError>`

All command return types migrated to `Result<T, AppError>`. Frontend catch sites updated to handle both legacy string errors and new tagged-union shape. The wire format now carries `{kind, message}` instead of free-form string.

**Backend changes:**
- All 15 `commands/*.rs` files use `AppError` in their return types
- `From<String> for AppError` and `From<&str> for AppError` (both → `AppError::internal`) preserved for compatibility with port call sites that still return `Result<_, String>`
- Mechanical `.map_err(AppError::from)` added at command-layer port boundaries

**Frontend changes:**
- 5 catch sites updated in `App.tsx` and `ProjectSettings.tsx` to handle string OR `{message: string}` error shapes

**Final verification:** `grep -RnE 'Result<.*, String>' src/commands/` returns 0 lines. `cargo clippy --all-targets -- -D warnings` is clean. `npx tsc --noEmit` is clean. 153 unit tests pass.

---

## Frontend Error Handling (U1–U4) — COMPLETED

Parallel work on the React frontend to complete the error-display picture after F5 changed the IPC wire format:

| Task | What | Status |
|------|------|--------|
| **U1** | Eliminate `String(err)` from all catch sites (22 sites) — replaced with `formatError(err)` | COMPLETED |
| **U2** | Centralized error display — `errorBus.tsx` singleton store, `ErrorBusProvider`, `useErrorBus()` hook, imperative `reportError()`, `ErrorToast` bottom-right stack with auto-dismiss | COMPLETED |
| **U3** | Kind-specific CTAs — provider→"Open providers", conflict→"Open feature", transport→"Retry", agent/internal→"View logs", copy button on every toast | COMPLETED |
| **U4** | Catch-site audit — migrated all silent `console.error`/`console.warn` in catch blocks to `reportError()` across 10 component files; eliminated all `err.toString()` calls | COMPLETED |

**Files created:** `src/lib/errors.ts`, `src/lib/errorBus.tsx`, `src/components/ErrorToast.tsx`
**Files modified:** `src/types.ts`, `src/App.tsx`, `src/components/FeatureDetail.tsx`, `src/components/GateView.tsx`, `src/components/NewProjectView.tsx`, `src/components/ProjectHome.tsx`, `src/components/ProjectSettings.tsx`, `src/components/WorkflowEditor.tsx`, `src/components/WorkflowList.tsx`, `src/components/ArtifactViewer.tsx`

---

## Phase G — Final Verification

After Phase F, the verification checklist (from the original plan):

```bash
# Frontend
npx tsc --noEmit                              # already passes

# Rust
cd src-tauri && cargo fmt
cd src-tauri && cargo clippy --all-targets -- -D warnings
cd src-tauri && cargo test --lib              # 155+ tests passing
cd src-tauri && cargo test --tests            # integration tests in tests/

# App boots
npm run tauri dev                             # confirm no console errors
```

**Acceptance:**
- Zero `cargo clippy` warnings with `-D warnings`
- All tests pass (155 unit + new integration tests)
- App boots, ProjectRail renders, and a feature can be started end-to-end

---

## Effort Estimate

| Phase | Tasks | Effort | Risk |
|-------|-------|--------|------|
| C — Missing Ports | C1–C6 | 4-5 days | Medium (touches every command file) |
| D — Deep Modules | D1–D11 | 8-10 days | High (large refactor of 5 monolith files) |
| E — Application Layer | E1 | 2-3 days | Medium |
| F — Polish | F1–F5 | 2 days | Low |

**Total:** ~16-20 working days.

## Priority Order

1. **D1** (event_stream extraction) — single biggest impact, kills 5× duplication
2. **C1** (`WorktreeOpsPort`) — closes the biggest hexagon leak
3. **F3** (block_in_place removal) — correctness risk
4. **C2** (`ProviderHttpPort` + delete `/tmp/demeteo_fetch.log`) — security risk
5. **D6** (`StepStatusWriter` extraction) — DRY win in the executor
6. **D2–D5** (parallel/agent/artifacts/git_ops splits) — file-size hygiene
7. **D7–D11** (impl_traits, sync, models, repos, test migration) — incremental
8. **C3–C6** (remaining ports + consolidations) — smaller cleanups
9. **E1** (application layer) — depends on C and D
10. **F1–F5** (polish) — final pass
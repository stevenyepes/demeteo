# Loop Engineering Gaps — Implementation Plan

> **Scope**: Close three concrete loop-engineering gaps in Demeteo's pipeline.
> Skills and MCP connectors are deliberately out of scope — those belong in the
> project repository alongside `AGENTS.md`.

---

## What's Already There (Don't Re-Build)

Before writing a line of code, verify this from the source:

| Primitive | Status |
|-----------|--------|
| `max_iterations` + `on_failure → goto` | ✅ **Fully implemented** in `driver.rs::evaluate_on_failure` |
| Real parallel subtask DAG from planner | ✅ **Fully implemented** in `steps/parallel.rs` (planner → fan-out → merge) |
| Worktree isolation per step/subtask | ✅ **Fully implemented** |
| Named Test Harnesses Support (L0) | ✅ **Fully implemented** |
| Maker-Checker / Verifier Kind (L1) | ✅ **Fully implemented** |
| ProjectMemory / Context Injection (L2) | ✅ **Fully implemented** |
| Workflow Scheduling / Cron Loop (L3) | ✅ **Fully implemented** |

The three **actual gaps** are now fully closed and implemented:

1. **Maker-Checker** — ✅ **Fully implemented**
2. **ProjectMemory** — ✅ **Fully implemented**
3. **Workflow Scheduling** — ✅ **Fully implemented**

---

## Open Questions

> [!IMPORTANT]
> Please clarify these before execution starts.

1. **Verifier agent identity**: Should the verifier always be a *different* agent kind
   than the implementer (e.g., implementer = `opencode`, verifier = `antigravity`)?
   Or can it be the same agent kind with different instructions?

2. **ProjectMemory injection strategy**: Should memory entries be injected as a
   preamble to the prompt template (replacing `{{project_memory}}`), or always
   appended regardless of the template? What's the token budget cap?

3. **Scheduler trigger model**: Should the scheduler only support simple cron
   expressions, or also event-based triggers (e.g., "run after a PR is merged")?
   V1 cron-only is the safe default.

4. **Scheduler storage**: Store pending schedules in SQLite (durable, survives
   restart) or only in-memory (simpler, loses on restart)? SQLite is recommended
   for a durable loop, but needs a migration.

---

## Proposed Changes

---

### L0 — Multiple Harnesses Support

**Goal**: Allow projects to define multiple test harnesses in their settings (e.g. unit tests vs e2e tests), and allow verifier steps to run a specific harness by name.

#### [NEW] Migration: `V8__project_harnesses.sql`

```sql
ALTER TABLE projects ADD COLUMN harnesses TEXT; -- JSON array or map
```

#### [MODIFY] `domain/models.rs`

Update `WorktreeStrategy` to include `harnesses: Option<HashMap<String, String>>`. This allows storing multiple named test commands alongside the default `test_command`.

#### [MODIFY] `ProjectSettings.tsx`

Add a UI to manage the multiple harnesses (key-value pairs of harness name and command) under the Project Settings -> Worktree Strategy section.

---

### L1 — Maker-Checker (Verifier Step Kind)

**Goal**: The implementer cannot mark its own work done. An independent verifier
agent runs the project's test command, reads the implementer's artifact, and emits
a structured `pass | fail | retry(reason)` verdict. On `fail`, the driver loops
back via the step's `on_failure` edge (uses the already-implemented conditional
edge logic). On `max_iterations` exhaustion, it surfaces a `GateRequired` event.

#### [MODIFY] [models.rs](file:///Users/stevenyepes/Projects/MostRecent/demeteo/src-tauri/src/domain/models.rs)

Add `verifier: Option<VerifierConfig>` to `StepConfig`. `VerifierConfig` contains:
- `agent_kind: Option<String>` — defaults to the same agent as the step
- `instructions: String` — what the verifier should check
- `verdict_key: String` — JSON key to extract from the verifier's output
  (default `"verdict"`)

No new step kind needed. The verifier is an optional post-flight sub-agent that
runs *after* `TurnComplete` from the implementer, before the step is marked
`completed`.

#### [MODIFY] [steps/agent.rs](file:///Users/stevenyepes/Projects/MostRecent/demeteo/src-tauri/src/adapters/step_executor/steps/agent.rs)

After the implementer's event loop finishes with a non-failed outcome:
1. If `step_conf.verifier` is `Some(vc)`, spawn a second agent session in the
   **same worktree** (read-only — no git write-back from the verifier).
2. Verifier prompt = `vc.instructions` + the step's artifact summary + the
   specific harness command (looked up by `vc.harness_name` from the project's `WorktreeStrategy.harnesses`, falling back to `test_command` if none).
3. Drain the verifier's stream. Extract `{ "verdict": "pass" | "fail",
   "reason": "..." }` from the text output using the same JSON-fence extractor
   that `parallel.rs` uses for `SubtaskDag`.
4. On `"pass"` → proceed to `StepOutcome::Completed` as today.
5. On `"fail"` → return `StepOutcome::Failed(reason)`. The driver's existing
   `evaluate_on_failure` logic then applies: if `on_failure` is set and the
   budget allows, the driver loops back; otherwise it gates or fails.

#### [MODIFY] [steps/parallel.rs](file:///Users/stevenyepes/Projects/MostRecent/demeteo/src-tauri/src/adapters/step_executor/steps/parallel.rs)

Same verifier hook after all worker subtasks complete and merge succeeds. The
verifier runs against the feature branch (not an individual subtask worktree).

#### [NEW] [domain/verifier.rs](file:///Users/stevenyepes/Projects/MostRecent/demeteo/src-tauri/src/domain/verifier.rs)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerifierConfig {
    /// Agent kind for the verifier. `None` = same as the step's agent_kind.
    pub agent_kind: Option<String>,
    /// Instructions injected as the verifier's prompt preamble.
    pub instructions: String,
    /// Name of the harness to run (e.g. "lint", "integration"). If `None`, falls back to the project's default `test_command`.
    pub harness_name: Option<String>,
    /// JSON key whose value must be `"pass"` or `"fail"`. Default: `"verdict"`.
    #[serde(default = "default_verdict_key")]
    pub verdict_key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VerifierVerdict {
    Pass,
    Fail(String), // reason
}
```

#### [MODIFY] [WorkflowEditor.tsx](file:///Users/stevenyepes/Projects/MostRecent/demeteo/src/components/WorkflowEditor.tsx)

Add a collapsible "Verifier" section to the step editor form:
- Toggle to enable/disable
- Agent kind dropdown (defaults to same as step)
- Instructions textarea
- Harness selector (dropdown of harnesses defined in project settings)
- Preview of the verifier prompt

#### [MODIFY] [FeatureDetail.tsx](file:///Users/stevenyepes/Projects/MostRecent/demeteo/src/components/FeatureDetail.tsx)

When a step has a verifier active, show a distinct "Verifying…" sub-state badge
between "Running" and "Completed" (uses the existing `StepProgress` event stream,
no new Tauri event needed — the verifier emits into the same `AgentStream`).

---

### L2 — ProjectMemory (Cross-Run Persistent Context)

**Goal**: Gate feedback and key agent-written facts persist across feature runs
within a project. Each new feature's prompt context is seeded with relevant
memories. Human overrides in Gate feedback are automatically written as
high-priority memory entries.

#### [NEW] Migration: `V6__project_memory.sql`

```sql
CREATE TABLE project_memory (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL,
    key         TEXT NOT NULL,          -- semantic label, e.g. "last_test_failure"
    value       TEXT NOT NULL,          -- markdown / prose
    source      TEXT NOT NULL CHECK(source IN ('agent','human')),
    confidence  REAL NOT NULL DEFAULT 1.0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);
CREATE INDEX idx_pm_project ON project_memory(project_id, updated_at DESC);
```

> [!WARNING]
> This is an additive migration (new table, no column drops). Safe for silent
> auto-migration under Decision 30.

#### [NEW] [domain/memory.rs](file:///Users/stevenyepes/Projects/MostRecent/demeteo/src-tauri/src/domain/memory.rs)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMemoryEntry {
    pub id: String,
    pub project_id: ProjectId,
    pub key: String,
    pub value: String,
    pub source: MemorySource,  // Agent | Human
    pub confidence: f64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource { Agent, Human }
```

#### [NEW] [ports/memory.rs](file:///Users/stevenyepes/Projects/MostRecent/demeteo/src-tauri/src/ports/memory.rs)

```rust
pub trait ProjectMemoryPort: Send + Sync {
    fn memory_upsert(&self, entry: ProjectMemoryEntry) -> Result<(), String>;
    fn memory_list(&self, project_id: &ProjectId, limit: usize)
        -> Result<Vec<ProjectMemoryEntry>, String>;
    fn memory_delete(&self, id: &str) -> Result<(), String>;
}
```

#### [MODIFY] [adapters/database/sqlite.rs](file:///Users/stevenyepes/Projects/MostRecent/demeteo/src-tauri/src/adapters/database/sqlite.rs)

Implement `ProjectMemoryPort` with standard `INSERT OR REPLACE` on `(project_id, key)`.

#### [MODIFY] [domain/prompt_context.rs](file:///Users/stevenyepes/Projects/MostRecent/demeteo/src-tauri/src/domain/prompt_context.rs)

Add `project_memory: String` to `PromptContext`. Populated at feature-start time
by serialising the top-N memory entries (sorted by `confidence DESC, updated_at
DESC`, capped at ~1500 tokens of text) into a markdown block:

```
## Project Memory
- [human] last_test_failure: "The integration tests fail when REDIS_URL is unset"
- [agent] preferred_fix_style: "Minimal diffs; no gratuitous refactors"
```

#### [MODIFY] `adapters/step_executor/setup.rs` (feature start)

After resolving the `PromptContext`, query `ProjectMemoryPort::memory_list` and
inject into `base_ctx.set("project_memory", &memory_block)`.

#### Gate feedback → memory (automatic write-back)

In `handle_gate_step` (`steps/gate.rs`), when a gate decision is `"approve"` or
`"redirect"` with non-empty `feedback`:
1. Call `memory_upsert` with `key = "gate_feedback_<step_id>"`,
   `value = feedback`, `source = Human`.
2. This makes human corrections available to the *next* feature run without any
   UI work.

#### Tauri commands (in `commands/project.rs`)

- `project_memory_list(project_id) -> Vec<ProjectMemoryEntry>`
- `project_memory_upsert(project_id, key, value, source) -> ()`
- `project_memory_delete(id) -> ()`

#### [MODIFY] [ProjectSettings.tsx](file:///Users/stevenyepes/Projects/MostRecent/demeteo/src/components/ProjectSettings.tsx)

Add a "Project Memory" tab: a sortable list of memory entries with edit/delete.
Each entry shows key, value preview, source badge (Agent/Human), and timestamp.

---

### L3 — Workflow Scheduling

**Goal**: A `Workflow` can declare a cron schedule. A background `SchedulerTask`
in `AppState` checks every 60 seconds and fires `feature_start` for due
workflows. The next-run timestamp is durable (stored in SQLite).

#### [NEW] Migration: `V7__workflow_schedule.sql`

```sql
ALTER TABLE workflows ADD COLUMN schedule_cron TEXT;   -- NULL = manual only
ALTER TABLE workflows ADD COLUMN schedule_title_template TEXT; -- e.g. "Daily CI sweep {{date}}"
ALTER TABLE workflows ADD COLUMN schedule_next_run_at INTEGER; -- unix ms; NULL = not scheduled
ALTER TABLE workflows ADD COLUMN schedule_project_id TEXT;     -- which project to target
```

> [!WARNING]
> Additive `ALTER TABLE` only. Safe for silent auto-migration.

#### [MODIFY] [domain/models.rs](file:///Users/stevenyepes/Projects/MostRecent/demeteo/src-tauri/src/domain/models.rs)

Add `schedule: Option<WorkflowSchedule>` to `Workflow`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSchedule {
    pub cron: String,                      // standard 5-field cron expression
    pub title_template: String,            // e.g. "Daily sweep {{date}}"
    pub project_id: ProjectId,             // which project to spawn features on
    pub next_run_at: Option<i64>,          // unix ms; maintained by scheduler
}
```

#### [NEW] [adapters/scheduler.rs](file:///Users/stevenyepes/Projects/MostRecent/demeteo/src-tauri/src/adapters/scheduler.rs)

A `SchedulerTask` that:
1. Spawned once at app startup inside `lib.rs` alongside other background tasks.
2. Polls every 60 seconds.
3. Queries `workflow_list_with_schedule()` for workflows where
   `schedule_next_run_at <= now_ms`.
4. For each due workflow, calls `feature_start` via the existing
   `FeatureOrchestrator` port (not a raw Tauri command — respects the hexagon).
5. Updates `schedule_next_run_at` to the next cron occurrence using a tiny
   cron-parser helper (no new `cargo` dependency — a simple 5-field parser in
   ~100 LOC is sufficient for `*/5`, `0 9 * * 1-5`, etc.).
6. Emits `DomainEvent::FeatureStatusChanged` so the UI reflects the auto-started
   feature without any additional plumbing.

> [!IMPORTANT]
> Adding a cron parser dependency (e.g. `cron`) requires approval per
> AGENTS.md §7. The plan assumes a lightweight inline parser to avoid this.
> If you'd prefer using `cron` crate, flag it for approval.

#### [MODIFY] [WorkflowEditor.tsx](file:///Users/stevenyepes/Projects/MostRecent/demeteo/src/components/WorkflowEditor.tsx)

Add a "Schedule" section (collapsed by default):
- Enable/disable toggle
- Cron expression input with human-readable preview
  (e.g. `0 9 * * 1-5` → "Every weekday at 09:00")
- Project selector (dropdown of existing projects)
- Title template input
- "Next run" preview (computed locally)

#### [MODIFY] [ProjectHome.tsx](file:///Users/stevenyepes/Projects/MostRecent/demeteo/src/components/ProjectHome.tsx)

Show a "Scheduled" badge on workflow cards that have a cron schedule active.
Show `next_run_at` formatted as "in 3h 20m" in the workflow list.

---

## Dependency Order

```
L1 (Verifier) — no schema change; pure Rust + UI
L2a (Memory schema + port + DB) — prerequisite for L2b/L2c
L2b (PromptContext injection + gate write-back) — depends on L2a
L2c (ProjectSettings UI) — depends on L2a commands
L3a (Workflow schema + scheduler) — independent
L3b (WorkflowEditor + ProjectHome UI) — depends on L3a
```

Suggested execution order: **L1 → L2a → L2b → L3a → L2c → L3b**

---

## Verification Plan

### Automated Tests

```bash
# Rust
cd src-tauri
cargo test --lib step_executor          # verifier path + loopback
cargo test --lib memory                 # upsert/list/prune
cargo test --lib scheduler              # cron next-occurrence calc
cargo clippy -- -D warnings
cargo fmt --check

# Frontend
npx tsc --noEmit
```

### Manual Smoke Tests

| # | Test | Expected |
|---|------|----------|
| 1 | Run an `agent` step with `verifier.instructions = "emit {\"verdict\":\"fail\"}"` | Step loops back via `on_failure`; Gate surfaces after `max_iterations` exhausted |
| 2 | Approve a Gate with non-empty feedback; start a new feature on the same project | Gate feedback appears in `{{project_memory}}` in the new feature's prompt |
| 3 | Set workflow schedule to `* * * * *`; wait 2 minutes | Two features auto-started; `schedule_next_run_at` updated in DB |
| 4 | Delete a scheduled workflow | No further features started; scheduler ignores it |
| 5 | App restart mid-schedule | Scheduler re-fires any overdue workflows within first 60s poll |

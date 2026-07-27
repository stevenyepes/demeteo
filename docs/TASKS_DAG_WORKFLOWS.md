# Task Plan — True DAG Workflows (Phase 4, remaining)

**Source PRD:** [`PRD_DAG_WORKFLOWS.md`](PRD_DAG_WORKFLOWS.md)

Phases 0–3 shipped (P0.1 → P3.6, 2026-07-23 → 2026-07-26); their task bodies are
retired — the code is the record, and `git log --grep='(dag)'` has the
commit-by-commit trail. What survives here is **Phase 4**: the four tasks that
have not been built, plus the shared code coordinates each of them needs.

## How to run a task

1. Read this file's header + the single task section. Read the PRD sections the
   task references — not the whole PRD.
2. Load only the files under **Context** (respect line ranges; several source
   files are >1k lines).
3. Stay inside **Touch**. If the task turns out to require edits outside it,
   stop and report — that's a decomposition bug in this plan; fix the plan first.
4. Run the task's **Done when** checks, plus `npm run checks`.
5. Commit per task (`feat(dag): P4.1 write-scope lint + parallelism`) and flip
   the checkbox below.

**Sizing rule:** ≤ ~2,000 lines of required reading, one coherent diff, no task
depends on holding two subsystems in context at once.

**Migrations:** V31 `step_attempts`, V32 durable checkpoints, V33
`features.workflow_version_id`, V34 `workflow_versions.definition_json` are
built — the next free number is **V35**.

### Key code coordinates (shared reference — don't re-discover these)

> Line numbers drift. Re-verify with `git grep` before relying on one.

| What | Where |
|---|---|
| `ExecutionDriver` struct + dispatch loop | `crates/demeteo-core/src/adapters/step_executor/driver.rs` |
| Node-type registry + step handlers | `crates/demeteo-core/src/adapters/step_executor/steps/` (`command.rs` is the newest handler template) |
| `StepOutcome` enum | `crates/demeteo-core/src/adapters/step_executor/steps/mod.rs` |
| Ready-set scheduler | `crates/demeteo-core/src/adapters/step_executor/scheduler.rs` |
| Retry/failure handling | `.../step_executor/driver/failure.rs` (`evaluate_on_failure`) |
| Definition model + lint | `crates/demeteo-core/src/domain/models/workflow.rs`, `workflow_graph.rs` |
| Expression grammar | `crates/demeteo-core/src/domain/expr.rs` |
| Starters (7 JSON files) | `src-tauri/workflows/*.json`, seeded via `src-tauri/src/commands/workflows.rs` |
| DB repos | `crates/demeteo-core/src/adapters/database/repos/` |
| Conformance suites | `crates/demeteo-core/tests/conformance/` — see [`EXECUTION_PARITY.md`](EXECUTION_PARITY.md) |
| Frontend canvas | `src/components/canvas/` (`connectRules.ts`, `ConfigPanel.tsx`) |
| Status vocabulary | `src/lib/runStatus.ts` (`runStatusMeta`, `TERMINAL_STATUSES`) |
| Tauri event hook | `src/hooks/useTauriEvent.ts` |

---

## Phase 4 — DAG payoff

Dependency order: `P4.1 ─▶ P4.2`. `P4.3` needs P1.5 + P3.2 (both shipped);
`P4.4` is independent.

### P4.1 — Write-scope exclusion lint + `max_parallel_nodes > 1`

- **Goal:** The §5.6 invariant twice: save-time lint (editor warning) and schedule-time hard check — concurrent nodes only if both `ReadOnly`/`Artifacts`, or `Implement` on disjoint repos; same-repo implement concurrency impossible by construction. Scheduler dispatches up to the ceiling.
- **Size:** large-ish. The only task that touches scheduler *and* driver concurrency.
- **Context:** PRD §5.6, §10 (merge-storm row); `scheduler.rs`; the `driver.rs` dispatch loop; `workflow.rs` `effective_capability` (or its v2 successor); `workflow_graph.rs`.
- **Touch:** `scheduler.rs` (ceiling + exclusion), `workflow_graph.rs` (lint rule), driver concurrency plumbing (bounded join set); tests: two ReadOnly nodes run concurrently under the stub agent, two same-repo implement nodes provably serialize.
- **Done when:** the invariant test cannot be defeated by config; default remains 1.

### P4.2 — Parallel shapes for Standard Feature + Refactor starters

- **Goal:** Ship the PRD §7 shapes: Standard = `research ∥ baseline-harness(command)` → `tickets`; `validate ∥ critic` → `gate-ship`. Refactor = `baseline(command)`; `regression ∥ api-drift-review` → `gate-diff`. The other five starters stay chains and must remain **bit-identical** to the P0.2 baseline snapshots.
- **Depends:** P4.1. **Size:** medium.
- **Context:** PRD §7; `src-tauri/workflows/standard-feature-pipeline.json` + `refactor.json`; seeding drift logic in `src-tauri/src/commands/workflows.rs`; the P0.2 baseline harness.
- **Touch:** the two starter JSONs (as v2), new baseline snapshots for those two, seeding test updates.
- **Done when:** both run green under the stub agent with genuine concurrency observed in `run_events` ordering; the five untouched starters' snapshots are unchanged.

### P4.3 — Conditional edges in the builder

- **Goal:** Expose `when` guards (the P1.5 grammar) in design mode: an edge inspector with an expression input, validated against the grammar + referenced node outputs. Skip-reason rendering is already handled by run mode.
- **Size:** medium.
- **Context:** PRD §5.1 (expressions), §6.3; the `expr.rs` public API; `ConfigPanel.tsx` patterns.
- **Touch:** edge inspector UI, client-side validation via an `expr_validate` Tauri command — **do not reimplement the grammar in TypeScript**.
- **Done when:** the PRD's critic example (`verdict != 'FAIL'`) is authorable end-to-end and skips correctly at runtime.

### P4.4 — `subworkflow` node type

- **Goal:** Reference a saved workflow version as a node; child run linked to parent; nesting depth 1, enforced by lint. Registry-only backend diff — the same rule P3.5 proved.
- **Size:** large-ish.
- **Context:** PRD §5.2 (subworkflow row); `steps/command.rs` as the handler template; `repos/feature.rs` (parent/child linkage columns — next free migration is **V35**); `driver_registry.rs` (child driver spawn dedup).
- **Touch:** new `steps/subworkflow.rs`, registration, migration for parent-run linkage, run-mode canvas "enter child" affordance.
- **Done when:** the parent run shows the child node with roll-up status; depth-2 nesting is rejected at lint.

---

## Status

| Task | Title | Status |
|---|---|---|
| P0.1 – P3.6 | Phases 0–3 (schema v2, engine, canvas, builder) | ✅ 2026-07-23 → 2026-07-26 |
| P4.1 | Write-scope lint + parallelism | ☐ |
| P4.2 | Parallel starter shapes | ☐ |
| P4.3 | Conditional edges UI | ☐ |
| P4.4 | `subworkflow` node | ☐ |

# Task Plan — True DAG Workflows

**Source PRD:** `docs/PRD_DAG_WORKFLOWS.md` (2026-07-23)
**Purpose:** decompose the PRD into tasks sized for **one agent session each**, with an explicit, bounded context budget per task. An agent working a task should load *only* the files listed under "Context", produce the diff described under "Touch", and prove it with the listed "Done when" checks.

---

## How to run a task

1. Read this file's header + the single task section. Read PRD sections referenced by the task — not the whole PRD.
2. Load only the files under **Context** (respect the line ranges when given; several source files are >1k lines).
3. Stay inside **Touch**. If the task turns out to require edits outside it, stop and report — that's a decomposition bug in this plan, fix the plan first.
4. Run the task's **Done when** checks (plus `cargo test -p demeteo-core` / `npm test` for the crate/app you touched).
5. Commit per task (`feat(dag): P1.4 workflow graph + structural lint`), and flip the task's checkbox in the status table below.

**Sizing rule used throughout:** ≤ ~2,000 lines of required reading, one coherent diff, no task depends on holding two subsystems in context at once. Tasks marked **(L)** are the largest and should be the *only* thing in their session.

### Facts that correct/augment the PRD (verified 2026-07-23)

- There is **no test that executes the 7 starters end-to-end** today — only lint-level coverage (`src-tauri/tests/workflows_lint.rs`). The PRD's Phase-1 exit ("byte-equivalent behavior vs baseline") therefore needs a baseline harness *built first* → task **P0.2**.
- There is **no JSON Schema validation anywhere** (no `jsonschema`/`schemars` dep). Validation is hand-rolled in `lint_workflow_steps` (`crates/demeteo-core/src/domain/models/workflow.rs:184`) → task **P1.3** introduces it.
- Frontend global state is **React Context** (`src/context/`), not zustand. `@xyflow/react`, `elkjs` are **not installed** → added in **P2.1**.
- `blocked_by` is **not** a parsed field on `StepConfig` — it exists only on planned tasks; no engine work needed to remove it.
- Migrations run V1–V30 (Flyway-style, `crates/demeteo-core/migrations/`); new tables start at **V31**.

### Key code coordinates (shared reference — don't re-discover these)

| What | Where |
|---|---|
| Step-kind dispatch `match` | `crates/demeteo-core/src/adapters/step_executor/driver.rs:810` |
| `ExecutionDriver` struct + in-memory state | `driver.rs:63` (`cached_plans` :240, `sequence_checkpoints` :254, `env_retried` :260, `retry_ctx` :188) |
| `StepOutcome` enum | `crates/demeteo-core/src/adapters/step_executor/steps/mod.rs:2` |
| Retry/failure handling | `crates/demeteo-core/src/adapters/step_executor/driver/failure.rs` (`evaluate_on_failure` :63) |
| Iteration-budget precedence | `driver.rs:612-618` |
| Definition model + lint | `crates/demeteo-core/src/domain/models/workflow.rs` (357 lines; `StepConfig` :39, `lint_workflow_steps` :184) |
| Starters (7 JSON files) | `src-tauri/workflows/*.json`, seeded via `src-tauri/src/commands/workflows.rs:11` |
| DB repos | `crates/demeteo-core/src/adapters/database/repos/` (`workflow.rs` 340, `feature_steps.rs` 252, `run_events.rs` 66, `gate.rs` 169) |
| `run_events` table | migration `V22__run_events.sql`; port impl `repos/run_events.rs` |
| Restart watchdog | `crates/demeteo-runner/src/reconcile.rs` (`reconcile_on_startup` :28) |
| Conformance suites | `crates/demeteo-core/tests/conformance/` (`topology_equivalence.rs` 494, `harness_triage.rs` 360); wired via `#[path]` mods from `src/` (e.g. `src/ports/execution.rs:283`) |
| Frontend surfaces | `src/components/FeatureDetail.tsx` (1963), `WorkflowEditor.tsx` (784), `RunEventTimeline.tsx` (426), `GateView.tsx` (281), `StartFeatureModal.tsx` (1064), `ArtifactViewer.tsx` (421) |
| Status vocabulary | `src/lib/runStatus.ts` (`runStatusMeta`, `TERMINAL_STATUSES`) |
| Tauri event hook | `src/hooks/useTauriEvent.ts` |

---

## Dependency graph (task level)

```
P0.1 ─┐
P0.2 ─┴─▶ P1.1 ─▶ P1.2 ─▶ P1.3
                │
                ├─▶ P1.4 ─▶ P1.5
                ├─▶ P1.6 ─▶ P1.7 ─────────────┐
                ├─▶ P1.8 ─▶ P1.9              ├─▶ P1.11 ─▶ P1.12 (L) ─▶ P1.13 ─▶ P1.16
                ├─▶ P1.10 ────────────────────┘                │
                └─▶ P1.15                          P1.14 ──────┘
P1.16 ─▶ P2.1 ─▶ P2.2 ─▶ P2.3 ─▶ P2.4 ─▶ P2.5 ─▶ P2.6
P2.6  ─▶ P3.1 ─▶ P3.2 ─▶ P3.3 ─▶ P3.4        P3.5 (needs P1.7 only)   P3.6 (last)
P3.x  ─▶ P4.1 ─▶ P4.2        P4.3 (needs P1.5+P3.2)   P4.4
```

---

## Phase 0 — Decisions & baseline (before any engine change)

### P0.1 — Decision records for PRD open questions
- **Goal:** Turn PRD §11's five open questions into entries in `docs/DECISIONS.md` so Phase 1 tasks don't stall on policy calls.
- **Depends:** none. **Size:** small (docs only).
- **Context:** PRD §11, §5.1, §5.6; `docs/DECISIONS.md`; `docs/OPEN_QUESTIONS.md`.
- **Touch:** `docs/DECISIONS.md`, `docs/OPEN_QUESTIONS.md` (cross-reference/close items).
- **Do:** Record decisions for: (1) `Feature.workflow_version_id` column — PRD recommends **yes** (pin end-to-end); (2) gate join default — **all_success**, critic PASS_WITH_NOTES maps to success; (3) `conflict_policy` — becomes a `sync`-node config field; (4) `schedule` stays outside `nodes/edges`; (5) read-only Monaco source tab in Phase 3.
- **Done when:** decisions numbered and cross-linked; OPEN_QUESTIONS updated.

### P0.2 — Baseline behavioral harness for the 7 starters
- **Goal:** A golden-snapshot integration test that *executes* every bundled starter through the engine with the stub agent and records the resulting step/status/event sequence. This is the regression gate every Phase-1 task runs against.
- **Depends:** none. **Size:** medium.
- **Context:** `crates/demeteo-core/tests/conformance/topology_equivalence.rs` (pattern: `minimal_workflow`, `StubRuntime`, `agent_kind: "stub"`, `DEMETEO_STUB_AGENT`); `tests/conformance/execution_port.rs`; `src-tauri/workflows/*.json`; the `#[path]` wiring at `src/ports/execution.rs:283-288`.
- **Touch:** new `crates/demeteo-core/tests/conformance/starter_baseline.rs` + its `#[path]` mod hookup; golden snapshot fixtures (JSON) beside it; `crates/demeteo-core/src/adapters/agent/stub_runtime.rs` (*plan amendment 2026-07-23:* `stub_body` must emit a valid `TaskPlan` JSON when the directive path is `task-list.json`, or the two sequence-bearing starters die at plan resolution — the fixed markdown body can never satisfy `extract_task_plan`).
- **Do:** For each starter: load JSON → mechanically inject stub directives (`@stub-write` per declared `last_write_to` artifact, `@stub-verdict` into verifier instructions — prompts are template text, not baselined behavior) → run to completion under stub runtime (auto-approve gates) → serialize ordered (step_id, kind, final status, iterations, normalized error) list + artifacts → compare to committed snapshot. Provide `UPDATE_SNAPSHOTS=1` regen path.
- **Done when:** `cargo test -p demeteo-core starter_baseline` green on master; snapshots committed; README comment explains the regen flow.

---

## Phase 1 — Engine core (backend only, no UX change)

### P1.1 — Schema v2 data model (structs only)
- **Goal:** Rust model for workflow-as-data v2: `WorkflowDefinitionV2 { schema_version, nodes, edges, defaults }`, `NodeConfig { id, type, type_version, title, config, retry, position }`, `EdgeConfig { from, to, when }`, `JoinSemantics { AllSuccess | AnySuccess | AllDone }`, `PortType { Text | File | TaskList | Verdict | Approval | Any }`, `RetryPolicy` keyed by failure class (`environment | verdict | agent_failure | non_retryable`) with `strategy: in_place | redirect | fail`, `max_attempts`, `backoff_secs`, `feedback`, `redirect_to`. Pure data + serde; **no engine changes**.
- **Depends:** P0.1. **Size:** medium.
- **Context:** PRD §5.1, §5.4; `crates/demeteo-core/src/domain/models/workflow.rs` (all 357 lines — v1 model being superseded); one starter JSON for realistic config payloads.
- **Touch:** new `crates/demeteo-core/src/domain/models/workflow_v2.rs`; register in `domain/models/mod.rs`.
- **Do:** serde round-trip with `deny_unknown_fields` off for `config` (per-type payload stays `serde_json::Value` until P1.6 gives each handler a schema); unit tests: round-trip the PRD §5.1 example verbatim.
- **Done when:** unit tests pass; no other module references v2 yet.

### P1.2 — v1 → v2 pure auto-migration
- **Goal:** `fn migrate_v1_to_v2(steps: &[StepConfig], ...) -> WorkflowDefinitionV2`, pure and total: list order → chain edges; `on_failure` → `retry.verdict = { strategy: redirect, to, max_attempts (from max_iterations precedence), feedback: true }`; `task_list_from` → typed `task_list` edge into the sequence node; `parallel` kind → `sequence`; synthesized `position` (simple vertical layout).
- **Depends:** P1.1. **Size:** medium.
- **Context:** PRD §5.1 (migration bullet); `workflow.rs` `StepConfig` (:39-183) + `lint_workflow_steps` (:184-357); `workflow_v2.rs` (from P1.1); all 7 `src-tauri/workflows/*.json`.
- **Touch:** new `crates/demeteo-core/src/domain/models/workflow_migrate.rs` + tests.
- **Do:** tests assert: all 7 starters migrate without error; migrated graph node count/edge chain matches list; `on_failure` targets become retry redirects; idempotent on already-v2 input (pass-through).
- **Done when:** tests green; migration is still *unused* by runtime (wired in P1.12/P1.15).

### P1.3 — Published JSON Schema + validation at boundaries
- **Goal:** Machine-checkable schema for v2 (via `schemars` derive on the P1.1 structs), enforced at `workflow_import`/`workflow_create`/`workflow_update`; schema JSON emitted to `docs-site/`.
- **Depends:** P1.1, P1.2. **Size:** small/medium.
- **Context:** `workflow_v2.rs`; `src-tauri/src/commands/workflows.rs` (import :339, create :192, update :240 only); PRD §5.1.
- **Touch:** `workflow_v2.rs` (derive), new schema-emit test or xtask, `src-tauri/src/commands/workflows.rs` (validate on the three write paths), `docs-site/` schema file, `Cargo.toml` (add `schemars`).
- **Done when:** invalid v2 JSON is rejected with a readable error at import; committed schema file regenerates cleanly from a test.

### P1.4 — WorkflowGraph + structural lint
- **Goal:** Graph utilities and the lint pass the editor and engine will share: adjacency/ancestors/descendants, topological order, cycle rejection at construction, reachability, "exactly one finalize sink", dangling `redirect_to` must-be-ancestor, typed-port compatibility on edges, deadlock detection (a node whose join can never be satisfied), unknown node type.
- **Depends:** P1.1. **Size:** medium. Pure module, no I/O.
- **Context:** PRD §5.1 (key decisions), §5.3 step 4, §6.3 (lint list); `workflow_v2.rs`; v1 lint for parity ideas: `workflow.rs:184-357`.
- **Touch:** new `crates/demeteo-core/src/domain/workflow_graph.rs` (+ `LintFinding { severity: Error|Warning, node/edge ref, code, message }`).
- **Done when:** table-driven unit tests cover each lint rule both firing and passing; migrated starters lint clean.

### P1.5 — Expression mini-evaluator
- **Goal:** Sandboxed evaluator for `${{ nodes.<id>.outputs.<name> }}` plus `== != < <= > >=` and string/number/bool literals — nothing else (PRD risk table caps scope). Used later by edge `when` guards and input bindings.
- **Depends:** P1.1. **Size:** small. Pure module.
- **Context:** PRD §5.1 (Expressions bullet), §10 (scope-creep risk row); `workflow_v2.rs` edge type.
- **Touch:** new `crates/demeteo-core/src/domain/expr.rs` + exhaustive unit tests (including rejection of anything outside grammar).
- **Done when:** grammar documented in module doc; fuzz-ish negative tests pass.

### P1.6 — NodeTypeRegistry + NodeCtx; wrap `agent` and `sync`
- **Goal:** The PRD §5.2 `NodeHandler` trait (`kind`, `config_schema`, `lint`, `execute -> StepOutcome`, `cancel_grace`), a `NodeTypeRegistry` (mirror `AgentRegistry`'s pattern), and a `NodeCtx` bundling what handlers actually consume today (db handle, event emitter, workspace, step config, retry ctx view). Wrap the two most self-contained handlers: `agent`, `sync`.
- **Depends:** P1.1, P0.2. **Size:** large-ish — the `NodeCtx` design is the real work.
- **Context:** PRD §5.2; `driver.rs:63-320` (struct + state only), dispatch arms `driver.rs:810-884`; `steps/agent/mod.rs` (994 — skim the signature/entry fn and what it takes from `ExecutionDriver`, not the whole file); `steps/sync.rs` (306); `steps/mod.rs` (`StepOutcome`).
- **Touch:** new `crates/demeteo-core/src/adapters/step_executor/registry.rs`; `steps/agent/mod.rs` + `steps/sync.rs` (impl trait, keep bodies intact); `driver.rs` dispatch arms for `agent`/`sync` route through registry — `gate`/`sequence`/`finalize` arms stay as-is this task.
- **Done when:** P0.2 baseline snapshots unchanged; registry lookup replaces two match arms.

### P1.7 — Wrap `gate`, `sequence`, `finalize`; delete the match
- **Goal:** Finish re-homing: remaining three handlers implement `NodeHandler`; `driver.rs:810-884` match deleted; unknown kind becomes a registry miss with the same "Unknown step kind" failure.
- **Depends:** P1.6. **Size:** large-ish.
- **Context:** `registry.rs` (P1.6); `steps/gate.rs` (529), `steps/sequence/mod.rs` entry (skim of 891), `steps/finalize/mod.rs` (444); dispatch remnants in `driver.rs:810-950`.
- **Touch:** the three handler files, `driver.rs` (dispatch removal), `registry.rs` (registration of five).
- **Done when:** P0.2 baseline snapshots unchanged; `grep 'kind.as_str()' driver.rs` finds nothing; conformance suites green.

### P1.8 — `step_attempts` table + repo + write path
- **Goal:** Migration **V31** `step_attempts` (`step_execution_id`, `attempt_no`, `status`, `cost`, `tokens`, `wall_clock_ms`, `error_class`, `failure_fingerprint`, `started_at`, `ended_at`) per PRD §5.3; repo CRUD; driver records an attempt row per try instead of overwriting.
- **Depends:** P1.7 (so writes go through one seam). **Size:** medium.
- **Context:** PRD §5.3 (last paragraph); `migrations/V22__run_events.sql` (style reference); `repos/feature_steps.rs` (252); the retry/iteration sites: `driver.rs:600-700`, `driver/failure.rs` (185).
- **Touch:** new migration `V31__step_attempts.sql`; new `repos/step_attempts.rs` + `repos/mod.rs`; driver/failure write hooks.
- **Done when:** a seeded retry run produces N attempt rows with distinct `error_class`; baseline snapshots unchanged (attempts are additive).

### P1.9 — Durable checkpoints; delete `env_retried`
- **Goal:** Move `sequence_checkpoints` (`driver.rs:254`) and `cached_plans` (`driver.rs:240`) into a **V32** migration (per-(feature, node) rows; plan stored with the attempt that produced it); on resume, hydrate from DB. Delete `env_retried` (`driver.rs:260`) — derive "already env-retried" from `step_attempts.error_class`.
- **Depends:** P1.8. **Size:** medium/large.
- **Context:** PRD §5.4 (Durable checkpoints); `driver.rs:235-320`; `steps/sequence/runner.rs` (841 — checkpoint read/write sites only, grep `sequence_checkpoints`); `steps/sequence/plan.rs` (plan cache sites); `crates/demeteo-runner/src/reconcile.rs` (136).
- **Touch:** migration `V32__durable_checkpoints.sql`; new repo; `driver.rs`, `steps/sequence/{runner,plan}.rs`, `reconcile.rs`.
- **Done when:** new test: kill driver mid-sequence (stub), restart, run resumes **from the exact task** not the step head; `env_retried` symbol gone.

### P1.10 — Declarative per-class retry policy engine
- **Goal:** Replace the scattered precedence (engine default 3 / project setting / step `max_iterations` / `on_failure` / env one-shot — `driver.rs:612-618`, `failure.rs:63`) with evaluation of the P1.1 `RetryPolicy`: failure class (from `StepOutcome`/`VerifierError`) → policy rule → action (`in_place` | `redirect(ancestor)` | `fail`), `feedback: true` preserving today's `RetryContext` append. Legacy v1 definitions flow through P1.2's mapping so behavior is identical.
- **Depends:** P1.2, P1.7, P1.8. **Size:** large-ish.
- **Context:** PRD §5.4 (retry block); `driver/failure.rs` (all 185); `driver.rs:600-700` + `RetryContext` (:37); `steps/mod.rs` (`StepOutcome`); `domain/verifier.rs:93` (`VerifierError`); `workflow_v2.rs` policy structs.
- **Touch:** `driver/failure.rs` (rewrite around policy), `driver.rs` (budget sites), new unit tests; keep `RetryBudgetExhausted` notification/event exactly as-is (`failure.rs:117,133` — conformance-tested in `tests/conformance/harness_triage.rs`).
- **Done when:** `harness_triage.rs` conformance green unmodified; new tests: each failure class × strategy; applied rule id recorded on the attempt row.
- *Amendments (2026-07-24, as built):* (1) "applied rule id on the attempt row" required an `applied_rule` column — added to V31 (fresh DBs) plus a defensive `add_column_if_missing` in `migration.rs` (branch DBs that already ran V31), threading through `ports/db.rs` / `repos/step_attempts.rs` / `repos/feature.rs` — Touch list was underspecified. (2) **P1.12 landmine:** the runtime deriver (`retry_policy::legacy_policy_for_step`) maps `on_failure` to redirect rules for **both** `verdict` and `agent_failure` (v1 sent both classes through the same path), but P1.2's `migrate_v1_to_v2` maps it to `verdict` only. When P1.12 starts executing *migrated v2* definitions, plain agent failures would silently stop redirecting — extend the P1.2 mapping (or the v2 policy deriver) to cover `agent_failure` before switching the run path, and let the P0.2 baseline arbitrate.

### P1.11 — Ready-set scheduler core (pure)
- **Goal:** Pure scheduling module: given graph (P1.4) + node states, compute the ready set (join satisfied, `when` guards pass via P1.5), propagate `skipped(reason)` per join semantics, detect the "empty ready set with non-terminal nodes" invariant violation. Node state machine formalized as a Rust enum incl. `skipped(reason)` and `awaiting_retry` (PRD §5.3 diagram). **No driver changes.**
- **Depends:** P1.4, P1.5. **Size:** medium. Pure + heavily table-tested.
- **Context:** PRD §5.3 (all), §5.6 (ceiling default 1); `workflow_graph.rs`; `expr.rs`.
- **Touch:** new `crates/demeteo-core/src/adapters/step_executor/scheduler.rs` + tests (chain, diamond fan-in per join mode, skip propagation, when-guard skip, deadlock invariant).
- **Done when:** unit suite covers every transition edge in the PRD state diagram.

### P1.12 — Driver integration: replace the step_index loop **(L)**
- **Goal:** `ExecutionDriver` walks the scheduler's ready set (still `max_parallel_nodes = 1`, one tokio task per feature, `DriverRegistry`-deduped) instead of `step_index += 1`. Persist every transition in a SQLite tx **before** acting, emit events after commit. Ancestor-aware guard replaces predecessor guard (gate decisions / manual retries refused while any ancestor is non-terminal). Exit handlers formalize `feature_cancel` + finalize cleanup. Restart reconciliation (`reconcile.rs`) maps to the new state machine.
- **Depends:** P1.7, P1.9, P1.10, P1.11. **Size:** **the largest task** — nothing else in this session.
- **Context:** `driver.rs` (full read, 1228); `scheduler.rs`; `driver_registry.rs` (114); `updates.rs` (87); `impl_traits/replay.rs` (230, replay/retry guard); `reconcile.rs`; `src-tauri/src/commands/features.rs:144-199` (`gate_decide`, `step_retry`, `replay_from_step`).
- **Touch:** `driver.rs`, `reconcile.rs`, `impl_traits/replay.rs`, `features.rs` guard sites.
- **Done when:** **P0.2 baseline snapshots byte-identical** (chains are DAGs — behavior must not change); topology-equivalence conformance green; P1.9's crash-resume test green.
- *Amendments (2026-07-24, as built):* (1) `driver.rs` had been split into `driver/run_loop/{mod,dispatch,outcome,attempt,cleanup}.rs` submodules before this task — the loop rewrite lives in `run_loop/mod.rs` plus a new `run_loop/schedule.rs` (state derivation, skip persistence, redirect rewind), not in a 1228-line `driver.rs`. (2) Node states are **derived from `step_executions` rows each tick** (`completed`→Completed, `skipped`→Skipped, everything else→Pending — exactly v1's resume semantics), so restart reconciliation needed **zero** `reconcile.rs`/watchdog changes and there is no in-memory cursor to resync; `features.rs` guard sites also needed no edits (the guard lives in `DagStepExecutor::assert_no_active_predecessors`, now graph-ancestor-aware with an index fallback for unresolvable legacy features). (3) Redirects (policy, gate, sync) rewind target + graph *descendants* to `pending`, persisted before re-evaluation — the DAG form of the v1 cursor jump; `replay_steps_from` got the same descendants-cone reset. (4) The P1.10 landmine was defused in `migrate_v1_to_v2` itself: `on_failure` now maps to identical `verdict` **and** `agent_failure` redirect rules (runtime policy still derives from v1 `StepConfig` via `legacy_policy_for_step`, so behavior is unchanged either way). (5) A scheduler `Deadlock`/`UnknownNode` fails the stuck rows + feature loudly (`fail_unschedulable`); unreachable for migrated chains. (6) Multi-row SQLite tx per transition deferred — repos expose no transactional seam; each per-row write stays durable-first, event-after-write.

### P1.13 — Unified `run_events` for local transport
- **Goal:** Every transition, retry decision (with policy rule id), gate decision, harness verdict, and cost sample appends a `run_events` row locally; Tauri events become live push of the same record shape the remote path polls (PRD §5.4 Event log). Old ad-hoc `StepProgress`/`AgentStream` emissions stay until P2.6 deletes the split path.
- **Depends:** P1.12. **Size:** medium.
- **Context:** `repos/run_events.rs` (66); `V22__run_events.sql`; emission sites in `driver.rs`/`updates.rs` (grep `emit`); `src-tauri/src/commands/remote_runner.rs` (event shape the remote poller returns).
- **Touch:** `updates.rs` (central emit → append+push), `repos/run_events.rs` (if payload kinds need enum), event-kind doc comment.
- **Done when:** a local stub run's `run_events` rows replay into the same ordered story the Tauri events told; kinds documented.
- *Amendments (2026-07-24, as built):* (1) There is no single emit choke point in `updates.rs` — emissions are scattered across `driver/{failure,status,verifier}.rs`, `run_loop/schedule.rs`, steps, etc. The central seam is the **`NotificationPort` itself**: a new `RunEventRecorder` decorator (`crates/demeteo-core/src/adapters/run_event_log.rs`) wraps the Tauri emitter in `src-tauri/lib.rs` (late-bound to `ctx.run_events` after `build_core_context`, same pattern as the runner's `RunEventBridge`), so every site records without edits. Local rows are keyed by **feature id** (a local run has no runner run row). (2) The `DomainEvent → (kind, payload)` translation was extracted into that module as `run_event_record()` and the runner's `notify_bridge.rs` now calls it too — local and remote payload shapes can no longer drift; the bridge keeps only run-id resolution, progress throttling, and `AgentStream → step_output` coalescing (locally `AgentStream` is *not* logged: the durable transcript already lives in `messages`). (3) "Retry decision with policy rule id" and "gate decision" had no `DomainEvent`s — added `RetryDecision` (emitted in `run_loop/outcome.rs`, carrying `rule_id`/`error_class`/`action`/`attempt`/`max`; the harness-verdict story is its `class=verdict` case + the existing step-status events) and `GateDecided` (emitted in `gate_decide` after the durable upsert), plus `RunEventAppended` — the live push of the exact stored record, which the Tauri adapter re-emits as the `run_event` event (P2.2's `useRunEvents` consumes this). (4) Event-kind vocabulary documented in `run_event_log.rs`'s module doc; parity proven by `tests/conformance/run_event_parity.rs`.

### P1.14 — Workspace fingerprint + idempotency keys
- **Goal:** Record repo HEAD + dirty flag at node start; on resume mismatch → existing synthetic-gate path (extends Decision 14). Idempotency key (node id + attempt + fingerprint) stored per attempt; groundwork for `command` nodes' `idempotent: false` → always synthetic-gate on interrupt (PRD §5.4).
- **Depends:** P1.8, P1.12. **Size:** small/medium.
- **Context:** PRD §5.4 (fingerprint + idempotency); `reconcile.rs`; `setup.rs` (196, workspace prep); `repos/step_attempts.rs`.
- **Touch:** `setup.rs`, `reconcile.rs`, attempt repo (+ column on V31 table if not landed yet — coordinate with P1.8).
- **Done when:** test: dirty-worktree mutation between crash and resume yields synthetic gate, not re-execution.
- *Amendments (2026-07-24, as built):* (1) V31 grew **two** columns — `workspace_fingerprint` (`<HEAD>:<dirty|clean>`, recorded at every attempt open via a probe in `setup.rs::workspace_fingerprint`) and `idempotency_key` (`<se_id>#<attempt_no>#<fp>`, derived in the repo where `attempt_no` is assigned) — edit-in-place + `add_column_if_missing`, same pattern as P1.10's `applied_rule`. (2) The resume check lives in the **driver run loop** (`run_loop/resume.rs`), not `reconcile.rs`: the pre-P1.14 engine *blindly re-dispatched* watchdog-`interrupted` nodes the moment `resume_interrupted_features` armed the driver, making the watchdog's `gd-syn-*` prompt advisory-only — the guard now parks on that same row + `GateWaiter` rendezvous when the last attempt's recorded fingerprint mismatches the live workspace, and `reconcile.rs` (runner) needed zero changes since it reuses this engine machinery. Match/unknown fingerprint → auto-resume (pre-P1.14 behavior; keeps the P1.9 crash-resume gate green — its resume goes through `step_retry`→replay which resets rows to `pending` and never meets the guard). (3) Approve re-runs (the new attempt records the blessed state as the next baseline); reject/cancel/redirect all fail the step+feature — redirect targeting stays a real-gate affordance. Guard fires once per driver life. (4) Gate: `tests/conformance/resume_fingerprint.rs` (mutation parks until approve; untouched auto-resumes). Note the fingerprint is deliberately coarse — a mutation on an *already-dirty* tree is invisible; the settle-to-clean forge in the test documents this.

### P1.15 — Pin `workflow_version_id` on Feature end-to-end
- **Goal:** Migration **V33** adds `features.workflow_version_id`; `start_feature` resolves latest version once, stores it; run path + `RunSpec` read the pinned row (remote already carries `workflow_json`). Required for historical run-mode rendering (P2).
- **Depends:** P0.1 (decision), P1.2. **Size:** small/medium.
- **Context:** `repos/feature.rs` (353); `src-tauri/src/commands/features.rs:38-79` (`start_feature`); `repos/workflow.rs` (340); `application/run_view.rs` (96).
- **Touch:** migration V33, `repos/feature.rs`, `features.rs`, wherever the driver currently re-resolves the workflow (grep `steps_json` outside repos).
- **Done when:** feature started → version pinned; editing the workflow mid-run demonstrably doesn't change the running graph.

### P1.16 — Phase-1 exit gate
- **Goal:** Prove the phase: all baseline snapshots byte-identical; crash-mid-sequence resumes from exact task; every failure in `run_events` carries failure class + applied policy rule (PRD §9 reliability metrics). Fix whatever this surfaces.
- **Depends:** P1.12–P1.15. **Size:** small (verification + fixes).
- **Context:** P0.2 harness; `tests/conformance/`; this file.
- **Touch:** fixes only; update status table.
- **Done when:** full `cargo test -p demeteo-core` + `src-tauri` tests green; exit criteria checked off in this doc.
- *Exit criteria (verified 2026-07-24):*
  - ☑ **Baseline snapshots byte-identical** — `starter_baseline` green unmodified through P1.12–P1.15 (chains are DAGs; behavior unchanged).
  - ☑ **Crash-mid-sequence resumes from the exact task** — `tests/conformance/durable_checkpoints.rs` green; plus the P1.14 guard (`resume_fingerprint.rs`) proves an *untouched* workspace auto-resumes while a mutated one gates.
  - ☑ **Every failure in `run_events` carries failure class + applied policy rule** — `run_event_parity.rs::failed_run_logs_failure_class_and_policy_rule` runs a deterministically-failing stub feature end-to-end and finds the `retry_decision` row (`error_class=agent_failure`, `rule_id=agent_failure.fail`) in the durable log. No fixes were surfaced — the phase landed clean.

---

## Phase 2 — Run visibility

### P2.1 — Canvas foundation (deps + static graph render)
- **Goal:** Add `@xyflow/react` + `elkjs` (elk in a web worker); new `WorkflowCanvas` component rendering a pinned version's v2 graph read-only: node cards with kind icon/title, edges, minimap auto-hidden under ~8 nodes, fit-view, elk layout button, keyboard nav. Design language: existing dark neon glassmorphism, `ui/` kit, lucide.
- **Depends:** P1.15 (pinned version to render), P1.2 (v1 defs render via migration). **Size:** medium/large.
- **Context:** PRD §6.1; `package.json`; `src/components/ui/` (skim exports); `src/lib/runStatus.ts`; one starter JSON migrated to v2 for fixture.
- **Touch:** `package.json`; new `src/components/canvas/WorkflowCanvas.tsx`, `src/components/canvas/nodes/*.tsx`, `src/lib/elkLayout.worker.ts`; a fixture-driven test.
- **Done when:** canvas renders all 7 migrated starters from fixtures; no console errors; **battery rule respected — opacity-only animation, no infinite transforms** (see webview perf work).
- *Amendments (2026-07-24, as built):* (1) The 7 v2 fixtures are **emitted from the live Rust migration**, not hand-authored: an env-gated `canvas_fixtures_are_current` regen test co-located in `workflow_migrate/migrate_tests.rs` writes `src/components/canvas/__fixtures__/<starter>.v2.json` under `UPDATE_CANVAS_FIXTURES=1` and otherwise asserts they're current — so the canvas provably renders exactly what the engine migrates and the fixtures can't silently drift. (2) Touch was slightly under-specified: added `types.ts` (TS mirror of `workflow_v2.rs` + node-type icon/tone metadata), `flowGraph.ts` (pure def→ReactFlow transform, the testable seam), and `useElkLayout.ts` (client owner of the worker); the worker itself is `src/lib/elkLayout.worker.ts` as planned. `toFlowGraph` already carries an optional `statusByNode` overlay hook and `WorkflowCanvas` an `onNodeActivate` seam so P2.2/P2.3 layer on without a fork. (3) React Flow needs `ResizeObserver`/`DOMMatrixReadOnly`/`matchMedia`/`getBoundingClientRect` stubbed under jsdom — done locally in the test; the primary guarantee is the DOM-free `toFlowGraph` assertion over all 7 starters, with a per-starter smoke mount on top. (4) **Pre-existing, out-of-scope:** `cargo fmt --all --check` flags ~10 *committed* P1.10–P1.15 files (`driver.rs`, `failure.rs`, `run_event_log.rs`, `run_event_parity.rs`, …) as needing reformatting under the local rustfmt despite the `1.97.0` pin — left untouched here (P2.1 keeps its own diff clean); worth a dedicated `cargo fmt` pass / CI-toolchain check.

### P2.2 — Run mode: live status overlay + Graph|Timeline toggle
- **Goal:** Embed `WorkflowCanvas` in `FeatureDetail` behind a "Graph | Timeline" toggle (list stays default). Status overlay from the unified `run_events` stream (P1.13): running pulses (opacity-only), completed shows duration+cost chips, failed glows with failure class, skipped dims with reason tooltip, gate shows amber shield opening the existing `GateView`.
- **Depends:** P2.1, P1.13. **Size:** large-ish.
- **Context:** PRD §6.1; `FeatureDetail.tsx` — **do not read all 1963 lines**: read the header/imports (:1-60), steps state (:200-260), and the step-list render + event wiring sections (grep `useTauriEvent`, `StepProgress`); `useTauriEvent.ts` (37); `runStatus.ts`; `GateView.tsx` (281).
- **Touch:** `FeatureDetail.tsx` (toggle + mount), `WorkflowCanvas.tsx` (run-mode props), new `src/hooks/useRunEvents.ts` (single stream consumer both modes share).
- **Done when:** live stub run animates correctly in both modes from the same hook; toggle preserves selection.
- *Amendments (2026-07-24, as built):* (1) **Touch under-specified a backend command.** The canvas needs a *migrated v2* definition and migration is Rust-only, so a new `feature_workflow_graph(featureId) -> WorkflowDefinitionV2` command was added (`commands/workflows.rs` + registration in `lib.rs`): it reads the feature's **pinned** version (P1.15), falls back to latest for pre-pin features, and returns `migrate_v1_to_v2(...)`. FeatureDetail fetches it once per feature id (the run's graph is immutable). (2) **Overlay source.** `useRunEvents(featureId, steps)` derives `statusByNode` from the authoritative `step_executions` snapshot (already reloaded on every `step_progress`/`feature_status_changed` event) — correct on first mount with no delta replay — and enriches it with the **failure class** from the `run_events` `retry_decision` stream (P1.13), which the step row can't carry. Both run-mode surfaces read node status from this one hook. (3) `WorkflowCanvas`'s `statusByNode` prop grew from `Record<string,string>` to `Record<string, NodeRunStatus>` (status + cost + duration + errorClass + `stepExecutionId`); the node card renders tone-driven glow + an **opacity-only** pulsing dot + duration/cost chips + failure-class chip (battery rule honored). (4) A gate node's card is the click target: `onNodeActivate` opens the existing full-screen `GateView` via `navigate(..., gateStepExecutionId)`; non-gate nodes are inert until the drill-down panel (P2.3). (5) `formatDuration` lifted from FeatureDetail-local into `lib/utils` (shared by chips); FeatureDetail's own copy left untouched to keep the diff scoped. Toggle defaults to Timeline; the Graph option appears only when a definition + a started run both exist.

### P2.3 — Node drill-down panel: Overview + Output
- **Goal:** Clicking a node opens a right side panel (split-panel pattern like `ArtifactViewer`): **Overview** = status, attempt table (class, cost, duration, outcome) from `step_attempts` via a new Tauri command; **Output** = artifacts (Monaco), harness output, verifier verdict with failing tests/implicated files.
- **Depends:** P2.2, P1.8. **Size:** medium/large.
- **Context:** PRD §6.2; `ArtifactViewer.tsx` (421); `src-tauri/src/commands/features.rs` (`step_get` :110, `artifact_body` :216); `repos/step_attempts.rs`.
- **Touch:** new Tauri command `step_attempts_list` (features.rs + repo call); new `src/components/canvas/NodePanel.tsx` (+ Overview/Output tabs); `WorkflowCanvas.tsx` selection wiring.
- **Done when:** seeded failing run → root-cause artifact reachable in ≤3 clicks (the J2 metric).
- *Amendments (2026-07-24, as built):* (1) **No repo work needed** — `FeatureRepository::attempts_for_step` (V31, P1.8) already existed and was documented as feeding P2.3; the command wires through a new read-only `RunView::step_attempts` delegation (application layer, so a runner-owned step's attempts resolve from the C4.2 shadow like `step_get`), not a fresh repo call. (2) **Shared artifact classifier** lifted into `src/lib/artifacts.tsx` (`classifyArtifact` + kind labels/colors + `ArtifactIcon`) for the panel's Output tab; `FeatureDetail`'s local copy left untouched to keep the diff scoped — the same precedent P2.2 set for `formatDuration`. (3) **Canvas selection wiring** = a new *controlled* `selectedNodeId` prop on `WorkflowCanvas` that syncs the node `selected` flag (notably clearing the highlight when the panel closes); click/keyboard selection still works when the prop is omitted. (4) **`onNodeActivate` split:** an *awaiting* gate node still opens the full-screen `GateView` (the actionable HITL path, kept from P2.2); every other node toggles the drill-down panel — so a completed gate opens its Overview too. (5) **Output tab's "verifier/harness output" is `step.error_message`** — the failing-tests / implicated-files text lives there in the `StepExecution` model (same field the timeline renders), not as separate typed columns; the tab pairs it with the deduped artifact chooser → `ArtifactViewer`. (6) `openEditorForPath` (worktree-ref → code editor, shared with the timeline) had to be declared *after* `resolveWorktreeInfo` to stay out of its TDZ. (7) Gate: `NodePanel.test.tsx` (attempt table from `step_attempts_list`, failure-class chip, not-started skip, Output error text + artifact chooser + empty state); `tsc` + 467 vitest green.

### P2.4 — Node panel: Live + Actions tabs
- **Goal:** **Live** tab hosts the existing `agent_stream` transcript (moves from the inline toggle in `FeatureDetail`). **Actions** tab: Retry (shows which policy rule will apply), Replay-from-node (existing `replay_from_step`, now highlights the downstream subgraph before confirm), Stop node, Decide gate — all respecting the ancestor guard with disabled-button explanations (kept UX, PRD §6.4).
- **Depends:** P2.3. **Size:** medium/large.
- **Context:** `FeatureDetail.tsx` agent-stream section (grep `agent_stream`/`AgentStream`); `features.rs` (`step_retry` :160, `replay_from_step` :181, `gate_decide` :144); `impl_traits/replay.rs` (230, what replay invalidates).
- **Touch:** `NodePanel.tsx` (two tabs), `FeatureDetail.tsx` (remove inline transcript), small backend addition if replay needs to return the affected-subgraph preview.
- **Done when:** each action verified against a stub run; guard states render with explanations.
- *Amendments (2026-07-24, as built):* (1) **No backend change** — "the affected-subgraph preview" is computed client-side from the already-pinned migrated v2 graph via a new pure `canvas/graphOps.ts` (`descendantIds`/`replayCone`), so the replay modal's downstream count is now **DAG-accurate** for the panel path (the timeline keeps its index-based count). (2) **"Highlights the downstream subgraph"** = a violet *will-re-run* ring, threaded `flowGraph`→`WorkflowNode`→`WorkflowCanvas` as a new `highlightedNodeIds` prop; `FeatureDetail` sets it to the replay cone while the confirm modal is open and clears it on cancel/confirm (`closeReplay`). Distinct from the cyan selection ring; selection still wins the border. (3) **Inline transcript kept in the timeline, not removed** — the timeline is a *permanent* toggle (PRD §6.1), so stripping its live-reasoning would regress the default surface; the Live tab instead reads the **same** `streamContent` buffer (dual surfaces, one source — the P2.2 pattern). Deleting genuinely-duplicate surfaces stays P2.6's job. (4) **Actions reuse FeatureDetail's existing handlers via callback props** (zero run-logic in the panel): Retry→`handleRetryStep`, Stop→`handleStopStep`, Replay→the existing confirm modal, Decide-gate→the `GateView` route — so canvas and timeline drive identical code paths. The ancestor guard is `findActivePredecessor` (index-based, same as the timeline's Retry button) surfaced as `blockedBy` with a disabled-button + spelled-out explanation (PRD §6.4). (5) Panel is now **four tabs Overview·Live·Output·Actions** (PRD §6.2 order); the `step_attempts` fetch was lifted to panel level so the Overview table and the Actions "which policy rule applied" hint (last failed attempt's `applied_rule`) share one read. (6) Gate: `graphOps.test.ts` (cone traversal over a diamond) + six new `NodePanel.test.tsx` cases (Live buffer/empty; Actions retry-blocked-with-explanation, retry+replay fire, decide-gate, empty state); `tsc` + 477 vitest green.

### P2.5 — Sequence node expansion (landed-prefix legibility)
- **Goal:** Sequence nodes expand in place (accordion in node or panel) showing the task list with per-task status/cost; the landed prefix visually distinct from pending tasks — making Decision 13 semantics legible (PRD §6.2).
- **Depends:** P2.3, P1.9 (durable checkpoints are the data source). **Size:** medium.
- **Context:** `steps/sequence/runner.rs` checkpoint shape (grep only); checkpoint repo (P1.9); `NodePanel.tsx`.
- **Touch:** canvas sequence node component, `NodePanel.tsx`, a Tauri command exposing checkpoint/task state if not already in `run_events`.
- **Done when:** mid-sequence stub run shows landed vs pending split matching DB checkpoint.
- *Amendments (2026-07-24, as built):* (1) **New Tauri command `sequence_tasks_list(featureId, nodeId, executionId) -> SequenceState`** — checkpoint/plan/subtask state is *not* in `run_events`, so the panel reads it on demand through the C3 read-model (`RunView::sequence_state`), same seam as `step_attempts_list`. It joins **three** durable sources the engine already writes: `sequence_plan_cache` (ordered task id+title), `sequence_checkpoints` (the landed prefix — the load-bearing Decision-13 split), and `subtask_runs` (per-task status/cost/tokens/error). `nodeId` (== v1 `step_id`) keys plan+checkpoint; `executionId` keys the subtask rows. (2) **Status precedence: landed wins** — a checkpointed task is `landed` regardless of its `subtask_runs` row (a rev-parse hiccup can leave a `completed` row uncheckpointed, but the checkpoint is the resume authority); no run row + not landed ⇒ `pending`. The merge is a pure `assemble_tasks` in `domain/models/sequence_view.rs` (DB-free unit-tested); a new `FeatureRepository::subtask_runs_for_step` read lives beside the checkpoint reads so `RunView` assembles from one repo. (3) **Surface = panel Overview accordion, not the node card.** PRD §6.2 says "accordion inside the node *or* panel"; the panel was chosen so the fetch only fires for the *selected* sequence node — a per-node canvas fetch would hit every sequence node on every event. `WorkflowNode.tsx` is untouched. (4) Renders solid emerald-railed landed rows (filled check) vs dimmed pending vs tone-matched running/failed, an `N/M landed` summary, per-task cost, and the failing task's error; silent (renders nothing) for a sequence node that hasn't planned yet — the norm before it runs. (5) Remote/runner-owned features read `unplanned` until their sequence state is mirrored locally (out of this task's scope; C4 concern). (6) Gate: `sequence_view.rs` unit tests (landed-wins + failed-task) + `subtask_runs_for_step` repo test (start-order, step-scoped) + three `NodePanel.test.tsx` cases (landed/pending split render, silent-when-unplanned, no-fetch-for-non-sequence); `tsc` + 483 vitest + 855 core tests green.

### P2.6 — Remote runs on the same canvas; delete the split path
- **Goal:** Remote/detached runs render on `WorkflowCanvas` from the same `run_events` stream; `RunEventTimeline` survives only as the raw event feed inside the panel's Overview tab. Delete the separate polling surface + dead parallel UI (audit F36) — PRD targets **≥1k LOC net removal**.
- **Depends:** P2.2, P1.13. **Size:** large-ish.
- **Context:** `RunEventTimeline.tsx` (426 — full read; it's being absorbed); `src-tauri/src/commands/remote_runner.rs` (grep command names `remote_get_status`, `remote_run_for_feature`); `useRunEvents.ts` (P2.2); `FeatureDetail.tsx` remote-branch sections (grep `RunEventTimeline`).
- **Touch:** `useRunEvents.ts` (poll fallback for remote), `NodePanel.tsx` (raw feed), delete/shrink `RunEventTimeline.tsx`, `FeatureDetail.tsx` cleanup.
- **Done when:** remote stub run renders live on canvas; `RemoteGateActions`/`ReinjectCredentials` still reachable; LOC delta reported in PR description.
- *Amendments (2026-07-24, as built):* (1) **Remote-on-canvas already worked** — the C4.2 shadow (`remote_runs/reconcile.rs::hydrate_shadow_feature`) hydrates the runner's feature (carrying `workflow_id`) + steps into the laptop's own tables, so `feature_workflow_graph` (P2.2) resolves the migrated v2 graph from the *local* workflow catalog and `runStatusByNode` (P2.2, derived from `steps`) already tracks live status via the existing 3s `remote_refresh_run` re-hydrate. `canShowGraph = graphDef && steps.length > 0` is transport-agnostic — the Graph|Timeline toggle + canvas appear for remote with **zero** new wiring. No `feature_workflow_graph` change and no poll relocation were needed. (2) **The unification is at the render + surfacing layer, not the poll.** Extracted `describeEvent` + the row markup out of `RunEventTimeline` into a shared, transport-agnostic `RunEventFeed` (`src/components/RunEventFeed.tsx`); both the remote **Activity** strip and the node panel's **Overview** now render identical rows from the same `RunEvent` shape (local Tauri push / remote poll). `RunEventTimeline` shrank ~164→~30 body lines. Added `retry_decision`/`gate_decided` kinds (P1.13 vocabulary) to the feed. (3) **Raw feed now lives in the panel Overview** (PRD §6.2) — `NodePanel` gained a `runEvents` prop; FeatureDetail passes the unified feed (`localRunEvents` from `useRunEvents`, else `remoteRunEvents` captured from the Activity strip's existing `onEvents` batch, de-duped by offset + 500-cap). No second remote poll: `useRunEvents` was **not** given a remote fallback — the strip already tails `remote_stream_events`, so tapping its batch avoids a duplicate SSH poll. (4) **`RemoteGateActions`/`ReinjectCredentials` untouched**, still inline on the Activity strip. (5) **F36 premise is stale — big deletion NOT done.** The audit's "~3,200 lines of dead parallel UI" no longer holds: `CreateFromZeroWizard` is imported by live `NewProjectView.tsx` + `lib/createProject.ts`, and `ShortcutHelp`/`ShortcutsContext` by `lib/shortcuts.ts` — deleting them breaks the build. So the PRD §9 "≥1k LOC net removal alongside F36" target is **not met** (it was predicated on that dead code); the split-path was consolidated at the rendering level instead (net production LOC ~flat: `RunEventTimeline` −164, `RunEventFeed` +192 shared, small NodePanel/FeatureDetail additions). A real F36 sweep is now its own audit-refresh task. (6) Gate: `RunEventFeed.test.tsx` (describeEvent per-kind + feed render/empty) + a NodePanel Overview raw-feed case; `tsc` + 486 vitest green.

---

## Phase 3 — Builder

### P3.1 — Registry metadata over IPC + palette + connect rules
- **Goal:** Tauri command `node_types_list` returning each registered handler's `kind`, `config_schema()`, display metadata. Design-mode canvas: palette derived from it, drag-from-handle into empty canvas → type-compatible "what can connect here" picker, Cmd+K node search, connect-time port type checking and cycle prevention (client-side mirror of P1.4 rules).
- **Depends:** P2.1, P1.7. **Size:** large-ish.
- **Context:** PRD §6.3 (palette bullets); `registry.rs`; `src-tauri/src/commands/workflows.rs` (:1-130 for command style); `WorkflowCanvas.tsx`.
- **Touch:** new command in `workflows.rs`; `WorkflowCanvas.tsx` design-mode; new `src/components/canvas/Palette.tsx`, connect-validation util (shared with P1.4 semantics — port the rules table, don't re-derive).
- **Done when:** every registered type appears in palette automatically (verify by the P3.5 `command` type appearing with zero frontend edits).
- *Amendments (2026-07-25, as built):* (1) **The registry had no display metadata to serve.** `NodeHandler` published `kind` + `config_schema` only, so the palette had nothing to render a label or a "what can connect here" filter from. Added three trait members: `display() -> NodeDisplay { label, summary }` (**no default** — a new node type cannot register without introducing itself, which is what makes the zero-frontend-edit guarantee structural rather than aspirational), `ports() -> NodePorts { inputs, outputs }`, and `max_instances()`. All five launch handlers implement them. (2) **Ports are declared honestly, not aspirationally.** Every launch type accepts `[Any]` on input because the engine genuinely refuses no predecessor by type — `gate → sequence` is a shipped starter edge, and a narrower declaration would make the editor reject graphs the engine runs. The rule earns its keep on *outputs*: `finalize` declares none, which is what makes "nothing may follow finalize" (the `finalize-not-sink` lint error) enforceable at connect time. A `node_catalog` test pins the six starter edge shapes so a future handler can't narrow itself into rejecting a runnable graph. (3) **New public seam `adapters/step_executor/node_catalog.rs`** — the registry is `pub(crate)` (handlers reach into `ExecutionDriver`), so `src-tauri` can't read it; `node_type_catalog() -> Vec<NodeTypeInfo>` is the serializable projection the `node_types_list` command returns verbatim. Retired aliases (`parallel`) resolve in `handler_for` but are excluded from the catalog — offering one would mint new definitions on a dead kind name. (4) **Connect rules ported, not re-derived** (`connectRules.ts`): every rejection carries the same machine code as the equivalent Rust finding (`cycle`, `port-type-mismatch`, `finalize-not-sink` via empty outputs, `multiple-finalize` via `max_instances` at add-time). Two deliberate divergences, documented inline: `duplicate-edge` is editor-only (the engine tolerates it; React Flow needs unique edge ids), and a node's *declared* `config.inputs/outputs` now falls back to the **type-level** registry defaults so a freshly-dropped node with empty config is still checked — the Rust lint only reads the per-node declaration. (5) **Mutations split from validation** (`graphEdits.ts`, pure `def → def'`): the canvas asks `connectRules` first, then applies. Beyond keeping rejection messages renderable without a half-applied edit, this hands P3.3 ready-made immutable snapshots for undo/redo. `removeNode` also rewrites any retry redirect aimed at the deleted node to `strategy: fail` — leaving a dangling `redirect_to` is precisely the audit-F39 bug class the builder exists to eliminate, so it's fixed at the edit rather than left for lint. (6) **`WorkflowCanvas` stays IPC-free**: design mode takes `nodeTypes` as a prop (the owning screen supplies `useNodeTypes()`), preserving the fixture-testability P2.1 established. No route is wired — P3.6 owns replacing `WorkflowEditor`; P3.1 lands the capability plus its tests. (7) **Pre-existing gate breakage surfaced and fixed — two layers, each masking the next.** `scripts/checks.sh` (the gate CI runs verbatim) had been failing at `cargo fmt --all --check` since ~P1.10 on ten committed files — flagged in the P2.1 amendment and deferred by P2.2–P2.6, so the shared gate was red for all of Phase 2. Fixed as its own mechanical `cargo fmt` commit, keeping this task's diff reviewable. That turned the checks step green and **revealed a second failure that had been `skipped` on every prior run**: the `Lint commits` step rejected ten P1.x-era subjects (`feat(dag): P1.3 …` reads as sentence-case under `subject-case`). Resolved by rewording just those subject lines to the lowercase `land P1.x …` form the P2.x commits already used — a message-only `filter-branch` (tree hash byte-identical, commit count unchanged) plus a force-push, so those commits' SHAs moved. Lesson for future tasks: a green *local* `checks.sh` was never sufficient evidence the PR was green, because a failing step hides every step after it. (8) Gate: 30 `connectRules.test.ts` cases + 16 `Palette.test.tsx` cases (incl. `renders a node type it has never heard of`, the P3.5 acceptance test in miniature); `tsc` + 532 vitest + 861 core tests green. Two `git_ops` chmod tests fail **in this sandbox only** — the session runs as root, which bypasses the permission bits they assert; verified failing identically on the untouched base commit.

### P3.2 — Config side panel from JSON Schema
- **Goal:** Node config panel (never modals) rendered from the type's `config_schema`; prompt templates open Monaco full-height; verifier + retry policy as structured sub-forms with defaults; node cards show config essence (agent/model/effort badges, capability chip, retry summary like `verdict→implement ×3`).
- **Depends:** P3.1. **Size:** large-ish.
- **Context:** PRD §6.3; `WorkflowEditor.tsx` (784 — mine it for existing field editors/verifier form, it gets deleted in P3.6); `@monaco-editor/react` usage elsewhere (grep).
- **Touch:** new `src/components/canvas/ConfigPanel.tsx` + schema-form renderer; node card components.
- **Done when:** editing every field of a starter's agent node round-trips to valid v2 JSON.
- *Amendments (2026-07-26, as built):* (1) **The registry was keeping an enum implicit.** `capability` shipped as a bare `["string","null"]` with the four classes named only in prose, so a schema-driven panel would have had to hardcode them — reintroducing exactly the per-kind frontend knowledge P3.1 removed. Fixed at the source: `agent` and `sequence` now publish `enum: [read_only, artifacts, verify, implement, null]`, and the panel derives the select. A new Rust test (`every_config_property_is_renderable`) keeps every catalog property inside the vocabulary the renderer models, so a future schema can't silently downgrade a field to the raw-JSON escape hatch. (2) **Tests render from the live registry, not stand-ins.** A second env-gated regen test (`catalog_fixture_is_current`, sharing P2.1's `UPDATE_CANVAS_FIXTURES=1`) emits `node_catalog.json` beside the canvas fixtures, so `schemaForm.test.ts` and `ConfigPanel.test.tsx` exercise the **real** schemas — the Done-when round-trip drives the actual `standard-feature-pipeline` agent node through the panel and type-checks the result back against the schema its fields came from. A schema change that breaks the form now fails in Rust (stale fixture) rather than shipping. (3) **Three layers, deliberately separated.** `schemaForm.ts` is the pure derivation (schema → ordered `SchemaField[]`, plus the `def → def'` writes); `ConfigPanel.tsx` renders. Structured sub-forms are the two things a schema *cannot* express: `retry` isn't in `config` at all (first-class on `NodeConfigV2`), and `verifier`'s schema publishes no inner shape. Enabling a verifier writes the same literal `WorkflowEditor` wrote, and `defaultRetryRule('redirect')` matches what `migrate_v1_to_v2` emits for an `on_failure` step, so a hand-authored node and a migrated one are byte-identical. (4) **Property ordering is derived, not curated.** The registry serializes `properties` from a `BTreeMap` (no `preserve_order`), so they arrive alphabetically — which buries a node's prompt between `model` and `verifier`. Fields are grouped by *control height* (scalars → booleans → JSON → Monaco) rather than by a key list, keeping the top of the panel scannable without the frontend knowing a single key name. (5) **`artifacts` gets a JSON editor, not a fabricated form.** Its schema declares `items: {type: "object"}` and nothing more; inventing a name/path/capture form would model structure the registry never published and would silently drop any key it didn't cover on save. The JSON control round-trips anything and is the generic escape hatch for a future type's complex config — an unparseable edit stays in the box with an inline error instead of being dropped. (6) **Two catalog-backed value sources**, both degrading to the schema's own control: `agent_kind` becomes a select over `list_agents` (the registry can't know which agents an install has), and `effort` is clamped to the pinned agent's declared levels via `effortLevelsFor` — honoring the `reconcileEffort` contract `WorkflowEditor` ignored. (7) **Panel is mounted by the owning screen, not by the canvas** — same shape as run mode's `NodePanel`, which keeps `WorkflowCanvas` IPC-free per P3.1. The canvas's contribution is `showEssence`, which puts `nodeSummary.ts`'s badges (agent/model/effort/capability/net/shell chips, verifier shield, `verdict→implement ×3`) on design-mode cards only — run mode's second card row stays cost/duration. `nodeEssence` is the one place that reads conventional key names, and its contract is silent degradation: a node type using none of them gets no badge row at all. (8) No route is wired — P3.6 still owns replacing `WorkflowEditor`. (9) Gate: 22 `schemaForm.test.ts` + 19 `ConfigPanel.test.tsx` + 8 `nodeSummary.test.ts` cases; `tsc` clean, **581 vitest**, `cargo fmt`/`clippy -D warnings` clean, 863 core + 153 demeteo + 26 demeteo-runner tests passing (the same two `git_ops` chmod tests still fail under this root sandbox and only there).

### P3.3 — Validation surface, dirty guard, undo/redo
- **Goal:** Live lint badges on nodes (findings from P1.4 via a `workflow_lint` command), **save blocked only by errors**; dirty-state guard on navigation/Escape (closes audit F38) + 30s local draft autosave; undo/redo on all graph edits.
- **Depends:** P3.2. **Size:** medium/large.
- **Context:** PRD §6.3; `workflow_graph.rs` findings shape; `src/context/NavigationContext` (guard hook point); `docs/ux-audit/findings.md` F38.
- **Touch:** new `workflow_lint` Tauri command; canvas edit-history store (module-level reducer — codebase uses Context, not zustand); guard wiring.
- **Done when:** invalid save is impossible (error toast names findings); refresh mid-edit restores draft; ⌘Z/⇧⌘Z work across node+edge ops.
- *Amendments (2026-07-26, as built):* (1) **The Touch list had no home for the work.** All three deliverables are *document-level*: they need something that owns the definition being edited, whether it may be saved, and whether it is safe to leave. P3.1/P3.2 deliberately kept the canvas and config panel IPC-free and route-less, so this task adds `src/components/canvas/WorkflowBuilder.tsx` — the design-mode screen that composes canvas + palette + config panel + lint surface + history + guard + autosave. Persistence stays a prop (`onSave`); P3.6 still owns the route and the deletion of `WorkflowEditor`. (2) **Blocker recorded for P3.6: there is no v2 persistence yet.** `workflow_create`/`workflow_update` take a v1 `Vec<StepConfig>` and store `steps_json`, and a v2 graph cannot round-trip through that shape — node `position` (co-persisted layout, PRD §5.1), `join`, per-class `retry`, and edge `when` guards have no v1 representation, so a v2→v1 down-projection would silently discard the author's layout on every save. P3.6 must land v2 storage (or an explicitly lossy projection) before wiring the builder's save; nothing in P3.3 depends on it, because the `onSave` seam is enough to prove the save *gate*. (3) **The registry finally owns "known node types".** `lint_workflow_v2` (P1.4) takes its known-types list from the caller and `NodeHandler::lint` was `#[allow(dead_code)]` with a note pointing at this task; a new `adapters/step_executor/node_lint.rs` joins the two and feeds it the **registry's** kinds. Retired aliases (`parallel`) count as known here even though the palette excludes them — lint answers *"will the engine dispatch this"*, the palette answers *"should new work be authored on it"*, and conflating them would make a pre-rename workflow unsavable in the builder meant to edit it. A node type added in Rust now stops linting as unknown with zero edits, extending P3.5's guarantee from the palette to validation. (4) **Enforcement is at the write path, not only the button.** "Invalid save is impossible" is a convention if only a disabled button enforces it, so P1.3's `ensure_valid_v2_projection` now also rejects error-severity findings — covering `workflow_import` of a hand-edited file, not just the builder. Proven safe by `every_migrated_starter_lints_clean`: all seven starters lint error-free, so no existing workflow becomes unsavable. `workflow_graph::has_errors` is the single predicate both sides read, so "invalid" cannot come to mean two things. (5) **`LintFinding` gained `Serialize` only** — `code` is a `&'static str` from this module's fixed vocabulary, so a finding can travel out to the builder but cannot be minted outside the crate and handed back as ours; the edge anchor serializes as a `[from, to]` pair. `workflow_lint` takes the raw payload rather than a typed definition so an unreadable graph comes back as a renderable `schema-invalid` finding instead of an opaque IPC error. (6) **The dirty guard lives on the navigation context, not the screen.** F38 is precisely the bug of covering one exit and missing three, so `NavigationProvider` grew vetoable `NavigationIntent`s: the Back arrow, global `Escape`, `Cmd+W`, and the mouse back button all funnel through `navigate`/`goBack`/`goForward` and are covered by one prompt. Guards stack innermost-first, hold the blocked intent, and replay it verbatim through `proceed` once the user has saved or discarded. (7) **Dirty is a comparison, not a flag.** `graphHistory.ts` keeps whole-definition snapshots (`graphEdits` already returns immutable `def'`, so undo needs no inverse operations) and compares the present against the last *saved* snapshot — undoing back to the saved shape correctly reads clean, which a boolean can't express. No-op commits are dropped: the canvas re-commits on gestures that land where they started, and those would otherwise fill the undo stack with steps that appear to do nothing. (8) **Draft autosave is `localStorage`, deliberately not a `WorkflowVersion`.** Every save mints an immutable version row that a run can pin; half-finished graphs must not pollute that history. The 30s interval is installed once per dirty transition rather than per edit — a timer restarted by every keystroke would never fire for a fast typist. (9) Lint round-trips are debounced and keyed on the *serialized* graph, so the canvas's constant re-derivation costs no IPC; the previous findings stay visible while a new lint is in flight (badges that blink off mid-edit read as flicker) and out-of-order replies are dropped. (10) Gate: `graphHistory` (9), `workflowDraft` (6), `lint`/`flowGraph` overlay (9), `useWorkflowLint` (5), `WorkflowBuilder` (13), navigation guards (3) = +45 → **626 vitest**; 6 new `node_lint` Rust tests → 871 core, 26 runner, src-tauri suite green; `tsc`, `cargo fmt`, `clippy -D warnings` clean. Trap worth remembering: `beforeEach(() => vi.mocked(fn).mockReset())` returns the *mock* — vitest treats a returned function as the hook's teardown and calls it after the test. Use a block body.

### P3.4 — Version history drawer (diff / restore / revert)
- **Goal:** UI for the long-existing `workflow_versions` command (closes audit F39's missing-UI half): drawer with version list, structural diff highlighted on canvas (added/removed/changed nodes), restore-as-new-version, revert-to-default for starters.
- **Depends:** P3.3. **Size:** medium.
- **Context:** `src-tauri/src/commands/workflows.rs` (`workflow_versions` :301, `workflow_revert_to_default` :395); `repos/workflow.rs` (340); `WorkflowCanvas.tsx` overlay API.
- **Touch:** new `src/components/canvas/VersionDrawer.tsx`; pure graph-diff util + tests.
- **Done when:** diff between two starter versions renders; restore creates a new version row (immutability preserved).
- *Amendments (2026-07-26, as built):* (1) **Touch was missing two backend commands.** `workflow_versions` hands back `steps_json` strings and migration is Rust-only, so the drawer had no way to obtain a *graph* to diff — added `workflow_version_graph(workflowId, versionId)`, the design-mode twin of P2.2's `feature_workflow_graph` (which resolves the version a *run* pinned; this one resolves the version an *author* picked). It also proves the version belongs to the workflow named: version ids are guessable by construction (`<workflow-id>-v3`), so an unchecked pair would let one workflow's history be restored onto another. (2) **Restore is a storage-layer copy, not an editor round-trip.** `workflow_restore_version` copies the stored `steps_json` **verbatim** into a new row. Routing it through the builder's v2 model was the obvious alternative and is wrong today: v2→v1 storage is still lossy (the P3.6 blocker), so "restore v3" would have returned something that merely *migrates to the same graph* rather than v3. This also makes restore independent of P3.6 — it works now, and it puts restore on the same footing as `workflow_revert_to_default`, which has always copied the bundled starter's steps directly. Name/description aren't versioned, so a restore leaves them alone. (3) **One append seam.** Edit, revert-to-default, and restore all mint versions; the numbering arithmetic existed in two copies and was about to become three, so `append_version` now owns it (`next_version_number` = one past the highest, never reusing a value even across a gap). "Saving is an append, never an edit" is one fact in one place, and the Done-when's immutability claim is tested against a real in-memory SQLite repo rather than a stand-in — `restore_version`/`version_graph` are extracted command cores the `#[tauri::command]` wrappers delegate to, the pattern `tray_notification.rs` already set. (4) **Diff needs a union graph.** A node v3 had and the working copy doesn't exists in neither definition alone, so `graphDiff.mergeForDiff` builds the union (removed nodes keep the older version's position) and `diffGraphs` supplies verdicts keyed to it. Two judgment calls are load-bearing: **position is not structure** (a `moved` flag on an otherwise `unchanged` node — otherwise one auto-layout lights up the whole canvas and buries the real edit), and **absent / `null` / `undefined` are the same value** (the two sides come from different producers — the Rust migration vs. the editor's own `graphEdits` — which disagree about how an empty optional is written; treating that as a change would make every stored version look different from the graph it round-tripped into). (5) **Comparing is read-only.** The builder swaps the canvas to `mode="run"` over the merged graph while a comparison is active: the palette, lint badges, and config panel all go away, because half of what's on screen is a version that no longer exists to be edited. Diff colors get the visual channel to themselves (emerald added / rose dashed removed / amber changed, on cards and edge strokes); the header lint chip still reports on the *working copy*, which is what it was always about. (6) **Restore and revert are refused while the editor is dirty**, with the reason spelled out on the card rather than a silently dead button (PRD §6.4, the shape P2.4's ancestor guard uses) — both replace the graph on the canvas, and swallowing an unsaved prompt template is exactly the F38 failure the builder exists to prevent. Adopting a restored version resets undo history: it is already a persisted version, and undoing "back past" it would offer to restore state the store no longer agrees with. (7) The drawer takes a `reloadToken` the builder bumps after any write, so a normal save shows up in the list without a remount (which would drop the compare selection). (8) Gate: 13 `graphDiff.test.ts` + 8 `VersionDrawer.test.tsx` + 3 `WorkflowBuilder.test.tsx` cases (compare renders the diff on the canvas and is read-only; a restore lands clean at the new version; dirty blocks it) = **650 vitest**; 5 new Rust tests → 158 src-tauri, 871 core, 26 runner green; `tsc`, `cargo fmt`, `clippy -D warnings`, and `scripts/checks.sh` all clean.

### P3.5 — `command` node type (the extensibility proof)
- **Goal:** Deterministic shell-command node via the existing `ExecutionPort` (un-defers Decision 8): registry-only backend diff — **zero scheduler-file edits** (PRD §9 metric). Config: command, cwd, env allowlist, timeout, `idempotent: true|false` (P1.14 semantics: non-idempotent interrupted → synthetic gate). Output as `text`/`file` port.
- **Depends:** P1.7, P1.14. Frontend appears automatically via P3.1. **Size:** medium.
- **Context:** PRD §5.2 (command row); `registry.rs` + one existing handler as template (`steps/sync.rs`, 306 — closest shape); `ExecutionPort` (`src/ports/execution.rs` :1-100 + trait fns); Decision 8 in `docs/DECISIONS.md`.
- **Touch:** new `steps/command.rs`, registration line in `registry.rs`, tests. **Nothing else** — if `driver.rs`/`scheduler.rs` need edits, the seam failed; stop and report.
- **Done when:** starter-style workflow with a command node runs under stub harness; `git diff --stat` shows registry-only backend change; node appears in builder palette untouched.

### P3.6 — Templates, import/export v2, launch integration, delete `WorkflowEditor`
- **Goal:** "New workflow" from starter clone or three shapes (blank / plan-implement-validate / plan-gate-implement-validate-gate); export/import now schema-v2 with positions; `StartFeatureModal` per-step override list gains mini-graph preview; **delete `WorkflowEditor.tsx`** and its routes; read-only Monaco source tab (P0.1 decision 5).
- **Depends:** P3.2–P3.4. **Size:** medium/large.
- **Context:** `WorkflowList.tsx` (323); `StartFeatureModal.tsx` — read only the per-step override section (grep `override`); `useLaunchRun.ts` (170); `workflows.rs` export/import (:311-394).
- **Touch:** `WorkflowList.tsx`, `StartFeatureModal.tsx`, delete `WorkflowEditor.tsx`, template fixtures.
- **Done when:** J3 script passes: build "bugfix + security-scan branch" via UI in <10 min unaided (self-run the script, note friction); no references to `WorkflowEditor` remain.
- ⚠️ **Prerequisite surfaced by P3.3 — v2 persistence.** The builder (`WorkflowBuilder`, P3.3) produces a schema-v2 definition, but storage is still v1: `workflow_create`/`workflow_update` accept `Vec<StepConfig>` and write `workflow_versions.steps_json`. Node `position`, `join`, per-class `retry`, and edge `when` have no v1 representation, so saving through the existing commands would discard the author's layout and any non-linear construct. Before wiring the route, this task must either store v2 (new column / v2 `steps_json` with a read-time `migrate_definition` fallback — the reader already tolerates both, see `migrate_definition`) or make the loss explicit and refuse to save graphs that would lose information. Same decision governs export/import v2 with positions, which is already in this task's Goal. *P3.4 note:* restore and revert-to-default deliberately sidestep this by copying `steps_json` at the storage layer, so history operations already work — but that trick is only available to operations that never pass through the editor's model. A save does, which is why it is still blocked on this.

---

## Phase 4 — DAG payoff

### P4.1 — Write-scope exclusion lint + `max_parallel_nodes > 1`
- **Goal:** The §5.6 invariant twice: save-time lint (editor warning) and schedule-time hard check — concurrent nodes only if both `ReadOnly`/`Artifacts`, or `Implement` on disjoint repos; same-repo implement concurrency impossible by construction. Scheduler dispatches up to the ceiling.
- **Depends:** P1.12, P3.3. **Size:** large-ish. Only task that touches scheduler *and* driver concurrency.
- **Context:** PRD §5.6, §10 (merge-storm row); `scheduler.rs`; `driver.rs` dispatch loop (post-P1.12 shape); `workflow.rs` `effective_capability` (:123) or its v2 successor; `workflow_graph.rs`.
- **Touch:** `scheduler.rs` (ceiling + exclusion), `workflow_graph.rs` (lint rule), driver concurrency plumbing (bounded join set), tests: two ReadOnly nodes run concurrently under stub; two same-repo implement nodes provably serialize.
- **Done when:** invariant test cannot be defeated by config; default remains 1.

### P4.2 — Parallel shapes for Standard Feature + Refactor starters
- **Goal:** Ship the PRD §7 shapes: Standard = `research ∥ baseline-harness(command)` → `tickets`; `validate ∥ critic` → `gate-ship`; Refactor = `baseline(command)`; `regression ∥ api-drift-review` → `gate-diff`. Other five stay chains and must remain **bit-identical** to P0.2 snapshots.
- **Depends:** P4.1, P3.5. **Size:** medium.
- **Context:** PRD §7; `src-tauri/workflows/standard-feature-pipeline.json` + `refactor.json`; seeding drift logic `src-tauri/src/commands/workflows.rs:11-60`; P0.2 harness.
- **Touch:** the two starter JSONs (as v2), new baseline snapshots for those two, seeding test updates.
- **Done when:** both run green under stub with genuine concurrency observed in `run_events` ordering; five untouched starters' snapshots unchanged.

### P4.3 — Conditional edges in the builder
- **Goal:** Expose `when` guards (P1.5 grammar) in design mode: edge inspector with expression input, validation against the grammar + referenced node outputs, skip-reason rendering already handled by run mode.
- **Depends:** P1.5, P3.2. **Size:** medium.
- **Context:** PRD §5.1 (expressions), §6.3; `expr.rs` public API; `ConfigPanel.tsx` patterns.
- **Touch:** edge inspector UI, client-side expression validation (call a `expr_validate` Tauri command rather than reimplementing the grammar).
- **Done when:** the PRD's critic example (`verdict != 'FAIL'`) is authorable end-to-end and skips correctly at runtime.

### P4.4 — `subworkflow` node type
- **Goal:** Reference a saved workflow version as a node; child run linked to parent; nesting depth 1 (enforced by lint). Registry-only backend diff, same rule as P3.5.
- **Depends:** P3.5 (pattern), P1.15 (version pinning). **Size:** large-ish.
- **Context:** PRD §5.2 (subworkflow row); `steps/command.rs` (as the newest handler template); `repos/feature.rs` (parent/child linkage columns — likely migration **V34**); `driver_registry.rs` (child driver spawn dedup).
- **Touch:** new `steps/subworkflow.rs`, registration, migration for parent-run linkage, run-mode canvas "enter child" affordance.
- **Done when:** parent run shows child node with roll-up status; depth-2 nesting rejected at lint.

---

## Status

| Task | Title | Status |
|---|---|---|
| P0.1 | Decision records | ✅ 2026-07-23 |
| P0.2 | Starter baseline harness | ✅ 2026-07-23 |
| P1.1 | Schema v2 structs | ✅ 2026-07-24 |
| P1.2 | v1→v2 migration | ✅ 2026-07-24 |
| P1.3 | JSON Schema validation | ✅ 2026-07-24 |
| P1.4 | WorkflowGraph + lint | ✅ 2026-07-24 |
| P1.5 | Expression evaluator | ✅ 2026-07-24 |
| P1.6 | Registry + agent/sync handlers | ✅ 2026-07-24 |
| P1.7 | gate/sequence/finalize; match deleted | ✅ 2026-07-24 |
| P1.8 | step_attempts (V31) | ✅ 2026-07-24 |
| P1.9 | Durable checkpoints (V32) | ✅ 2026-07-24 |
| P1.10 | Declarative retry policy | ✅ 2026-07-24 |
| P1.11 | Ready-set scheduler core | ✅ 2026-07-24 |
| P1.12 | Driver integration (L) | ✅ 2026-07-24 |
| P1.13 | Unified run_events | ✅ 2026-07-24 |
| P1.14 | Fingerprint + idempotency | ✅ 2026-07-24 |
| P1.15 | Pin workflow_version_id (V33) | ✅ 2026-07-24 |
| P1.16 | Phase-1 exit gate | ✅ 2026-07-24 |
| P2.1 | Canvas foundation | ✅ 2026-07-24 |
| P2.2 | Run-mode overlay + toggle | ✅ 2026-07-24 |
| P2.3 | Panel: Overview/Output | ✅ 2026-07-24 |
| P2.4 | Panel: Live/Actions | ✅ 2026-07-24 |
| P2.5 | Sequence expansion | ✅ 2026-07-24 |
| P2.6 | Remote on canvas; split-path deletion | ✅ 2026-07-24 |
| P3.1 | Registry palette + connect rules | ✅ 2026-07-25 |
| P3.2 | Schema-driven config panel | ✅ 2026-07-26 |
| P3.3 | Lint surface, dirty guard, undo | ✅ 2026-07-26 |
| P3.4 | Version history drawer | ✅ 2026-07-26 |
| P3.5 | `command` node (seam proof) | ☐ |
| P3.6 | Templates, import/export, editor deletion | ☐ |
| P4.1 | Write-scope lint + parallelism | ☐ |
| P4.2 | Parallel starter shapes | ☐ |
| P4.3 | Conditional edges UI | ☐ |
| P4.4 | `subworkflow` node | ☐ |

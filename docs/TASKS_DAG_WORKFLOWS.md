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

---

## Phase 2 — Run visibility

### P2.1 — Canvas foundation (deps + static graph render)
- **Goal:** Add `@xyflow/react` + `elkjs` (elk in a web worker); new `WorkflowCanvas` component rendering a pinned version's v2 graph read-only: node cards with kind icon/title, edges, minimap auto-hidden under ~8 nodes, fit-view, elk layout button, keyboard nav. Design language: existing dark neon glassmorphism, `ui/` kit, lucide.
- **Depends:** P1.15 (pinned version to render), P1.2 (v1 defs render via migration). **Size:** medium/large.
- **Context:** PRD §6.1; `package.json`; `src/components/ui/` (skim exports); `src/lib/runStatus.ts`; one starter JSON migrated to v2 for fixture.
- **Touch:** `package.json`; new `src/components/canvas/WorkflowCanvas.tsx`, `src/components/canvas/nodes/*.tsx`, `src/lib/elkLayout.worker.ts`; a fixture-driven test.
- **Done when:** canvas renders all 7 migrated starters from fixtures; no console errors; **battery rule respected — opacity-only animation, no infinite transforms** (see webview perf work).

### P2.2 — Run mode: live status overlay + Graph|Timeline toggle
- **Goal:** Embed `WorkflowCanvas` in `FeatureDetail` behind a "Graph | Timeline" toggle (list stays default). Status overlay from the unified `run_events` stream (P1.13): running pulses (opacity-only), completed shows duration+cost chips, failed glows with failure class, skipped dims with reason tooltip, gate shows amber shield opening the existing `GateView`.
- **Depends:** P2.1, P1.13. **Size:** large-ish.
- **Context:** PRD §6.1; `FeatureDetail.tsx` — **do not read all 1963 lines**: read the header/imports (:1-60), steps state (:200-260), and the step-list render + event wiring sections (grep `useTauriEvent`, `StepProgress`); `useTauriEvent.ts` (37); `runStatus.ts`; `GateView.tsx` (281).
- **Touch:** `FeatureDetail.tsx` (toggle + mount), `WorkflowCanvas.tsx` (run-mode props), new `src/hooks/useRunEvents.ts` (single stream consumer both modes share).
- **Done when:** live stub run animates correctly in both modes from the same hook; toggle preserves selection.

### P2.3 — Node drill-down panel: Overview + Output
- **Goal:** Clicking a node opens a right side panel (split-panel pattern like `ArtifactViewer`): **Overview** = status, attempt table (class, cost, duration, outcome) from `step_attempts` via a new Tauri command; **Output** = artifacts (Monaco), harness output, verifier verdict with failing tests/implicated files.
- **Depends:** P2.2, P1.8. **Size:** medium/large.
- **Context:** PRD §6.2; `ArtifactViewer.tsx` (421); `src-tauri/src/commands/features.rs` (`step_get` :110, `artifact_body` :216); `repos/step_attempts.rs`.
- **Touch:** new Tauri command `step_attempts_list` (features.rs + repo call); new `src/components/canvas/NodePanel.tsx` (+ Overview/Output tabs); `WorkflowCanvas.tsx` selection wiring.
- **Done when:** seeded failing run → root-cause artifact reachable in ≤3 clicks (the J2 metric).

### P2.4 — Node panel: Live + Actions tabs
- **Goal:** **Live** tab hosts the existing `agent_stream` transcript (moves from the inline toggle in `FeatureDetail`). **Actions** tab: Retry (shows which policy rule will apply), Replay-from-node (existing `replay_from_step`, now highlights the downstream subgraph before confirm), Stop node, Decide gate — all respecting the ancestor guard with disabled-button explanations (kept UX, PRD §6.4).
- **Depends:** P2.3. **Size:** medium/large.
- **Context:** `FeatureDetail.tsx` agent-stream section (grep `agent_stream`/`AgentStream`); `features.rs` (`step_retry` :160, `replay_from_step` :181, `gate_decide` :144); `impl_traits/replay.rs` (230, what replay invalidates).
- **Touch:** `NodePanel.tsx` (two tabs), `FeatureDetail.tsx` (remove inline transcript), small backend addition if replay needs to return the affected-subgraph preview.
- **Done when:** each action verified against a stub run; guard states render with explanations.

### P2.5 — Sequence node expansion (landed-prefix legibility)
- **Goal:** Sequence nodes expand in place (accordion in node or panel) showing the task list with per-task status/cost; the landed prefix visually distinct from pending tasks — making Decision 13 semantics legible (PRD §6.2).
- **Depends:** P2.3, P1.9 (durable checkpoints are the data source). **Size:** medium.
- **Context:** `steps/sequence/runner.rs` checkpoint shape (grep only); checkpoint repo (P1.9); `NodePanel.tsx`.
- **Touch:** canvas sequence node component, `NodePanel.tsx`, a Tauri command exposing checkpoint/task state if not already in `run_events`.
- **Done when:** mid-sequence stub run shows landed vs pending split matching DB checkpoint.

### P2.6 — Remote runs on the same canvas; delete the split path
- **Goal:** Remote/detached runs render on `WorkflowCanvas` from the same `run_events` stream; `RunEventTimeline` survives only as the raw event feed inside the panel's Overview tab. Delete the separate polling surface + dead parallel UI (audit F36) — PRD targets **≥1k LOC net removal**.
- **Depends:** P2.2, P1.13. **Size:** large-ish.
- **Context:** `RunEventTimeline.tsx` (426 — full read; it's being absorbed); `src-tauri/src/commands/remote_runner.rs` (grep command names `remote_get_status`, `remote_run_for_feature`); `useRunEvents.ts` (P2.2); `FeatureDetail.tsx` remote-branch sections (grep `RunEventTimeline`).
- **Touch:** `useRunEvents.ts` (poll fallback for remote), `NodePanel.tsx` (raw feed), delete/shrink `RunEventTimeline.tsx`, `FeatureDetail.tsx` cleanup.
- **Done when:** remote stub run renders live on canvas; `RemoteGateActions`/`ReinjectCredentials` still reachable; LOC delta reported in PR description.

---

## Phase 3 — Builder

### P3.1 — Registry metadata over IPC + palette + connect rules
- **Goal:** Tauri command `node_types_list` returning each registered handler's `kind`, `config_schema()`, display metadata. Design-mode canvas: palette derived from it, drag-from-handle into empty canvas → type-compatible "what can connect here" picker, Cmd+K node search, connect-time port type checking and cycle prevention (client-side mirror of P1.4 rules).
- **Depends:** P2.1, P1.7. **Size:** large-ish.
- **Context:** PRD §6.3 (palette bullets); `registry.rs`; `src-tauri/src/commands/workflows.rs` (:1-130 for command style); `WorkflowCanvas.tsx`.
- **Touch:** new command in `workflows.rs`; `WorkflowCanvas.tsx` design-mode; new `src/components/canvas/Palette.tsx`, connect-validation util (shared with P1.4 semantics — port the rules table, don't re-derive).
- **Done when:** every registered type appears in palette automatically (verify by the P3.5 `command` type appearing with zero frontend edits).

### P3.2 — Config side panel from JSON Schema
- **Goal:** Node config panel (never modals) rendered from the type's `config_schema`; prompt templates open Monaco full-height; verifier + retry policy as structured sub-forms with defaults; node cards show config essence (agent/model/effort badges, capability chip, retry summary like `verdict→implement ×3`).
- **Depends:** P3.1. **Size:** large-ish.
- **Context:** PRD §6.3; `WorkflowEditor.tsx` (784 — mine it for existing field editors/verifier form, it gets deleted in P3.6); `@monaco-editor/react` usage elsewhere (grep).
- **Touch:** new `src/components/canvas/ConfigPanel.tsx` + schema-form renderer; node card components.
- **Done when:** editing every field of a starter's agent node round-trips to valid v2 JSON.

### P3.3 — Validation surface, dirty guard, undo/redo
- **Goal:** Live lint badges on nodes (findings from P1.4 via a `workflow_lint` command), **save blocked only by errors**; dirty-state guard on navigation/Escape (closes audit F38) + 30s local draft autosave; undo/redo on all graph edits.
- **Depends:** P3.2. **Size:** medium/large.
- **Context:** PRD §6.3; `workflow_graph.rs` findings shape; `src/context/NavigationContext` (guard hook point); `docs/ux-audit/findings.md` F38.
- **Touch:** new `workflow_lint` Tauri command; canvas edit-history store (module-level reducer — codebase uses Context, not zustand); guard wiring.
- **Done when:** invalid save is impossible (error toast names findings); refresh mid-edit restores draft; ⌘Z/⇧⌘Z work across node+edge ops.

### P3.4 — Version history drawer (diff / restore / revert)
- **Goal:** UI for the long-existing `workflow_versions` command (closes audit F39's missing-UI half): drawer with version list, structural diff highlighted on canvas (added/removed/changed nodes), restore-as-new-version, revert-to-default for starters.
- **Depends:** P3.3. **Size:** medium.
- **Context:** `src-tauri/src/commands/workflows.rs` (`workflow_versions` :301, `workflow_revert_to_default` :395); `repos/workflow.rs` (340); `WorkflowCanvas.tsx` overlay API.
- **Touch:** new `src/components/canvas/VersionDrawer.tsx`; pure graph-diff util + tests.
- **Done when:** diff between two starter versions renders; restore creates a new version row (immutability preserved).

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
| P1.14 | Fingerprint + idempotency | ☐ |
| P1.15 | Pin workflow_version_id (V33) | ✅ 2026-07-24 |
| P1.16 | Phase-1 exit gate | ☐ |
| P2.1 | Canvas foundation | ☐ |
| P2.2 | Run-mode overlay + toggle | ☐ |
| P2.3 | Panel: Overview/Output | ☐ |
| P2.4 | Panel: Live/Actions | ☐ |
| P2.5 | Sequence expansion | ☐ |
| P2.6 | Remote on canvas; split-path deletion | ☐ |
| P3.1 | Registry palette + connect rules | ☐ |
| P3.2 | Schema-driven config panel | ☐ |
| P3.3 | Lint surface, dirty guard, undo | ☐ |
| P3.4 | Version history drawer | ☐ |
| P3.5 | `command` node (seam proof) | ☐ |
| P3.6 | Templates, import/export, editor deletion | ☐ |
| P4.1 | Write-scope lint + parallelism | ☐ |
| P4.2 | Parallel starter shapes | ☐ |
| P4.3 | Conditional edges UI | ☐ |
| P4.4 | `subworkflow` node | ☐ |

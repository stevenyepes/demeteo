# PRD — True DAG Workflows: Engine, Run Visibility & Visual Builder

**Status:** **Built through Phase 3** (2026-07-26). Phase 4 remains — see [`TASKS_DAG_WORKFLOWS.md`](TASKS_DAG_WORKFLOWS.md).
**Date:** 2026-07-23
**Author:** Steven Yepes (drafted with Claude)
**Supersedes / revises:** Decision 19 (form-first authoring) — already recorded in `DECISIONS.md` §2; parts of USER_STORIES Story 8 "Workflow Authoring"; UX_JOURNEYS Journey 6/10.
**Related docs:** `DDD_MODEL.md`, `RELIABILITY_PLAN.md`, `EXECUTION_PARITY.md`, `DECISIONS.md`, `OPEN_QUESTIONS.md`, `docs-site/workflows.md`, `docs/ux-audit/findings.md`

> **How to read this doc now.** Sections 5 and 6 are the **specification of the
> system as built** — the definition model, the registry seam, the scheduler,
> the retry policy, and the two-mode canvas. They stay because they carry the
> *reasoning*, which the code does not. Sections 1–3 are the original
> motivation, kept as the record of why the linear-list model was abandoned.
> Section 7 describes the starter migration, whose parallel shapes are the
> unbuilt P4.2.

---

## 1. Why now (the state this replaced)

Demeteo's public positioning is: *"Demeteo plans the work as a directed acyclic graph of steps, runs multiple agents in parallel worktrees, and keeps you in control of what gets merged, and when"* (`docs-site/index.md`). Before this work the implementation did not match the promise:

- **The "DAG" was a linear list.** `WorkflowVersion.steps_json` was an ordered `Vec<StepConfig>` and the executor walked `step_index` forward one at a time. The only edges were backward `on_failure` retry redirects and the advisory `blocked_by` on planned tasks, which execution ignored.
- **Step kinds were a hardcoded `match`.** `agent | gate | sequence(+parallel alias) | sync | finalize` were dispatched inline in the driver. Adding a step kind meant editing the engine — no registry, no plugin seam, and the deferred `command` step (Decision 8) had nowhere clean to land.
- **Reliability state was partly in-memory.** Sequence checkpoints, cached task plans, the env-retry set, gate waiters, and the driver registry all evaporated on restart. A crash mid-sequence re-ran committed work (safe but wasteful); recovery leaned on a startup watchdog that converted interruptions into synthetic gates.
- **Visibility was a flat timeline.** `FeatureDetail.tsx` rendered a vertical list with per-step chips; structure inside a step (sequence tasks, verifier passes, harness runs, retry loops) was invisible. Remote runs polled a separate event log with a different component.
- **The editor was a stacked form.** `WorkflowEditor.tsx` reordered with chevrons, had no dirty-state guard (audit F38), left dangling `on_failure` pointers after delete/reorder (F39), had no version-history UI despite versions existing in the DB, and could not express anything the engine couldn't run — which meant: nothing non-linear.

Meanwhile the roadmap bets on workflows as the moat ("*Vibe Kanban runs an agent per task; Demeteo runs a governed pipeline per task*"), Kanban cards will carry pinned workflow versions (Epic C1), and the CLI (Epic D) will run workflows headless. All three bets get stronger with a real, well-defined, extensible DAG — and weaker if we keep stretching a linear list.

## 2. Product goals

1. **Reliability** — a run survives crashes, restarts, and multi-day gate waits with zero lost paid-for work; every failure is classified and mapped to a deliberate policy, not an accident of control flow.
2. **Extensibility** — new step kinds (command, sub-workflow, fan-out) are added behind a registry without touching the scheduler; workflows are data with a versioned schema that the CLI, Kanban, and future marketplace can all consume.
3. **Visibility** — the user can answer "what is happening, what happened, what did it cost, and why did it fail" for any node, any attempt, at any zoom level, live or historical, local or remote, from one surface.
4. **Authorability** — a user can build a correct custom workflow in under 10 minutes without reading docs, and cannot save an invalid one.

### Non-goals (v-this)

- Multi-user / collaborative editing, marketplace sharing (OPEN_QUESTIONS §17, v3+).
- WASM plugin host for third-party step types (deferred; the registry is the seam it will plug into later).
- Re-introducing parallel *worktree* execution inside the implement step. The July 2026 parallel→sequence rewrite conclusions stand (Decision 13): concurrent worktrees on one repo produced merge storms. DAG parallelism in this PRD is for **read-only/artifact-producing branches and cross-repo work**, not for concurrent writes to one repo (enforced — see §5.6).
- Pre-launch LLM cost estimation (Decision 23 stands).
- Migrating in-flight runs to edited definitions. Runs pin the version they started with, forever.

## 3. Users & jobs

Same persona as today (terminal-native power users running CLI coding agents), three jobs:

- **J1 — Run with confidence:** launch a governed pipeline, walk away, come back to either a gate or a finished branch, and trust nothing was silently lost or double-billed.
- **J2 — Diagnose fast:** when a run fails or a verdict is BLOCKED, see *which node*, *which attempt*, *what the agent saw and said*, and *what the harness printed* in ≤3 clicks.
- **J3 — Author and evolve:** clone a starter, reshape it (add a security-scan branch, a second verifier, a command step), validate it, and version it — visually, without hand-editing JSON.

## 4. What must be preserved

The gap table that sat here described the pre-DAG system and no longer holds —
§1 keeps the summary. What survives, and is **load-bearing for anything built on
top**: harness-first validation (a red harness fails at zero token cost),
regression-vs-environment triage with failure fingerprints
([`EXECUTION_PARITY.md`](EXECUTION_PARITY.md)), keep-the-prefix +
targeted-retry semantics for task lists (Decision 13), durable DB-first gates
with a waiter fast-path, the predecessor-running guard, per-step cost/duration
telemetry, one-shot CLI agent invocation (Decision 34), and the allow/deny-only
permission model with gates as the sole human-in-the-loop surface (Decision 35).

## 5. The target: a real DAG system

### 5.1 Definition model (workflow-as-data v2)

Canonical machine format: JSON, stored in `workflow_versions`, validated by a published JSON Schema, snapshotted immutably into every run. Shape (n8n-inspired nodes+edges, GitHub-Actions-inspired expressions):

```jsonc
{
  "schema_version": 2,
  "id": "wf-starter-standard",
  "name": "Standard Feature Pipeline",
  "nodes": [
    {
      "id": "research",
      "type": "agent",            // registry key
      "type_version": 1,           // per-node-type evolution without breaking old defs
      "title": "Research Codebase",
      "config": {
        "prompt_template": "...",
        "agent_kind": null,        // null = project default (fixes audit F39 hardcoded opencode)
        "model": null,
        "effort": null,
        "capability": "artifacts",
        "outputs": [ { "name": "report", "type": "file", "path": "artifacts/research-report.md" } ]
      },
      "retry": { /* see 5.4; optional, defaults from workflow.defaults */ },
      "position": { "x": 0, "y": 0 }   // editor layout co-persisted
    }
  ],
  "edges": [
    { "from": "research", "to": "tickets" },
    { "from": "critic", "to": "gate-ship", "when": "${{ nodes.critic.outputs.verdict != 'FAIL' }}" }
  ],
  "defaults": { "retry": {...}, "join": "all_success" }
}
```

Key decisions:

- **Static declarative DAG with bounded dynamism.** The graph is fully renderable and validatable before running. The only runtime expansion is the `sequence` node (task list → N task attempts, already the model) and a future `fan_out` node that maps a declared sub-graph over a runtime list. No Prefect-style implicit graphs.
- **Edges are forward dependencies.** A node is *ready* when its join condition over incoming edges is met. Join semantics per node: `all_success` (default) | `any_success` | `all_done`. Failure/skip propagate: an unsatisfied `when` or failed dependency marks downstream `skipped(reason)` unless the join says otherwise.
- **`on_failure` becomes retry policy, not an edge.** Backward "goto" edges are replaced by the declarative retry block (§5.4) with `redirect_to` as one strategy. The lint rule "must point strictly earlier" becomes "redirect target must be an ancestor" — cycles remain impossible by construction (the definition graph is acyclic; iteration lives in run state, as today with `iteration_count`).
- **Coarse typed ports.** Output/input types: `text | file | task_list | verdict | approval | any`. Checked at connect-time in the editor and at lint-time in the engine. `task_list_from` becomes a normal typed edge (`task_list` output → sequence node input) instead of a magic field.
- **Expressions** use a minimal sandboxed syntax `${{ nodes.<id>.outputs.<name> }}` (+ a handful of comparison operators) for edge conditions and input bindings. No general scripting.
- **`schema_version: 1` (linear lists) auto-migrates**: list order becomes a chain of edges; `on_failure` fields become retry policies; `task_list_from` becomes an edge. Migration is pure and covered by tests against all seven bundled starters plus DB-found customs. `parallel` alias resolves to `sequence` and is dropped from the schema.

### 5.2 Node type registry (extensibility)

Replace the `match` in `driver.rs` with a `NodeTypeRegistry` (same pattern as the existing `AgentRegistry`):

```rust
trait NodeHandler: Send + Sync {
    fn kind(&self) -> &'static str;
    fn config_schema(&self) -> &'static serde_json::Value; // JSON Schema; editor renders + validates from this
    fn lint(&self, node: &NodeConfig, graph: &WorkflowGraph) -> Vec<LintFinding>;
    async fn execute(&self, ctx: NodeCtx<'_>) -> StepOutcome;   // existing StepOutcome enum, unchanged
    fn cancel_grace(&self) -> CancelBehavior;                    // graceful vs hard-kill contract
}
```

Launch set of node types (existing five, re-homed, plus two unlocked cheaply by the registry):

| Type | Notes |
|---|---|
| `agent` | Unchanged semantics; config schema formalized |
| `gate` | Unchanged; durable suspend node; still the only HITL surface (Decision 35) |
| `sequence` | Unchanged keep-the-prefix semantics; consumes a `task_list` port |
| `sync` | Unchanged |
| `finalize` | Unchanged; lint keeps "single trailing finalize" as a per-graph rule ("exactly one sink of type finalize") |
| `command` **(new)** | Deterministic shell command via existing `ExecutionPort` (un-defers Decision 8/OPEN_QUESTIONS §8). Zero-token harness/build/script steps stop being emulated by agent prompts |
| `subworkflow` **(new, phase 3)** | Reference a saved workflow version as a node; child run linked to parent; nesting depth 1 |

The editor's node palette, config panels, and validation all derive from `config_schema()` + `lint()` — adding a node type automatically lights up the builder. This is the seam the future WASM plugin host plugs into; we do not build the host now.

### 5.3 Execution engine: ready-set scheduler

Replace the linear `step_index` loop with a small in-process scheduler per feature (still one tokio task per feature, still `DriverRegistry`-deduped):

1. Compute the **ready set**: nodes whose join condition is satisfied and whose `when` guards pass.
2. Dispatch up to `max_parallel_nodes` (default **1** — see §5.6) via the registry.
3. Persist every state transition in a SQLite transaction **before** acting on it; emit the corresponding Tauri/run event after commit.
4. On terminal outcome, re-evaluate the ready set; on empty ready set with non-terminal nodes remaining → deadlock lint should have prevented this; fail loudly.
5. Run **exit handlers** on cancel/failure (Argo pattern): kill agent sessions, optionally reset worktree — formalizing what `feature_cancel` + finalize cleanup do ad hoc today.

Node state machine (formalizing today's strings, one addition):

```
pending → ready → running → { completed | failed | cancelled | skipped(reason) }
                running → verifying → { completed | failed }
                running → awaiting_gate → { completed | failed | cancelled }
        any-active → interrupted   (watchdog, app restart)
        failed → awaiting_retry → ready   (policy-driven, §5.4)
```

`StepExecution` grows a child table `step_attempts` (attempt_no, status, cost, tokens, wall_clock, error_class, failure_fingerprint, started/ended) so retries stop overwriting history — the UI's per-attempt drill-down (J2) reads this directly. The predecessor-running guard generalizes to: *gate decisions and manual retries are refused while any **ancestor** of the target node is non-terminal* (same invariant, graph-aware).

### 5.4 Reliability: declarative per-class retry policy + durable checkpoints

**Retry policy** (per node, defaulted at workflow level), unifying today's scattered knobs (`on_failure`, `max_iterations`, env one-shot retry, engine default 3, loop_iterations override):

```jsonc
"retry": {
  "environment":   { "strategy": "in_place", "max_attempts": 2, "backoff_secs": 30 },
  "verdict":       { "strategy": "redirect", "to": "implement", "max_attempts": 3, "feedback": true },
  "agent_failure": { "strategy": "in_place", "max_attempts": 2, "feedback": true },
  "non_retryable": { "strategy": "fail" }
}
```

Failure classes map 1:1 onto the existing `StepOutcome`/`VerifierError` taxonomy (`Failed`, `VerdictFailed` with failing_tests/implicated_files, `Environmental`, `NonRetryable`) — the engine already *classifies*; this makes the *response* declarative, per-node, visible in the editor, and consistent between local and remote runs. `feedback: true` = today's `RetryContext` prompt-append behavior. Exhaustion → existing `RetryBudgetExhausted` event + notification.

**Durable checkpoints.** Everything the driver currently keeps in memory moves to the DB:

- `sequence_checkpoints` → persisted per (feature, node) so a crash mid-list resumes from the exact task, not the step (closes the documented re-run-committed-tasks waste).
- `cached_plans` → persisted with the attempt that produced them.
- `env_retried` → derivable from `step_attempts.error_class`, so it's deleted.
- Workspace fingerprint (repo HEAD + dirty flag) recorded at node start; on resume, a mismatch surfaces as the existing synthetic gate rather than blind re-execution (extends Decision 14).

**Idempotency rule:** every side-effecting node records an idempotency key (node id + attempt + workspace fingerprint); `command` nodes must declare `idempotent: true|false` — non-idempotent interrupted commands always go to synthetic gate, never auto-rerun.

**Event log.** `run_events` (exists since V22, used mainly for remote) becomes the **single append-only source of truth for both transports**: every transition, retry decision, gate decision, harness verdict, and cost sample is an event row locally too. The UI consumes one stream shape; Tauri events become live push of the same records the remote path polls. This collapses the current local/remote split (`FeatureDetail` events vs `RunEventTimeline` polling) into one model.

### 5.4b Rework cycles: a verdict re-decomposes, it does not re-run

A `sequence` node executes a task list some other node **produces**
(`task_list_from`, a typed `task_list` edge in v2). When a verdict step behind
the sequence node fails, the redirect target is the **producer**, not the
sequence node.

```jsonc
// s-validate
"retry": { "verdict": { "strategy": "redirect", "to": "s-tickets", "feedback": true } }
```

The producer then renders `rework_prompt_template` instead of
`prompt_template` and writes a *delta* — one ticket per defect the verdict
named, marked `"kind": "rework"`. The sequence node runs that list whole
(no selection: the producer already made it, holding the verdict, the spec
and the diff) against the branch the previous cycle landed, reporting the
earlier cycles as `already_landed` so each fresh agent is told the feature is
already built and its ticket is a fix on top of it.

**Why the producer and not the sequence node.** A sequence node cannot *act*
on a verdict; it runs whatever list it is handed. Redirecting to it means
re-running the list it already ran — the whole feature, re-implemented over
itself. That is what this replaced.

**Which re-entries count** (`domain/rework.rs`): the failing step must be a
strict descendant of the **consumer**, not of the producer. A gate between
the producer and the consumer is downstream of the producer while nothing has
been implemented, so a reviewer rejecting the decomposition there gets a
revised whole list, not a delta. Everything uncertain resolves the same way —
re-running is merely expensive, skipping is wrong.

**Placeholders** available to every agent node, empty on a fresh run:
`{{rework_mode}}` · `{{retry_origin}}` · `{{implicated_files}}` ·
`{{failing_tests}}` · `{{rework_cycle}}`.

**Gate feedback follows the same route.** A reviewer's free text at a ship
gate targets the producer rather than the sequence node whenever the producer
declares a rework template (`resolve_redirect_target` priority 3) — turning
"the empty state looks wrong" into two tickets instead of a full re-run.
Explicitly naming a step, or a declared `on_failure`, still wins.

**Cycle history** lives inside `sequence_plan_cache.plan_json` (`kind`,
`cycle`, `history[]`) rather than a new column, so no migration is needed and
old rows parse unchanged. The drill-down groups the accordion by cycle.

**Lint:** `rework-target-without-template` (warning) fires when a redirect
targets a task-list producer that declares no rework prompt — it will answer
with a whole decomposition, which is strictly worse than not having redirected
there at all.

**Convergence.** A verdict step may return a third value, `environment`, when
the only unsatisfied criteria demand a command the project is not configured
to run. It routes to `VerifierError::Environment` and terminates once with
remediation, instead of opening a rework loop no agent can close.

### 5.5 Visibility

Every question in J2 answerable from data we now persist: node → attempts → per-attempt transcript ref, harness output ref, cost/tokens/cache, failure class, fingerprint, retry decision taken and why (policy rule id). Detailed UI in §6.

### 5.6 Parallelism, scoped safely

`max_parallel_nodes` defaults to 1 (today's behavior; zero new risk at rollout). Raising it is gated by a **write-scope exclusion lint**: two nodes may only be scheduled concurrently if their write scopes cannot collide — i.e. both `ReadOnly`/`Artifacts` capability, or `Implement` on **disjoint repos** (multi-repo features). Two `implement`-capability nodes on the same repo are never concurrent, preserving the parallel-rewrite lesson while still unlocking the valuable cases: research ∥ baseline-tests, critic ∥ docs-draft, per-repo implement lanes. The lint runs at save time (editor warning) and at schedule time (hard invariant). Multi-feature concurrency (Decision 18) is orthogonal and unchanged; the open concurrency-ceiling items in OPEN_QUESTIONS §1 remain open.

## 6. UI/UX redesign

Design language stays: dark neon glassmorphism, Tailwind v4, existing `ui/` kit, lucide icons, `runStatusMeta` tone vocabulary. New dependency: **@xyflow/react (React Flow v12, MIT)** + **elkjs** (layered auto-layout, run in a web worker). No other new libs.

### 6.1 One canvas, two modes

A single `WorkflowCanvas` component renders the graph in two modes:

- **Design mode** (the builder, replaces `WorkflowEditor` form): editable nodes/edges.
- **Run mode** (embedded in `FeatureDetail`, replaces the flat list as the primary visualization): the *pinned version's* graph with live status overlay — running node pulses (reuse the battery-safe opacity-only pulse from the webview perf work), completed nodes show duration+cost chips, failed nodes glow ruby with the failure class, skipped nodes dim with reason tooltip, gates show an amber shield that is the click target for the existing `GateView`.

Shared elements: minimap (auto-hidden under ~8 nodes), fit-view, elk auto-layout button, keyboard nav (arrows between nodes, Enter to open panel). The list timeline **remains available as a toggle** ("Graph | Timeline") — it is better for skimming long sequential runs and preserves muscle memory; both read the same store.

**Which mode a run opens in:** the graph, since the UI redesign's Phase 2 gave both surfaces the same step inspector (`FeatureDetail/StepInspector.tsx`) and the timeline stopped being the only place a step's detail was reachable. The toggle itself is offered only when there is a pinned definition *and* the run has at least one step, so a feature predating workflows opens on the timeline with no toggle at all — a fallback, not a second default.

### 6.2 Run mode: drill-down

Clicking a node opens a right side panel (same split-panel pattern as `ArtifactViewer` today) with tabs:

- **Overview:** status, attempt count, per-attempt table (class, cost, duration, outcome) from `step_attempts`.
- **Live:** the existing `agent_stream` transcript for a running node (moves here from the inline toggle).
- **Output:** artifacts (Monaco viewer), harness output, verifier verdict with failing tests/implicated files.
- **Actions:** Retry (policy-aware: shows which rule will apply), Replay-from-node (existing `replayFromStep`, now graph-aware: highlights the downstream subgraph that will re-run before confirming), Stop node, Decide gate.

Sequence nodes expand in-place (accordion inside the node or panel) to show the task list with per-task status/cost — the landed-prefix is visually distinct from pending tasks, making Decision 13's semantics legible for the first time.

Remote/detached runs render on the same canvas from the same `run_events` stream (§5.4); `RunEventTimeline` survives as the raw event feed inside the panel's Overview tab rather than a separate surface.

### 6.3 Design mode: the builder

- **Palette & add-flow:** drag from palette, or drag from an output handle into empty canvas → filtered "what can connect here" picker (type-compatible node types only). Cmd+K in-canvas node search. Palette content derives from the registry (`config_schema`), so `command` and future types appear automatically.
- **Node cards show their config essence** (agent/model/effort badges, prompt title, capability chip, verifier dot, retry summary like `verdict→implement ×3`) so a graph is scannable without opening panels — the anti-"identical boxes" rule.
- **Config side panel** (never modals) rendered from the node type's JSON Schema; prompt templates open Monaco full-height. Verifier and retry policy are structured sub-forms with sane defaults.
- **Live validation:** connect-time type checking on ports (reject or warn), cycle prevention on connect, node badges for lint findings (missing prompt, unreachable node, dangling redirect target, two same-repo implement nodes marked parallel, no finalize sink). **Save is blocked only by errors, not warnings.** This kills audit F39's dangling-`on_failure` class of bugs by construction.
- **Dirty-state guard** (audit F38): navigation/Escape prompts to save/discard; local draft autosave every 30s to survive crashes.
- **Undo/redo** on all graph edits (table stakes).
- **Versioning UI:** every save = new `WorkflowVersion` (unchanged); new version-history drawer with list, structural diff (added/removed/changed nodes highlighted on the canvas), restore-as-new-version, and revert-to-default for starters. The existing `workflow_versions` command finally gets a UI (audit F39).
- **Templates:** "New workflow" starts from a starter clone or from three shapes (blank / plan-implement-validate / plan-gate-implement-validate-gate).
- **Import/export:** existing JSON export/import continues, now schema-v2 with positions included.
- **Launch integration:** `StartFeatureModal`'s per-step override list gains a mini-graph preview; Kanban cards (Epic C1) pin `(workflow_id, version)` and deep-link into run mode.

### 6.4 Explicitly kept from today

Gate full-screen takeover flow, predecessor (now ancestor) guard behavior with disabled-button explanations, start-feature entry points, cost/token telemetry chips, `BootstrapStepper` for phase 0, notification bell taxonomy.

## 7. Standard workflow migration

All seven starters migrate mechanically (chain edges). Two get genuinely better shapes in the same release, proving the DAG earns its keep:

- **Standard Feature Pipeline:** `research ∥ baseline-harness(command)` fan-in to `tickets`; `validate` and `critic` run as parallel verify branches fan-in to `gate-ship` (both ReadOnly/Verify — safe under §5.6); `finalize` unchanged.
- **Refactor Pipeline:** `baseline(command)` becomes a zero-token command node; `regression ∥ api-drift-review` fan-in to `gate-diff`.

The other five ship as straight chains (a chain is a valid DAG); their behavior must be bit-identical pre/post migration, which is the core migration acceptance test.

## 8. Phasing

**Phases 1–3 shipped** 2026-07-24 → 2026-07-26: schema v2 + auto-migration,
`NodeTypeRegistry` with all five handlers re-homed and the dispatch `match`
deleted, ready-set scheduler, `step_attempts`, durable checkpoints, declarative
retry policy, unified `run_events`, the two-mode `WorkflowCanvas` (run overlay
+ drill-down panel + design-mode builder with lint/undo/version history), and
the `command` node type as the registry seam proof.

**Phase 4 — DAG payoff — remains:** `max_parallel_nodes>1` behind write-scope
exclusion lint; parallel shapes for the Standard Feature + Refactor starters;
conditional edges (`when`) exposed in the builder; `subworkflow` node. Task
breakdown in [`TASKS_DAG_WORKFLOWS.md`](TASKS_DAG_WORKFLOWS.md).

## 9. Success metrics

- **Reliability:** 0 lost-work incidents (re-running committed tasks) after restart, measured by resume-from-checkpoint telemetry; 100% of failures carry a failure class + applied policy rule in `run_events`.
- **Visibility:** time-to-root-cause on seeded failures ≤3 clicks / <30s; single event stream powering both transports (delete the FeatureDetail/RunEventTimeline split-path code, target ≥1k LOC net removal alongside the dead parallel UI F36).
- **Authorability:** new-workflow completion <10 min unaided; invalid saves = structurally impossible; editor-defect audit findings F38/F39 closed.
- **Extensibility:** `command` node type lands with **zero** scheduler-file edits (registry-only diff) — the proof the seam works.
- **Adoption:** ≥50% of new custom workflows use at least one non-linear construct (branch, parallel verify pair, command node) within a month of Phase 4.

## 10. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Engine rewrite destabilizes the hard-won reliability behavior | Phase 1 changes representation, not semantics; existing integration + conformance suites are the gate; starters must run bit-identical |
| Graph UI is worse than the list for simple linear runs | Keep the timeline toggle permanently. Parity was signed off with the UI redesign's Phase 2 — both surfaces now drive one step inspector — so the graph is the default where there is one to draw; a feature with no pinned definition still opens on the timeline |
| Parallel nodes resurrect the merge-storm problem | Write-scope exclusion is a scheduler invariant, not a convention; same-repo implement concurrency is impossible by construction |
| React Flow bundle/perf cost in a battery-sensitive webview | Graphs are tens of nodes; virtualization unnecessary; reuse opacity-only animation rules from the webview perf work; elk in a worker |
| Schema v2 breaks exported/community workflows | v1 import auto-migrates forever; JSON Schema published in docs-site |
| Expression language scope creep | Ship only `nodes.<id>.outputs.<name>` + equality/comparison; anything more requires a new decision record |

## 11. Open questions — all resolved

The five questions this PRD opened were answered as decision records in P0.1
and are locked in [`DECISIONS.md`](DECISIONS.md):

| Q | Resolved by | Answer |
|---|---|---|
| 1 — Pin the workflow version on `Feature`? | Decision 38 | Yes — `features.workflow_version_id` (V33) |
| 2 — Join semantics for multi-branch gates | Decision 39 | `all_success`; `PASS_WITH_NOTES` maps to success |
| 3 — Fate of `conflict_policy` | Decision 40 | Becomes a `sync`-node config field |
| 4 — Where cron scheduling lives | Decision 41 | Stays a workflow-level sibling of `nodes`/`edges` |
| 5 — Monaco source view | Decision 42 | Read-only source tab; editable binding still deferred |

---

## Appendix A — Pattern sources (research digest)

- **Static declarative DAG + bounded dynamic fan-out:** Argo Workflows (`dependencies`, `withParam`, suspend templates, exit handlers); GitHub Actions (`needs`, `if`, `${{ }}` expressions).
- **Declarative per-class retry policy attached to steps, not workflows:** Temporal RetryPolicy, adapted with agent-specific failure classes (validation-failed → feedback retry).
- **Embedded durable execution over SQLite (checkpoint-at-step-boundary, no deterministic replay):** DBOS Transact model; Morling's SQLite durable-execution engine (execution-log table, WAITING_FOR_SIGNAL gates, idempotency keys); Obelisk (SQLite for AI-agent orchestration).
- **Run-definition pinning, never migrate in-flight:** Temporal worker-versioning, simplified for single-user.
- **Builder UX:** n8n (drag-from-handle picker, JSON nodes+connections+positions format, per-node typeVersion), Dify (per-node run inspection as the debugging gold standard), LangGraph Studio (re-run-from-node with edited inputs — our Replay-from-node), Node-RED (spaghetti mitigation), React Flow + elkjs as the de-facto stack.
- **Typed ports, coarse-grained:** Dagster's edit-time validation win without a full type system; `any` escape hatch.

Full source list with URLs: see research digest in the PR description accompanying this PRD.

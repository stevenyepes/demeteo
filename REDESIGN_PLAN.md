# Demeteo Redesign Plan: Multi-Agent Orchestrator

> **Status:** Locked design. Source of truth for the multi-agent orchestrator
> pivot. 33 decisions, 7 bounded contexts, one phase plan. Anything not in
> this doc is by definition not v1 scope.

## 0. Pivot Summary

Demeteo stops being a chat-style supervisor for one coding agent and becomes
a fleet-style orchestrator that takes a feature goal, decomposes it through
a workflow, delegates work to coding agents, and keeps the user in the loop
at explicit gates. The chat UX is gone; the orchestration plane is the
product. LLM work is delegated to a coding agent ("the planner") running on
the project's host — demeteo itself never holds API keys, never talks to a
model provider, and never competes with the agent's own UIs.

The vocabulary:
- **Workflow** — a reusable, versioned template that defines how a feature is executed.
- **Project** — the top-level container: a host machine (local or remote SSH), a set of repositories, a planner assignment.
- **Feature** — a running instance of a Workflow on a Project. The unit the user starts, watches, and approves.
- **Step** — a node in a Workflow. Three types in v1: `agent`, `parallel`, `gate`.
- **Subtask** — a unit of work inside a `parallel` step. One (host, agent) pair on a worktree.
- **Provider instance** — a (kind, host) tuple for GitHub/GitLab (and self-hosted variants). Holds the PAT for clone + publish.

## 1. Locked Decisions (the table)

| #  | Decision                           | Locked answer                                                                  |
|----|------------------------------------|--------------------------------------------------------------------------------|
| 1  | Top-level entity shape             | Project → Feature (Mission → Subtask DAG)                                      |
| 2  | Demeteo's role                     | Orchestrator, not chat client — drop the supervisor plane                      |
| 3  | Brain role                         | Advisor; declarative, embedded in workflow steps                               |
| 4  | LLM provider scope                 | Delegate to a coding agent acting as planner (no demeteo-side LLM)             |
| 5  | Planner selection                  | Per-project planner `(machine_id, agent_kind)`                                 |
| 6  | Project structure                  | One host per project (local or remote SSH); repos cloned via PAT               |
| 7  | Workflows as templates             | First-class, versioned, importable; starter pack shipped in binary              |
| 8  | Step execution model               | Typed: `agent` / `parallel` / `gate`; `command` deferred                        |
| 9  | Context propagation                | Artifact pointer (C) + planner-summary fallback for chat-shaped (B)             |
| 10 | Workflow versioning                | Local + versioned + importable, JSON format, starter pack in binary             |
| 11 | Project bootstrap depth            | Clone + detect (B) + propose worktree strategy (C); no repo writes (D deferred)|
| 12 | Gate UX                            | Planner summary card + artifact/diff list + Approve/Redirect/Cancel            |
| 13 | `parallel` failure semantics       | Continue-and-report (D) + opt-in retry with cost cap (C layered)               |
| 14 | Workflow re-entry / resume         | Per-step checkpoints; synthetic gate on mid-step interrupt                     |
| 15 | Workflow telemetry                 | Per-step cost + duration; **no pre-launch cost estimate**                      |
| 16 | Repo merge model                   | `feature/<slug>` branch from canonical; subtasks merge into it; optional MR    |
| 17 | PAT scope                          | Per-provider global, keyed by `(kind, host)` for multi-instance support        |
| 18 | Multi-feature concurrency          | Strict serial (A) — one feature per project                                    |
| 19 | Workflow authoring UX              | Form-first (v1.0); YAML view (v1.1); "save run as template" (v1.2)             |
| 20 | Conflict resolution UX             | Smart cascade: auto-agent → manual (Monaco 3-way) → skip/abort                 |
| 21 | Project overview                   | Current feature + queue + lazy-loaded repo map                                 |
| 22 | "Start a feature" entry point      | Slim modal with description + inferred chips; "Customize…" expands              |
| 23 | Workflow pre-flight                | Static: step list + risks + repo fit (no cost)                                 |
| 24 | Cross-project navigation           | Left rail, main pane = current project; command palette for power users        |
| 25 | "Describe a feature" inference     | Repo chips + conflict detection, local keyword matching (no LLM in modal)      |
| 26 | Completed feature lifecycle        | Archive by default; per-project `keep`/`archive`/`auto_delete` setting         |
| 27 | First-run UX                       | State-driven empty card; "Try a sample project" with real LLM-backed run       |
| 28 | Step output conventions            | Type-driven artifacts; `full`/`summary_only`/`none` per workflow               |
| 29 | Settings surface                   | Global Preferences + per-project settings + command palette                     |
| 30 | Update / migration                 | Greenfield v1; wipe-and-reinit on breaking, silent on additive                 |
| 31 | Telemetry                          | None in v1                                                                     |
| 32 | Keyboard shortcuts                 | Standard desktop set; command palette for discoverability                      |
| 33 | Docs                               | Bundled markdown in binary; no separate strategy                                |

## 2. New Bounded Contexts

1. **Identity & Fleet** — global app settings, provider instances, machine registry.
2. **Project Management** — projects, repositories, project bootstrap, worktree strategy.
3. **Workflow Catalog** — workflow templates, versions, starter pack, import/export.
4. **Feature Orchestration** — features, runs, step executions, gate decisions, retry state.
5. **Worktree & Git** — per-feature branch, per-subtask worktrees, merge into feature branch, publish MR.
6. **Agent Runtime** — `CliRuntime` (one-shot CLI + JSON-lines); `AcpRuntime`, `JsonRpcClient`, `ToolBridge`, and both transports deleted. `PermissionPolicyPort` + `WorktreeScopedPolicy` renders `OPENCODE_PERMISSION` env var. `AgentRegistry` simplified (no session dedup needed; `Arc<AgentSession>` lives for one `prompt` call). See [`AGENT_INTEGRATION.md`](AGENT_INTEGRATION.md) for the full spec.
7. **UI & Telemetry** — UI state, disk usage, migration log, command palette, docs.

Full entities, value objects, and aggregates per context: [`docs/REDESIGN_DDD_MODEL.md`](docs/REDESIGN_DDD_MODEL.md).

## 3. New Architecture

Hexagonal layout preserved. New ports:
- `WorkflowRepository` (CRUD + version retrieval)
- `ProjectRepository` (CRUD + repo map + bootstrap state)
- `FeatureOrchestrator` (start, pause, resume, cancel; gate decisions; retry policy)
- `StepExecutor` (the small DAG executor: agent / parallel / gate; conditional edges; max iterations)
- `WorktreeManager` (per-feature branch creation, per-subtask worktree provisioning, merge into feature branch, publish MR)
- `ProviderInstanceRepository` (CRUD for `(kind, host, encrypted_pat)`; validate-on-connect)
- `ArtifactStore` (read/write markdown + JSON under `features/<id>/artifacts/`)
- `PricingTable` (model → cost; hard-coded in v1, editable in Preferences)

Carried from v1 with simplifications:
- `DatabasePort` — now with the new tables, loses `thread_sessions` complexity.
- `AgentRuntime` — `CliRuntime` replaces `AcpRuntime`; one-shot CLI invocation per step, JSON-lines event stream, no JSON-RPC, no tool-call bridge, no capability negotiation, no 5-minute `session/new` timeout.
- `ExecutionPort` — `spawn_interactive` is now used only for remote agents (local agents use `tokio::process::Command` directly).
- `NotificationPort` — fewer events; no per-turn `Text`/`Usage`/`Plan` streams (telemetry events only).

Removed:
- `thread_sessions` (replaced by `features` + `feature_runs` + `step_executions`)
- `thread_working_memory` (no chat; no need)
- `InterceptPayload.tool_call_id` plumbing at the UI boundary (UI is no longer rendering the stream)

Full port surface and adapter layout: [`docs/REDESIGN_ARCHITECTURE.md`](docs/REDESIGN_ARCHITECTURE.md).

## 4. Phase Plan (high-level)

Each phase is "Done means…". Sequential. Don't start the next until the current is verified.

### Phase R0 — Domain & docs (this commit series)
- The six `docs/REDESIGN_*.md` files exist.
- `AGENTS.md`, `AGENT_INTEGRATION.md`, `ARCHITECTURE.md`, `DDD_MODEL.md`, `EXECUTION_PLAN.md` are either updated or archived as `docs/LEGACY_*.md`.
- The 33-decision table is the single source of truth, referenced by all other docs.

### Phase R1 — Greenfield schema & ports (Rust)
- New `db.rs` schema: `projects`, `repositories`, `provider_instances`, `workflows`, `workflow_versions`, `features`, `feature_runs`, `step_executions`, `gate_decisions`, `subtask_runs`, `subtask_merges`, `project_settings`, `app_settings`.
- New ports: `WorkflowRepository`, `ProjectRepository`, `FeatureOrchestrator`, `StepExecutor`, `WorktreeManager`, `ProviderInstanceRepository`, `ArtifactStore`, `PricingTable`.
- No `cargo build` regressions on legacy code (we keep the legacy `db.rs` paths compiling until R3).
- Tests: per-table CRUD + per-port contract tests with an in-memory SQLite fixture.

### Phase R2 — Project bootstrap & provider wiring (Rust + minimal UI)
- Project create/edit UI (slim form: name, type, repos, default workflow, planner).
- Clone repos via provider instance PAT.
- Detect default branch, PR template, CI config; record into project metadata.
- Propose worktree strategy; user approves/edits in the project settings.
- Validate provider instance on connect (`/user` for GitHub, `/api/v4/user` for GitLab).
- Done: a project can be created from a list of repo URLs, repos are cloned, the bootstrap detection runs, the user sees the proposed worktree strategy.

### Phase R3 — Workflow catalog & authoring (Rust + UI)
- Workflow CRUD (form editor v1.0).
- Versioning (`workflow_versions` table).
- Import/export (JSON).
- Starter pack bundled in the binary (`workflows/` directory at build time).
- "New workflow" / "Edit workflow" / "Save version" UIs.
- Done: a user can author a workflow, save versions, export JSON, import JSON, and use the starter pack.

### Phase R4 — Step executor (Rust)
- The `StepExecutor` port + concrete impl.
- `agent` step: spawn an ACP session, drive it to completion, capture output as artifact.
- `parallel` step: planner agent produces subtask DAG; fan out across available workers; structured result.
- `gate` step: pause, render summary, capture user decision.
- Conditional edges: `on_failure → goto`, `max_iterations`.
- Done: a feature can run a 5-step workflow (research → spec → plan → tasks → implement-stub) end-to-end with deterministic step transitions and gate pauses.

### Phase R5 — Feature orchestrator (Rust + UI)
- The `FeatureOrchestrator` port + impl.
- "Start feature" slim modal with description textarea + inferred chips.
- Custom expand to full form (workflow picker, target repos, conflict policy, budget).
- Per-step checkpoint persistence.
- Re-entry on launch (synthetic gate on mid-step interrupt).
- Cost/duration telemetry (per-step, per-feature).
- Done: a feature can be started, watched in the project home, paused, resumed, completed; the user sees cost/duration per step.

### Phase R6 — Worktree & merge (Rust)
- Per-feature `feature/<slug>` branch creation off canonical.
- Per-subtask worktree provisioning.
- Sequential merge: subtask branches merge into `feature/<slug>` in DAG order.
- Conflict detection; conflict resolution cascade (auto-agent → manual → skip/abort).
- Optional `publish` step → open MR via provider instance.
- Done: a `parallel` step's subtasks land in `feature/<slug>` via the engine; conflicts surface at a gate; `publish` opens an MR if the user configured it.

### Phase R7 — UX polish & docs (UI + content)
- Project overview wired.
- Cross-project navigation wired.
- Settings & preferences wired.
- First-run UX wired.
- Docs panel (`?` icon) populated with the markdown files in `src/docs/`.
- Keyboard shortcuts wired.
- Empty states, sample project, command palette.
- Done: the app is usable end-to-end by a new user with no prior context.

### Phase R8 — Hardening & migration (v1.0 → v1.x)
- Additive schema migrations (silent).
- Wipe-and-reinit flow for breaking changes (gated, with JSON export of workflows + projects before wipe).
- Pre-migration backup (`demeteo.db.bak.<timestamp>`, 7-day retention).
- Migration log (`~/.local/share/demeteo/migrations.log`).
- Done: the app can ship v1.1 with additive schema changes silently, and v2.0 with a clean wipe-and-reinit flow.

Detailed file touch list and verification checkpoints per phase:
[`docs/REDESIGN_EXECUTION_PLAN.md`](docs/REDESIGN_EXECUTION_PLAN.md).

Reliability hardening (cancel-vs-future, conditional edges,
mid-step interrupt, SSH stale-session reconnect, etc.) is tracked in
[`docs/REDESIGN_RELIABILITY_PLAN.md`](docs/REDESIGN_RELIABILITY_PLAN.md)
and is worked in parallel with R4–R6.

## 5. Open / Deferred Questions

Full list with rationale: [`docs/REDESIGN_OPEN_QUESTIONS.md`](docs/REDESIGN_OPEN_QUESTIONS.md). Headline items:
- Multi-feature concurrency (deferred — strict serial is the v1 truth).
- Workflow YAML view (v1.1).
- "Save this run as a workflow template" (v1.2).
- Deep dry-run (v1.x).
- Cost rollup dashboard (v1.x).
- Smart project home with activity feed (deferred).
- Project tabs / split view / activity feed home (deferred).
- `command` step type (v1.1).
- WASM plugin host (deferred).
- Per-machine `AgentConfig` (model, workdir, env) (deferred).
- Telemetry (v3-or-later).
- Auto-update (v2-or-later).
- WASM provider plugins (v2-or-later).
- Second non-ACP runtime (Anthropic-first; v1.1).
- Self-hosted provider instance key rotation (v1.x).

## 6. What This Plan Replaces

The following docs are **archived** (moved to `docs/LEGACY_*.md`) rather than deleted:
- `docs/LEGACY_ARCHITECTURE.md` (was `ARCHITECTURE.md`) — describes the single-agent hexagon.
- `docs/LEGACY_DDD_MODEL.md` (was `DDD_MODEL.md`) — describes the legacy bounded contexts (Thread, Machine, AgentProfile, etc.).
- `docs/LEGACY_EXECUTION_PLAN.md` (was `EXECUTION_PLAN.md`) — describes Phase 0–5.

`AGENT_INTEGRATION.md` is **rewritten in place**, not archived, because the `CliRuntime` spec is the source of truth for the agent runtime that drives both planner and subtask sessions — the rewrite replaces the `AcpRuntime` spec with the `CliRuntime` spec and removes all ACP-specific details.

The current `AGENTS.md` is updated (not replaced) to:
- Add a "Redesign" section pointing to the new docs.
- Mark Phase 1–6 as "Legacy single-agent work; preserved in git history."
- Reference the new 9-phase plan (R0–R8) as the active plan.
- Add a one-paragraph "pivot summary" so anyone reading the repo for the first time understands the product direction without reading all 33 decisions.

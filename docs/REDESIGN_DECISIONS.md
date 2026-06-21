# Demeteo Redesign: Locked Decisions Reference

> **Standalone reference for the 33 locked design decisions** that emerged
> from the multi-agent orchestrator redesign interview. This is the same
> table that lives in [`REDESIGN_PLAN.md`](../REDESIGN_PLAN.md) §1, rendered
> here for easy linking from other docs. If the two ever disagree, this
> doc and the master plan should be considered the source of truth; flag
> the conflict and re-align.

## 1. The 33 Decisions

| #  | Decision                           | Locked answer                                                                  | Source           |
|----|------------------------------------|--------------------------------------------------------------------------------|------------------|
| 1  | Top-level entity shape             | Project → Feature (Mission → Subtask DAG)                                      | Interview Q1     |
| 2  | Demeteo's role                     | Orchestrator, not chat client — drop the supervisor plane                      | Interview        |
| 3  | Brain role                         | Advisor; declarative, embedded in workflow steps                               | Interview Q3     |
| 4  | LLM provider scope                 | Delegate to a coding agent acting as planner (no demeteo-side LLM)             | Interview Q4/Q5  |
| 5  | Planner selection                  | Per-project planner `(machine_id, agent_kind)`                                 | Interview Q6     |
| 6  | Project structure                  | One host per project (local or remote SSH); repos cloned via PAT               | Interview Q7/C   |
| 7  | Workflows as templates             | First-class, versioned, importable; starter pack shipped in binary              | Interview Q8     |
| 8  | Step execution model               | Typed: `agent` / `parallel` / `gate`; `command` deferred                        | Interview Q8     |
| 9  | Context propagation                | Artifact pointer (C) + planner-summary fallback for chat-shaped (B)             | Interview Q10    |
| 10 | Workflow versioning                | Local + versioned + importable, JSON format, starter pack in binary             | Interview Q11    |
| 11 | Project bootstrap depth            | Clone + detect (B) + propose worktree strategy (C); no repo writes (D deferred)| Interview Q12    |
| 12 | Gate UX                            | Planner summary card + artifact/diff list + Approve/Redirect/Cancel            | Interview Q13    |
| 13 | `parallel` failure semantics       | Continue-and-report (D) + opt-in retry with cost cap (C layered)               | Interview Q14    |
| 14 | Workflow re-entry / resume         | Per-step checkpoints; synthetic gate on mid-step interrupt                     | Interview Q15    |
| 15 | Workflow telemetry                 | Per-step cost + duration; **no pre-launch cost estimate**                      | Interview Q16    |
| 16 | Repo merge model                   | `feature/<slug>` branch from canonical; subtasks merge into it; optional MR    | Interview Q17    |
| 17 | PAT scope                          | Per-provider global, keyed by `(kind, host)` for multi-instance support        | Interview Q17a   |
| 18 | Multi-feature concurrency          | Strict serial (A) — one feature per project                                    | Interview Q18    |
| 19 | Workflow authoring UX              | Form-first (v1.0); YAML view (v1.1); "save run as template" (v1.2)             | Interview Q19    |
| 20 | Conflict resolution UX             | Smart cascade: auto-agent → manual (Monaco 3-way) → skip/abort                 | Interview Q20    |
| 21 | Project overview                   | Current feature + queue + lazy-loaded repo map                                 | Interview Q21    |
| 22 | "Start a feature" entry point      | Slim modal with description + inferred chips; "Customize…" expands              | Interview Q22    |
| 23 | Workflow pre-flight                | Static: step list + risks + repo fit (no cost)                                 | Interview Q23    |
| 24 | Cross-project navigation           | Left rail, main pane = current project; command palette for power users        | Interview Q24    |
| 25 | "Describe a feature" inference     | Repo chips + conflict detection, local keyword matching (no LLM in modal)      | Interview Q25    |
| 26 | Completed feature lifecycle        | Archive by default; per-project `keep`/`archive`/`auto_delete` setting         | Interview Q26    |
| 27 | First-run UX                       | State-driven empty card; "Try a sample project" with real LLM-backed run       | Interview Q27    |
| 28 | Step output conventions            | Type-driven artifacts; `full`/`summary_only`/`none` per workflow               | Interview Q28    |
| 29 | Settings surface                   | Global Preferences + per-project settings + command palette                     | Interview Q29    |
| 30 | Update / migration                 | Greenfield v1; wipe-and-reinit on breaking, silent on additive                 | User pivot       |
| 31 | Telemetry                          | None in v1                                                                     | User pivot       |
| 32 | Keyboard shortcuts                 | Standard desktop set; command palette for discoverability                      | Interview Q32    |
| 33 | Docs                               | Bundled markdown in binary; no separate strategy                                | Interview Q33   |
| 34 | Agent protocol                     | `CliRuntime` (one-shot CLI + JSON-lines); ACP removed — no JSON-RPC, no tool-call bridge, no capability negotiation. `opencode run --format json` for opencode/hermes; `--output-format stream-json` for claude-code; `agy --print -` for antigravity. | 2026-06-19   |
| 35 | Agent permission enforcement       | `OPENCODE_PERMISSION` env var per spawn; `external_directory: "deny"` to scope the worktree; `PermissionPolicyPort` renders the policy. Gate-step approval is the only real-time human-in-the-loop surface. | 2026-06-19   |
| 36 | Cross-step session continuity      | `--session <uuid> --continue` flag per `opencode run` invocation so a multi-step workflow shares conversation context. Parallel subtasks each get their own session id. | 2026-06-19   |

## 2. Cross-References

- **Domain model** (entities, value objects, aggregates, ports): [`REDESIGN_DDD_MODEL.md`](REDESIGN_DDD_MODEL.md)
- **Architecture** (hexagon, port surface, file layout, Tauri commands, frontend state): [`REDESIGN_ARCHITECTURE.md`](REDESIGN_ARCHITECTURE.md)
- **Execution plan** (phase breakdown, file touch list, verification): [`REDESIGN_EXECUTION_PLAN.md`](REDESIGN_EXECUTION_PLAN.md)
- **Open / deferred questions**: [`REDESIGN_OPEN_QUESTIONS.md`](REDESIGN_OPEN_QUESTIONS.md)
- **Master plan** (pivot summary, this table, phase plan summary): [`REDESIGN_PLAN.md`](../REDESIGN_PLAN.md)

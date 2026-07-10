# Demeteo: Locked Decisions Reference

> **Standalone reference for the 36 locked design decisions** that emerged
> from the multi-agent orchestrator design. This is the same
> table that guides the project. If any conflicts ever arise, this
> doc should be considered a source of truth; flag the conflict and re-align.

## 1. The 36 Decisions

| #  | Decision                           | Locked answer                                                                  | Source           |
|----|------------------------------------|--------------------------------------------------------------------------------|------------------|
| 1  | Top-level entity shape             | Project → Feature (Mission → Subtask DAG)                                      | Interview Q1     |
| 2  | Demeteo's role                     | Orchestrator, not chat client — drop the supervisor plane                      | Interview        |
| 3  | Brain role                         | Advisor; declarative, embedded in workflow steps                               | Interview Q3     |
| 4  | LLM provider scope                 | Delegate to a coding agent acting as planner for *runs*. **Exception:** the opt-in **Memory Agent** (`adapters/memory_llm.rs`, `adapters/memory_worker.rs`) calls a user-configured OpenAI-compatible endpoint directly, in the background, only to distill run signals into project memories. It never drives a feature run; it is disabled by default and its API key lives in the OS keyring. | Interview Q4/Q5  |
| 5  | Planner selection                  | Per-project planner via `ProjectSettings::default_agent_kind` + `default_model`; overrideable per-workflow (`ProjectWorkflowOverride` with `step_id = None`) and per-step (`step_id = Some(...)`); loses to a run-time override chosen in `StartFeatureModal`. | Interview Q6     |
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
| 20 | Conflict resolution UX             | Smart cascade: auto-agent → manual → skip/abort; no dedicated Monaco 3-way UI component (conflict resolution reuses `GateView` plus `feature_resolve_sync_conflicts` to spawn a resolution agent and revalidate the step). | Interview Q20    |
| 21 | Project overview                   | Current feature + queue + lazy-loaded repo map                                 | Interview Q21    |
| 22 | "Start a feature" entry point      | Slim modal with description + inferred chips; "Customize…" expands              | Interview Q22    |
| 23 | Workflow pre-flight                | Static: step list + risks + repo fit (no cost)                                 | Interview Q23    |
| 24 | Cross-project navigation           | Left rail, main pane = current project; command palette for power users        | Interview Q24    |
| 25 | "Describe a feature" inference     | Repo chips + conflict detection, local keyword matching (no LLM in modal)      | Interview Q25    |
| 26 | Completed feature lifecycle        | Archive by default; per-project `keep`/`archive`/`auto_delete` setting         | Interview Q26    |
| 27 | First-run UX                       | State-driven empty card; "Try a sample project" with real LLM-backed run       | Interview Q27    |
| 28 | Step output conventions            | Type-driven artifacts; `full`/`summary_only`/`none` per workflow               | Interview Q28    |
| 29 | Settings surface                   | Global Preferences + per-project settings + command palette                     | Interview Q29    |
| 30 | Update / migration                 | `refinery`-based; additive migrations apply silently; breaking changes prompt wipe-and-reinit; pre-migration backup with 7-day retention; migration log next to `demeteo.db`. The schema is at V19+; v1 is no longer greenfield. | User pivot       |
| 31 | Telemetry                          | None in v1                                                                     | User pivot       |
| 32 | Keyboard shortcuts                 | Standard desktop set; command palette for discoverability                      | Interview Q32    |
| 33 | Docs                               | Bundled markdown in binary; no separate strategy                                | Interview Q33   |
| 34 | Agent protocol                     | `UnifiedCliRuntime` (one-shot CLI + JSON-lines); ACP removed — no JSON-RPC, no tool-call bridge, no capability negotiation. `opencode run --format json` for opencode; `hermes run --format json` for hermes; `claude --print --verbose --output-format stream-json` for claude-code. Install commands: opencode = `curl -fsSL https://opencode.ai/install \| bash`; hermes = `curl -fsSL https://hermes-agent.nousresearch.com/install.sh \| bash`; claude-code = `npm install -g @anthropic-ai/claude-code`. | 2026-06-19   |
| 35 | Agent permission enforcement       | Each `StepCapability` compiles to a four-axis `PermissionProfile` (`read_fs`, `write_fs`, `execute`, `network`, each `Allow` or `Deny`) plus a path-shaped `WriteScope` (`None` \| `ArtifactsOnly` \| `All`). The compiled policy only ever uses `allow` / `deny`, never `ask`. The abstract profile is translated to the agent's native dialect at spawn: opencode / hermes → `OPENCODE_PERMISSION` env (`{"edit":…,"read":…,"bash":…,"webfetch":…,"websearch":…,"external_directory":"deny","doom_loop":"allow"}`); claude-code → `--disallowedTools` (`Bash` / `Edit` / `Write` / `MultiEdit` / `NotebookEdit` / `WebSearch` / `WebFetch` as applicable) + `--exclude-dynamic-system-prompt-sections` + `--setting-sources user,project` + `--strict-mcp-config` for prompt-cache determinism. The `artifacts/` vs source path-shape is enforced uniformly by the OS-level chmod fence in `adapters/worktree/git_ops/scope.rs`. Gate-step approval is the only real-time human-in-the-loop surface. | 2026-06-19   |
| 36 | Cross-step session continuity      | One captured `session_id` per feature; threaded through every subsequent agent invocation. opencode: `--session <uuid> --continue` (`adapters/agent/opencode/mod.rs:404-408`). hermes: `--resume <sid>` (`adapters/agent/hermes/mod.rs:155-156`). claude-code: `--resume <sid>` (`adapters/agent/claude_code/mod.rs:388-394`), plus `--exclude-dynamic-system-prompt-sections` / `--setting-sources user,project` / `--strict-mcp-config` for byte-identical prompt-cache prefix. Parallel subtasks each get their own session id so they don't pollute each other's context. On context-window saturation (>80% of budget from `PricingTable::context_window`) the driver's watchdog kills the session and the next step's spawn injects a one-shot recap. | 2026-06-19   |

## 2. Cross-References

- **Domain model** (entities, value objects, aggregates, ports): [`DDD_MODEL.md`](DDD_MODEL.md)
- **Architecture** (hexagon, port surface, file layout, Tauri commands, frontend state): [`ARCHITECTURE.md`](ARCHITECTURE.md)
- **Open / deferred questions**: [`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md)
- **Reliability plan**: [`RELIABILITY_PLAN.md`](RELIABILITY_PLAN.md)
- **Agent runtime spec**: [`AGENT_INTEGRATION.md`](../AGENT_INTEGRATION.md)
- **Known platform issues**: [`KNOWN_ISSUES.md`](KNOWN_ISSUES.md)
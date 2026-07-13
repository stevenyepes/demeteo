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
| 18 | Multi-feature concurrency          | **Concurrent — N features per project.** Features on one project run at the same time, each on its own `feature/<slug>` branch and its own feature-scoped worktree. ⚠️ **Supersedes the original "strict serial (A)" answer** — see [§2](#2-superseded-decisions). | 2026-07-12 (was Interview Q18) |
| 19 | Workflow authoring UX              | Form-first (v1.0); YAML view (v1.1); "save run as template" (v1.2)             | Interview Q19    |
| 20 | Conflict resolution UX             | Smart cascade: auto-agent → manual → skip/abort; no dedicated Monaco 3-way UI component (conflict resolution reuses `GateView` plus `feature_resolve_sync_conflicts` to spawn a resolution agent and revalidate the step). | Interview Q20    |
| 21 | Project overview                   | Running features (plural) + queue + lazy-loaded repo map. Revised with [decision 18](#2-superseded-decisions): there is no single "current feature" slot, because a project may have several features in flight. | 2026-07-12 (was Interview Q21) |
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
| 34 | Agent protocol                     | `UnifiedCliRuntime` (one-shot CLI + JSON-lines); ACP removed — no JSON-RPC, no tool-call bridge, no capability negotiation. `opencode run --format json` for opencode; `hermes run --format json` for hermes; `claude --print --verbose --output-format stream-json` for claude-code; `codex exec --json` for codex. Install commands: opencode = `curl -fsSL https://opencode.ai/install \| bash`; hermes = `curl -fsSL https://hermes-agent.nousresearch.com/install.sh \| bash`; claude-code = `npm install -g @anthropic-ai/claude-code`; codex = `npm install -g @openai/codex`. | 2026-06-19   |
| 35 | Agent permission enforcement       | Each `StepCapability` compiles to a four-axis `PermissionProfile` (`read_fs`, `write_fs`, `execute`, `network`, each `Allow` or `Deny`) plus a path-shaped `WriteScope` (`None` \| `ArtifactsOnly` \| `All`). The compiled policy only ever uses `allow` / `deny`, never `ask`. The abstract profile is translated to the agent's native dialect at spawn: opencode / hermes → `OPENCODE_PERMISSION` env (`{"edit":…,"read":…,"bash":…,"webfetch":…,"websearch":…,"external_directory":"deny","doom_loop":"allow"}`); claude-code → `--disallowedTools` (`Bash` / `Edit` / `Write` / `MultiEdit` / `NotebookEdit` / `WebSearch` / `WebFetch` as applicable) + `--exclude-dynamic-system-prompt-sections` + `--setting-sources user,project` + `--strict-mcp-config` for prompt-cache determinism. The `artifacts/` vs source path-shape is enforced uniformly by the OS-level chmod fence in `adapters/worktree/git_ops/scope.rs`. Gate-step approval is the only real-time human-in-the-loop surface. | 2026-06-19   |
| 36 | Cross-step session continuity      | One captured `session_id` per feature; threaded through every subsequent agent invocation. opencode: `--session <uuid> --continue` (`adapters/agent/opencode/mod.rs:404-408`). hermes: `--resume <sid>` (`adapters/agent/hermes/mod.rs:155-156`). claude-code: `--resume <sid>` (`adapters/agent/claude_code/mod.rs:388-394`), plus `--exclude-dynamic-system-prompt-sections` / `--setting-sources user,project` / `--strict-mcp-config` for byte-identical prompt-cache prefix. Parallel subtasks each get their own session id so they don't pollute each other's context. On context-window saturation (>80% of budget from `PricingTable::context_window`) the driver's watchdog kills the session and the next step's spawn injects a one-shot recap. | 2026-06-19   |

## 2. Superseded decisions

A decision you silently overwrite stops being a decision *record*. When a
locked answer changes, the row above is updated **and** the original is kept
here with the reason it moved, so the next reader can tell "we thought hard and
changed our minds" from "nobody ever considered this".

### 18 — Multi-feature concurrency

| | |
|---|---|
| **Was** | Strict serial (A) — one feature per project at a time. The project view shows a single "Current feature" slot plus a queued list. |
| **Now** | Concurrent — N features per project run at the same time. |
| **Changed** | 2026-07-12 |

**Why it changed.** Serial-per-project was the conservative default, chosen to
sidestep shared-state hazards rather than because a user wanted to wait. In
practice a project is a repo, and there is no reason two independent features
on one repo cannot be in flight at once — they touch different branches and
different worktrees. Waiting is a cost with no corresponding benefit.

**What made it safe to change.** The invariant was never actually enforced in
code (`feature_start` validated only the title and description), so features
*were* already running concurrently — just without anything guaranteeing they
would not collide. Two facts closed the gap:

* **Git is concurrency-safe here.** Git locks per-ref, and features touch
  disjoint refs (`feature/<slug>` and `feature/<slug>_subtask_*`). Eight
  concurrent features provisioning worktrees, committing, force-moving their
  own branch refs, and running `worktree prune` against one shared `.git`
  complete without a single failure.
* **Worktree paths are feature-scoped.** Worktree directories derive from
  `{repo_dir}_wt_{subtask_id}`, and `subtask_id` now carries the feature id.
  Before that, two features on one project derived the *same* directory, and
  provisioning force-removes its target — so starting feature B deleted feature
  A's live worktree and its uncommitted work. That was the concrete bug behind
  the old `parallel` step's removal.

**What this decision *requires* to hold.** Concurrency is only correct if
features share nothing mutable. The one place they still did was the
**dependency cache**: every worktree symlinked to the *same physical*
`{repo}/node_modules`, `{repo}/target`, `{repo}/.venv`. Feature B's install
overwrote feature A's, and — worse — a `verify` step's harness verdict could be
decided by another feature's build output, which then drove Demeteo's retry and
critic loops. The rule that replaces it:

> **Share content-addressed *download* caches; never share *build* output.**
> Download caches (the Cargo registry, npm's `_cacache`, the pip wheel cache)
> are immutable-by-content and safe to share across features. Build and install
> outputs (`node_modules`, `target`, `.venv`, `.next`) are per-branch state and
> must be per-feature.

Anything else added to a "cache" list must be classified against that rule
before it is shared.

**Still open:** a concurrency ceiling. N features × M tasks × one agent process
each is unbounded resource use. The per-project axis (how many features on one
repo) and the cross-project axis (how many anywhere — bounded by CPU/RAM, not
correctness) both want a limit.

## 3. Cross-References

- **Domain model** (entities, value objects, aggregates, ports): [`DDD_MODEL.md`](DDD_MODEL.md)
- **Architecture** (hexagon, port surface, file layout, Tauri commands, frontend state): [`ARCHITECTURE.md`](ARCHITECTURE.md)
- **Open / deferred questions**: [`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md)
- **Reliability plan**: [`RELIABILITY_PLAN.md`](RELIABILITY_PLAN.md)
- **Agent runtime spec**: [`AGENT_INTEGRATION.md`](../AGENT_INTEGRATION.md)
- **Known platform issues**: [`KNOWN_ISSUES.md`](KNOWN_ISSUES.md)
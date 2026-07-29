# Demeteo: Locked Decisions Reference

> **Standalone reference for the 44 locked design decisions** that emerged
> from the multi-agent orchestrator design. This is the same
> table that guides the project. If any conflicts ever arise, this
> doc should be considered a source of truth; flag the conflict and re-align.

## 1. The 44 Decisions

| #  | Decision                           | Locked answer                                                                  | Source           |
|----|------------------------------------|--------------------------------------------------------------------------------|------------------|
| 1  | Top-level entity shape             | Project → Feature (Mission → Subtask DAG)                                      | Interview Q1     |
| 2  | Demeteo's role                     | Orchestrator, not chat client — drop the supervisor plane                      | Interview        |
| 3  | Brain role                         | Advisor; declarative, embedded in workflow steps                               | Interview Q3     |
| 4  | LLM provider scope                 | Delegate to a coding agent acting as planner for *runs*. **Exception:** the opt-in **Memory Agent** (`adapters/memory_llm.rs`, `adapters/memory_worker.rs`) calls a user-configured OpenAI-compatible endpoint directly, in the background, only to distill run signals into project memories. It never drives a feature run; it is disabled by default and its API key lives in the OS keyring. | Interview Q4/Q5  |
| 5  | Planner selection                  | Per-project planner via `ProjectSettings::default_agent_kind` + `default_model`; overrideable per-workflow (`ProjectWorkflowOverride` with `step_id = None`) and per-step (`step_id = Some(...)`); loses to a run-time override chosen in `StartFeatureModal`. | Interview Q6     |
| 6  | Project structure                  | One host per project (local or remote SSH); repos cloned via PAT               | Interview Q7/C   |
| 7  | Workflows as templates             | First-class, versioned, importable; starter pack shipped in binary              | Interview Q8     |
| 8  | Step execution model               | Typed node types behind the `NodeTypeRegistry`: `agent` / `gate` / `sequence` (superseding `parallel`) / `sync` / `finalize` / `command`. **`command` un-deferred 2026-07-26** (task P3.5): a deterministic shell command through the `ExecutionPort`, run in a disposable worktree at zero token cost, non-zero exit classified `verdict`. It does **not** merge back — a step that changes tracked files is an agent step. | Interview Q8; PRD_DAG_WORKFLOWS §5.2 |
| 9  | Context propagation                | Artifact pointer (C) + planner-summary fallback for chat-shaped (B)             | Interview Q10    |
| 10 | Workflow versioning                | Local + versioned + importable, JSON format, starter pack in binary             | Interview Q11    |
| 11 | Project bootstrap depth            | Clone + detect (B) + propose worktree strategy (C); no repo writes (D deferred)| Interview Q12    |
| 12 | Gate UX                            | Planner summary card + artifact/diff list + Approve/Redirect/Cancel            | Interview Q13    |
| 13 | Implement-step failure semantics   | **Keep the prefix.** A failing task stops the `sequence` list (later tasks may depend on it, so they do not run), but the tasks that already completed are merged to the feature branch before the step reports failure, and the retry resumes from the failed task instead of re-running the whole list. ⚠️ **Supersedes the `parallel`-era "continue-and-report (D)" answer** — see [§2](#2-superseded-decisions). | 2026-07-12 (was Interview Q14) |
| 14 | Workflow re-entry / resume         | Per-step checkpoints; synthetic gate on mid-step interrupt                     | Interview Q15    |
| 15 | Workflow telemetry                 | Per-step cost + duration; **no pre-launch cost estimate**                      | Interview Q16    |
| 16 | Repo merge model                   | `feature/<slug>` branch from canonical; subtasks merge into it; optional MR    | Interview Q17    |
| 17 | PAT scope                          | Per-provider global, keyed by `(kind, host)` for multi-instance support        | Interview Q17a   |
| 18 | Multi-feature concurrency          | **Concurrent — N features per project.** Features on one project run at the same time, each on its own `feature/<slug>` branch and its own feature-scoped worktree. ⚠️ **Supersedes the original "strict serial (A)" answer** — see [§2](#2-superseded-decisions). | 2026-07-12 (was Interview Q18) |
| 19 | Workflow authoring UX              | **Visual DAG builder** (`WorkflowCanvas` design mode) replaces the form-first editor; read-only Monaco source tab per [decision 42](#1-the-44-decisions); "save run as template" still v1.2. ⚠️ **Supersedes the form-first (v1.0) / YAML-view (v1.1) answer** — see [§2](#2-superseded-decisions). | 2026-07-23 (was Interview Q19) |
| 20 | Conflict resolution UX             | **Inline, at the point of conflict — no cascade layer.** A step's task-branch merge that conflicts costs one agent turn in the step's own worktree and session (`steps/conflict_pass`); an upstream-sync conflict is surfaced to the user, who triggers `feature_resolve_sync_conflicts` ("Resolve with agent") from the UI. No dedicated Monaco 3-way component. ⚠️ **Supersedes the original "smart cascade" answer** — the `ConflictResolver` port, its stub adapter, and the `subtask_merges` audit table were deleted as never-called; see [§2](#2-superseded-decisions). | 2026-07-12 (was Interview Q20) |
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
| 37 | Reasoning effort                   | A **peer of the model**, not a property of it. One canonical ladder — `low` < `medium` < `high` < `xhigh` < `max` (`EffortLevel`, lowercase on every wire) — resolved by the same 5-tier chain as `model` (per-step run override → feature-wide run override → workflow `StepConfig.effort` → project `default_effort` → **`high`**). Each adapter **clamps down** to what its agent declares (`AgentCapabilities.effort_levels`) before emitting: claude-code `--effort` + `CLAUDE_CODE_EFFORT_LEVEL`, codex `-c model_reasoning_effort=`, opencode `--variant`, hermes nothing. See the detail block below. | 2026-07-14 |
| 38 | Feature pins its workflow version  | **Yes — `features.workflow_version_id` column** (migration V33). `start_feature` resolves the latest version exactly once and stores the row id; the run path and `RunSpec` read the pinned row (remote already snapshots `workflow_json`). Editing a workflow mid-run can never change a running graph; historical runs render the graph they actually executed. Resolves PRD DAG §11 Q1. | PRD DAG §11 (2026-07-23) |
| 39 | DAG join-semantics default         | **`all_success`** is the default join for every node, including gates fed by multiple verify branches. The critic's `PASS_WITH_NOTES` verdict maps to *success* for join purposes, so a strict join doesn't block on a passing-with-notes critic. `any_success` / `all_done` remain per-node opt-ins in schema v2. Resolves PRD DAG §11 Q2. | PRD DAG §11 (2026-07-23) |
| 40 | `conflict_policy` becomes sync-node config | The decorative `ProjectSettings.conflict_policy` (decision 20's known loose end) becomes a **config field on the `sync` node type** in schema v2, where the upstream-sync merge it governs actually happens. The project-settings dropdown is removed; v1→v2 migration seeds the node field from the project value. Resolves PRD DAG §11 Q3. | PRD DAG §11 (2026-07-23) |
| 41 | Scheduling stays outside the graph | `WorkflowSchedule` (cron) remains a **workflow-level sibling of `nodes`/`edges`**, not a node type — schema v2 does not entrench scheduling in the graph, so it can move to the Kanban card/board layer (Epic C1) later without a schema break. Resolves PRD DAG §11 Q4. | PRD DAG §11 (2026-07-23) |
| 42 | Workflow source view               | The DAG builder ships a **read-only Monaco JSON source tab in Phase 3** (fulfilling decision 19's deferred source-view promise in v2 form). Editable source with two-way canvas binding stays deferred — a later decision record is required to add it. Resolves PRD DAG §11 Q5. | PRD DAG §11 (2026-07-23) |
| 43 | Rework is a decomposition, not a re-run | A verdict failure downstream of a `sequence` step redirects to the step that **produces** its task list, not the step that executes it. That producer, seeing it is in a rework cycle (`domain/rework.rs` — the failing step is a descendant of the consumer), renders its `rework_prompt_template` and emits a **delta**: one ticket per defect the verdict named. The sequence step runs that list whole against the branch the previous cycle already landed, and reports the earlier cycles as `already_landed`. The file-overlap `select_targeted_tasks` heuristic survives only where there is no producer to ask (legacy `parallel` workflows). Corollary: the decomposition step must come **after** the spec step, so a rework redirect cannot rewind the spec and move the acceptance criteria the validator judges against. | 2026-07-28 |
| 44 | Validate judges a delta, not an absolute | A harness failure is retryable **iff** the harness was proven runnable **and** the failure is *new relative to a measured baseline*; everything else is terminal with remediation. The baseline is an engine **measurement** — exit status plus a normalized failure fingerprint per named harness, taken against the run's base commit — never an agent's reading of its own test run. Persisted as one JSON column, `features.harness_baseline_json` (migration V37). See the detail block below and [docs/HARNESS_BASELINE.md](HARNESS_BASELINE.md). | 2026-07-28 |

### 44 — Harness baseline (detail)

Three things here were decided against a plausible-sounding alternative, so the
reasoning is recorded rather than left to be re-derived.

**Why a delta and not an absolute.** `run_harness_first` treats any non-zero exit
as this step's verdict. Nothing establishes what the suite did on the base branch,
so a repository whose tests were already red sends the run into a rework loop for
a defect it did not introduce — and a missing system library arrives wearing the
same costume. Both are then fed to `s-implement`, which cannot fix either. The
delta is what makes "is this the feature's fault?" answerable at all, and the
retry rule above is just that question restated.

**Why a measurement and not an agent.** The obvious alternative is to prompt the
validate agent to run the tests and report the verdict. It is rejected because the
thing being judged would then control the evidence: an agent can report a pass
through a subset, a `--no-fail-fast`, a misread, or plain optimism. Three
supporting reasons — the `Verify` capability's write fence is `ArtifactsOnly`, so
`cargo test`/`npm test` would need the fence widened, which [decision
35](#1-the-44-decisions) and AGENTS.md §2 forbid; build output streamed into
context is the cost `run_harness_first` exists to avoid; and an agent-chosen
command breaks the attempt-to-attempt comparability that
`normalize_failure_fingerprint` and `should_triage` depend on. The agent keeps a
real role — *interpreting* a failure into an actionable rework reason — but never
produces the pass/fail evidence.

**Why a `features` column and not a `run_events` record.** The original framing
assumed the runner→desktop sync carried only events, which would have decided it
by construction. That was false twice. `hydrate_shadow_feature`
(`application/remote_runs/reconcile.rs`) already pulls the runner's whole `Feature`
and `StepExecution` rows over the `get_feature`/`list_steps` RPCs, so a column
replicates along the path `pr_title` and `effort` already travel. More
fundamentally, in a detached run the writer and the reader are **the same process
on the runner against one SQLite file**, so the sync path is a display concern,
not a correctness one. With that constraint gone, `run_events` is strictly worse:
`RunEventsPort` is `append` + `list_since` only — no by-kind lookup, so every read
scans the feature's whole log — it is append-only, so a re-measured baseline could
only shadow the old one, and the two transports key it differently (`feature_id`
locally, `run_id` on the runner, and the runner never wires `RunEventRecorder`), so
an engine-written row would land in a key space `stream_events` never queries. One
JSON column follows `features.attachments_json` (V19) and V36's own note blessing a
single JSON column for state that is always read and written together and never
queried on — which is exactly this record's shape.

**Why the baseline records a red gate instead of failing on it.** The obvious
reading of "run the harness first" is that a red one should stop the run, and
that is what an early draft of the plan assumed. It is wrong for the *baseline*
producer specifically: failing at the head of the graph, before a line has been
written, restates the exact misattribution the baseline exists to remove — a
repository that was already red is not this feature's defect — and it makes the
"red before, identically red now ⇒ pre-existing" row unreachable, since no run
against such a repo would ever reach validate. So a red gate at the base
completes the node with `exit_ok: false` on the record, and the *only* terminal
outcome there is an environment that can produce no measurement at all: a
failing `prepare_command`, or gates that never reach an exit status. The
asymmetry generalizes to the whole mechanism — an absent baseline degrades to
today's behaviour, while a fabricated one excuses a real regression, so every
ambiguity (transport failure, timeout, failed prepare) records **nothing**
rather than a plausible red.

**Why the granularity ladder stops at the fingerprint.** The comparison escalates
cheapest-first: exit status, then `normalize_failure_fingerprint`. A third rung —
an agent reading the base output and the current one and answering "which
failures in B are absent from A" — is designed and **deliberately not built**. It
would scope a verdict to individual test names rather than to the whole gate,
which is real value, but it costs an agent call on *every* red validate, and the
question it answers only arises when rung 2 concedes too much. That rate is
unknown until the cheap rungs have been watched in practice, and rung 2's failure
direction is the safe one: a perturbed fingerprint reads as *new failures*, i.e.
today's behaviour, never as pre-existing. Build it when a real run shows it is
needed, not before.

**What survives of the triage agent.** The classifier (C6) is narrowed rather
than deleted, because a baseline turns most of the environment-vs-regression
question into a measurement: red-before-and-identically-red-now is not this
feature's, and red-before-differently-red-now is answered too — the gate reached
an exit status at the base, so the machine can run it, and the output changed
under this feature's changes. What is left is green at the base and red now,
which may be a fault that appeared *during* the run (a disk filled, a service
died), and that genuinely needs judgement. The narrowing can only ever withhold a
call, never cause one, and every malfunction still falls back to `Verdict` — the
property that made the agent safe to keep at all.

### 37 — Effort level (detail)

Four things about this one are load-bearing and were easy to get wrong, so they
are recorded rather than left to be re-derived from the code:

**The ladder is canonical and the clamp only goes down.** `EffortLevel` is one
five-rung ladder shared by every agent, and `clamp_for(kind, level)` projects a
requested level onto what that agent actually declares: the level itself if
supported, else the **highest supported level strictly below it**, else the
lowest supported level. So `max` on codex runs as `xhigh` (codex only exposes
`max` on some `gpt-5.6-*` models) — it never rounds *up* into a costlier tier
the user didn't ask for, and it can never emit a level outside the declared set.
The clamp lives in the adapter, not the caller, so no calling site needs
per-agent knowledge. That is deliberate defence-in-depth alongside the
capability-driven picker: the UI shouldn't offer an unsupported level, and if it
somehow does, the adapter still can't emit one. Neither codex nor opencode would
catch it for us — codex wraps an unknown effort as `Custom(String)` and sends it,
opencode silently no-ops an unsupported `--variant`.

**The default is `high`.** The terminal fallback of the resolution chain is
`EffortLevel::DEFAULT = High`, not "whatever the agent does on its own". This is
the literal product requirement, and it is worth being explicit about the cost
consequence: features that ran at each agent's own default (typically medium)
now run at high effort with no user action.

**Hermes shipped effort-unsupported, on purpose.** Hermes exposes effort only
through `agent.reasoning_effort` in `$HERMES_HOME/config.yaml`; there is no
per-invocation flag. The tempting workaround — generate a per-run config under an
isolated `HERMES_HOME` — was rejected: `HERMES_HOME` also relocates hermes's
credentials and its session `state.db`, so doing it blind risks breaking both
authentication and the cross-step `--resume` continuity the adapter depends on,
trading a missing feature for a broken agent. Hermes therefore declares
`effort_levels: &[]`, emits nothing, and the frontend greys its effort control
out with a tooltip. Honest degradation; revisit only against a real hermes
install.

**Internal turns are pinned, not inherited.** The verifier, env-triage,
finalize, sequence and conflict-resolution turns each build their own
`AgentContext`. Letting them inherit a blanket `high` would apply high effort to
*every verifier retry* — a job `VerifierConfig`'s own doc comment calls a
small-model task — and would interact badly with the `max_cost_usd` cap. So:
triage → `Low`; verifier → `VerifierConfig.effort ?? Low`; finalize → `Medium`;
sequence + sync → the resolved step/feature effort (real agent work, inherits).
The constants live in one place (`domain/models/effort.rs`) and are trivially
tunable.

**Known, unfixed:** `steps/finalize/turn.rs:42` resolves the model as
`step_conf.model.or(feature.model)`, which **inverts tiers 2 and 3** and ignores
`step_overrides` + `default_model` entirely. That is a pre-existing model bug,
not an effort one. Effort deliberately does **not** copy that shape, and the bug
was left unfixed here as out of scope — recorded so the next reader doesn't
mistake the inconsistency for intent.

### 43 — Rework cycles (detail)

The shape this replaced cost a real feature **26.9M tokens**: a 25-ticket run
whose validator flagged four defects re-ran the entire ticket list twice, and
was on its third pass when it was stopped. Four things about the replacement
are load-bearing.

**The classification is a graph question, not a step-kind question.** "Has my
output been implemented?" is answered by asking whether the failing step is a
strict descendant of the *consumer* — the `sequence` node whose
`task_list_from` names this producer. Not of the producer itself: the shipped
pipeline puts `s-gate-review` between the two, so a reviewer rejecting the
decomposition there is downstream of the producer while nothing has been built.
Classifying that as rework would emit a delta against code that does not exist.

**Uncertainty resolves to `Revision`, always.** Revision re-emits the whole
list: correct, merely expensive. Rework skips work on the claim that it is
already committed. A producer with no consumer, a failing step the graph does
not contain, a synthesized retry context naming no step — all resolve to
Revision.

**`kind: "rework"` on the task-list artifact is the contract, with an evidence
fallback.** The producer declares it; a producer that forgets is caught by id
overlap (a delta names work that did not exist before; a revision reissues the
same ticket ids). The fallback only runs when the graph already says this is a
rework cycle, so the worst a wrong answer does is re-run a list — never the
reverse.

**Cycles accumulate in `plan_json`, not in a new column.** `sequence_plan_cache`
keeps `(feature_id, step_id)`; the row's JSON grows `kind`, `cycle` and a
`history` array. Serde ignores unknown fields, so pre-existing rows parse
unchanged and new rows parse in older builds — no migration, and the drill-down
can show "Original decomposition · 25 tickets / Rework 1 · 4 tickets" instead of
silently replacing one with the other.

**The loop still has to be able to converge.** A spec whose acceptance criteria
demand a command the project harness does not run can never be satisfied, and
before this change that burned the whole retry budget discovering it. The spec
step is now told to phrase criteria against `{{test_command}}` / `{{build_command}}`,
and the validator returns a third verdict — `environment` — which routes to
`VerifierError::Environment` and terminates once with remediation instead of
opening a rework loop no agent can close.

## 2. Superseded decisions

A decision you silently overwrite stops being a decision *record*. When a
locked answer changes, the row above is updated **and** the original is kept
here with the reason it moved, so the next reader can tell "we thought hard and
changed our minds" from "nobody ever considered this".

### 19 — Workflow authoring UX

| | |
|---|---|
| **Was** | Form-first (v1.0): stacked step-list editor with chevron reorder; Monaco YAML view with two-way form binding promised for v1.1; "save run as template" v1.2. |
| **Now** | Visual DAG builder — `WorkflowCanvas` design mode with palette, schema-driven config panels, live lint, versioning UI; read-only Monaco source tab (decision 42). "Save run as template" remains v1.2. |
| **Changed** | 2026-07-23 |

**Why it changed.** The form editor could only express what the engine could
run — a linear list. `PRD_DAG_WORKFLOWS.md` makes the workflow model a real
DAG (nodes + edges, joins, conditional edges), which a stacked form cannot
represent at all, and the audit found the form's structural defects (no dirty
guard F38, dangling `on_failure` refs F39) are exactly the class of bug a
canvas with connect-time validation eliminates by construction. The YAML-view
promise survives in reduced form: a **read-only** source tab in builder
Phase 3 (decision 42); the two-way editable binding that made v1.1's version
expensive is still deferred.

### 13 — Implement-step failure semantics

| | |
|---|---|
| **Was** | Continue-and-report (D) + opt-in retry with cost cap (C layered) — when one `parallel` subtask failed, the others kept running, every result was reported at the end, and a capped retry could re-run the failures. |
| **Now** | Keep the prefix — a failing task stops the `sequence` list, the completed tasks' commits are merged to the feature branch, and the retry resumes from the failed task. |
| **Changed** | 2026-07-12 |

**Why it changed.** The original answer was written for the `parallel` step,
whose subtasks were independent by construction (disjoint file ownership,
separate worktrees), so "keep going past a failure" was safe: the surviving
subtasks could not have depended on the failed one. The `sequence` step that
replaced it (see decision 8's history) inverts that premise — tasks run in
order precisely *because* later tasks may build on earlier ones. Running the
tail past a failed task would hand each agent a worktree missing work its
task assumes, which produces confused agents and spend with no expected value.

**What survives, and how.** The part of the original decision worth keeping
was never the literal "continue" — it was *don't discard paid-for work, and
make the retry pay only for what actually failed*. That is exactly what the
checkpoint preserves: when task k of n fails, tasks 1..k-1 are already
committed in the step's worktree, so the worktree is reset to the last
completed task's commit (discarding the failed task's debris) and merged to
the feature branch before the step fails. The step records the landed task
ids (`ExecutionDriver::sequence_checkpoints`), and every subsequent attempt's
plan resolution filters them out, so the retry resumes from the failed task
with its prompt naming the work already on the branch. The rewrite that
introduced the sequence step initially shipped the opposite (roll back
everything, re-run everything) — that was fallout, not a decision, which is
why this row exists.

**Boundaries.** Cancellation still rolls back — the user asked to stop, not
to bank partial work. A verifier verdict against the *complete* list still
rolls back too: every task "completed", so there is no failed-task boundary
to checkpoint at, and the verdict impugns the content rather than the
execution. And if the prefix merge itself fails (the feature branch moved and
conflicts), the step falls back to the old full rollback rather than spending
agent budget salvaging a partial prefix.

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

### 20 — Conflict resolution UX

| | |
|---|---|
| **Was** | Smart cascade: auto-agent → manual → skip/abort, driven by a per-project `ConflictPolicy` (`always_gate` / `auto_agent` / `auto_human`), with a `ConflictResolver` port orchestrating it and a `subtask_merges` audit table recording every merge and conflict report. |
| **Now** | Inline resolution at the point of conflict: `steps/conflict_pass` for task-branch merges, the user-triggered "Resolve with agent" flow for upstream-sync conflicts. |
| **Changed** | 2026-07-12 |

**Why it changed.** The cascade was designed for the `parallel` step's world:
N independent subtask branches merging back separately, each merge a fresh
chance to conflict, needing an orchestrated policy for who resolves what. Two
things made it fiction rather than architecture. First, the `sequence` step
(decision 8's history) collapsed N merges into one, so the volume of
conflicts the cascade was sized for never materialized. Second, the steps
that do merge already hold everything a resolver needs — the worktree, the
conflict markers, and a way to spawn a session — so resolution grew up
*inside* the steps (`steps/conflict_pass`) instead of behind a port.

**What was deleted.** The `ConflictResolver` port and its
`CascadeConflictResolver` adapter (whose `resolve_via_agent` was a stub
returning "not implemented" and which was never even constructed in
composition); `MergeExecutor::merge_subtask_into_feature` / `skip_merge` /
`abort_in_progress` and the `subtask_merges` table they wrote (dropped in
migration V28 — it never held a row); the `precheck_merge` /
`MergePreCheck` preflight only that path called; and the `MergeOutcome` /
`SubtaskMerge` / `ConflictPolicy` domain types. The sync half of
`MergeExecutor` (`sync_feature_with_upstream`, `feature_syncs` audit) is
live and stays.

**Known loose end — resolved by decision 40 (2026-07-23).**
`ProjectSettings.conflict_policy` still exists as a stored string and the
project settings UI still renders a "Conflict Resolution Policy" dropdown —
but nothing has ever read the value to make a decision. Decision 40 settles
it: the policy becomes a config field on the `sync` node type in workflow
schema v2, and the project-settings dropdown is removed.

## 3. Cross-References

- **Domain model** (entities, value objects, aggregates, ports): [`DDD_MODEL.md`](DDD_MODEL.md)
- **Architecture** (hexagon, port surface, file layout, Tauri commands, frontend state): [`ARCHITECTURE.md`](ARCHITECTURE.md)
- **Open / deferred questions**: [`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md)
- **Reliability plan**: [`RELIABILITY_PLAN.md`](RELIABILITY_PLAN.md)
- **Agent runtime spec**: [`AGENT_INTEGRATION.md`](../AGENT_INTEGRATION.md)
- **Known platform issues**: [`KNOWN_ISSUES.md`](KNOWN_ISSUES.md)
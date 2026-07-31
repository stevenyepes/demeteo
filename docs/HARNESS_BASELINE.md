# Harness Baseline — Design and Task Plan

> **Scope:** whether the evidence the validate step is handed is *real*,
> *complete*, and *attributable to this feature*. Three documents already own
> adjacent ground and none owns this: [`TASKS_DAG_WORKFLOWS.md`](TASKS_DAG_WORKFLOWS.md)
> owns DAG topology, [`RELIABILITY_PLAN.md`](RELIABILITY_PLAN.md) owns transport
> and crash recovery, [`roadmap/stories/UX2-truthful-ui.md`](roadmap/stories/UX2-truthful-ui.md)
> owns UI/backend mismatch. The preflight designed here runs inside
> `run_bootstrap_tail_inner` **before any graph is walked**, and must work
> whichever workflow the user picked — so it belongs to none of them.

---

## How to run a task

1. Read this file's header + the single task section. Read the cross-referenced
   sections the task names — not the whole of each document.
2. Load only the files under **Context**. Several are >1k lines; respect the
   ranges.
3. Stay inside **Touch**. If the task turns out to require edits outside it,
   stop and report — that's a decomposition bug in this plan; fix the plan first.
4. Run the task's **Done when** checks, plus `npm run checks`.
5. Commit per task (`feat(harness): HB1 bootstrap preflight phase`) and flip the
   checkbox below.

**Sizing rule:** ≤ ~2,000 lines of required reading, one coherent diff, no task
depends on holding two subsystems in context at once.

**Migrations:** V36 `sequence_checkpoint_produced` and V37 `harness_baseline`
are built — the next free number is **V38**.

**Dependency order:**

```
HB1 ─▶ HB4 ──┐
             ├─▶ HB6
HB5 ─────────┘
  └─▶ HB2a ─▶ HB2b ─▶ HB2c ─▶ HB7
```

`HB3` was independent of everything, and **every task in this plan is now
done**. The verdict is a delta, the subtraction is auditable in the UI, and
detection no longer produces confidently wrong commands in the first place.

### Key code coordinates (shared reference — don't re-discover these)

> Line numbers drift. Re-verify with `git grep` before relying on one.

| What | Where |
|---|---|
| Bootstrap tail | `crates/demeteo-core/src/adapters/step_executor/impl_traits/bootstrap.rs` (`run_bootstrap_tail_inner`, `run_harness_preflight`) |
| Phase vocabulary | `crates/demeteo-core/src/adapters/step_executor/impl_traits/mod.rs` (`bootstrap_phase`) |
| Harness execution primitive | `.../step_executor/driver/verifier.rs` (`run_harness_first`, `harness_shell_options`) |
| Preflight probe (HB1, built) | `.../step_executor/preflight.rs` |
| Ecosystem detection | `.../adapters/worktree/git_ops/strategy.rs` (`detect_worktree_strategy`) — the recipes it decides from are `domain/ecosystem.rs` (HB3) |
| Shell contract | `crates/demeteo-core/src/ports/execution.rs` (`ShellOptions`, `TRANSPORT_ERROR_PREFIX`, `TIMEOUT_ERROR_PREFIX`) |
| Sequence prompt binding | `.../step_executor/steps/sequence/prompt.rs` |
| Phase order (frontend) | `src/types.ts` (`BOOTSTRAP_PHASE_ORDER`), rendered by `src/components/BootstrapStepper.tsx` |
| Run-event bridge | `.../adapters/run_event_log.rs`, `crates/demeteo-runner/src/notify_bridge.rs` |
| Starters (7 JSON files) | `src-tauri/workflows/*.json` |

---

## 1. Invariants currently violated

These are the claims the tasks below exist to restore. Each was verified by
`git grep` at the time of writing; re-verify before relying on one.

### I1. The engine executes exactly one user-authored command per run

`run_harness_first` is the only place the orchestrator runs a project's
`prepare_command` / `test_command`. It has two call sites — the single-turn
validate path in `steps/agent/mod.rs` and the dedicated verifier path in
`driver/verifier.rs` — and **both are gated on the step declaring a `verifier`
block**. Across the seven starters that is one node in most, and in
`standard-feature-pipeline.json` it is `s-validate` alone.

`s-implement` (a `sequence` node) declares no verifier. The per-ticket
`test_command` a ticket carries is bound into the prompt in
`sequence/prompt.rs` and read by the *agent*; the engine never executes it. The
only `run_command` in the whole `steps/sequence/` tree is git plumbing.

**Consequence:** the first and only moment the engine finds out whether the
project's commands work at all is the second-to-last step, after the entire
implement budget has been spent.

### I2. A pre-existing red harness is attributed to the feature

`run_harness_first` treats any non-zero exit as this step's verdict. Nothing
ever established what the suite did on the *base branch*, so a repository whose
tests were already failing before the feature started sends the run into a
rework loop for a defect it did not introduce. `refactor.json` had an
`s-baseline` step that addressed this — but it was an `agent` step reading its
own test run and writing prose, not an engine measurement, and the standard
pipeline had no equivalent at all. (Deleted by [F2](#f2--refactorjson-had-two-baselines--done-2026-07-29);
every test-gated starter now opens on the `s-baseline-harness` measurement.)

### I3. A misconfigured command is indistinguishable from a broken feature

`detect_worktree_strategy` guesses commands from repo-root marker files and
never verifies that the guess runs. `prepare_command` is never detected at all
(`strategy.rs` returns `prepare_command: None` unconditionally). The user's
first signal that the guess was wrong is a validate failure, phrased as an
implementation defect.

### The cost of leaving these open

Feature `f-1785157902856` in the dev DB, against a repo whose spec criterion 1
demanded `cargo build` and the frontend gates:

| Attempt | Status | Cost | Tokens |
|---|---|---|---|
| `s-implement` #7 | completed | $21.68 | 15,905,377 |
| `s-validate` #1 | failed (`verdict`) | $1.11 | 857,217 |
| `s-implement` #8 — rework cycle 1 | completed | **$14.63** | **11,039,774** |
| `s-validate` #2 | failed (`verdict`) | $0.41 | 304,014 |
| `s-implement` #9 — rework cycle 2 | running | — | — |

Neither validate attempt failed on a red harness. Both failed on an *agent
verdict* that the acceptance criteria could not be shown met — because the
configured harness never ran the commands those criteria named. The rework loop
then re-implemented a feature that was not the problem, at $14.63 and 11M
tokens per cycle.

---

## 2. The shape of the answer

Validate today asks one question — *is the harness green?* — and that one
question is doing three jobs badly. The tasks below split it into three, each
answerable at a different time by a different mechanism:

| Question | Answerable | Mechanism | Deterministic |
|---|---|---|---|
| Can the harness run at all? | before anything | `command -v` probes | yes |
| What does the harness say about the code as it stands? | once, at the head of the run | one harness run, zero tokens | yes |
| Did *this feature* change that answer? | at validate | delta against the baseline | yes |

**The load-bearing change is the third row: validate's verdict becomes a delta,
not an absolute.** That is what stops a pre-existing red suite and a missing
system library from both arriving dressed as "your feature is broken" — and it
is what yields the retry rule this whole document is for:

> A harness failure is retryable **iff** the harness was proven runnable **and**
> the failure is new relative to the baseline. Everything else is terminal, with
> remediation.

Retrying anything else spends an implement budget on a defect the implement step
cannot reach. That is not a heuristic; it is the definition of what `s-implement`
is able to change.

### What this does to C6

`classify_harness_failure` currently reaches the environment-vs-regression
question through a triage *agent*, gated on the failure reproducing unchanged
(`should_triage`). Its cheapest possible detection is therefore **one full rework
cycle** — and if the agent perturbs the output at all in between, the fingerprint
comparison resets and it is two.

With a baseline most of that becomes a measurement rather than a judgment:
green-before / red-now **is** a regression, and red-before / identically-red-now
**is not** this feature's. The triage agent survives, narrowed to the one case
that genuinely needs judgment — green at baseline, red now, for an environmental
reason that appeared *during* the run (disk filled, a service died, a registry
went down). **Shrink its remit; do not delete it.** Its fail-safe property (any
malfunction falls back to `Verdict`) is what makes it safe to keep.

**And move the call it does keep earlier.** "Red-before / identically-red-now is
not this feature's" is true only when the gate *ran*. A gate that was red at the
base because the machine cannot run it — a missing system library, an absent
toolchain, an unprovisioned service — produced no evidence, so subtracting it
passes the step on nothing. Telling those two apart is exactly what the
classifier is for, and the cheapest moment to ask is when the baseline is
measured: at the head of the graph, with **zero implement budget spent**, versus
`should_triage`'s minimum of one full rework cycle. So the baseline producer
classifies each *red* gate once and stores the answer on the record; the
comparison reads it rather than re-asking. Same agent, same prompt, same
fail-safe — a different, earlier call site. A green baseline is not classified at
all, because there is nothing to classify.

**And act on it there too.** Once the answer is in hand at the head of the graph,
recording it and continuing spends the entire implement budget to reach a
terminal `Environment` the engine could already state. So a gate classified
`environment` at measurement time ends the run *at the baseline node*. This is
the one thing the earlier call site buys that the comparison cannot, and it is
[I1](#i1-the-engine-executes-exactly-one-user-authored-command-per-run) closed
from the other end: the first moment the engine finds out is no longer late.

### Detached runs are the hard case, and they constrain the design

Everything above must hold for a run executing inside `demeteo-runner` with
nobody watching. Three consequences, and they are the reason the design is
deterministic rather than agentic:

1. **No human means every "ask" is a pre-declared policy.** There is no dialog to
   fall back on, so the classification must be a table the engine can evaluate.
   Note that `RunSpec.unattended` relaxes *gates* only — it must never relax the
   harness gate.
2. **The runner box is not the developer's laptop**, so "missing tool" is *more*
   likely there, not less. The case where a human cannot intervene is exactly the
   case where the environment-vs-regression distinction matters most.
3. **It already runs in the right place, and this is stronger than it looks.**
   `run_bootstrap_tail` lives in shared core and the runner calls the same
   `feature_start`, with `RunSpec.project_settings` overlaid *before* it is read —
   so HB1's preflight already probes the runner's `PATH` using the launching
   client's harnesses. More than that: the runner drives the entire engine itself
   (`demeteo-runner/src/run.rs` → `ctx.executor.feature_start`), so **every
   producer and consumer below is co-located in one process against one SQLite
   file.** Detached execution is not a distributed-state problem here; it is a
   display problem. Everything stays orchestrator-side and transport-blind, so the
   `ExecutionPort` parity invariant holds with no new branching.

---

## 3. Tasks

### HB1 — Bootstrap preflight phase — **[Done]**

- **Goal:** measure the project's configured commands **once, before the
  pipeline starts**, and surface the result on the bootstrap stepper. Runs for
  every launch regardless of which workflow was chosen, because it is a phase
  of `feature_start`, not a node in a graph.

- **Shape:** a new entry in the `bootstrap_phase` vocabulary, emitted from
  `run_bootstrap_tail_inner` **between `creating_branch` and `registering`** —
  after the branch exists (so the preflight measures the real base) and before
  the step rows are seeded (so a hard stop leaves no half-registered run). Add
  the same id to `BOOTSTRAP_PHASE_ORDER` in `src/types.ts`.

- **No new UI work.** `BootstrapStepper` already renders an arbitrary phase id,
  its `label` verbatim, a `detail` line, and a `skipped` status. Remote and
  detached runs already carry `BootstrapProgress` through `run_event_log.rs`
  and the runner's `notify_bridge.rs`, so the phase reaches them for free.

- **Probes only — the suite does not run here.** *(Decided 2026-07-28.)* The
  bootstrap phase resolves and probes; it never executes the test suite. The
  full baseline run is [P4.2a](TASKS_DAG_WORKFLOWS.md)'s `baseline-harness`
  command node, inside the graph.

  The reason is wall-clock. A suite at bootstrap delays *every* launch before the
  user sees anything happen, on a cold worktree with no `node_modules` and no
  `target/` — minutes for a Tauri app. As node 1 of the graph it costs
  effectively nothing: in `f-1785157902856` research + tickets + spec ran ~31
  minutes before implement started, and once P4.1 lands the node goes parallel
  with research for free. The two halves also sit at the right altitudes —
probes guard *every* launch whatever workflow was chosen; the node guards every
test-gated starter and produces the durable baseline HB2 consumes.

  The cost of the split, stated plainly: a **custom** workflow with no
  `baseline-harness` node gets probes but no baseline, so HB2's pre-existing-red
  subtraction does not apply to it. Revisit if custom workflows become common.

- **Execution:** *(as built — simpler than designed.)* No worktree. `command -v`
  needs the login shell, not the repo, so the probe runs against the project's
  existing target dir and the throwaway-worktree step disappeared. It runs under
  an interactive login shell for the usual `PATH`/shim reason, with its own
  `PREFLIGHT_PROBE_TIMEOUT_S` (20s) rather than the run's `wall_cap_s` — the two
  answer different questions, and a `command -v` that takes 20s means the machine
  is unwell, not that the binary is slow to find.

- **What the probes are.** `command -v` on every binary the configured
  `test_command` names — every stage of an `&&`/`;`/`|` chain, not just the first,
  since any one missing breaks the whole harness. This is the 127-detection
  insight `classify_harness_failure` already encodes, moved to where it costs
  nothing instead of one full implement budget.

  **`prepare_command` is *not* run here**, contrary to the original design.
  `npm i` is ~40s on every launch, and the P4.2a baseline node runs prepare at
  the same point in the timeline (it is node 1, ahead of research) — so paying it
  twice buys nothing but latency the user watches.

- **The bias, and why it shapes everything.** A false negative is cheap: the user
  lands in today's behaviour. A false positive blocks a legitimate launch. So
  anything that cannot be resolved to a plain binary name without running a shell
  is **skipped, not guessed at** — shell builtins, `VAR=value` assignments,
  `$(…)`/backtick substitutions, globs. And a transport failure or probe timeout
  is treated as *no evidence*, never as a missing binary: refusing to start work
  over a network blip would be strictly worse than not checking at all.

- **Classification and what each does to the launch:**

  | Result | Meaning | Phase status | Launch |
  |---|---|---|---|
  | `not_configured` | `test_command` blank | `skipped`, detail names the setting | proceeds |
  | `command_not_found` | `command -v` on the head binary fails | `failed`, detail names the binary and the reproduce line | **blocked** — nothing downstream can pass |
  | `prepare_failed` | `prepare_command` exits non-zero | `failed`, detail carries the output tail | **blocked** — the worktree can never be made runnable |
  | `ok` | every probe resolved | `completed` | proceeds |

  `timed_out` and pre-existing-red are **not** rows here — neither is knowable
  without running the suite, so both belong to the P4.2a node. The node already
  classifies a timeout as `Environmental` and a non-zero exit as `verdict`
  (`steps/command.rs`), which is the behaviour those rows described.

- **Size:** medium. One engine phase, one const, one frontend array entry.

- **Context:** §1 above; `impl_traits/mod.rs` (`bootstrap_phase`,
  `run_bootstrap_tail_inner`); `driver/verifier.rs` (`harness_shell_options`,
  `classify_harness_failure` for the 127 vocabulary); `worktree.rs`
  (`provision_subtask_worktree`); `src/types.ts` `BOOTSTRAP_PHASE_ORDER`.

- **Touch:** `impl_traits/mod.rs`, a new preflight module under
  `step_executor/`, `src/types.ts`. Tests: each of the five classification rows
  reached deterministically; the blocking rows leave no seeded step rows; the
  non-blocking rows leave the run identical to today's.

- **Done:** `adapters/step_executor/preflight.rs` plus the phase in
  `run_bootstrap_tail_inner` and one entry in `BOOTSTRAP_PHASE_ORDER`. 16 unit
  tests over the pure decision (against a port double that errors on anything it
  was not told to answer) and a two-leg conformance gate,
  `tests/conformance/preflight_gate.rs`, which runs a **real** shell: one leg
  proves an unresolvable binary blocks the launch *with zero step rows seeded*,
  the other proves a resolvable one leaves the run untouched. Both were watched
  fail — an inert gate reddens the first, an over-eager one reddens the second.

### HB4 — Probe every configured command, not just `test_command` — **[Done]**

- **Goal:** close the gap HB1 shipped with. `probe_configured_commands` takes a
  single `test_command` and probes that. Two things it therefore never checks:

  - **`prepare_command`.** HB1 deliberately does not *run* it (`npm i` is ~40s on
    every launch, and P4.2a's node runs it at the same point in the timeline) —
    but *probing* it is `command -v`, which costs nothing. A `prepare_command`
    naming a missing binary launches today and dies in `run_harness_first` after
    the entire implement budget.
  - **The `harnesses` map.** `verifier.harness_name` selects an entry from
    `ProjectSettings.worktree_strategy.harnesses`; a step naming `integration`
    gets preflight coverage of a command it will never run. For any workflow that
    uses a named harness the preflight is currently checking the wrong string.

- **Shape:** probe the **deduplicated union** of `prepare_command`,
  `test_command`, and every value in the `harnesses` map. `probeable_binaries`
  already dedupes order-preservingly within one command; lift that across a slice
  of commands. `PreflightVerdict::NotConfigured` must now mean *none of the
  three* is configured — today it means "no `test_command`", which would wrongly
  report a project that configures only named harnesses as having no harness.

- **The bias is unchanged and still load-bearing.** Widening the *input* set
  widens the chance of wrongly blocking a legitimate launch, so nothing about the
  skip rules (builtins, assignments, substitutions, globs) may be relaxed to
  accommodate this. A harness the user configured but no current workflow names is
  still worth probing — they will name it eventually — and a missing binary in it
  blocks with the same certainty. If that trade turns out wrong in practice the
  fix is to probe only what the chosen workflow can reach; note that, don't
  pre-emptively build it.

- **Depends:** HB1. **Size:** small — one signature, one call site, tests.

- **Context:** `preflight.rs` in full (~260 lines); `impl_traits/mod.rs` at the
  `harness_preflight` phase; `domain/models/project.rs` for `WorktreeStrategy`.

- **Touch:** `preflight.rs`, `impl_traits/mod.rs`,
  `tests/infrastructure/step_executor/preflight_tests.rs`,
  `tests/conformance/preflight_gate.rs`.

- **Done when:** a project whose `prepare_command` names an unresolvable binary is
  blocked at launch naming *that* binary — today it launches and dies at
  `s-validate`. Same for a binary appearing only in a `harnesses` entry. Both legs
  go in the conformance gate (real shell), and both must be watched fail against
  today's code first.

### HB5 — Named harnesses become an ordered list — **[Done]**

- **Goal:** `VerifierConfig::harness_name: Option<String>` selects exactly one
  entry from the `harnesses` map. A validate step that should gate on lint *and*
  unit *and* integration can only say so by `&&`-chaining them into one
  `test_command` — which loses per-gate attribution, makes the whole thing
  fail-fast whether or not that was wanted, and hands the agent one
  undifferentiated output blob.

- **Shape:** `harness_names: Vec<String>`, run in declared order, each as its own
  command with its own labelled output block and its own exit status.
  `harness_name` stays accepted as a one-element list — the seven starter JSONs
  and any user-authored workflow must keep parsing unchanged.

- **Run all the *declared* ones, even after one fails.** This is about the
  declared list, **not** about running every harness in the project's map — which
  harnesses gate a step is a resolved decision, see the next bullet. Within the
  resolved list, do not stop at the first failure: if lint and unit are both red
  the user wants both, and stopping turns one wasted cycle into two, which is the
  thing this document exists to prevent. An opt-in `stop_on_first_failure` is the
  right escape hatch for an expensive tail suite — add it when someone asks.

- **Which harnesses gate a step is a resolution chain, not a single field.**
  Today `harness_name` is declared per-step in the *workflow*, and **all seven
  starters declare it `null`** (`git grep harness_name src-tauri/workflows`). So
  the `harnesses` map is dead config across the entire shipped starter pack: a
  user can add `lint → npm run lint`, see it accepted, and nothing will ever run
  it, because the only thing that selects a harness is a field no shipped workflow
  sets. Forking a starter to add your own gate is too much to ask.

  Resolve it the way [decision 5](DECISIONS.md) (planner) and
  [decision 37](DECISIONS.md) (effort) already resolve theirs — most specific
  wins:

  1. the step's `verifier.harness_names` — the workflow author was explicit;
  2. the **project's selected validation gates** (new — the tier that fixes the
     dead config, authored in HB6);
  3. the project's `test_command` — today's fallback, unchanged.

  Because the starters declare nothing they fall straight through to tier 2, so
  ticking `lint` gates every workflow with no forking. A project with no harnesses
  map behaves exactly as it does today.

  **Do not make tier 2 additive** ("always *also* run these"), tempting as it is
  for a safety property. Additive semantics make it impossible to narrow, and
  produce the surprise where a workflow pinned to `unit` still runs the 20-minute
  integration suite. Revisit only if someone is actually burned by losing a gate.

- **The deadline is per harness, not per step.** `harness_shell_options` carries
  `wall_cap_s` as *the ceiling one command may consume* (S10). N harnesses means N
  ceilings — do not divide it, and do not leave the unbounded sum unremarked in
  the doc comment.

- **`HarnessOutcome` becomes plural.** It renders one section today
  (`Ran { name, cmd, output }` / `NotConfigured`). It becomes a list of
  per-harness results, with `render_section` labelling each by name so a failure
  says *which gate* went red. This is also the shape HB2a records and HB7
  renders — get it right here and both of those become mechanical.

- **Depends:** none. **Size:** medium.

- **Context:** `domain/verifier.rs` (`VerifierConfig`); `driver/verifier.rs`
  `run_harness_first` + `HarnessOutcome` (lines ~40–200); `steps/agent/mod.rs`
  harness-section injection; the seven `src-tauri/workflows/*.json`.

- **Touch:** those, plus
  `tests/infrastructure/step_executor/verifier/harness_outcome_tests.rs`.

- **Done when:** a verifier declaring two harnesses runs both *even when the first
  fails*, and the failure names the failing one. A workflow declaring the old
  singular `harness_name` behaves bit-identically — assert against the P0.2
  starter snapshots.

- **Done:** `VerifierConfig::harness_names: Vec<String>` (accepting the singular
  spelling's `null` / `"lint"` / list shapes through one deserializer, so no
  starter changed), the pure `resolve_harnesses` chain in `domain/verifier.rs`,
  and a plural `HarnessOutcome::Ran(Vec<HarnessRun>)` whose per-gate blocks and
  per-gate verdict reason are what HB2a records and HB7 renders. 26 unit tests
  over the pure decisions plus a two-leg conformance gate,
  `tests/conformance/harness_gates.rs`, which runs a **real** shell: one leg
  proves a red first gate does not stop the second and that both are named, the
  other proves a starter-shaped workflow (declaring `null`) is gated by the
  project's selection rather than its `test_command`. Every test was watched
  fail — stopping at the first failure, dividing the deadline, making tier 2
  additive, checking tier 2 first, dropping the tier-3 fallback, dropping the
  gate name from the reason, reporting only the first failure, unlabelling the
  blocks, dropping the `harness_name` alias, letting `Ran([])` render, paying
  the tail budget per failure, and both ways of failing to persist the
  selection.

  **Storage of tier 2 — one correction to this plan.** `WorktreeStrategy` is
  *not* serialized as a whole: `repos/project.rs` gives every field its own
  column, so an `Option<Vec<String>>` needed somewhere to live after all. It
  went into the existing `harnesses` `serde_json` column (V8), which now accepts
  two shapes — the bare map, still written whenever no gate is selected, and
  `{harnesses, validation_gates}` once one is. That keeps V37 free for HB2a and
  keeps every pre-HB5 row byte-identical. The map must be tried **first** when
  reading: every envelope field is optional, so an untagged match against the
  envelope accepts any object and silently discards a legacy map's entries —
  which is exactly what it did until the conformance leg caught it.

### HB2a — The baseline record: shape and storage — **[Done]**

- **Goal:** one durable, per-feature, per-harness record of what the harness said
  *before* the feature. Everything downstream is a read of this.

- **Shape, and why each field is load-bearing:**
  - `base_sha` — **what was measured.** A baseline taken against a different base
    commit is not evidence about this run, and without recording the sha there is
    no way to notice. Easiest field to omit, most expensive to have omitted.
  - `measured_at`, and *which producer* wrote it (node vs. HB2b's fallback) — the
    two have very different wall-clock stories and a support question will ask.
  - per harness: `name`, `command`, `exit_ok`, and `fingerprint`
    (`normalize_failure_fingerprint`, empty when green).
  - the output as an **artifact reference, never inline.** Harness output is
    megabytes; a baseline you cannot afford to read is not a baseline.

- **Storage — decided 2026-07-28: one JSON column, `harness_baseline_json`, on
  `features`, at migration V37.** Record the reasoning in `docs/DECISIONS.md`; it
  is invisible in the code afterwards. The evidence, since it overturned the
  premise this task was originally written on:

  - **The detached path does not decide it, because there is nothing to decide.**
    In a detached run the runner drives the whole engine itself
    (`demeteo-runner/src/run.rs` → `ctx.executor.feature_start`), so the writer
    (the early baseline) and the reader (validate) are *the same process against
    one SQLite file* — both inside `ExecutionDriver` calling `run_harness_first`.
    The sync path is a **display** concern, not a correctness one.
  - **The runner→desktop path was never events-only.** `hydrate_shadow_feature`
    (`application/remote_runs/reconcile.rs`) pulls the runner's entire `Feature`
    and `StepExecution` rows over the `get_feature`/`list_steps` RPCs and writes
    them into the desktop's tables. A new `features` column therefore replicates
    along a path `pr_title`, `effort` and `max_budget_usd` already travel — at the
    cost of ~4 mechanical edits (the column list in `repos/feature.rs`,
    `FeaturePatch` in `ports/db.rs`, and the patch literal in `reconcile.rs`).
  - **`run_events` would have silently failed the one property it was chosen
    for.** `RunEventsPort` is `append` + `list_since` only — no by-kind lookup, so
    every read is a full scan and JSON-parse of the feature's whole log (871 rows
    / 143 KB for `f-1785157902856` alone), against an append-only table with no
    update path, so a re-measured baseline could only shadow the old one, never
    replace it. Worse, the two transports **key the table differently** —
    `feature_id` locally, `run_id` on the runner — and the runner wires only the
    bridge, never `RunEventRecorder`. An engine-written baseline row on the runner
    would land in a key space `stream_events` never queries.

  **Why one JSON column and not a `harness_baselines` table:** the record is
  written as a whole (one measurement covers every harness) and read as a whole
  (validate needs all of them at once), and no harness is ever queried
  individually. That is exactly the criterion V36's own migration note blesses for
  a JSON column, and `features.attachments_json` (V19) is the existing precedent.
  A partial write from HB2b's fallback is a read-modify-write within one process,
  which the blob handles fine.

  Follow the house convention: the `.sql` file **plus** a defensive
  `add_column_if_missing` in `adapters/database/migration.rs`.

- **Depends:** HB5 (the record is per-harness; recording the wrong shape here
  makes HB2c a rewrite). **Size:** medium.

- **Context:** §1 above; `driver/verifier.rs` (`normalize_failure_fingerprint`);
  `crates/demeteo-core/migrations/`; `adapters/run_event_log.rs`;
  `crates/demeteo-runner/src/notify_bridge.rs`; `ports/db.rs`.

- **Done when:** a baseline written at the head of a run is read back by the
  validate step of the same run, **in both a local run and a detached runner
  run.** The detached leg is what decides the storage, so it is not optional.

- **Done:** `domain/harness_baseline.rs` — `HarnessBaseline { base_sha,
  harnesses }` over `HarnessBaselineRun { name, command, exit_ok, fingerprint,
  output_ref, measured_at, producer }`, persisted as `features
  .harness_baseline_json` (V37, plus the defensive `add_column_if_missing`) and
  carried on `Feature` so it replicates along `pr_title`'s path. Merge and
  lookup are pure functions in `domain/`, reachable from a test with no port
  doubles; the adapter's `merge_harness_baseline` only supplies them the stored
  value under the connection lock.

  **Two shape decisions worth not re-deriving.** *Provenance is per harness,
  not per record*: a partial re-measurement merges, so one record legitimately
  holds gates measured at different times by different producers, and a
  record-level `producer` would be false about half its own entries. *There is
  no record-level "was it green" accessor* — every question goes through
  `harness(name) -> Option<&_>`, so a record holding no measurement answers
  nothing rather than answering "fine", and every decode failure (NULL, empty,
  corrupt, a producer a newer build named) degrades to `None`. Absent must
  never read as green or HB2c's table inverts.

  24 tests, every one watched fail: clobbering instead of upserting, blending
  two base shas, fabricating an empty record out of a NULL or corrupt column,
  a name-blind lookup, `covers` accepting any sha, dropping `base_sha` or the
  artifact reference from the payload, an empty record fabricating a green
  gate, and each of the four persistence sites in turn — insert, update patch,
  the read-modify-write accessor, and the defensive column add. The
  `hydrate_shadow_feature` patch is now built by a pure `shadow_feature_patch`,
  which is what makes the replication site — silent, update-only, detached-run
  only — assertable at all.

  **Not proven here, by scope:** the end-to-end "written at the head, read by
  validate" leg needs a producer (HB2b) and a consumer (HB2c). What is proven
  is that the record survives insert, update, and re-read byte-identically, and
  that the runner→desktop patch carries it.

### HB2b — Measure the baseline: the node, plus a lazy fallback

- **Goal:** produce the HB2a record. Two producers, one shape.

- **1. The node — [P4.2a](TASKS_DAG_WORKFLOWS.md).** A `baseline-harness` command
  node at the head of the Standard + Refactor starters: zero tokens, runs
  `prepare_command` plus every harness, and its wall-clock hides behind research
  (in `f-1785157902856`, research + tickets + spec ran ~31 minutes before implement
  started). This is the cheap path and the default.

- **2. The lazy fallback — the part that makes subtraction unconditional.** HB1
  conceded that a *custom* workflow with no baseline node gets no subtraction. It
  does not have to. If validate's harness goes red and no baseline record exists,
  the verifier measures one itself against a detached worktree at the merge-base,
  then decides.
  - It fires **only on the failure path** — the path that otherwise costs a full
    implement cycle. One harness run (minutes, zero tokens) against $14.63 and 11M
    tokens is not a close call.
  - It needs a **worktree detached at a sha**, which `provision_subtask_worktree`
    cannot do — it takes a branch. `git worktree add --detach <path> <sha>` is the
    primitive, and adding it to `git_ops` is the real work in this task.
  - It must run `prepare_command` there (a fresh worktree has no `node_modules`,
    no `target/`) and inherits the same per-harness deadline. On a cold Tauri repo
    that is minutes — acceptable on the failure path, which is precisely why it is
    not the default producer.
  - **Cache it:** the fallback writes the same HB2a record, so a second validate
    attempt in the same run does not re-measure.

- **Prior art to supersede:** `refactor.json`'s `s-baseline` does this by hand — an
  agent runs the suite and writes prose. An orchestrator measurement is both
  cheaper and trustworthy (a measurement, not a reading). Once this lands,
  `s-baseline`'s continued existence needs a decision: keep it for narrative value,
  or delete it as redundant. **Put that to the user; do not presume it.**
  *(Asked and answered — deleted; see [F2](#f2--refactorjson-had-two-baselines--done-2026-07-29).)*

- **Depends:** HB2a. **Size:** large — it is the only task here that adds a git
  primitive.

- **Context:** HB2a's record; `steps/command.rs` in full; `adapters/worktree/git_ops/`
  (`provision_subtask_worktree`, `scope.rs`); `driver/verifier.rs`
  `run_harness_first`; `src-tauri/workflows/standard-feature-pipeline.json`.

- **Done when:** a run of a *custom* workflow with no baseline node still subtracts
  a pre-existing failure. That is the leg that proves the fallback, and adding the
  node to the fixture cannot fake it.

- **Done:** both producers, funnelled through one `measure_gates` in
  `adapters/step_executor/baseline.rs` so a record cannot depend on which wrote
  it beyond the per-gate `producer` stamp.

  - **The node** is `s-baseline-harness`, a `command` node carrying one new
    field — `measure_baseline` — at the head of the Standard and Refactor
    starters. It is the one command node whose command is *not* in the
    workflow: a workflow file cannot know this project's `prepare_command` or
    its gates, so it resolves them through **the same
    `resolve_harnesses` chain validate resolves through**. It records the sha
    `git rev-parse HEAD` reports in the worktree it measured, never the one it
    assumed.
  - **The fallback** fires from `run_harness_first`'s harness-failure path,
    measuring the gates that just went red against a worktree detached at the
    merge-base. The git primitive is `provision_detached_worktree`
    (`git worktree add --detach`) plus `cleanup_detached_worktree`, which
    deliberately does *not* end in a `branch -D`: a detached worktree has no
    branch, and that is the safety property — nothing can commit onto it or
    merge it back, so a measurement can never contaminate the feature.

  **The node records a verdict; it does not judge one — one correction to this
  plan.** §3's HB1 aside says a pre-existing-red suite reaches `verdict` through
  this node. It does not, and should not: failing the run at its very first node
  would restate exactly the misattribution [I2](#i2-a-pre-existing-red-harness-is-attributed-to-the-feature)
  exists to remove, before a line has been written, and it would make HB2c's
  `red → red, same fingerprint → pre-existing` row unreachable through the node
  path. A **genuinely red** gate completes the step with `exit_ok: false` on the
  record. **This supersedes HB1's aside; P4.2a's "Done when" was written against
  the same assumption and is superseded with it.**

  **What the node *does* end the run on.** Two outcomes, and they are one
  statement — *this machine cannot produce evidence about this project*:

  1. **No measurement at all** — a failing `prepare_command`, or gates that
     never reach an exit status. Terminal `Environment` with remediation,
     matching HB2c's `prepare` row.
  2. **A measurement classified `environment`** — the gate reached an exit
     status but was red *because it could not run here*, so it proved nothing
     and will prove nothing at validate either. Same terminal `Environment`,
     carrying the classifier's own remediation.

  Row 2 is not in tension with "a red gate completes the node", because the two
  answer different questions: *whose defect is this red gate?* versus *can this
  gate produce evidence at all?* Only the first is the misattribution the
  baseline exists to remove, and a repository with a known-failing or flaky test
  is classified `regression` and takes the completing path. The value is
  timing — [HB2c](#hb2c--subtraction-and-classification) already terminates for
  this exact gate off this exact field, but only after the whole implement
  budget; the node knows it before a single agent turn, which is
  [I1](#i1-the-engine-executes-exactly-one-user-authored-command-per-run)
  restated. And no earlier phase can reach it: HB1/HB4 probe with `command -v`,
  which catches a missing *binary*, while the motivating `gdk-3.0` case is a
  missing *library* — `cargo` resolves, the build fails, exit 1. One unrunnable
  gate among green ones still halts, for the reason HB1 blocks a launch when one
  probed binary of several fails to resolve. The fail-safe direction is
  unchanged: only a *positive* classification halts, so a broken classifier
  withholds the halt and can never manufacture one.

  The decision is
  `unrunnable_baseline_gate` in `domain/harness_baseline.rs` — pure, reachable
  with no port doubles; the node owns only the notification and the
  `StepOutcome`. 9 pure tests and a conformance leg
  (`a_gate_that_cannot_run_here_ends_the_run_before_a_line_is_written`), every
  one watched fail: the policy made inert, the `exit_ok` guard dropped, an
  absent classification defaulting to unrunnable (the fail-safe inverted),
  scanning back-to-front, halting only when *every* gate is unrunnable, the
  escalation removed entirely (i.e. the pre-HB9 code, which completes), halting
  on merely-red instead, the notification dropped, and each of the command and
  the remediation dropped from the message. The "red is not unrunnable" half is
  pinned on the existing caching leg rather than in a copy of its fixture.

  **Every ambiguity records nothing.** An absent baseline degrades to today's
  behaviour; a *fabricated* one inverts HB2c's table and excuses a real
  regression. So a transport failure, a timeout, and a failed `prepare_command`
  all record **no gate**, never a red one — a suite run without its install step
  fails for reasons that have nothing to do with the base commit. The fallback
  returns `()` for the same reason: with no value to branch on it is
  structurally incapable of changing the verdict it runs beside.

  33 tests, every one watched fail. Pure: each of the five conditions in
  `fallback_baseline_needed` inverted in turn (green measuring, an empty base
  accepted, an empty gate list accepted, `covers` skipped, partial coverage
  accepted), and eight ways `measure_gates` could lie (continuing past a failed
  prepare, recording a transport failure or a timeout as red, abandoning the
  remaining gates after one, sharing one deadline, dropping the `2>&1` wrap,
  fingerprinting the raw output instead of the labelled block, fingerprinting a
  green gate, stopping at the first red one). Wiring, in a six-leg conformance
  gate over a **real shell and real git**
  (`tests/conformance/harness_baseline.rs`): the node assuming its sha, the node
  judging a red baseline, the fallback hook removed, the fallback firing on
  green, the teardown removed, the primitive taking a branch instead of a sha,
  `-b` reintroduced, and the leftover-state clearing removed.

  **Left to the user:** `refactor.json`'s `s-baseline` **agent** step still
  exists beside the new node. It does by hand what the node now measures — an
  agent runs the suite and writes prose — so it is arguably redundant, but it
  also carries narrative value the node does not. Deleting a step from a shipped
  starter is not this task's call; it is asked, not presumed. *(Answered
  2026-07-29: deleted. [F2](#f2--refactorjson-had-two-baselines--done-2026-07-29) records what
  the deletion cost and what it did not.)*

  **Not proven here, by scope:** nothing yet *reads* the record to change an
  outcome. HB2c owns the subtraction, and its "Done when" is still the leg that
  proves this arm end to end.

### HB2c — Subtraction and classification

- **Goal:** evaluate the retry rule from §2 as a table the engine can compute,
  replacing today's "any non-zero exit is this feature's verdict".

  | Baseline | Now | Determination |
  |---|---|---|
  | binary unresolvable | — | blocked at launch, terminal (HB1/HB4) |
  | `prepare_command` fails | — | terminal `Environment` — the worktree can never be made runnable |
  | red, same fingerprint, classified `regression` at measurement | red, same fingerprint | **pre-existing** — not this feature; pass, with the exclusion named in the report |
  | red, same fingerprint, classified `environment` at measurement | red, same fingerprint | **gate never ran** — terminal `Environment` with remediation. Identical to the row above on every input the comparison can see; only the classification separates them. |
  | green | red | **regression** — `Verdict`, retry. No agent needed to know this. |
  | red | red, *different* | new failures atop pre-existing — `Verdict`, retry, scoped to the delta |
  | — | 127 / transport / timeout | terminal `Environment` (already built) |

- **Granularity ladder, cheapest first** — escalate only when the coarser level is
  ambiguous: exit-code equality → `normalize_failure_fingerprint` equality
  (already built) → per-test-name extraction. The third rung is the honest place
  for an agent: reading two outputs and answering "which failures in B are absent
  from A" is comprehension, not judgment, and it never touches an exit code, so it
  cannot manufacture a pass. **All three rungs are now built** — see the rung-3
  block at the end of this section.

- **Narrow C6, don't delete it — and move its earliest call.** See §2.
  `classify_harness_failure` consults the triage agent only for the residue:
  green at baseline, red now, and the failure looks environmental. The *other*
  call happens when the baseline is measured, once per red gate, and is what
  keeps a gate that cannot run from being subtracted. Its fail-safe fallback to
  `Verdict` must survive intact at both sites.

- **The other two consumers, in value order after subtraction:**
  1. **`s-spec` is told factually what the harness can prove.** The prompt already
     handles a blank command (fixed in `2257ffb`); what it lacks is the *positive*
     statement of what the configured commands actually covered on this repo. That
     is what stops criteria being phrased against commands the harness never runs —
     the actual cause of both failed validate attempts in §1.
  2. **`not_configured` reaches `environment` up front.** Today validate can only
     discover an unprovable criterion after the implement budget is gone, and only
     if the agent picks `environment` over `fail`. A baseline saying "no harness
     configured" makes that available at spec time.

- **Depends:** HB2b. **Size:** medium-to-large — touches prompt context and the
  validate verdict path.

- **Context:** HB2a's record; `driver/verifier.rs` (`run_harness_first`,
  `classify_harness_failure`, `should_triage`); `steps/agent/mod.rs`; `setup.rs`
  (`build_base_ctx`, where a `{{harness_baseline}}` placeholder binds); the two
  starter prompts.

- **Done when:** a test red at baseline and red after the feature does **not**
  produce a verdict failure, and a test green at baseline and red after **does** —
  the two legs are the whole task. End to end: a run against a repo with a known
  pre-existing failure reaches `s-critic` instead of looping `s-tickets`, and the
  validate report names the pre-existing failure as excluded.

- **Done:** the comparison is `domain/harness_delta.rs` — pure, synchronous,
  reachable from a test with no port doubles — and `run_harness_first` calls it
  rather than containing it. Per red gate it returns one of five
  determinations: `PreExisting` (subtracted), `Environment` (terminal, with
  remediation), `Regression`, `NewFailures`, and `NoBaseline`. Three of those
  are verdicts; they are still distinct because they answer two further
  questions differently — whether an exclusion must be named, and whether C6 has
  anything left to add. `GateDetermination::outcome()` is the only way to ask
  what to *do* about a gate, and it returns three arms rather than a boolean:
  the boolean it replaced had exactly two answers, "fail the step" and
  "subtract it", so `Environment` had nowhere to go but into the arm it most
  resembles — which is precisely how the first cut of this task shipped a gate
  that could not run as a silent pass.

  **The subtraction applies to a verdict, not to a gate that never ran.** Red at
  the base because a system library is missing looks byte-for-byte like red at
  the base because a test is broken: same command, same fingerprint, same exit
  status both sides. The first proved nothing, so passing on it is evidence-free
  — and it exits **1**, not 127, so the fast path cannot catch it. The record
  therefore carries a per-gate classification (`HarnessBaselineRun.environment`,
  `Option`-shaped, `#[serde(default)]`, no migration — the column is JSON), and
  the comparison reads it as its final rung.

  **Rungs 1–2, plus a lookup.** Exit status settles a green baseline outright; a
  fingerprint match settles same-vs-different; the recorded classification then
  settles what a *same* failure was. That last one is not a comparison rung —
  nothing about the live run is examined — which is why it costs nothing at
  validate: it was answered once, at measurement time. **The per-test rung was
  deferred here and is now built** — see the rung-3 block below for what changed
  the cost argument.

  **C6 narrowed, not deleted — and given an earlier call site.** At validate the
  classifier is consulted only for `Regression` and `NoBaseline` — green at the
  base and red now, which may be a fault that appeared *during* the run, and the
  no-measurement case that keeps today's behaviour. `NewFailures` is answered by
  the measurement (the gate reached an exit status at the base, and its output
  changed under this feature); `PreExisting` never reaches a classifier because
  there is no failure left to classify; and `Environment` was *already*
  classified, at measurement time, so asking again would pay twice for one
  answer. The narrowing only ever *withholds* a validate-time call: the
  reproduce-unchanged gate still has to fire first, so a first-sight regression
  is still a plain verdict at zero tokens, and every malfunction still falls
  back to `Verdict`.

  **Why the classification moved to measurement time.** `should_triage` requires
  a failure to have reproduced unchanged, so C6's cheapest possible detection of
  "this machine cannot run the gate" costs one full rework cycle — two if the
  implementer perturbs the output in between. Measuring the baseline happens at
  the head of the graph, where **no implement budget has been spent at all**, and
  the gate is already red and already in hand. So `measure_gates` hands each red
  gate to the *same* `triage_harness_failure` (through the `BaselineTriage`
  trait, so the function stays testable over one port rather than over an
  `ExecutionDriver`) and stores the answer. Cost: one call per red gate per
  measurement, none for a green baseline, and none at all on the subtraction
  path.

  **The fail-safe direction is unchanged, and it is the reason the field is
  `Option`.** `triage_harness_failure` returns `Regression` on every
  spawn/timeout/cancel/parse failure, so a malfunctioning classifier records no
  fault — and no fault means `PreExisting`, which is the behaviour with no
  classification at all. A record written before the field existed decodes the
  same way. Only a *positive* `environment` answer can terminate a run, so a
  broken classifier withholds an escalation and can never manufacture one.

  **One row deliberately reads before the baseline.** The exit-127 fast path
  runs on the *unsubtracted* failure set. A missing binary is red at the base
  with the identical diagnostic, so it is the most subtractable failure there
  is — and subtracting it would quietly pass a step that tested nothing.

  **The other two consumers.** `{{harness_baseline}}` binds the gates that will
  judge the run (`resolve_gating_harnesses`, the union over every verifier-
  bearing step, through the same chain each will resolve through) plus what
  each already said on this repository; `s-spec` renders it in place of the
  "exactly ONE command … `{{test_command}}`" claim, which became false the
  moment harnesses went plural. With no gate configured the block says so, says
  what it costs a criterion, and names the setting that would change it — at
  spec time rather than after the implement budget is gone.

  36 tests, every one watched fail. Pure: nothing ever subtracted, a regression
  excused too, `covers` skipped, an unmeasured gate reading as green, the
  command-equality check dropped, rung 1 skipped so two empty fingerprints
  match, the empty-fingerprint guard dropped on a red record, the narrowing
  widened to `NewFailures`, an unconfigured harness rendering as an empty gate
  list, an unmeasured gate briefed as passing, and a step's pinned gates
  ignored. Wiring, in an eight-leg conformance gate over a **real shell and
  real git** (`tests/conformance/harness_subtraction.rs`): the exclusion never
  reaching the prompt, the verdict not naming what it is not asking for, an
  all-excluded pass collapsing into the no-harness block, the two sides
  fingerprinted differently, the record read before the fallback writes it, the
  127 path moved behind the subtraction, the narrowing computed but never
  applied, C6's fail-safe replaced by an escalation, and the
  `{{harness_baseline}}` binding removed.

  The classification lookup added eleven more, likewise all watched fail: the
  lookup removed from
  `compare_gate`, an absent classification defaulting to `environment` (the
  fail-safe inverted), `outcome()` routing `Environment` into `Excluded` (the
  shipped defect, restored on purpose to watch it), `measure_gates` never
  classifying, classifying a *green* gate, mapping `Regression` onto a recorded
  fault, `run_harness_first` dropping the escalation, and the remediation
  dropped on its way to the message. The conformance gate grew a ninth leg
  (`a_gate_that_could_not_run_at_the_base_terminates_rather_than_being_excluded`)
  whose fixture is built so a wrong answer is loud rather than ambiguous: the
  gate output carries `@stub-verdict verdict` as well, so an incorrectly
  excluded gate reaches the validate turn and the step completes **green** —
  which is exactly what it does against the pre-fix code.

  **Two fixture techniques worth not re-deriving.** `git symbolic-ref -q HEAD`
  tells HB2b's *detached* baseline worktree from the step's own branch
  checkout, which is how a green-then-red gate is produced deterministically
  without a second commit — the three older harness fixtures adopted it too,
  since their red gates were red at the base as well and were (correctly) being
  subtracted. And `StubRuntime::PROMPT_LOG` records what the stub was actually
  handed: a prompt is otherwise write-only, so "the exclusion reached the
  agent" and "`{{harness_baseline}}` was bound" were unobservable from outside.

  **Not proven here, by scope:** the "reaches `s-critic` instead of looping
  `s-tickets`" wording of the Done-when is asserted at the step level (the
  validate step completes rather than producing a verdict failure) rather than
  by driving the full ten-node starter, which the conformance harness cannot do
  without an LLM. And the *report artifact* naming the exclusion is the agent's
  own prose; what is asserted is that the exclusion, and the instruction to
  record it, reach the turn that writes it.

#### Rung 3 — per-test names — **[Done, 2026-07-29]**

HB2c deferred this on a cost argument and said the judgement was better made
after rungs 1–2 had been watched. What made it load-bearing was not the
false-miss rate: it is [F2](#f2--refactorjson-had-two-baselines--done-2026-07-29). Deleting
`refactor.json`'s `s-baseline` **agent** step downgrades three downstream
consumers that read *individual test identifiers* out of `artifacts/s-baseline.md`
— `s-analyse`'s Test Coverage Map ("the test names verbatim"), its
`rework_prompt_template`, and `s-regression`'s per-test comparison ("for every
test listed as PASSING… if it now FAILS, mark as REGRESSION"). The engine record
carried `exit_ok` plus a whole-output fingerprint per gate, so deleting the agent
step would have taken the refactor pipeline from "these 3 of 500 regressed" to
"the suite is red". **The record now carries what they need.**

*Postscript, 2026-07-29.* F2 wired the prompts and did **not** need this field to
do it — it attaches the node's captured gate output instead, which is what
`failing_tests` is a reading *of*, so the prompts get more than the reading would
give them. That does not make the rung idle: it is what
`VerdictFailure::failing_tests` scopes a rework template with, on every workflow,
and it is what made the "the prompt must not read `None` as nothing-was-failing"
hazard cheap enough to design around. See §5 F2 for the wiring that shipped.

- **On the record:** `HarnessBaselineRun.failing_tests: Option<Vec<String>>`.
  **No migration** — the column is JSON, and the field is `Option` +
  `#[serde(default)]`, so a record written before this decodes cleanly and
  compares at rungs 1–2. `None` deliberately collapses four histories that must
  behave identically: green gate, extractor could not answer, extractor timed
  out, record predates the field. An empty list is never recorded, because
  "nobody read this" and "the runner named no failing test" are different claims
  and only the first is true of a spawn failure.

- **In `compare_gate`:** `NewFailures` gains a `new_failures` payload — the live
  reading minus the base's, in live order, deduplicated. **Empty means unscoped,
  never "nothing is new"**: the gate is red and *differently* red, so something
  is new by construction, and an empty list says only that nobody could name it.
  Either side silent yields empty, because with no base reading every live name
  would read as new — which is not a narrower statement but a fabricated one.

- **The determination never moves.** Rung 3 scopes a verdict; it cannot convert
  one. Rungs 1 and 2 and the classification lookup are all read *before* it, so a
  reading cannot make a regression pre-existing, or a subtracted gate a verdict,
  or an unrunnable gate anything but terminal.

- **Why an agent is allowed here** is decision 44's own boundary restated:
  decision 44 rejects agent-produced *evidence*, and this is agent-produced
  *reading of* evidence the engine already owns. Structurally, not by promise —
  the extractor spawns with an **empty tool allowlist** so it cannot run a
  command, its prompt is never offered a verdict vocabulary, and the exit status
  was decided before it was asked. It reuses `triage_harness_failure`'s
  plumbing exactly (same registry, pinned-low effort, `BUDGET_FRACTION_TRIAGE`,
  same cancellation race), so there is one cheap-agent path, not two.

- **The cost, which is where the deferral's argument was wrong.** It is *not* a
  call on every red validate. Two sites, both bounded:
  1. **measurement time** — one call per **red** gate, cached on the record and
     never re-paid, including across validate attempts;
  2. **validate** — only for a gate `GateComparison::extraction_would_scope()`
     flags: rungs 1–2 conceded (`NewFailures`, unscoped) *and* the record already
     holds names to diff against. A green suite, a first-sight regression, a
     subtracted pre-existing failure, and a run with no baseline cost nothing.

  Worst case is therefore `red gates at measurement` + `red-and-differently-red
  gates × validate attempts`, at one tool-less, two-turn, cheap-model call each.
  That is what `compare_gate` being pure and free buys: the cheap pass is what
  decides whether the expensive one is worth paying for.

- **The carrier is `VerdictFailure::failing_tests`**, which `RetryContext`
  already threads into a rework template's `{{failing_tests}}`. No parallel
  channel, and the verdict *reason* is untouched — the scope is added to the
  evidence, never substituted for it, so a reading that came back empty costs the
  reader nothing.

- **Where the code lives:** the comparison is `domain/harness_delta.rs` (pure,
  synchronous, no ports); the extraction and the two-pass escalation are
  `adapters/step_executor/failing_tests.rs`, a free function over the one port it
  needs, so *which gates cost an agent call* is assertable against a single
  double rather than an `ExecutionDriver` (AGENTS.md §3). `measure_gates`'s three
  collaborators became a `MeasurementPorts` bundle rather than a fourth argument
  plus a `too_many_arguments` allow.

  16 mutations were watched redden the new tests: rung 3 removed from
  `compare_gate`; the delta ignoring the base set; the delta neither deduplicating
  nor trimming; `extraction_would_scope` forced true and forced false; a reading
  allowed to override rung 2; `measure_gates` never extracting, extracting on
  *green* gates, and recording an empty reading as an answer; the record not
  persisting the names; a pre-rung-3 record defaulting to "the runner named
  nothing" (the fail-safe inverted); the scope never reaching the verdict; the
  union never collected from the comparisons; the cap on a reading dropped; an
  unusable reply not filtered; and the prompt offering a verdict vocabulary.

  The conformance gate grew a tenth leg
  (`a_differently_red_gate_reports_only_the_failures_that_are_new`) over a real
  shell and real git. Its fixture drives both readings out of one command — a new
  `@stub-tests` directive in the gate's own output, plus `git symbolic-ref` to
  tell the detached baseline worktree from the branch checkout — so the base names
  `alpha` and the tip names `alpha,beta` with no second commit. It asserts on the
  *step's* error message, because that is the carrier, and it asserts `alpha` is
  **absent**: naming it would be the mis-scoping failure, a ticket to fix a test
  that was already broken.

  **Not proven here, by scope:** how well a real model reads a real runner's
  output. That is unknowable from a test and is the reason the whole rung is
  built to be advisory — nothing it says can change a determination, and the
  verdict reason carries the full output either way. If it reads badly in
  practice the observable symptom is an under-scoped rework prompt, not a wrong
  pass; `extraction_would_scope` is the one place to switch it off.

### HB3 — Ecosystem detection — **[Done]**

- **Goal:** stop `detect_worktree_strategy` from producing confidently wrong
  commands, and make it emit the *right shape*. Independent of the rest — it
  reduces how often everything upstream has bad input to report on.

- **Emit named harnesses, not one mashed command.** Detection currently produces a
  single `test_command`, and for a polyglot repo that is a hand-rolled
  accumulator — `set +e; rc=0; npm test; rc=$((rc||$?)); cargo test;
  rc=$((rc||$?)); exit $rc` (there is a preflight test pinning that exact string).
  **That command exists only because there was nowhere to put multiple named
  harnesses.** It also throws away *which* ecosystem failed, which is precisely
  the attribution HB5 exists to recover.

  Once harnesses are plural and gate-selectable, detection should emit
  `{js-test: "npm test", rust-test: "cargo test"}` with both pre-ticked as gates,
  and the accumulator should be deleted rather than fixed. This is also the honest
  answer to "should there be a global harness config": the reusable knowledge is
  the ecosystem *recipe*, which lives here, not the command string, which is
  repo-specific and belongs to the project.

- **Three defects, all in `strategy.rs`:**
  1. **Root-only marker stat.** The `ECOSYSTEMS` loop stats
     `{repo_dir}/{marker}` and nothing deeper. A Tauri app whose `Cargo.toml`
     lives in `src-tauri/` therefore matches `package.json` only — the entire
     Rust half of the project is invisible to detection, and the generated
     `test_command` silently covers half the repo.
  2. **`prepare_command` is never detected.** `strategy.rs` returns
     `prepare_command: None` unconditionally. A fresh validate worktree is a
     clean `git worktree add` with no `node_modules` and no `target/`, so a
     detected `npm test` fails on a project that works fine for the human.
  3. **Watch-mode runners.** A detected `npm test` resolves to whatever the
     repo's `scripts.test` says, which is frequently a watch-mode runner that
     never exits. Before S10 that was an unkillable hang; it now terminates at
     the wall-clock cap with remediation naming watch mode, which is a far better
     failure but still a wasted ceiling's worth of wall-clock on every run.
     Detecting it up front is what removes the cost entirely.

- **Worked example — the Stratosbar project in the dev DB.** Root
  `package.json` with `"test": "vitest"` (watch mode, no `run`), `Cargo.toml`
  only under `src-tauri/`. Detection yields `npm test` alone: half the repo
  unmeasured, no install step, and a command that never returns. All three
  defects in one repo, which makes it the natural fixture.

- **Size:** medium.

- **Context:** `strategy.rs` in full (it is short); `ports/execution.rs`
  (`get_metadata`); the existing `run_all` accumulator comment, which already
  documents why every detected ecosystem must run rather than the first match.

- **Touch:** `strategy.rs` and its test module. Tests: a Tauri-layout fixture
  detects both ecosystems; a `scripts.test` naming a watch-mode runner is
  either corrected or flagged rather than emitted bare.

- **Done when:** the Stratosbar layout produces named harnesses covering both
  ecosystems, a `prepare_command`, and no watch-mode invocation — and no `rc=`
  accumulator anywhere in the output.

- **Done:** the *recipe* is a new pure `domain/ecosystem.rs` —
  `compose_commands` over a slice of `MarkerSite`, reachable from a test with
  no port double — and `strategy.rs` shrinks to gathering the evidence it
  decides from. That split is the answer to the global-harness-config
  question restated as code: the ecosystem recipe is reusable and lives in
  `domain/`, the command string it derives is repo-specific and is written to
  the project. Detection emits `{js-test, js-build, rust-test, rust-build, …}`
  with the *test* gates pre-ticked in `validation_gates`; the accumulator is
  deleted, not fixed.

  **Build gates are emitted but not pre-ticked.** A test run almost always
  builds first, so ticking both doubles the wall-clock for one extra signal —
  but §1's motivating feature had an acceptance criterion that demanded
  `cargo build` specifically. Putting them in the map makes that a checkbox in
  HB6's panel rather than a string somebody has to know how to write.

  **The scan is bounded three ways, and one `list_dir` answers each
  directory.** Root plus one level, never a dot-dir or a `SKIPPED_DIRS` entry
  (`node_modules` alone would otherwise contribute a `package.json` per
  dependency), and never more than `MAX_SCANNED_SUBDIRS` of them. `list_dir`
  rather than four `get_metadata` calls per directory because one listing
  answers markers, lockfiles and subdirectories together — over SSH that is
  the difference between a detection and a wait. A **root manifest shadows its
  own ecosystem below it**: a Cargo workspace, npm workspaces and a Go module
  all describe their members from the root, so emitting a gate per member as
  well would run one suite twice under two names. Where a root manifest is
  absent, siblings each get their own gate and the name carries the directory,
  or the map silently loses one.

  **A command below the root is wrapped `(cd dir && …)`, not `cd dir && …`.**
  These commands are chained, and a `cd` that leaks would run the second
  install in the first one's directory. The separator is `/` rather than
  `PathBuf::join` throughout, deliberately: these are addresses and shell
  fragments on the *target* machine — Linux for every remote project — so
  joining with the host's separator would emit a backslash from a Windows
  desktop and break the command everywhere it was sent.

  **Two things detection now declines to emit**, on the principle that a
  confidently wrong command is what this whole document exists to stop: a
  watch-mode script with no one-shot form (`nodemon`, or a watcher that is not
  the *last* command in the chain, where an appended `--run` would land on
  something else), and a package with no real `scripts.test` — `npm test`
  there exits 1 with "Missing script", which reaches validate wearing the
  feature's costume. `vitest` and `jest --watch` *are* corrected, since they
  have one. The one place ignorance is not read as absence is an unreadable or
  unparseable manifest, which falls back to today's `npm test`. The install
  step follows the lockfile that is actually present, because `npm ci` in a
  pnpm repository does not merely fail — it writes a lockfile that was never
  meant to exist.

  **`test_command` is set only for a single-ecosystem repo.** It is tier 3 and
  therefore unreachable whenever `validation_gates` is populated; its
  remaining job is to render `{{test_command}}` in prompts authored before
  harnesses went plural, and a polyglot repo has no honest single value for
  it.

  **One edit outside this task's stated Touch, and it was load-bearing.**
  `bootstrap_project` returns the proposal *without persisting it*, and the
  new-project wizard wrote back only the four fields its form edits — so every
  detected harness, gate and prepare command was dropped the moment the wizard
  finished. Survivable while detection produced one `test_command`; not now.
  `NewProjectView` holds the proposal whole, writes it on approval, and shows
  the detected gates read-only, because a polyglot repo's empty test-command
  field otherwise reads as "nothing was found".

  19 tests — 14 pure over `compose_commands`/`classify_test_script`, 5 in the
  adapter against a real `LocalSubprocessAdapter` and a real repository (a
  Tauri layout, a watch-mode script, the depth bound, a single-ecosystem repo,
  and one with no markers at all). Nineteen mutations were watched redden
  them: the root-only stat restored, the skip list and depth bound removed,
  `prepare_command` back to `None`, the watch-mode read made inert, a
  non-final watcher corrected anyway, an already-one-shot `vitest` corrected
  anyway, `Missing`/`Uncorrectable` emitted bare, the `rc=` accumulator
  reintroduced, root shadowing removed, sibling gate names collapsed onto one
  key, `validation_gates` never populated, the subshell dropped so a chained
  `cd` leaks, the lockfile ignored, an unparseable manifest read as "no
  tests", tier 3 handed one ecosystem's command on a polyglot repo, wrapper
  and assignment stripping removed, the adapter never reading the manifest,
  a repo with no markers guessing `npm test`, and — in `preflight.rs` — `(`
  dropped from the unresolvable set, which is what keeps `(cd` from being
  probed as a binary.

  `the_generated_polyglot_accumulator_probes_only_real_tools` pinned the
  deleted string. It was reworked rather than removed: the property it covered
  — the preflight finding the real tools inside a multi-command string
  detection emits — still holds, against the subshell and `--` forms that
  replaced it.

  **Not proven here, by scope:** the wizard change is covered by `tsc` and by
  reading. `NewProjectView` is one of the thin shells this repo's tests
  deliberately do not stand up (`App.test.tsx` says so in as many words).

### HB6 — Probe at configuration time, not only at launch

- **Goal:** the most valuable place to tell someone their `test_command` is wrong
  is the settings panel where they just typed it. HB1/HB4 catch it at launch,
  which is good; catching it where it was authored is better, and it is the same
  probe.

- **Shape:** a Tauri command wrapping `probe_configured_commands`, run against the
  project's own machine, returning a per-command resolved/missing result. The
  Strategy tab renders it inline next to each of `prepare_command`,
  `test_command`, and each `harnesses` entry.

- **Do not block a save on it.** A user may legitimately configure a command for a
  machine that is not the one they are sitting at — the remote runner especially.
  This is an indicator, not a gate; the gate stays at launch where the machine is
  known. Say which machine was probed, or the indicator is a lie on any project
  with a remote compute type.

- **This task also authors HB5's tier 2.** Each harness row gains a **"gates
  validation" checkbox** and a **user-controlled order** — cheap gates first (lint
  before integration), and since `harnesses` is a `HashMap` there is no order to
  inherit, so one must be stored. With HB6's indicator a row reads its definition,
  its health, and its role on one line:

  > `lint` → `npm run lint` · ✓ resolved on PATH · ☑ gates validation

- **Extract a `HarnessesSection`.** Those rows now carry five columns.
  `StrategyTab.tsx` is 292 lines and `ProjectSettingsContext.tsx` is 641 — already
  past the ~400 convention — so this is a component boundary, not a new tab and
  emphatically not a new *global* settings surface. Harness commands are
  repo-specific by nature (`npm run lint` vs `cargo clippy` vs `ruff check`); a
  global map would be wrong for nearly every project and would invent a
  global-vs-project shadowing question for no benefit. The reusable knowledge is
  the *ecosystem recipe*, and that belongs in HB3's detection, not in a settings
  surface.

- **Four inline hints, and do not write new copy for them.** These are the things
  nobody guesses, each of which costs hours:
  1. commands run under `bash -l -i -c` — an **interactive login** shell, which is
     the whole mise/asdf/nvm class of "but it works in my terminal";
  2. the validate worktree is a *fresh* `git worktree add` with no `node_modules`
     and no `target/` — which is **why `prepare_command` exists**;
  3. a watch-mode `npm test` consumes the entire wall-clock cap and then fails;
  4. on a remote-compute project the command runs on the **project's** machine.

  `PreflightVerdict::detail()` already carries (1) and its reproduce line, and has
  already earned its keep as the launch-blocking message. **Render that same
  string** rather than authoring a parallel copy that will drift out of agreement
  with the error message. Use **inline hints, not hover tooltips**: hover is
  undiscoverable, absent on touch, and hides exactly what should be visible.

- **Depends:** HB4 (it is that union that makes one probe cover the whole panel)
  and HB5 (tier 2 has to exist to be checked). **Size:** medium.

- **Context:** `preflight.rs`; `src-tauri/src/commands/` for the command shape;
  `src/lib/project.ts` for the typed wrapper (never `invoke()` raw);
  `src/components/settings/StrategyTab.tsx` + `ProjectSettingsContext.tsx`.

- **Touch:** those, plus the new `HarnessesSection` component. Design tokens per
  AGENTS.md §4 — emerald for resolved, ruby for missing, read from `App.css`,
  never hard-coded.

- **Done when:** typing a nonexistent binary into `test_command` shows it
  unresolved without leaving the settings panel; the panel names the machine it
  asked; and a harness ticked as a gate is run by a starter workflow that declares
  no `harness_names` of its own.

- **Done:** `probe_project_commands` in `preflight.rs` wraps HB4's probe and
  pairs each configured command with the binaries it names, through a pure
  `attribute_verdict` that is decidable with no port double. The Tauri command
  of the same name resolves *which* machine from the project's compute type —
  never from the caller — and the panel calls it through
  `probeProjectCommands` in `src/lib/project.ts`. The new
  `HarnessesSection` owns the five-column rows plus the two commands that
  belong beside them, so `test_command` moved out of the Git-isolation card:
  it is the resolution chain's fallback tier and deserves the same indicator
  as the harnesses it competes with.

  **Three shape decisions worth not re-deriving.**

  *The probe reads no repository directory.* HB1 passes `ctx.target_dir`
  because it has one; the panel does not, and a project may never have been
  provisioned on the machine it is configured for. A cwd that does not exist
  fails every probe at spawn time and reads as a missing toolchain — the exact
  false positive the module is built around avoiding — so an empty `cwd` now
  means "the adapter's default". `command -v` needs the login shell, not the
  repo.

  *Order is stored only for the selection.* `validation_gates` is the one
  ordered thing the engine reads, and an unticked harness never runs, so it
  has no position to store. The rows therefore render gates first in run
  order, then the rest alphabetically, and the arrows act on ticked rows only.

  *No copy of its own.* `PreflightVerdict::detail` travels to the panel
  verbatim, and the fresh-checkout/watch-mode sentence `baseline.rs` already
  emits became `FRESH_CHECKOUT_REMEDIATION`, read by both. The panel and the
  failure a user meets mid-run cannot drift apart because there is only one
  string.

  10 pure/wiring Rust tests and 10 frontend ones, every one watched fail:
  dropping the harness name from the attribution, letting `configured_commands`
  walk its own list, marking every binary resolved, guessing a binary from the
  raw first word, reporting a fixed machine, paraphrasing the launch-blocking
  string, a second copy of the guidance, ignoring the verdict, naming a
  repository directory, a transport failure read as a missing binary; and
  frontend-side: painting every binary resolved, dropping the machine name,
  never rendering the engine's message or its guidance, a probe that stops
  following what was typed, `validation_gates` dropped from the save payload,
  an empty list written instead of a cleared one, a stale tick re-persisted,
  reordering made inert, a deleted harness's gate surviving a re-add, and each
  of the two ways a probe could wrongly block a save.

  **Not proven here, by scope:** the third leg of the Done-when — that a
  ticked gate is *run* by a starter declaring no `harness_names` — is HB5's
  `resolve_harnesses` chain and is pinned by its conformance gate
  (`tests/conformance/harness_gates.rs`). What this task proves is that the
  selection reaches `validation_gates` in the user's order.

### HB7 — Make the verdict legible — **[Done]**

- **Goal:** the subtraction is worthless if the user cannot see it happen, and a
  terminal environment failure currently renders as an ordinary failed feature —
  so the remediation text, which is the entire payload, arrives looking like a
  stack trace.

- **Two surfaces:**
  1. **Validate's report becomes a per-harness table** — baseline vs. now, per
     gate, with excluded pre-existing failures named. This is what makes HB2c
     legible rather than magic; a subtraction the user cannot audit will not be
     trusted the first time it is wrong.
  2. **An environment failure must not look like a failed feature.** It is not the
     feature's defect and it has an action attached. `EnvironmentNotReady` already
     exists as a `NotificationKind` and is bridged to detached runs; the
     feature-level presentation should follow it, with the remediation as the
     body rather than an error string.

- **Depends:** HB5 (per-harness shape) and HB2c (the subtraction to render).
  **Size:** medium.

- **Context:** `src/components/FeatureDetail.tsx`; `NotificationBell.tsx`;
  `src/types.ts` (`EnvironmentNotReadyEvent`); AGENTS.md §4 for tokens.

- **Done when:** a run that excluded a pre-existing failure says so in the UI
  naming the test, and a terminal environment failure renders its remediation as
  the primary content rather than as an error message.

- **Done:** `HarnessGateTable` renders baseline vs. now per gate above the
  Graph|Timeline toggle — a property of the run, not of one rendering of it —
  and `EnvironmentNotReadyPanel` replaces the ruby error dump on the step that
  ended in a terminal environment failure. Both read the V37 record, which now
  exists on the frontend `Feature` type; the join and the two parsers are pure
  functions in `src/lib/harnessVerdict.ts`.

  **Where the "now" side comes from — one correction to this plan.** The
  baseline half is structured and authoritative. The other half is not stored
  per gate *anywhere*: `run_harness_first` folds it into the step's
  `error_message`, and on the all-excluded pass path the exclusion travels into
  the validate prompt only, so the report naming it is the agent's own prose
  (HB2c's own "not proven here"). So the UI reads back the two engine-authored
  strings that *are* persisted — `build_environment_message`, and
  `build_failure_reason` + `build_exclusion_note` — rather than a structured
  record. `isEnvironmentError` already parsed the first by prefix; this widens
  that, it does not introduce it. **If a later task wants this to be a
  structured read, the engine has to persist a per-gate result** (a `run_events`
  kind, or a column) — that is engine work and was deliberately out of scope
  here.

  Every parse is conservative in the same direction the record is. An
  unparseable message yields *nothing reported*, a gate the record never
  measured renders **not measured**, and a gate no step named renders **no
  failure reported** — neither is a pass. That is the one inversion decision 44
  cannot survive, and it is why the pass path still has something to show: a
  gate red at the base is named as excluded from the verdict off the *record*,
  which is the only evidence a passing run leaves behind.

  **Amber, not ruby, for the environment panel.** AGENTS.md §4 gives ruby to
  errors and failures, and an environment failure is one — but the whole task is
  that it must not read as *this feature's*. `NotificationBell` already accents
  `environment_not_ready` amber, so the panel follows the convention that
  existed rather than inventing a fifth colour. Ruby stays on the verdict, which
  is the failure the feature answers for.

  19 pure tests and 9 component ones, every one watched fail: the remediation
  truncated to its first line, the environment prefix check dropped (caught by a
  verdict whose own output quotes the labels), the exclusion note never parsed,
  the evidence scanned front-to-back, an unmeasured gate defaulting to passed,
  any red baseline gate reading as environmental (the fail-safe inverted), the
  feature payload cast instead of guarded, the baseline classification ignored,
  the environment failure not joined onto a gate, only the first failing gate
  reported; and component-side: the exclusion note dropping the commit and the
  producer, the exclusion rendered as an ordinary failure, the no-baseline state
  rendering nothing, an empty table rendering a header anyway, the remediation
  back in a monospace error string, `atBase` ignored, an empty remediation still
  rendering the "Do this" box, and the reproduce block dropped.

  **Not proven here, by scope:** that `FeatureDetail` *mounts* both. It is the
  thin shell this repo's tests deliberately do not stand up (`App.test.tsx` says
  so in as many words), so the wiring is covered by `tsc` and by reading, not by
  a test.

---

## 4. Cross-references

- **[`RELIABILITY_PLAN.md`](RELIABILITY_PLAN.md) S10–S14** — the engine-side
  silent-failure fixes in this subsystem, **all five now fixed**: the harness
  deadline and cancellation (S10, HB1's hard prerequisite — a preflight that can
  hang forever is worse than no preflight), the dropped stderr on a green
  harness (S11), the no-harness fallback wearing an "already executed" heading
  (S12), the `environment` verdict missing from the JSON menu (S13), and the
  artifacts unrecorded on a failing verdict (S14). **Do not restate them here** —
  reference them by number and let that file stay their record.

  What they do *not* do is make the project well configured; they make a
  misconfigured project fail honestly and cheaply instead of silently or
  expensively. Everything in §3 is the prevention half, and remains the reason
  this document exists.

- **[`TASKS_DAG_WORKFLOWS.md`](TASKS_DAG_WORKFLOWS.md) P4.2** — ships a
  `baseline-harness(command)` node inside the Standard starter's graph. That is
  the **in-graph complement** to HB1's workflow-independent phase, and neither
  replaces the other: the node gives a workflow author an explicit, positionable,
  artifact-producing baseline they can wire edges from; the phase protects every
  launch including the ones whose workflow has no such node. **P4.2a is HB2b's
  first producer** — the two are the same work seen from two plans, and HB2b adds
  the lazy fallback that covers the graphs P4.2a cannot reach. Both write the
  HB2a record; HB2c does not care which produced it.

- **[`PRD_DAG_WORKFLOWS.md`](PRD_DAG_WORKFLOWS.md) §7** — the source of the
  `research ∥ baseline-harness(command)` shape P4.2 implements.

- **[`EXECUTION_PARITY.md`](EXECUTION_PARITY.md)** — the D3 contract. `stdout`
  on success, `stdout + stderr` on failure, `transport:` and `timeout:` prefixes
  for the two non-verdict failure modes. This is what makes stderr handling a
  *caller's* responsibility: `steps/command.rs` wraps its command in
  `( … ) 2>&1` precisely because the port will not merge it on a zero exit, and
  any new command execution added by these tasks owes the same wrap.

---

## Status

| Task | Title | Size | Status |
|---|---|---|---|
| HB1 | Bootstrap preflight phase | medium | ✅ |
| HB4 | Probe every configured command | small | ✅ |
| HB5 | Named harnesses become an ordered list | medium | ✅ |
| HB2a | Baseline record: shape & storage | medium | ✅ |
| HB2b | Measure the baseline: node + lazy fallback | large | ✅ |
| HB2c | Subtraction & classification | medium-large | ✅ |
| HB6 | Probe at configuration time | small-medium | ✅ |
| HB7 | Make the verdict legible | medium | ✅ |
| HB3 | Ecosystem detection | medium | ✅ |
| HB8 | An environmental red baseline is not subtracted | small | ✅ |
| HB9 | Halt at the baseline node on an unrunnable gate | small | ✅ |
| HB2c rung 3 | Per-test names on the record and in `compare_gate` | medium | ✅ |

HB8 and HB9 were not in the original decomposition; each was found by the task
after it and is recorded in its own section above. HB2c's third rung was
deliberately deferred by that task and built later, once
[F2](#f2--refactorjson-had-two-baselines--done-2026-07-29) made it load-bearing; it is
recorded inside HB2c rather than as a task of its own.

---

## 5. Open follow-ups

F1 blocks nothing and is still open. F2 is **done** — its record is kept below
rather than deleted, because the deletion it authorised cost one capability that
nothing else in this document records.

### F1 — per-gate results have no structured home

`run_harness_first` folds per-gate results into the step's `error_message` as a
**string**, and on the all-excluded *pass* path the exclusion reaches the validate
prompt only — the report naming it is the agent's prose. There is no run-event
kind and no column for it. So HB7's UI reads the two engine-authored strings that
*are* persisted, by prefix (`build_environment_message`, `build_exclusion_note`).

This is not new — `isEnvironmentError` already parsed by prefix, so HB7 widened an
existing seam rather than opening one — and the parse is pinned by tests. But it
means **a wording change in an engine message can silently break the UI**, which
is the coupling this codebase otherwise avoids. The fix is a structured per-gate
result on the step row (the `harness_baseline_json` precedent applies: a JSON
column needs no migration). Worth doing before anything else reads these strings.

### F2 — `refactor.json` had two baselines — **[Done, 2026-07-29]**

`refactor.json` carried both the `s-baseline-harness` command node and its
original `s-baseline` **agent** step, which ran the suite and wrote prose. HB2b
deliberately left the agent step alone because the plan called its fate a user
decision, not an assumption.

The orchestrator measurement is cheaper (zero tokens) and trustworthy in a way an
agent reading its own test run is not — that is decision 44's whole argument. What
the agent step still had is narrative value. **The user decided to keep the
measurement and delete the agent step**, and it is deleted: the graph is now
`s-baseline-harness → s-analyse → …`, and the P0.2 snapshot plus the canvas v2
fixture were regenerated. The other six starters are byte-identical.

**How the three consumers reach the measurement.** `s-analyse` (twice — its
prompt and its `rework_prompt_template`) and `s-regression` used to read
`artifacts/s-baseline.md`. They now read the measurement two ways, and the split
matters:

- **`[attached — from s-baseline-harness]`** — the node's own artifacts, which are
  the gates' captured stdout+stderr at the base commit (`store_baseline_output`,
  one per measured gate). This is the *evidence*, and it is why widening
  `{{harness_baseline}}` turned out not to be necessary: the record's
  `failing_tests` is a *reading of* this output, so attaching the output gives the
  consumers strictly more than the reading would, at no engine cost. It travels the
  path every other starter attachment travels (`resolve_attached_artifacts` →
  `materialize_external_artifact_paths`), so it lands inside the agent's worktree
  fence as a path manifest rather than as megabytes of inlined log.
- **`{{harness_baseline}}`** — the record's own per-gate status, which is the only
  thing that can say a gate was **not measured**. An attached output cannot say
  that: absence of a file is indistinguishable from a store failure.

**Where the asymmetry landed.** The agent step listed **passing** tests; the record
names only failing ones, because a measurement of what a green suite contains is
not something the engine takes. `s-regression`'s "for every test listed as
PASSING… if it now FAILS" is therefore rephrased as its contrapositive — *a test
failing now that was not failing at the base is a regression*. The two are not
equivalent, and each difference was decided rather than absorbed:

1. **A test that did not exist at the base and fails now.** The old rule could not
   see it; the new one flags it. **Kept as a regression, deliberately** — a
   refactor preserves behaviour and must not leave a new failing test behind. The
   report labels it `ABSENT` in the Baseline column so the rework prompt can tell
   it from a genuine regression.
2. **A test that passed at the base and is now absent** — deleted or renamed by the
   refactor. This is the one the contrapositive genuinely loses: nothing fails, so
   nothing in the comparison can find it, and *a refactor silently deleting a test
   is exactly the behaviour change this workflow exists to catch.* It is recovered,
   but **not by the record** — the record has no passing side to diff against.
   `s-regression` compares the test identifiers the *attached baseline output*
   names against the ones its own run names, and reports a `DELETED` verdict that
   fails the check. Where the runner does not enumerate individual tests, it falls
   back to comparing totals and says in the report that the comparison was
   count-level. **State the residue plainly: this leg is a comparison of two
   outputs by an agent, not an engine measurement, so it is weaker than the other
   two rules on this page.** It cannot manufacture a pass — every branch of it
   only ever *adds* a failure — but it can miss one. The engine-side fix, if this
   is ever wanted as a measurement, is a passing-side reading on the record, which
   is a rung-3-shaped change and was out of scope here.
3. **`failing_tests: None` vs. an empty list.** Rung 3 made these different claims
   on purpose, and the prompt must never read `None` as "nothing was failing". It
   cannot: the prompt never sees `failing_tests` at all. It sees the *output*
   (which is either attached or not) and the record's per-gate status (which says
   measured-and-passed, measured-and-failed, or **not measured**). The prompt
   spells out that a not-measured gate is `UNKNOWN`, never `PASS` — the same
   "absent is not green" invariant `HarnessBaseline::harness` enforces in code.

**The `NO_HARNESS` path survives**, driven by the record rather than by an agent's
prose verdict line: with no validation gate configured, `render_harness_briefing`
emits its "the harness can prove NOTHING" block and `s-regression` skips to
`VERDICT: ALL CLEAR`. Both sides of that coupling are pinned by one test
(`the_refactor_no_harness_skip_branch_is_keyed_on_what_the_engine_renders`), so a
reword of either strands the branch loudly instead of silently.

**No engine change.** `render_harness_briefing` was not widened, `verifier.rs` and
`harness_baseline.rs` are untouched, and the three new tests live in
`node_lint.rs`'s starter-pack module beside `every_migrated_starter_lints_clean`.
The one that generalises past this task is
`every_starter_attachment_names_exactly_one_step_it_declares`: a prompt referencing
a deleted step renders as "(Artifact '…' not found or not yet generated)" mid-run,
and nothing in the lint gate could see it, because the reference lives in prompt
*text*.

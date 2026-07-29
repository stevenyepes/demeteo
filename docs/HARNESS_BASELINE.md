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

**Migrations:** V35 `sequence_checkpoint_anchor` and V36
`sequence_checkpoint_produced` are built — the next free number is **V37**.

**Dependency order:**

```
HB1 ─▶ HB4 ─▶ HB6
        │
HB5 ────┴─▶ HB2a ─▶ HB2b ─▶ HB2c ─▶ HB7
```

`HB3` is independent of everything. **HB1 is done.** Work them in that order:
HB4 is small and unblocks HB6; HB5 fixes the *shape* of what gets recorded
before HB2a fixes its storage.

### Key code coordinates (shared reference — don't re-discover these)

> Line numbers drift. Re-verify with `git grep` before relying on one.

| What | Where |
|---|---|
| Bootstrap tail + phase vocabulary | `crates/demeteo-core/src/adapters/step_executor/impl_traits/mod.rs` (`bootstrap_phase`, `run_bootstrap_tail_inner`) |
| Harness execution primitive | `.../step_executor/driver/verifier.rs` (`run_harness_first`, `harness_shell_options`) |
| Preflight probe (HB1, built) | `.../step_executor/preflight.rs` |
| Ecosystem detection | `.../adapters/worktree/git_ops/strategy.rs` (`detect_worktree_strategy`) |
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
rework loop for a defect it did not introduce. `refactor.json` has an
`s-baseline` step that addresses this — but it is an `agent` step reading its
own test run and writing prose, not an engine measurement, and the standard
pipeline has no equivalent at all.

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
  probes guard *every* launch whatever workflow was chosen; the node guards the
  starters and produces the durable baseline HB2 consumes.

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

### HB5 — Named harnesses become an ordered list

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

- **Run all by default.** If lint and unit both fail the user wants both; stopping
  at the first turns one wasted cycle into two, which is the thing this document
  exists to prevent. An opt-in `stop_on_first_failure` is the right escape hatch
  for an expensive tail suite — add it when someone asks, not before.

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

### HB2a — The baseline record: shape and storage

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

- **Depends:** HB2a. **Size:** large — it is the only task here that adds a git
  primitive.

- **Context:** HB2a's record; `steps/command.rs` in full; `adapters/worktree/git_ops/`
  (`provision_subtask_worktree`, `scope.rs`); `driver/verifier.rs`
  `run_harness_first`; `src-tauri/workflows/standard-feature-pipeline.json`.

- **Done when:** a run of a *custom* workflow with no baseline node still subtracts
  a pre-existing failure. That is the leg that proves the fallback, and adding the
  node to the fixture cannot fake it.

### HB2c — Subtraction and classification

- **Goal:** evaluate the retry rule from §2 as a table the engine can compute,
  replacing today's "any non-zero exit is this feature's verdict".

  | Baseline | Now | Determination |
  |---|---|---|
  | binary unresolvable | — | blocked at launch, terminal (HB1/HB4) |
  | `prepare_command` fails | — | terminal `Environment` — the worktree can never be made runnable |
  | red, same fingerprint | red, same fingerprint | **pre-existing** — not this feature; pass, with the exclusion named in the report |
  | green | red | **regression** — `Verdict`, retry. No agent needed to know this. |
  | red | red, *different* | new failures atop pre-existing — `Verdict`, retry, scoped to the delta |
  | — | 127 / transport / timeout | terminal `Environment` (already built) |

- **Granularity ladder, cheapest first** — escalate only when the coarser level is
  ambiguous: exit-code equality → `normalize_failure_fingerprint` equality
  (already built) → per-test-name extraction. The third rung is the honest place
  for an agent: reading two outputs and answering "which failures in B are absent
  from A" is comprehension, not judgment, and it never touches an exit code, so it
  cannot manufacture a pass.

- **Narrow C6, don't delete it.** See §2. `classify_harness_failure` consults the
  triage agent only for the residue: green at baseline, red now, and the failure
  looks environmental. Its fail-safe fallback to `Verdict` must survive intact.

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

### HB3 — Ecosystem detection

- **Goal:** stop `detect_worktree_strategy` from producing confidently wrong
  commands. Independent of HB1/HB2 — it reduces how often the preflight has bad
  input to report on.

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

- **Done when:** the Stratosbar layout produces a command covering both
  ecosystems, a `prepare_command`, and no watch-mode invocation.

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

- **Depends:** HB4 (it is that union that makes one probe cover the whole panel).
  **Size:** small-to-medium — one command, one typed wrapper, one panel section.

- **Context:** `preflight.rs`; `src-tauri/src/commands/` for the command shape;
  `src/lib/project.ts` for the typed wrapper (never `invoke()` raw);
  `src/components/settings/StrategyTab.tsx` + `ProjectSettingsContext.tsx`.

- **Touch:** those. Design tokens per AGENTS.md §4 — emerald for resolved, ruby
  for missing, read from `App.css`, never hard-coded.

- **Done when:** typing a nonexistent binary into `test_command` shows it
  unresolved without leaving the settings panel, and the panel names the machine
  it asked.

### HB7 — Make the verdict legible

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
| HB5 | Named harnesses become an ordered list | medium | ☐ |
| HB2a | Baseline record: shape & storage | medium | ☐ |
| HB2b | Measure the baseline: node + lazy fallback | large | ☐ |
| HB2c | Subtraction & classification | medium-large | ☐ |
| HB6 | Probe at configuration time | small-medium | ☐ |
| HB7 | Make the verdict legible | medium | ☐ |
| HB3 | Ecosystem detection | medium | ☐ |

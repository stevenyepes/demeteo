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

**Dependency order:** `HB1 ─▶ HB2`. `HB3` is independent. **HB1 is done**; HB2 and HB3 remain.

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

## 2. Tasks

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

### HB2 — Persist the baseline and feed it forward

- **Goal:** make the preflight worth more than a warning. A measurement nobody
  reads changes no outcome; HB1 alone would still let `s-spec` write criteria
  against commands that never run and still let `s-validate` blame the feature
  for a base-branch failure.

- **Three consumers, in value order:**
  1. **`s-validate` subtracts pre-existing failures.** A test that was already
     red at baseline is not this feature's defect. This is the one that would
     have saved the $14.63 cycle above.
  2. **`s-spec` is told factually what the harness can prove.** The prompt
     already handles a blank command (see the fix in `2257ffb`); what it lacks
     is the *positive* statement of what the configured command actually
     covered on this repo. That is what stops criteria being phrased against
     commands the harness never runs.
  3. **`not_configured` reaches `environment` up front.** Today the validate
     step can only discover an unprovable criterion after the implement budget
     is gone, and then only if the agent picks the `environment` verdict over
     `fail`. With a baseline saying "no harness configured", that decision is
     available at spec time.

- **Storage decision required — do not pick one silently.** Two candidates,
  and the choice is load-bearing:
  - a column on `features` (next free migration is **V37**) — durable, queryable,
    survives the run, one row per feature, but adds schema for something that
    may be better modelled as an event;
  - a `run_events` record — no migration, already mirrored to remote and
    detached runs by construction, replayable, but read-back means scanning
    the log rather than a lookup.

  Whichever is chosen, record the reasoning in `docs/DECISIONS.md` — this is
  the kind of thing that is invisible in the code afterwards.

- **Prior art to supersede, not duplicate:** `refactor.json`'s `s-baseline`
  does consumer (1) by hand — an agent runs the suite, writes a prose baseline
  artifact, and a later step reads it. Doing it in the orchestrator is both
  free (the command already ran in HB1) and trustworthy (a measurement, not a
  reading). Once HB2 lands, `s-baseline`'s continued existence in `refactor.json`
  needs a decision: keep it for its narrative value, or delete it as redundant.

- **Depends:** HB1. **Size:** medium-to-large — touches prompt context and the
  validate verdict path.

- **Context:** §1 above; HB1's output shape; `driver/verifier.rs`
  (`run_harness_first`, the verdict parse); `steps/agent/mod.rs` (harness
  section injection); `setup.rs` (`build_base_ctx` — where a
  `{{harness_baseline}}` placeholder would bind);
  `src-tauri/workflows/standard-feature-pipeline.json` (`s-spec`, `s-validate`).

- **Touch:** the storage the decision picks, `setup.rs`, `driver/verifier.rs`,
  the two starter prompts. Tests: a test red at baseline and red after the
  feature does **not** produce a verdict failure; a test green at baseline and
  red after **does**.

- **Done when:** a run against a repo with a known pre-existing failure reaches
  `s-critic` instead of looping `s-tickets`, and the validate report names the
  pre-existing failure as excluded.

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

---

## 3. Cross-references

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
  expensively. HB1–HB3 are the prevention half, and remain the reason this
  document exists.

- **[`TASKS_DAG_WORKFLOWS.md`](TASKS_DAG_WORKFLOWS.md) P4.2** — ships a
  `baseline-harness(command)` node inside the Standard starter's graph. That is
  the **in-graph complement** to HB1's workflow-independent phase, and neither
  replaces the other: the node gives a workflow author an explicit, positionable,
  artifact-producing baseline they can wire edges from; the phase protects every
  launch including the ones whose workflow has no such node. HB2's consumers
  should read the same baseline shape whichever produced it.

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

| Task | Title | Status |
|---|---|---|
| HB1 | Bootstrap preflight phase | ✅ |
| HB2 | Persist the baseline & feed it forward | ☐ |
| HB3 | Ecosystem detection | ☐ |

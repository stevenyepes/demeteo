# Refactor: `crates/demeteo-core/src/adapters/step_executor`

> Worked instance of [`REFACTOR_FEATURE_TEMPLATE.md`](REFACTOR_FEATURE_TEMPLATE.md).
> Paste the body below into a Demeteo Feature description. Re-verify the grounding
> numbers before you start — they were measured 2026-07-30.

---

## Outcome

`adapters/step_executor` does the same thing it does today, observably identically, but
its policy is reachable from a test without an `ExecutionDriver`, and no file in it is
large enough to hide a decision. Nothing about the product changes — a user could not
tell this shipped.

**This is a behaviour-preserving refactor.** If you find a bug while working, do not fix
it: record it in the final report as a follow-up. A refactor commit that also changes
behaviour is unreviewable, because the reviewer can no longer use "the diff moves code"
as the safety argument.

## Grounding facts (verified 2026-07-30)

Checked against the working tree. If one is wrong, say so in your first report rather
than working around it.

**Source, 22 621 LOC across the module. Files over 400 lines:**

| file | LOC |
|---|---|
| `driver/verifier.rs` | 2679 |
| `steps/agent/mod.rs` | 1245 |
| `impl_traits/mod.rs` | 1114 |
| `baseline.rs` | 889 |
| `artifacts/attached.rs` | 847 |
| `steps/command.rs` | 754 |
| `steps/gate.rs` | 642 |
| `sync.rs` | 594 |
| `artifacts/declared.rs` | 576 |
| `steps/finalize/mod.rs` | 542 |
| `steps/sequence/mod.rs` | 516 |
| `preflight.rs` | 515 |
| `impl_traits/execution_context.rs` | 475 |
| `steps/sequence/plan.rs` | 473 |
| `registry.rs` | 456 |

**Tests, 10 934 LOC. Files over 400 lines:**

| file | LOC |
|---|---|
| `tests/e2e/step_executor.rs` | 1843 |
| `tests/infrastructure/step_executor/artifacts/attached.rs` | 913 |
| `tests/infrastructure/step_executor/preflight_tests.rs` | 841 |
| `tests/infrastructure/step_executor/artifacts/declared.rs` | 824 |
| `tests/infrastructure/step_executor/baseline_tests.rs` | 784 |
| `tests/infrastructure/step_executor/verifier/harness_outcome_tests.rs` | 495 |
| `tests/adapters/step_executor/scheduler_tests.rs` | 440 |
| `tests/infrastructure/step_executor/gate_redirect_target.rs` | 437 |

- **17** `#[allow(clippy::too_many_arguments)]` sites, spread over 15 files: `driver/failure.rs`,
  `driver/run_loop/attempt.rs`, `driver/run_loop/dispatch.rs`, `driver/verifier.rs`,
  `mod.rs`, `setup.rs`, `steps/agent/{artifacts,mod,spawn}.rs`, `steps/conflict_pass.rs`,
  `steps/finalize/{context,turn}.rs`, `steps/gate.rs`, `steps/sync.rs`, `updates.rs`.
- **2** `#[ignore]`d tests, both in `tests/infrastructure/step_executor/driver_watchdog.rs`
  (:123, :130), both reasoned `"requires a full ExecutionDriver fixture; see module
  docs"`. These are the module conceding the problem in writing. They are targets.
- `ExecutionDriver` (`driver.rs:130`) carries **18 port/service `Arc`s** plus ~10 more
  fields of pre-computed setup. Any test that constructs one is stubbing eighteen ports
  to exercise a function that reads one or two.

**Prior art — read this before planning anything.** `steps/sequence` was 1818 lines and
went through exactly this refactor. Its policy now lives in `domain/sequence/`
(`checkpoint`, `outcome`, `progress`, `sha`, `tasks`) and the adapter is 516 lines of
choreography. Read `crates/demeteo-core/src/domain/sequence/mod.rs`'s module doc header
— it states the rule this Feature is applying, in the words the repo already uses.
`domain/verifier.rs` and `domain/harness_delta.rs` are the same pattern applied to two
other decisions.

## Non-goals — out of scope

- **No behaviour changes.** No bug fixes, no perf work, no error-message rewording, no
  new logging, no changed retry/timeout numbers, no changed prompt text.
- **No new dependencies** (npm or cargo) — AGENTS.md §6 makes that a human Gate.
- **No DB migrations**, no column or file renames under `migrations/`.
- **No changes to `ports/step_executor.rs`** or any other port trait's name, signature,
  or semantics. If one genuinely must change, that is a separate Feature.
- **No changes to `src-tauri/capabilities/`**, agent spawn logic, or
  `OPENCODE_PERMISSION` construction — all human Gates.
- **No frontend changes.**
- **No reformatting sweeps.** `cargo fmt` output only.
- **Do not touch `steps/sequence` / `domain/sequence`.** It has already been through
  this; it is the reference, not the workload.

## Invariants that must not move

AGENTS.md §2 — no approved workaround. The ones this work will run into:

- **`ExecutionPort` is one behavioural contract, satisfied identically by local
  subprocess, desktop-over-SSH, and `demeteo-runner`.** Never branch on transport in
  calling code. If a refactor leaves local and SSH paths structurally different, the
  refactor is wrong. The contract is specified in the trait's own rustdoc
  (`ports/execution.rs`) → `docs/EXECUTION_PARITY.md`. **This module is the single place
  most able to break that**, which is why the conformance gates below are mandatory here.
- **Never bypass `PermissionPolicyPort`** when spawning agent processes, and never widen
  either fence: the chmod fence in `adapters/worktree/git_ops/scope.rs`, or the harness's
  own, which varies by harness and is declared by `PathContainment`.
- The compiled `PermissionProfile` is complete and uses only `allow`/`deny`, never `ask`.
- **Secrets live in the OS keyring only.** Moving code must not move a credential into a
  log line or a struct that gets serialised.
- **Never mutate a harness's own persisted config.** Per-invocation flags/env only.

## How to split it — the seam rule

Line count is the symptom, not the target. `driver/verifier.rs` split into seven
380-line files that each still take `&self` on `ExecutionDriver` has accomplished
nothing. Cut along these seams, in this order of preference:

1. **Policy out of `async fn`.** A `match` that decides *what should happen* — as
   opposed to performing it — belongs in `domain/`, synchronous and total: it takes what
   the adapter observed and returns what should happen. `domain/` has no `async fn`
   anywhere in it, and that is what keeps the boundary honest; policy that needs to
   `.await` cannot accidentally end up there. The adapter keeps the choreography —
   provisioning, probing, persisting, emitting. `run_harness_first` in `driver/verifier.rs`
   already does this correctly for *which* harnesses gate a step (delegating to
   `domain::verifier::resolve_harnesses`) and its rustdoc says so; the rest of that file
   is where the pattern was not carried through.
2. **A free function over the one port it needs**, not a method on `ExecutionDriver`. If
   a function reads only `projects`, it takes `&dyn ProjectRepository`. This is the
   highest-value move in this module — it is what makes the code reachable from a test
   at all, and what un-`#[ignore]`s `driver_watchdog.rs`.
3. **Bundle parameters that travel together into a named struct**, then delete the
   `#[allow(clippy::too_many_arguments)]`. Seventeen of them is the module telling you
   where the unnamed concepts are. `baseline.rs:115` and `driver/verifier.rs:1543`
   already contain written-out reasoning about this; read those comments — they name the
   bundles someone already identified and did not extract.
4. **Extract a stage when a module passes ~400 LOC *of code*.** Doc comments do not
   count toward it — they are the part worth keeping. Carry them with the code they
   explain.

**Restraint on ports.** "Apply ports and adapters" does not mean a trait per seam. A new
port is justified only when there is a real I/O boundary *and* a plausible second
implementation (as `ExecutionPort` has three). Otherwise extract a pure function — a
one-impl trait added to enable a mock pays no rent.

**Naming.** No `helpers.rs`, `utils.rs`, `common.rs`, `misc.rs`. Every new module is
named for the decision or stage it owns. If you cannot name it, you have not found the
seam — say so instead of inventing a bucket.

**Suggested order** — largest blast radius first while the diff is still small, and stop
after each so it can be reviewed:

1. `driver/verifier.rs` (2679) — verdict/triage/retry classification is policy; harness
   execution is choreography.
2. `steps/agent/mod.rs` (1245) and `impl_traits/mod.rs` (1114).
3. `baseline.rs` (889) — the two comments cited above suggest the author already knew.
4. `artifacts/{attached,declared}.rs` (847 / 576), with their oversized test files.
5. `steps/{command,gate}.rs` (754 / 642), `sync.rs` (594), `steps/finalize/mod.rs` (542).

## Comment policy

The governing rule is **AGENTS.md §3 "Comments"** — read it; it is the standard, and this
section only says what a *refactor* adds to it. In particular: value is measured in
non-recoverable information, never length, and **you must not trim or cap a comment by
size alone.** Concretely, in this module, do not flatten:

- `run_harness_first`'s header in `driver/verifier.rs` — the sole record of HB5 (why
  every resolved harness runs even after one fails, and why stopping early turns one
  rework cycle into two), plus why `HarnessOutcome` is a type and not a rendered string.
- `domain/sequence/mod.rs`'s header — the repo's own statement of the rule this Feature
  applies. It is the standard to hold every other header to.
- The `driver.rs` field comments explaining why `notifications`, `attachments`,
  `app_settings`, `subtask_runs`, and `mr_publisher` are separate ports.

What this Feature adds:

- **Verify before you keep.** A move is when staleness surfaces, and this module has
  been through several reliability passes. If a comment names a file, function, or flag,
  confirm it still exists — the next agent will act on it if it is wrong.
- **Comments move with their code.** One left behind, now above something it does not
  describe, is the worst outcome of a refactor: it reads as authoritative.
- **Delete on sight** the categories AGENTS.md §3 lists as review triggers, in files you
  are already touching: restatement, change narration, ticket echoes, play-by-play,
  stdlib explanation. Also commented-out code — git has it.
- **Comment work is its own commit** (`docs(exec): …`), never mixed into a structural
  move. A commit that both relocates a function and rewrites its comments cannot be
  reviewed as a move, which was the entire safety argument for doing this as a refactor.
- **Only files this Feature already touches.** No module-wide comment sweep.
- **Never delete a comment you do not understand.** Flag it. "I could not tell whether
  this still applies" is useful; guessing is not.
- **A `TODO` is not noise.** Either still true — keep it, with a scope — or dead, in
  which case say so in the report rather than silently dropping it. The module has
  exactly one today.
- If deleting a comment makes the code harder to follow, the fix is a better *name*.

## Testing strategy

The test tree mirrors the source split, and gets the same treatment — the oversized test
files above are the same failure, and they hide it better.

- **Pure policy → `tests/domain/verifier/`, `tests/domain/…`** — one file per decision,
  no port doubles, no async runtime, milliseconds to run. These should be the majority
  of the new tests. `tests/domain/{harness_baseline,harness_delta}.rs` show the shape.
- **Choreography → `tests/infrastructure/step_executor/…`**, against narrow doubles.
- **Never construct an `ExecutionDriver` in a test.** It carries eighteen ports the code
  under test does not read. When adapter code is unreachable from a test, the fix is to
  make it a free function over the one port it needs — not to stub the other seventeen.
  The two `#[ignore]`d watchdog tests are the acceptance criterion for this: un-ignore
  them, or explain in the report why the seam did not reach them.
- **A double that answers every call successfully is not a double.** The e2e `FakeExec`
  returns `Ok("")` for every command, so anything *reading* git's output was being
  asserted against a default rather than an answer. New doubles must error on any call
  they were not explicitly told to answer.
- **Moved tests must be proven to still bind.** For each moved or rewritten test file,
  break the code it covers, confirm that test — and ideally only that test — goes red,
  then revert. State in the report which files you did this for.
- **Do not weaken an assertion to make a moved test pass.** If a test only passes after
  its assertion changes, the refactor changed behaviour. Stop and report it.
- Split any test file over ~400 LOC along the same seams as the source. `tests/e2e/step_executor.rs`
  (1843) is the worst; it is also the one whose `FakeExec` masks parity drift, so treat
  its split as an opportunity to make its doubles strict.

## Definition of done — verification

```bash
npm run checks:code        # inside a Demeteo run — same gates, no commitlint
npm run checks             # locally / before push — adds commitlint on origin/master..HEAD
```

**`npm run checks` does not run the parity gates**, and the e2e suite will not catch a
local/remote divergence — it drives a per-test `FakeExec` that passes while masking
exactly the drift these suites exist to find. This module is the step executor, so both
are mandatory. Both need Docker:

```bash
crates/demeteo-core/tests/conformance/run-ssh-conformance.sh       # same exec_contract, local vs loopback sshd
crates/demeteo-core/tests/conformance/run-topology-conformance.sh  # topology equivalence, local + SSH
```

Green locally and in CI does **not** prove macOS or Windows compiles — PR checks run on
`ubuntu-22.04` only, and that breakage surfaces after merge. If you touched host-side
paths, shells, or process handling, reason about all three targets and say which ones
you could not verify.

## Sequencing and commits

One seam per commit. `npm run checks:code` green at every commit, not just at the end —
a refactor whose intermediate states do not build cannot be bisected, which is the one
thing a refactor series is for.

```
refactor(exec): <imperative subject, ≤72 chars, no trailing period>
```

`refactor` produces no version bump, which is correct — a bump would be a lie about
behaviour changing. **The trap:** `subject-case` rejects a subject starting with a
capitalised token. `refactor(exec): HarnessOutcome into domain` fails;
`refactor(exec): move harness outcome policy into domain` passes. Verify with
`echo "<message>" | npx commitlint`.

## Report this back

Evidence, not "done":

| | before | after |
|---|---|---|
| source files > 400 LOC | 15 | |
| largest source file | 2679 | |
| test files > 400 LOC | 8 | |
| largest test file | 1843 | |
| `#[allow(clippy::too_many_arguments)]` | 17 | |
| `#[ignore]`d tests | 2 | |
| tests reaching step-executor policy with zero port doubles | ~0 | |

Plus, in prose:

- The seam list: each new module, what decision or stage it owns, why that is a seam.
- Which moved tests you watched fail, and how you broke them.
- Both conformance suites, with results.
- Comments: roughly how many deleted vs compressed, every comment you could not judge
  and left alone, and every `TODO`/`FIXME` you found dead. Name any decision-carrying
  header you compressed, so a reviewer can check the decision survived.
- Any behaviour change you were forced into, with the reason. ("None" is expected; if it
  is not, that is the most important line in the report.)
- Follow-ups deliberately skipped, including any bug you found and left alone.

## Failure modes — these have all happened here before

- Splitting by line count with `ExecutionDriver` intact. File count improves; nothing
  becomes testable.
- Making an item `pub(crate)` purely so a test can reach it, instead of moving it to
  where the caller legitimately lives.
- Adding a trait with exactly one implementation to enable a mock.
- Carrying `#[allow(clippy::too_many_arguments)]` along to the new file — the smell
  travelling with the code instead of being removed by the move.
- Using the comment policy as cover for deleting rationale. It targets restatement, not
  reasoning. In this module the prose is frequently the only record of *why* —
  `run_harness_first`'s header explains HB5 and that reasoning exists nowhere else.
  Shortening a header until the decision is gone is a deletion wearing a smaller diff.
- Leaving a comment behind when its code moves, so it sits above something it no longer
  describes. Worse than deleting it, because it reads as authoritative.
- Declaring done on `cargo test` alone. "`cargo test` passed" is not "CI is green".

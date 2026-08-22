# Refactor Feature — prompt template

> **How to use this file.** Copy everything below the `─── CUT ───` line into a new
> Demeteo Feature description, then fill every `{{PLACEHOLDER}}`. Sections marked
> **(required)** are load-bearing: a refactor prompt without them produces a diff no
> reviewer can accept, because nothing in it says what *must not* change.
>
> Fill the **Grounding facts** section by actually running the commands in it before
> you start the Feature — pasted numbers that are already stale teach the agent that
> the prompt is fiction. Everything else is boilerplate that survives verbatim.
>
> Worked example: [`refactor-step-executor.md`](refactor-step-executor.md).

─── CUT ───

# Refactor: `{{MODULE_PATH}}`

## Outcome (required)

`{{MODULE_PATH}}` does the same thing it does today, observably identically, but its
policy is reachable from a test without standing up an adapter, and no file in it is
large enough to hide a decision. Nothing about the product changes — a user could not
tell this shipped.

**This is a behaviour-preserving refactor.** If you find a bug while working, do not
fix it: record it in the final report as a follow-up. A refactor commit that also
changes behaviour is unreviewable, because the reviewer can no longer use "the diff
moves code" as the safety argument.

## Grounding facts (verified {{DATE}}) (required)

Everything here was checked against the working tree; if you find one is wrong, say so
in your first report rather than working around it.

- Files over 400 LOC in scope:
  {{PASTE: find <path> -name '*.rs' | xargs wc -l | sort -rn | head -N}}
- Test files that cover it, with sizes:
  {{PASTE: the corresponding tests/ paths and line counts}}
- `#[allow(clippy::too_many_arguments)]` sites in scope: {{N}} — at {{FILE:LINE list}}
- `#[ignore]`d tests in scope: {{N}} — at {{FILE:LINE list, with the stated reason}}
- Ports this module actually consumes: {{list from the struct fields / fn signatures}}
- Prior art in this repo for the split you are being asked to do: {{PATH}} — read its
  module doc header before you plan anything. {{ONE LINE on what that refactor did}}

## Non-goals — out of scope (required)

Doing any of these turns a mechanical review into an argument. Don't.

- **No behaviour changes.** No bug fixes, no perf work, no error-message rewording, no
  new logging, no changed retry/timeout numbers.
- **No new dependencies** (npm or cargo). AGENTS.md §6 makes that a human Gate; if you
  believe one is needed, stop and ask.
- **No DB migrations**, no column or file renames under `migrations/`.
- **No public API churn** beyond what the split requires. Port traits in `ports/` keep
  their names, signatures, and semantics. If a trait genuinely must change, that is a
  separate Feature.
- **No frontend changes** unless the module is the frontend.
- **No reformatting sweeps.** `cargo fmt` output only; do not re-wrap comments, reorder
  imports by hand, or "tidy" files you are not otherwise touching. Noise buries signal.
- {{ANY MODULE-SPECIFIC EXCLUSION}}

## Invariants that must not move (required)

AGENTS.md §2 lists these and says they have no approved workaround. The ones this work
can plausibly touch:

- **`ExecutionPort` is one contract, satisfied identically by every transport.** Never
  branch on transport in calling code. If a refactor makes local and SSH paths diverge
  even in structure, the refactor is wrong. → `docs/EXECUTION_PARITY.md`
- **Never bypass `PermissionPolicyPort`** when spawning agent processes, and never widen
  either fence: the chmod fence in `adapters/worktree/git_ops/scope.rs`, or the harness's
  own, which varies by harness and is declared by `PathContainment`.
- **Secrets stay in the OS keyring.** Moving code must not move a secret into a log line
  or a struct that gets serialised.
- **Never mutate a harness's own persisted config.** Per-invocation flags/env only.
- {{ANY MODULE-SPECIFIC INVARIANT}}

## How to split it — the seam rule (required)

Line count is the symptom, not the target. Splitting an 1800-line file into five
360-line files that each still need twenty port doubles to test has accomplished
nothing. Cut along these seams, in this order of preference:

1. **Policy out of `async fn`.** A `match` that decides *what should happen* — as
   opposed to performing it — belongs in `domain/`, synchronous and total: it takes what
   the adapter observed and returns what should happen. `domain/` has no `async fn`
   anywhere in it, and that is what keeps the boundary honest; policy that needs to
   `.await` cannot accidentally end up there. The adapter keeps the choreography —
   provisioning, probing, persisting, emitting.
2. **A free function over the one port it needs**, not a method on the god-struct. If a
   function reads one repository, it takes `&dyn ThatRepo`, not `&self`. This is the
   single highest-value move: it is what makes the code reachable from a test at all.
3. **Bundle parameters that travel together into a named struct**, then delete the
   `#[allow(clippy::too_many_arguments)]`. The attribute is a review trigger, not a fix.
4. **Extract a stage when a module passes ~400 LOC *of code*.** Doc comments do not
   count toward it — they are the part worth keeping. Carry them with the code they
   explain; a doc comment that survives the move but no longer sits above its function
   is worse than none.

**Restraint on ports.** "Apply ports and adapters" does not mean introduce a trait per
seam. A new port is justified only when there is a real I/O boundary *and* a plausible
second implementation (as `ExecutionPort` has three). Otherwise extract a pure function
and call it directly — a one-impl trait added to enable mocking is indirection that pays
no rent, and it makes the next reader hunt for implementors that do not exist.

**Naming.** No `helpers.rs`, `utils.rs`, `common.rs`, `misc.rs`, or `part2.rs`. Every new
module is named for the decision or the stage it owns. If you cannot name it, you have
not found the seam yet — say so instead of inventing a bucket.

## Comment policy (required)

The governing rule is **AGENTS.md §3 "Comments"** — read it; it is the standard, and
this section only says what a *refactor* adds to it. In particular: value is measured in
non-recoverable information, never length, and **you must not trim or cap a comment by
size alone.** The decision-carrying headers in this repo are frequently the only record
of *why* a module exists; flattening one is a deletion wearing a smaller diff.

What this Feature adds:

- **Verify before you keep.** A move is when staleness surfaces. If a comment names a
  file, function, or flag, confirm it still exists. A confidently wrong comment is worse
  than none, because the next agent will act on it.
- **Comments move with their code.** A comment left behind, now sitting above something
  it does not describe, is the worst outcome of a refactor — it reads as authoritative.
- **Delete on sight** the categories AGENTS.md §3 lists as review triggers, when you
  find them in files you are already touching: restatement, change narration, ticket
  echoes, play-by-play, stdlib explanation. Also commented-out code — git has it.
- **Comment work is its own commit.** A commit that both relocates a function and
  rewrites its comments cannot be reviewed as a move, which was the entire safety
  argument for doing this as a refactor. Use `docs({{SCOPE}}): …`.
- **Only files this Feature already touches.** No repo-wide comment sweep — that is a
  thousand-line diff nobody reads.
- **Never delete a comment you do not understand.** Flag it in the report. "I could not
  tell whether this still applies" is a useful sentence; guessing is not.
- **A `TODO` is not noise to delete.** Either still true — keep it, with a scope — or
  dead, in which case say so in the report rather than silently dropping it.
- If deleting a comment makes the code harder to follow, the fix is a better *name*.

## Testing strategy (required)

The test tree mirrors the source split, and gets the same treatment: oversized test
files are the same failure, and they hide it better.

- **Pure policy → `tests/domain/{{AREA}}/`**, one file per decision, no port doubles, no
  async runtime. These should be the majority of new tests and should run in
  milliseconds.
- **Choreography → `tests/infrastructure/{{AREA}}/`**, exercising the adapter against
  narrow doubles.
- **Never construct an `{{GOD_STRUCT}}` in a test.** It carries ports the code under
  test does not read. When adapter code is unreachable from a test, the fix is to make
  it a free function over the one port it needs — not to stub the other nineteen. Any
  test currently `#[ignore]`d for exactly this reason is a target: un-ignore it, or
  explain in the report why the seam did not reach it.
- **A double that answers every call successfully is not a double.** The e2e `FakeExec`
  returning `Ok("")` for every command meant anything *reading* git's output was being
  asserted against a default rather than an answer. New doubles must error on any call
  they were not explicitly told to answer.
- **Moved tests must be proven to still bind.** For each moved or rewritten test file,
  break the code it covers, confirm that test — and ideally only that test — goes red,
  then revert. A suite that cannot fail is not coverage. State in the report which
  files you did this for.
- **Do not weaken an assertion to make a moved test pass.** If a test only passes after
  its assertion changes, the refactor changed behaviour. Stop and report it.
- Split any test file over ~400 LOC along the same seams as the source.

## Definition of done — verification (required)

```bash
npm run checks:code        # inside a Demeteo run — same gates, no commitlint
npm run checks             # locally / before push — adds commitlint on origin/master..HEAD
```

{{IF THE MODULE IS OBSERVED BY A TRANSPORT — step executor, ExecutionPort impls, exec
contract — KEEP THIS BLOCK, ELSE DELETE IT:}}

`npm run checks` does **not** run the parity gates, and the e2e suite will not catch a
local/remote divergence — it drives a per-test `FakeExec` that passes while masking
exactly the drift these suites exist to find. Both need Docker:

```bash
crates/demeteo-core/tests/conformance/run-ssh-conformance.sh
crates/demeteo-core/tests/conformance/run-topology-conformance.sh
```

Green locally and in CI does **not** prove macOS or Windows compiles — PR checks run on
`ubuntu-22.04` only. If you touched host-side paths, shells, or process handling, reason
about all three targets and say which ones you could not verify.

## Sequencing and commits (required)

One seam per commit. `npm run checks:code` green at every commit, not just at the end —
a refactor whose intermediate states do not build cannot be bisected, which is the one
thing a refactor series is for.

Conventional Commits, enforced by the `commit-msg` hook:

```
refactor({{SCOPE}}): <imperative subject, ≤72 chars, no trailing period>
```

`refactor` produces no version bump, which is correct here — a bump would be a lie about
behaviour changing. **The trap:** `subject-case` rejects a subject starting with a
capitalised token — a `TypeName`, acronym, or ticket id. `refactor(exec): HarnessOutcome
into domain` fails; `refactor(exec): move harness outcome into domain` passes. Verify
with `echo "<message>" | npx commitlint`.

## Report this back (required)

Do not just say "done". Give evidence, in this shape:

| | before | after |
|---|---|---|
| files > 400 LOC of code in scope | {{N}} | |
| largest file (LOC) | {{N}} | |
| `#[allow(clippy::too_many_arguments)]` | {{N}} | |
| `#[ignore]`d tests in scope | {{N}} | |
| tests reaching this policy with zero port doubles | {{N}} | |

Plus, in prose:

- The seam list: each new module, what decision or stage it owns, why that is a seam.
- Which moved tests you watched fail, and how you broke them.
- Which conformance suites you ran, and their results.
- Comments: roughly how many you deleted vs compressed, every comment you could not
  judge and left alone, and every `TODO`/`FIXME` you found dead. Name any decision-
  carrying header you compressed, so a reviewer can check the decision survived.
- Any behaviour change you were forced into, with the reason. ("None" is the expected
  answer; if it is not, that is the most important line in the report.)
- Follow-ups you deliberately did not do, including any bug you found and left alone.

## Failure modes — these have all happened here before

- Splitting by line count into `mod_a.rs` / `mod_b.rs` with the god-struct intact. The
  file count improves and nothing is testable.
- Making an item `pub` (or `pub(crate)`) purely so a test can reach it. Widen visibility
  only when the seam genuinely puts the caller elsewhere; otherwise the test is reaching
  past a boundary that the refactor was supposed to establish.
- Adding a trait with exactly one implementation to enable a mock.
- Carrying `#[allow(clippy::too_many_arguments)]` along to the new file. That is the
  smell travelling with the code instead of being removed by the move.
- Using the comment policy as cover for deleting rationale. It targets restatement, not
  reasoning. In this repo the prose is frequently the only record of *why* — several
  modules exist specifically to hold a decision, and the comment *is* the decision.
  Shortening one until the decision is gone is a deletion wearing a smaller diff.
- Leaving a comment behind when its code moves, so it now sits above something it does
  not describe. Worse than deleting it, because it reads as authoritative.
- Declaring done on `cargo test` alone. "`cargo test` passed" is not "CI is green".

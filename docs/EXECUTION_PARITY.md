# Local/Remote Execution Parity — What's Guaranteed, and What Enforces It

> **Shipped.** Milestones C0–C6 of the execution-consistency work landed; this
> doc replaces the build plan and keeps only what the code does not tell you.
> The normative contract lives in the trait's own rustdoc
> (`crates/demeteo-core/src/ports/execution.rs`) — read that first. This doc
> covers the *reasoning* behind it, the gates that hold it, and the one leg
> deliberately left undone.
>
> Source comments tag code with milestone ids (`C0.2`, `C1.3`, `C5`, `C6`).
> Read those as historical provenance; the guarantees they produced are stated
> below.

## The guarantee

A feature behaves and renders identically no matter which transport ran it —
local subprocess, desktop-over-SSH, or `demeteo-runner`. Four decisions produce
that, and they are the reason the code looks the way it does:

- **The engine is never forked.** Consistency comes from pinning the
  *transports* and unifying the *read model*, never from branching driver logic
  on transport. If two transports differ, the adapter or the contract is wrong —
  not the caller.
- **Context is explicit, never inherited.** No transport may depend on ambient
  state (the GUI process's `PATH`/`HOME`/cwd). Shell mode, cwd, and env travel as
  data in `ShellOptions` and every adapter honours every field identically. This
  is why `run_command_with` exists and why raw `run_command` should only appear
  where the documented default is genuinely intended.
- **Failures are loud and uniform.** A command that cannot run, or runs
  non-zero, is always `Err` carrying stderr — never `Ok("")`, never swallowed by
  `.ok()?` or `unwrap_or_default()`. Transport failures are distinguishable by
  message prefix (`TRANSPORT_ERROR_PREFIX`) so the verifier can treat them as
  non-retryable infrastructure rather than as a code regression.

  The SSH adapter retries *underneath* that prefix (S4,
  [`RELIABILITY_PLAN.md`](RELIABILITY_PLAN.md)) and the distinction is what
  bounds it. A retry is only permitted where the remote was handed nothing —
  before `channel.exec` — because arbitrary user shell cannot be re-run safely
  once it may be in flight, and the port carries no idempotency signal that
  could say otherwise. When the attempts are spent the **last failure's message
  is returned untouched**, so a persistent drop still reads as `transport:` to
  `classify_exec_failure` and still routes to `Infrastructure`. Retry that
  swallowed the prefix would look like a reliability improvement and behave like
  a regression detector pointed at the wrong thing.
- **A red harness is triaged before it feeds the retry loop.** See below.

## The gates — and the trap

Two conformance suites hold the guarantee. **Neither runs under `npm run
checks`**; both need Docker, and both are the only thing standing between a
change and a silent local/remote divergence:

```bash
crates/demeteo-core/tests/conformance/run-ssh-conformance.sh       # exec_contract, local vs loopback sshd
crates/demeteo-core/tests/conformance/run-topology-conformance.sh  # RunView equivalence, local + SSH
```

Run them when you touch an `ExecutionPort` impl, the step executor, or anything
a transport observes.

**The trap: the e2e suite will not catch transport drift.** It drives a per-test
`FakeExec` that passes while masking exactly the divergence these suites exist
to find. "e2e is green" is not evidence of parity.

`topology_equivalence.rs` gets its determinism from a no-LLM stub agent
(`adapters/agent/stub_runtime.rs`, `agent_kind: "stub"`, gated behind
`DEMETEO_STUB_AGENT` so it is never registered in production). It parses
`@stub-write <path>` directives out of the step prompt, so one workflow reaches
a terminal state with byte-identical output on any transport.

## Deliberately deferred: the runner leg of topology conformance

Topology equivalence asserts local and SSH. The third leg — a runner in a
container — is **deferred, not dropped**, and the reasoning matters if you pick
it up:

Its marginal correctness value is small. The runner runs the *identical*
`LocalOnly` engine the local leg already proves, and a runner-owned feature
renders through `RunView` with **no** runner-specific branch (the shadow hydrate
has unit coverage). The only thing a runner container additionally exercises is
the `control_rpc`→hydrate **reconcile transport**.

Standing it up requires two things first: (1) an in-container git host — the
runner's `execute_run` clones and pushes a real remote, and the clone can't be
pre-skipped because the `project_id` is minted in-container; and (2) a refactor
extracting the reconcile-and-hydrate core out of the Tauri-`State`-bound
`remote_reconcile_runs` / private `hydrate_shadow_feature` so a test can drive
it. **Start with the reconcile refactor** — the equivalence harness is already
there to plug into.

## Harness failure triage: regression vs environment

A non-zero harness exit that is *not* a transport failure is not automatically
the code's fault. Before it becomes a retryable `Verdict`, a classifier decides
between a **regression** (the change under test broke it → retry the implement
step) and an **environment** failure (missing system library, toolchain,
permission, network — editing source cannot fix it → terminate with remediation).
Without this, a missing `gdk-3.0` burned the entire implement retry budget and
reported a code failure.

Three design points that are not visible from reading the implementation:

**Why it's persistence-gated.** Triage does *not* fire on first sight; the
failure retries once as before. It fires only when the retry reproduces the
**same** failure. A genuine regression usually *changes* across a retry — a
different error, or the implement step fixes it — whereas an environment failure
reproduces near-identically. "Survived a retry unchanged" is therefore itself
strong evidence of `environment`, and it hands the classifier real signal
instead of a single-shot guess. The cost is that the environment case eats
exactly one retry cycle before terminating; the benefit is zero triage toll on
regressions that fix on the first retry.

**Why it fails safe toward `regression`.** Any classifier error, timeout,
cancellation, or unparseable/unknown category defaults to `Verdict`. We never let
a broken triage wrongly terminate a genuine regression — we only ever *withhold*
remaining retries once we're confident they're futile. Uncertainty defaults to
`regression` for the same reason.

**The fingerprint normalization is load-bearing and fails in two opposite
directions.** Both need guarding, and one of them is invisible:

- *Under-normalizes* (leaves timestamps, worktree paths, run-ids in): two truly
  identical environment failures look "different", triage never fires, and we
  silently fall back to burning the full budget — the original bug, unfixed and
  invisible. **The differing-output fixture does not catch this direction**;
  its mirror does — two runs of the same missing-lib failure differing only in
  worktree path and timestamp must fingerprint-*match*.
- *Over-normalizes*: a genuinely different regression error on the retry reads
  as "same", triggering triage and a possible premature terminate on what was
  real progress. The differing-output fixture guards this side.

Normalize conservatively: mask only known-volatile spans (absolute worktree
path, ISO/epoch timestamps, feature/step run-id) and nothing else.

## Related

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — the hexagon and port surface
- [`REMOTE_EXECUTION.md`](REMOTE_EXECUTION.md) — remote runner design
- [`MULTI_CLIENT_RUNNER.md`](MULTI_CLIENT_RUNNER.md) — the not-yet-built next step

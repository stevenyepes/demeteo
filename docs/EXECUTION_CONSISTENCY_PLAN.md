# Demeteo: Local/Remote Execution Consistency — Implementation Plan

> **The build plan for making a feature behave and appear identically no matter
> which transport executed it — local subprocess, desktop-over-SSH, or the
> autonomous `demeteo-runner`.** Task format follows
> [`REMOTE_EXECUTION_PLAN.md`](REMOTE_EXECUTION_PLAN.md) and
> [`BACKEND_REFACTOR_TASKS.md`](BACKEND_REFACTOR_TASKS.md): each task states
> **What / Where / Why / Definition of Done**. Cross-refs to the guiding
> decisions below use the `Dn` tags; this plan's late phases fold into
> `REMOTE_EXECUTION_PLAN.md`'s **M6** (Laptop UX), which they complete.

## Why this plan exists

The engine is already shared: local, desktop-over-SSH, and the runner all
instantiate the same `ExecutionDriver` against the same schema. Divergence does
**not** come from missing abstraction — it comes from two specific leaks:

1. **Transport semantic drift.** `ExecutionPort::run_command` is a raw shell
   string with *undocumented, per-adapter* meaning. `LocalSubprocessAdapter`
   runs `sh -c` inheriting the GUI process env
   (`adapters/local/execution.rs:83`); `SshClientAdapter` runs a bare
   `channel.exec` with no login shell, no PATH, no profile
   (`adapters/ssh/client.rs:367`). Same signature, different behaviour → every
   "works local, silently wrong on remote" bug lives here.
2. **Data-location divergence.** Local and desktop-SSH share the laptop DB +
   `FsArtifactStore`; the **runner is the sole outlier** — its own DB, its own
   store, and no artifact mirror (`crates/demeteo-runner/src/rpc.rs:157`
   exposes only status/events). Its features are invisible to the laptop UI.

Nothing pins the four `impl ExecutionPort` (`local`, `ssh`, `router`,
`agent/test_stubs`) to a shared behavioural contract, and the e2e tests use a
bespoke per-test `FakeExec` (`tests/e2e/step_executor.rs:62`) that passes while
masking exactly this drift. The result is that new functionality has to be
bug-hunted onto each path separately.

**This plan replaces per-path bug hunting with two guarantees:** (a) any code
written against `ExecutionPort` behaves identically on every transport, enforced
by a shared conformance suite, and (b) a feature's steps/artifacts/stream render
identically in the UI regardless of where it ran, through one read model.

## Guiding decisions

- **D1 — The engine is never forked.** Consistency is achieved by pinning the
  *transports* and unifying the *read model*, not by branching driver logic.
  (Extends `REMOTE_EXECUTION_PLAN.md` guiding principle 1.)
- **D2 — Context is explicit, never inherited.** No transport may depend on
  ambient state (the GUI's PATH/HOME/cwd). Shell semantics, cwd, and env are
  passed as data and honoured identically by every adapter.
- **D3 — Failures are loud and uniform.** A command that cannot run, or runs
  non-zero, is always an `Err` carrying stderr — never `Ok("")` and never
  swallowed by `.ok()?` / `unwrap_or_default()`. Divergence must surface as an
  error, not as an empty artifact.
- **D4 — One behavioural contract, enforced by conformance tests.** Every
  `ExecutionPort` impl passes the same assertion suite (including a real SSH
  target); every transport passes the same feature-equivalence test. New
  behaviour is added to the suite, not to each path.
- **D5 — The runner stays autonomous.** It must keep running with the laptop
  closed (`REMOTE_EXECUTION.md` R2/R8), so we *mirror* its state to the laptop;
  we never make the laptop drive it.
- **D6 — Every milestone is independently valuable.** We can stop after any `C`
  and have shipped something real. `C0`–`C2` alone resolve the current
  "silent no-artifacts on SSH" class.

## Milestone map

| C | Theme | Phase | Demoable outcome | Refs |
|---|-------|-------|------------------|------|
| C0 | Port contract | 1 | `ExecutionPort` has a written, testable behavioural contract | D1, D2, D3 |
| C1 | Explicit context + adapter parity | 1 | Local and SSH `run_command` behave identically for the same explicit options | D2 |
| C2 | Conformance gate + error surfacing | 1 | One suite runs both adapters (local + containerized sshd) in CI; swallowed errors now surface | D3, D4 |
| C3 | `RunView` read model | 2 | UI reads feature/steps/artifacts through one layer; no behaviour change for local/SSH | D1 |
| C4 | Runner state mirror | 3 | A runner feature renders on the laptop with full fidelity (steps, artifacts, stream, cost) | D5 — folds into `REMOTE_EXECUTION_PLAN.md` M6 |
| C5 | Topology conformance | 4 | The same feature run through all three transports asserts an equivalent `RunView` | D4 |

Ordering is linear for C0→C2 (each depends on the prior). C3 depends on C2. C4
depends on C3. C5 depends on C4. **C0–C2 is the prerequisite foundation and also
the standalone fix for the current SSH artifact-loss bug — ship it first.**

---

## C0 — ExecutionPort behavioural contract

**Context.** The contract today is folklore. `run_command(machine_id, cmd)`
means different things per adapter, and callers depend on the local meaning by
accident. Writing it down is the enabling step for C1 (make adapters comply) and
C2 (test compliance).

### C0.1 — Document the trait contract

- **What:** Add doc-comments to every `ExecutionPort` method stating the
  guarantee: shell semantics (`run_command` runs through a POSIX shell with a
  *specified* login/non-login mode), how `cwd` is expressed, what environment is
  present (only what's passed — see D2), and the error invariant (D3: non-zero
  exit ⇒ `Err` with stderr attached; a transport/connection failure is also
  `Err`, distinguishable by message prefix).
- **Where:** `crates/demeteo-core/src/ports/execution.rs`.
- **Why:** C1 and C2 need a single authority for "correct behaviour" to build
  and test against.
- **DoD:** The trait doc-comments fully specify inputs, cwd/env, and error shape;
  a reviewer can implement a new adapter from the doc alone. No code behaviour
  change in this task.

### C0.2 — Define the error contract

- **What:** Standardize the `Err` payload so callers can distinguish
  *command-failed* (ran, non-zero) from *transport-failed* (couldn't reach the
  machine). Minimal form: keep `Result<_, String>` but guarantee stderr is
  always included and prefix transport failures consistently; optional stronger
  form: a small `ExecError` enum. Decide here, apply in C1.
- **Where:** `ports/execution.rs`; touch points in `adapters/ssh/client.rs`
  (`run_command` already drains stderr at `:388` and checks exit at `:398`) and
  `adapters/local/execution.rs`.
- **Why:** D3 — uniform, actionable failure is what turns "silent empty
  artifact" into a message.
- **DoD:** The chosen error shape is documented on the trait; SSH and local both
  already satisfy the exit-code invariant (verified) — this task only formalizes
  it and covers the transport-failure case.

---

## C1 — Explicit execution context + adapter parity

**Context.** The #1 divergence: local inherits the GUI env and runs `sh -c`;
SSH runs a bare non-login `channel.exec`. `spawn_interactive` already models the
fix (it honours a `use_login_shell` flag and sets cwd/env explicitly,
`adapters/ssh/client.rs:753`), but `run_command` does not. We lift that model
into the command path so both adapters are configured, not ambient.

### C1.1 — Add explicit `ShellOptions` to the command path

- **What:** Introduce `run_command_with(machine_id, cmd, ShellOptions)` where
  `ShellOptions { login_shell: bool, cwd: Option<String>, env: BTreeMap<..> }`;
  keep `run_command` as a thin default (`login_shell` matching documented
  default, no extra env). Both adapters must honour every field identically.
- **Where:** `ports/execution.rs`; `adapters/local/execution.rs`;
  `adapters/ssh/client.rs`; `adapters/router.rs` (pure delegation, add the
  method).
- **Why:** D2 — removes the "local happens to inherit GUI PATH/HOME" assumption
  by making context data the caller passes, so SSH gets the same context.
- **DoD:** Given identical `ShellOptions`, a command that reads `$PATH`/`$HOME`/
  cwd returns equivalent results on local and SSH in the C2 conformance suite.

### C1.2 — SSH adapter honours login shell / cwd / env in `run_command`

- **What:** Route `run_command_with` through the same `bash -l -c` (login) /
  `cd … && …` (non-login) construction `spawn_interactive` already uses, with
  the passed env exported and cwd applied — replacing the bare `channel.exec`
  at `adapters/ssh/client.rs:367`.
- **Where:** `adapters/ssh/client.rs`.
- **Why:** Eliminates the remote-only PATH/mise/profile gap for anything the
  orchestrator (and any tool it shells out to) runs via `run_command`.
- **DoD:** A conformance test asserting `command -v git` / a mise-shimmed tool
  resolves over SSH passes; existing SSH-driven features unaffected.

### C1.3 — Migrate call sites to explicit context

- **What:** Set `ShellOptions` once at the choke points rather than per call.
  Most git already funnels through `GitOpsHelper`
  (`adapters/worktree/git_ops/`), so it's the primary site; the agent spawn path
  is already explicit via `spawn_interactive`.
- **Where:** `adapters/worktree/git_ops/*`; `adapters/step_executor/artifacts/*`;
  any remaining direct `exec.run_command` callers.
- **Why:** Centralizes the contract so new step logic inherits correct context
  for free (the by-construction goal).
- **DoD:** No caller relies on ambient env; `git grep` shows raw `run_command`
  only where a documented default is intended; suite green.

---

## C2 — Conformance gate + error surfacing

**Context.** This is the milestone that makes divergence fail CI instead of
production, and the one that fixes the reported bug directly.

### C2.1 — Shared `exec_contract` assertion suite

- **What:** `fn exec_contract(port: Arc<dyn ExecutionPort>)` exercising: write→
  read round-trip, `cwd` honoured, non-zero exit ⇒ `Err` with stderr, missing
  file ⇒ documented error (not `Ok("")`), `list_dir` entry shape, and
  login-shell env resolution.
- **Where:** new `crates/demeteo-core/tests/conformance/execution_port.rs`.
- **Why:** D4 — one place that defines "correct" for all adapters.
- **DoD:** Suite passes against `LocalSubprocessAdapter`; each assertion maps to
  a C0 contract clause.

### C2.2 — Run the suite against a real SSH target

- **What:** Stand up a loopback `sshd` (container / CI service), run
  `exec_contract` against `SshClientAdapter` pointed at it. Feature/CI-gated so
  local `cargo test` without Docker still passes.
- **Where:** `tests/conformance/`; a CI job; `.github/workflows/`.
- **Why:** D4 — the only way to prove local/SSH parity is to run both through
  the same asserts.
- **DoD:** CI runs the suite against both adapters; a deliberate reintroduction
  of the bare-`channel.exec` regression turns the job red.

### C2.3 — Surface the swallowed errors

- **What:** Replace swallow-and-default with propagate-and-surface at:
  `read_worktree_file` `.ok()?` (`artifacts/declared.rs:131`),
  `git_status_porcelain` `unwrap_or_default()` (`artifacts/snapshot.rs:88`),
  `compute_git_diff` `unwrap_or_default()` (`artifacts/declared.rs:158`), and
  emit a surfaced warning/telemetry when a declared `LastWriteTo` captures
  nothing (today explicitly a non-failure, `declared.rs:16`).
- **Where:** `adapters/step_executor/artifacts/*`.
- **Why:** D3 — this is what converts the current *silent* SSH artifact loss
  into an actionable message, independent of its underlying cause.
- **DoD:** A step whose declared artifact wasn't produced surfaces a diagnostic
  (event + log) instead of a green step with an empty artifact; local behaviour
  unchanged (the reads succeed).

### C2.4 — Fix the `is_likely_binary` panic (drive-by)

- **What:** `&content[..content.len().min(8192)]` slices a `String` at a byte
  index that may not be a char boundary → panic on >8 KiB text with a multibyte
  char near byte 8192. Use `content.get(..8192)` / a char-boundary-safe slice.
- **Where:** `adapters/step_executor/artifacts/declared.rs:376`.
- **Why:** Correctness; it crashes artifact capture on exactly the large reports
  this plan is trying to make reliable.
- **DoD:** Unit test with a multibyte char straddling byte 8192 no longer panics;
  binary detection still works.

---

## C3 — `RunView` read model

**Context.** The UI reads `features` + `FsArtifactStore` directly. That's fine
for local/SSH (shared laptop state) but is the seam C4 needs to plug the runner
into. Extracting it now is a no-op refactor that unlocks the topology unify.

### C3.1 — Define the read model

- **What:** A `RunView` read layer returning a feature's steps, per-step
  artifacts, agent stream, and cost — the exact shape the UI renders — behind one
  interface, independent of execution location.
- **Where:** new `application/run_view.rs` (or a port + adapter);
  consumers in `src-tauri/src/commands/` that today read features/artifacts.
- **Why:** D1 — one render path for every transport.
- **DoD:** For local/SSH features the UI renders through `RunView` with zero
  behaviour change (byte-identical to today's direct reads).

### C3.2 — Route the UI through `RunView`

- **What:** Point the feature-detail / artifact-viewer commands at `RunView`.
- **Where:** `src-tauri/src/commands/*`.
- **Why:** Establishes the single consumption point C4 feeds.
- **DoD:** No component reads `FsArtifactStore` / `features` directly for
  display; suite + `tsc` green.

---

## C4 — Runner state mirror (topology unify)

**Context.** The runner is autonomous (D5) with its own DB/store. To make its
features appear identically on the laptop we mirror its state — lazily and
offset-cached, reusing the `run_events` streaming pattern already proven in M3.
This milestone **completes `REMOTE_EXECUTION_PLAN.md` M6's "live remote view."**

### C4.1 — Extend the runner RPC beyond status

- **What:** Add `get_feature`, `list_steps`, `get_step_artifacts`,
  `stream_agent_events` to the runner control RPC, mirroring the existing
  `stream_events(run_id, from_offset)` offset/replay contract
  (`crates/demeteo-runner/src/rpc.rs:481`).
- **Where:** `crates/demeteo-runner/src/rpc.rs`; the laptop client methods in
  `src-tauri/src/commands/remote_runner.rs`.
- **Why:** D5 — the laptop can read the runner's ground truth without driving it.
- **DoD:** The laptop can fetch a runner feature's steps and one artifact body
  over the existing SSH-forwarded control socket; dropped-tunnel replay works via
  `from_offset`.

### C4.2 — Laptop hydrates a shadow feature + lazy artifact cache

- **What:** Extend the mirror (`ports/remote_run_mirror.rs`) to hydrate a shadow
  feature/steps into the laptop DB and pull artifact bodies into the laptop
  `FsArtifactStore` **on demand, cached by offset** (avoids eager full-sync
  drift). Clearly mark shadow rows as runner-owned (read-only on the laptop).
- **Where:** `adapters/database/repos/*` (or a dedicated shadow store);
  `src-tauri/src/commands/remote_runner.rs` reconcile path.
- **Why:** D1 + D5 — makes `RunView` able to serve a runner feature from local
  cache with the same shape as a native one.
- **DoD:** After reconcile, a runner feature has a populated step list and its
  artifacts are fetchable; re-reconcile only pulls new offsets.

### C4.3 — `RunView` renders runner features identically

- **What:** Teach `RunView` (C3) to source a runner-owned feature from the shadow
  + lazy artifact fetch, transparently to the UI.
- **Where:** `application/run_view.rs`.
- **Why:** The payoff: identical step/artifact/stream/cost fidelity regardless of
  transport.
- **DoD:** A runner run and a local run of the same workflow render the same UI
  affordances (steps, artifacts, agent stream, cost); the inbox deep-links into
  the full feature view, not just status.

### C4.4 — Reconcile `commit_artifacts` default for remote

- **What:** With the mirror in place, revisit `commit_artifacts=false`
  (`adapters/step_executor/setup.rs:191`): the reports no longer *need* to ride
  the PR to be visible, so keep the default clean, but make the choice explicit
  and documented rather than a silent data-loss trap.
- **Where:** `setup.rs`; settings UI hint.
- **Why:** Removes the "PR is the only channel and it's empty" surprise for
  remote users.
- **DoD:** Behaviour is documented; a remote user always has a path to the
  reports (mirror by default, PR if opted in).

---

## C5 — Topology conformance gate

**Context.** The by-construction guarantee at the top level: new step kinds and
artifact types must work on all three transports or CI is red.

### C5.1 — Same-feature-across-transports equivalence test

- **What:** Run one workflow through (a) local, (b) SSH-to-container, (c)
  runner-in-container, and assert the resulting `RunView` is equivalent — same
  step set, same declared artifacts present, same terminal status.
- **Where:** new `tests/conformance/topology_equivalence.rs`; CI job reusing the
  C2.2 SSH container plus a runner container.
- **Why:** D4/D6 — this is the mechanism that keeps future functionality
  consistent without per-path bug hunting.
- **DoD:** The test passes on all three transports; removing the C4 mirror (so
  the runner `RunView` is empty) turns it red.

---

## Sequencing & risk

- **C0–C2 first, always.** Small blast radius, and independently resolves the
  current SSH silent-artifact bug. C1.1 changes the `ExecutionPort` surface —
  contained but rippling; land it as a signature-add (new `*_with` method) so
  existing callers compile untouched, then migrate in C1.3.
- **C3 is a no-op refactor** — safe to land anytime after C2, gates C4.
- **C4 is the large lift** and overlaps `REMOTE_EXECUTION_PLAN.md` M6; treat this
  plan's C4 as that milestone's execution detail rather than duplicating it.
- **Ground truth is code, not this doc.** File:line anchors are current
  integration points on `master` at authoring time — verify when picking up a
  task.

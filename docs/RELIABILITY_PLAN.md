# Demeteo Reliability Plan: DAG / Pipeline / SSH

> **Scope:** Improvements to the `DagStepExecutor` pipeline and the SSH
> transport that materially reduce silent-failure modes, lost work on
> crash / network drop, and accumulated state drift. Source of truth for
> v1 reliability work; cross-references [`DECISIONS.md`](DECISIONS.md) for the
> locked decisions and [`ARCHITECTURE.md`](ARCHITECTURE.md) for transport-level
> invariants.
>
> **Status:** Plan only. No code changes. Sequenced in the order
> recommended at the bottom of this file.
>
> **File-line anchors** point at the current `main` of the redesign branch
> (commit at time of writing). Re-verify with `git grep` before
> implementing; the line numbers will drift as surrounding code changes.

---

## 0. Invariants this plan must preserve

1. **Strict serial per project** — at most one running feature per
   project at a time (see [`DECISIONS.md`](DECISIONS.md) decision 18).
2. **Per-step checkpoints are atomic** — a step is "complete" only when
   its artifact is written and (if it's a gate) its decision is recorded
   (see [`DDD_MODEL.md`](DDD_MODEL.md) §4 Feature Orchestration).
3. **Cost and duration are computed at step completion, not estimated
   mid-step** (see [`DECISIONS.md`](DECISIONS.md) decision 15).
4. **Step transitions are the UI contract, not agent transcripts**
   (see [`DDD_MODEL.md`](DDD_MODEL.md) §6 Agent Runtime).
5. **Keyring + ssh-agent for credentials, never plaintext**
   (see [`ARCHITECTURE.md`](ARCHITECTURE.md) §2).

---

## 1. Pipeline / DAG (`src-tauri/src/adapters/step_executor/`)

### P1. Cancel-vs-future distinction is lost — **real bug**

**Where:** `steps/agent.rs:62-66`, `steps/gate.rs:67-71`,
`steps/parallel.rs:111-116`.

All three `tokio::select!` arms collapse the cancel branch into a single
`None`:

```rust
tokio::select! {
    res = spawn_fut => Some(res),
    _ = cancel_watch_spawn.changed() => {
        if *cancel_watch_spawn.borrow() { None } else { None }
    }
};
```

The downstream `match` treats cancellation and a completed spawn
identically. Effect: `cancel_feature()` is sometimes called when the spawn
actually returned, and the step is wrongly marked `cancelled` instead of,
say, `awaiting_gate` or `failed`.

**Fix.** Introduce a local enum per call site:

```rust
enum SpawnResult<T> { Ready(T), Cancelled }
```

…return that from each `select!`, and match on it. Three call sites,
~30 LOC total.

**Verification.** New unit test in `adapters/step_executor/tests.rs`:
trigger a cancel between `get_or_spawn` returning and the first event;
assert the step's terminal status is `cancelled` (not `failed`) and the
feature row reflects the cancel.

**Cost:** ~30 LOC, 3 files. **Risk:** very low — confined to error
paths, behavior change is strictly more accurate.

---

### P2. Fire-and-forget DB writes during state transitions

**Where:** `driver.rs:89-95, 130-147, 190-203`, plus `steps/agent.rs`,
`steps/gate.rs`, `steps/parallel.rs` (every handler does
`let _ = self.features.step_update(...)`).

A crash mid-pipeline leaves `step_executions.status="running"`. The
resume scan (`driver.rs:57-65`) treats that as "not yet started" and
re-runs the step from scratch — **silently losing the agent's work and
its cost**.

**Fix (a) — short term.** Wrap each transition in a small
`commit(transition) -> Result<…>` that returns the error to the driver.
Log loudly on failure. ~80 LOC. **Risk: low.**

**Fix (b) — preferred.** Add a `Checkpoint { step_index, per_step_status,
accumulated_cost, started_at_ms }` struct and a `step_executions.checkpoint_id`
foreign key. Every state transition writes the checkpoint row and the
step row in a single SQLite transaction. Resume reads the latest
checkpoint and rehydrates the driver. This obsoletes P3's resume logic.

**Cost:** (a) ~80 LOC, (b) ~250 LOC + a migration. **Risk:** low for
(a), medium for (b) — touches the migration runner design from R8.

---

### P3. Resume from interruption is best-effort

**Where:** `driver.rs:57-65`.

Scans `step_executions` for the first non-`"completed"` row, then
advances `step_index` past preceding `completed` rows. No distinction
between "step in flight when killed" and "step completed but DB write
lost".

**Fix.** Add a third terminal status `interrupted` (already used at
`driver.rs:131`, partially). On launch, if any step is in `interrupted`
or `running`, insert a synthetic `GateDecision` row with
`decision = None, feedback = None` and emit `GateRequired` — the UI must
clear the gate before the step re-runs. Aligns with [`DECISIONS.md`](DECISIONS.md)
decision 14.

**Cost:** ~1 day (Rust) + UI work in `FeatureDetail.tsx` /
`GateView.tsx`. **Risk:** medium — touches gate UX; needs UI buy-in.

---

### P4. `parallel` step is a stub

**Where:** `steps/parallel.rs:28-31`.

```rust
let subtasks = vec![
    ("sub-1", "Implement core logic"),
    ("sub-2", "Write unit tests"),
];
```

Every user gets the same two subtasks regardless of their workflow.

**Fix.** Wire the planner-driven DAG decomposition:

1. Spawn a planner agent session with a structured-output prompt.
2. Parse the response into `Vec<SubtaskSpec>` (already typed in
   `domain/models.rs`).
3. Fan out across available workers using `ExecutionPort::spawn_interactive`.
4. Order merges with `MergeExecutor::merge_topological_order` (already
   in `adapters/worktree/`).
5. Aggregate results into the step's artifact.

**Verification.** A test that mocks the planner output with a 3-subtask
DAG, runs the executor, asserts:
- 3 subtask worktrees provisioned
- subtask branches merged in topological order
- parallel step artifact contains all 3 results
- cost = sum of subtask costs

**Cost:** 2–3 days. **Risk:** medium-high — planner output parsing is
the messy part. Use a JSON-schema-validated prompt to constrain the
planner's output shape.

---

### P5. Conditional edges / `max_iterations` are missing

**Where:** `steps/gate.rs:97-106`; `StepOutcome` at
`steps/mod.rs:2-11`.

`StepOutcome::RedirectTo(usize)` only fires from a gate's "redirect"
decision. [`DECISIONS.md`](DECISIONS.md) decision 14 lists `on_failure → goto`,
`on_all_success`, `on_any_failure`, `max_iterations` as v1 truth; none
are in the driver.

**Fix.**
- Extend `StepOutcome` with `Goto(usize)`, `Loop`, `Stop`.
- Add `step_max_iterations: Option<u32>` and
  `on_failure_step_id: Option<StepId>` to `StepConfig`
  (`domain/models.rs`).
- Track per-step iteration count in `ExecutionDriver`. When exceeded,
  return `Failed("max iterations reached")`.
- In `fail_step_and_feature` (`driver.rs:182-221`), consult
  `on_failure_step_id`; if set, `StepOutcome::Goto(idx)`; if absent,
  current behavior.
- In agent step, on `agent_failed` event, emit `Goto` instead of `Failed`.

**Verification.**
- Unit test: `agent_step → fail → Goto(fix_step)` chains correctly.
- Unit test: `max_iterations: 3` stops after 3 loops.

**Cost:** ~150 LOC. **Risk:** low — purely additive on the step
vocabulary.

---

### P6. `accumulated_cost: f64` plumbed through 3 handlers

**Where:** `driver.rs:105`, every step handler.

Each handler has to remember to update the counter; cost-on-failure is
computed in 3 different places. Drift-prone.

**Fix.** Move the counter into `ExecutionDriver` as a private
`f64` field. Expose `record_cost(step_id, delta: f64)` and
`record_duration(step_id, started: Instant)`. Both methods write to
`step_executions` in one place. Removes the `&mut` parameter from all
three handlers and from `fail_step_and_feature`.

**Verification.** Existing cost tests pass; new test asserts the sum at
feature completion equals the sum of per-step recordings.

**Cost:** ~100 LOC. **Risk:** low.

---

### P7. No backpressure on agent events to UI

**Where:** `steps/agent.rs:97-101` (and similar in `parallel.rs`).

Every `AgentEvent::Text { delta }` emits
`DomainEvent::AgentStream { content: delta.clone() }`. Tauri channels
buffer or drop; the UI's text renderer doesn't need every delta.

**Fix.** Coalesce text deltas on the Rust side into ~50ms windows before
emit. Use `tokio::time::interval` keyed by `step_execution_id`. Discard
intermediate deltas, emit only the latest. Other event kinds
(`ToolCall`, `Usage`, `TurnComplete`) pass through unchanged.

**Cost:** ~40 LOC + a small test. **Risk:** low.

---

## 2. SSH / transport (`src-tauri/src/adapters/ssh/`,
`src-tauri/src/adapters/router.rs`)

### S1. Stale sessions stay in cache

**Where:** `client.rs:39-53`.

`get_sftp` only removes a session when the next `readdir(".")` probe
fails. A half-open connection (TCP up, SSH dropped) still serves commands
that **timeout** rather than reconnecting immediately. Combined with
the 10s read/write timeout (`client.rs:82-83`), every command issued in
that window waits ~10s.

**Fix.**
- Track `last_used: AtomicU64` on `SftpSession`.
- Add `sess.closed()` check (ssh2 exposes it) before returning a cached
  session.
- Optional: a background reaper task closes sessions idle > 5 min.

**Cost:** ~60 LOC. **Risk:** low.

---

### S2. `Sftp` is serialized by a single `Mutex<Sftp>`

**Where:** `client.rs:13-16` — `SftpSession.sftp: Mutex<Sftp>`.

`ssh2::Sftp` is **not thread-safe**. Single global mutex per session
serializes all SFTP ops for one machine across the whole app. Two
impacts:

- Throughput bottleneck: SFTP file tree + Monaco editor block while a
  feature runs.
- **Deadlock risk** if any future call holds the mutex and awaits (e.g.
  an SFTP write inside an async callback).

**Fix (short-term).** Switch to `tokio::sync::Mutex`; document "never
hold across awaits". Verify by grep for any await inside
`SftpSession.sftp.lock()` scopes.

**Fix (long-term).** Migrate to `russh` (async-native, no mutex) or
split per-op session handles keyed by a generation counter.

**Cost:** short-term ~1 day; long-term 1–2 weeks. **Risk:** medium for
the `russh` migration (API differences, behavior changes in
channel/forwarding code).

---

### S3. `spawn_interactive` doesn't validate the wrapped command

**Where:** `client.rs:482-554`.

Builds `bash -l -c "cd <cwd> && export k='v'; exec <binary> <args>"`
and execs. If `cd <cwd>` fails (typo, removed dir, permission), the
agent starts in the wrong cwd silently. The recent `run_command`
exit-status fix at `client.rs:316-317` proves we care about this class
of silent failure.

**Fix.**
- Probe the cwd with `run_command("test -d <cwd> && echo OK")` before
  exec. Fail-fast with a typed error: `SpawnFailed("cwd not found:
  …")`.
- Drain stderr for the first 200 ms post-exec; surface the bash error
  if exec fails.
- Probe that the binary resolves on the remote `$PATH`
  (`command -v <binary>`); fail-fast with `SpawnFailed("binary not
  found: …")` if not.

**Cost:** ~50 LOC. **Risk:** low.

---

### S4. No retry on transient SSH drops mid-feature

**Where:** `client.rs:482-554` (`spawn_interactive`); used by
`CliRuntime` for remote agents and the planner in step handlers.

One dropped network = full pipeline stop. The driver only sees the
subprocess return and marks the step `Failed`.

**Fix.** Add a `with_ssh_retry(future, attempts: u32)` wrapper at the
`ExecutionPort` boundary that re-establishes the session on
`Err(SshError::ConnectionLost)` and re-execs the call. Re-establishment
goes through `SshClientAdapter::get_sftp` so the new session is cached
normally.

**Verification.** Kill the SSH server (`systemctl stop sshd`) mid-step;
assert the wrapper reconnects within `attempts * 2s`.

**Cost:** ~1 day. **Risk:** medium — must scope which errors are
retryable; over-retry will mask real failures.

---

### S5. Port-forwarding state isn't covered by the watchdog

**Where:** `forward.rs` (218 LOC). Local TCP listeners that tunnel over
the active SSH session.

Listeners aren't torn down when a machine is deleted or the SSH session
drops. Symptom: deleted-machine forwards keep accepting connections on
the local port until app restart.

**Fix.** Add `ForwardState::prune_for_machine(machine_id)` and call it
from `commands/machine::delete_machine` and on connection-drop in S1.

**Cost:** ~40 LOC. **Risk:** low.

---

### S6. `RouterExecutionPort` string-based dispatch

**Where:** `adapters/router.rs` (88 LOC).

Resolves `auth_type` via `match` with a default branch. Worth a targeted
audit before changing anything around it. The fix (if needed) is to
return a typed `RouterError::UnknownAuthType(String)` and bubble that
up.

**Cost:** audit ~half day; fix ~20 LOC if needed. **Risk:** low.

---

## 3. Cross-cutting

### X1. Type the pipeline state

**Where:** `driver.rs:24-51` — field soup on `ExecutionDriver`.

Replace with a `PipelineState { phase, step_index, accumulated_cost,
started_at, last_checkpoint_id, … }` enum + struct. Every transition is
a single `state.transition(…)` call that emits the right `DomainEvent`
*and* schedules a checkpoint write. Makes P2, P3, P5, P6, P7 all easier
to land cleanly.

**Cost:** ~1 day. **Risk:** low.

---

### X2. Add `step_executor::tests` integration suite

**Where:** `mod.rs:26` declares `#[cfg(test)] mod tests;`. Verify it
exists; if not, write it.

Required cases (after P1 + P5 + P6):
1. Happy path: 3-step workflow with 1 gate, ends `completed`.
2. Cancel during `agent` step → step `cancelled`, feature `cancelled`
   (validates P1).
3. Gate "redirect" → `Goto(target)` advances correctly.
4. `max_iterations: 2` stops the loop.
5. Resume after killing demeteo mid-step → synthetic gate surfaces
   (validates P3).

**Cost:** half a day. **Risk:** low.

---

## 4. Suggested sequencing

| Order | Items | Days | Notes |
|------:|------|----:|-------|
| 1 | P1 + P6 + X1 | 2 | Behavior-preserving foundation. No user-visible change. |
| 2 | P5 | 1 | Conditional edges + max_iterations. Pure additive. |
| 3 | P7 | 0.5 | Delta coalescing. UI latency win. |
| 4 | S1 + S3 | 1 | SSH stale-session + spawn cwd validation. |
| 5 | P3 | 1 | Synthetic gate on mid-step interrupt. Needs UI buy-in. |
| 6 | X2 | 0.5 | Integration tests covering items 1–5. |
| 7 | P4 | 2–3 | Real parallel subtasks. |
| 8 | S4 | 1 | Retry on SSH drop. |
| 9 | S2 / S5 / S6 | 1–2 weeks / 0.5 / 0.5 | Deferred follow-ups. |

**Total foundation + DAG correctness (items 1–6):** ~6 days.

---

## 5. Done-means per item (verification commands)

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib step_executor
cargo test --manifest-path src-tauri/Cargo.toml --lib ssh

# Manual smoke:
# 1. Launch a feature with a 5-step workflow
# 2. Kill demeteo mid-step
# 3. Relaunch — verify synthetic gate surfaces
# 4. SSH to the target, kill the sshd mid-step — verify retry kicks in
# 5. Inspect `demeteo.db` after each: step_executions rows must be in a
#    single coherent terminal state (no "running" rows older than the
#    kill timestamp)
```

---

## 6. Cross-references

- [`DECISIONS.md`](DECISIONS.md) decisions 14, 15 — feature re-entry, telemetry.
- [`DDD_MODEL.md`](DDD_MODEL.md) §4 Feature Orchestration invariants.
- [`ARCHITECTURE.md`](ARCHITECTURE.md) §2 Port Catalogue (StepExecutor, GatePresenter).

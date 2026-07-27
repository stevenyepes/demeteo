# Reliability — Invariants and the Open Backlog

> **Scope:** silent-failure modes, lost work on crash / network drop, and
> accumulated state drift in the `StepExecutor` pipeline and the SSH transport.
> Most of the original plan shipped; what remains here is the **invariant set**
> (which constrains anything new) and the **four items still open**.
>
> Cross-references: [`DECISIONS.md`](DECISIONS.md) decisions 14, 15 (feature
> re-entry, telemetry) · [`DDD_MODEL.md`](DDD_MODEL.md) §4 Feature Orchestration
> · [`ARCHITECTURE.md`](ARCHITECTURE.md) §2 Port Catalogue ·
> [`EXECUTION_PARITY.md`](EXECUTION_PARITY.md) for the transport contract and
> failure triage.

---

## 1. Invariants any reliability work must preserve

1. **Features on one project run concurrently** — a project may have N features
   in flight at once (Decision 18, which supersedes the original strict-serial
   answer). Nothing in a run may assume it is alone on the repo. Concretely:
   worktree paths must be feature-scoped, branch refs are already disjoint per
   feature, and **build output (`node_modules`, `target`, `.venv`) must never be
   shared across features** — only content-addressed download caches may be.
2. **Per-step checkpoints are atomic** — a step is "complete" only when its
   artifact is written and, if it's a gate, its decision is recorded.
3. **Cost and duration are computed at step completion, never estimated
   mid-step** (Decision 15).
4. **Step transitions are the UI contract, not agent transcripts.**
5. **Keyring + ssh-agent for credentials, never plaintext.**

---

## 2. Shipped

Now active behaviour; the code is the record. Cancel-vs-future distinction ·
atomic state transitions · resume from interruption · `parallel` planner fan-out
· conditional edges / `max_iterations` · `accumulated_cost` on the driver ·
coalesced agent events to the UI · `spawn_interactive` cwd + binary validation ·
typed pipeline state · the `step_executor` integration suite · the
predecessor-running guard.

The **predecessor-running guard** is worth knowing about before touching gate or
retry handling, because its origin is not obvious from the code: the pipeline
view could show a stale `awaiting_gate` chip on a step the user had already
retried, so "Retry Step" / "Approve Gate" reached `replay_steps_from` and the
gate waiter while the predecessor agent was still in flight. It is defence in
depth — a backend invariant *plus* a UI blocking contract — and removing either
half reopens the race.

Everything from the DAG rework (durable checkpoints, `step_attempts`,
declarative retry policy, unified `run_events`) landed under
[`PRD_DAG_WORKFLOWS.md`](PRD_DAG_WORKFLOWS.md) Phase 1 and superseded the
in-memory checkpoint items this plan originally carried.

---

## 3. Open

> Paths below are `crates/demeteo-core/src/`. Re-verify with `git grep`.

### S1. Stale sessions stay in cache — **[Partial]**

**Where:** `adapters/ssh/`

The session cache evicts on the next probe failure. A half-open connection (TCP
up, SSH dropped) still serves commands that time out rather than reconnecting
immediately. The keepalive-ack no-progress abort addressed the *symptom* of a
wedged read; the cache itself is still not health-checked.

**Fix:** track `last_used: AtomicU64` on `SftpSession` and probe `sess.closed()`
before returning a cached session; add a background reaper for sessions idle
> 5 min.

### S2. `Sftp` is serialized by a single `Mutex<Sftp>` — **[Partial]**

**Where:** `adapters/ssh/`

`ssh2::Sftp` is not thread-safe; the mutex-per-session design serializes SFTP
ops for one machine across the whole app.

**Fix:** `tokio::sync::Mutex` plus a "never hold across awaits" lint;
longer-term, migrate to `russh`.

### S4. Retry on transient SSH drops — **[Open]**

**Where:** `adapters/ssh/`

One dropped network = full pipeline stop. The driver only sees the subprocess
return and marks the step `Failed`.

**Fix:** a `with_ssh_retry(future, attempts: u32)` wrapper at the `ExecutionPort`
boundary that re-establishes the session on `Err(SshError::ConnectionLost)` and
re-execs the call. Note this must not swallow the transport-failure distinction
the verifier depends on — see [`EXECUTION_PARITY.md`](EXECUTION_PARITY.md).

### S5. Port-forwarding state not covered by watchdog — **[Open]**

**Where:** `src-tauri/src/forward.rs`

Listeners aren't torn down when a machine is deleted or the SSH session drops.
Symptom: deleted-machine forwards keep accepting connections until app restart.

**Fix:** add `ForwardState::prune_for_machine(machine_id)` and call it from
`commands::machine::delete_machine` and on connection-drop in S1.

### S6. `RouterExecutionPort` string-based dispatch — **[Audit only]**

**Where:** `adapters/router.rs`

Resolves `auth_type` via `match` with a default branch. Audit before changing
anything; the fix, if one is needed, is a typed
`RouterError::UnknownAuthType(String)` bubbled up.

---

## 4. Verifying reliability changes

`npm run checks`, plus the manual crash smoke that no suite covers:

1. Launch a feature with a multi-step workflow.
2. Kill demeteo mid-step.
3. Relaunch — a synthetic gate must surface.
4. SSH to the target, kill `sshd` mid-step — verify the transport failure is
   classified as infrastructure, not as a code regression.
5. Inspect the DB after each: `step_executions` rows must be in a single
   coherent terminal state (no `running` rows older than the kill timestamp).

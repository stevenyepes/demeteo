# Reliability — Invariants and the Open Backlog

> **Scope:** silent-failure modes, lost work on crash / network drop, and
> accumulated state drift in the `StepExecutor` pipeline and the SSH transport.
> Most of the original plan shipped; what remains here is the **invariant set**
> (which constrains anything new) and the **items still open**.
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
   artifact is written and, if it's a gate, its decision is recorded. Inside a
   `sequence` step the unit is finer: a **task** is durably landed the moment
   its commit exists, not when the step returns. Anything that records landed
   work only on a code path the process has to *survive* to reach is not a
   checkpoint — a kill lands between any two instructions, and the twenty
   finished tasks it forgets get re-run and re-paid for.
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

**Sequence crash-resume (V35)** completes that work for the case Phase 1 could
not express. V32 recorded landed task ids, but only from the mid-list *failure*
path — which merges the prefix to the feature branch before recording it, so
"skip these ids" was a complete instruction. A killed process never reaches that
path: its finished tasks stay committed on the step branch, which the next
attempt's `provision_subtask_worktree` resets away. So a 25-task step
interrupted at task 21 restarted at task 1, and the UI's `0/25 landed` was
correctly reporting an empty checkpoint next to twenty `COMPLETED` rows.

Now the task loop checkpoints each task as it commits, pinning the prefix with
`refs/demeteo/seq/<feature>/<step>` (a shared ref, so `git gc` cannot reclaim
it and provisioning cannot orphan it) and storing that commit as the row's
`anchor_sha`. At resume `merge-base --is-ancestor` asks the repo which shape it
is looking at — prefix already merged (skip the ids) or stranded on the step
branch (restore the worktree onto the anchor first, *then* skip them) — because
the row itself cannot say, both writers produce one, and guessing wrong either
re-runs paid work or drops it. Every uncertainty, including an unreachable
machine, resolves to a full re-run.

Two consequences worth knowing before touching this: `cleanup_and_rollback`
deliberately does *not* clear the checkpoint (it moves the feature branch, the
anchor lives on the step branch, and `base_sha` is always captured after any
earlier merge), and `step_retry` keeps the prefix while `replay_from_step`
drops it — a retry resumes, an explicit redo starts over.

---

## 3. Open

> Paths below are `crates/demeteo-core/src/`. Re-verify with `git grep`.

### S1. Stale sessions stay in cache — **[Partial]**

**Where:** `adapters/ssh/session.rs` (`SessionPool`)

The session cache evicts on the next probe failure. A half-open connection (TCP
up, SSH dropped) still serves commands that time out rather than reconnecting
immediately. The keepalive-ack no-progress abort addressed the *symptom* of a
wedged read; the cache itself is still not health-checked.

**Fix:** track `last_used: AtomicU64` on `SftpSession` and probe `sess.closed()`
before returning a cached session; add a background reaper for sessions idle
> 5 min.

### S2. `Sftp` is serialized by a single `Mutex<Sftp>` — **[Partial]**

**Where:** `adapters/ssh/sftp.rs` (`with_sftp`), `adapters/ssh/session.rs` (`SessionPool::get`)

`ssh2::Sftp` is not thread-safe; the mutex-per-session design serializes SFTP
ops for one machine across the whole app.

The lock is held for the **whole** operation, not just the `ssh2` call:
`with_sftp` takes the guard and hands it to the closure, which then runs the
entire transfer under it. Pushing the `demeteo-runner` binary therefore blocks
every other SFTP op on that machine for the duration of the upload, with no
ceiling — and because `SessionPool::get`'s liveness probe is itself a `readdir`
on the same mutex, a large transfer also delays the health check that decides
whether to reconnect. That is the head-of-line blocking behind S1's symptom.

**Fix:** `tokio::sync::Mutex` plus a "never hold across awaits" lint;
longer-term, migrate to `russh`. Narrowing the guard to the individual `ssh2`
call is not sufficient on its own — `File` borrows the session, so the read/write
loop genuinely needs it held; the transfer has to move off the shared handle.

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

### S7. `control_rpc` drains on the 30-minute transport cap — **[Open]**

**Where:** `adapters/ssh/control_rpc.rs`

`TRANSPORT_WALL_CAP` is sized for a one-shot command that may legitimately go
quiet for half an hour while a build compiles in silence. A control-socket
round-trip to `demeteo-runner` is a single request/response and has no such
excuse, but it still drains on that cap.

The 30 minutes is reachable: `NO_PROGRESS_ABORT` only fires once keepalives stop
being acked, so it does not cover a runner that holds the socket open while
never answering. `remote_runner_status` is on this path, and the Machines view
probes every configured machine on mount — so the visible symptom is a status
row that spins indefinitely rather than reporting a dead runner.

**Fix:** give it its own `DrainBudget`, the way the HOME probe now has
`HOME_PROBE_CAP` (`adapters/ssh/home.rs`). The budget type already carries the
cap alongside the deadline, so the timeout message names whichever budget
actually expired; only the constant and the call site need to change. Pick the
value against the runner's slowest legitimate control method, not against the
status probe.

### S8. `RouterExecutionPort::resolve` hits the DB on every forwarded call — **[Open]**

**Where:** `adapters/router.rs`

Every method the router forwards first calls `resolve(machine_id)`, which is a
synchronous `MachineRepository` query behind the connection mutex — so it is
paid per `run_command`, per `read_file`, per `resolve_user`, on whichever thread
the caller is on. The router is the impl actually wired into the app, so this is
on every remote call, not a corner case.

Unlike the HOME lookup there is no cache in front of it, and unlike the SSH
adapter's own methods the router does not put it on the blocking pool.

**Fix:** cache the resolved `Machine` per `machine_id` with invalidation on
machine update/delete (the same commands S5 needs a hook in), or hand the router
a pre-resolved handle at composition time. Whichever way, the lookup should not
be a synchronous DB hit on a runtime thread — see the `spawn_blocking` treatment
`SshClientAdapter::resolve_user` now gets.

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

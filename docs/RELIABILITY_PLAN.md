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
`anchor_sha`. At resume `git merge-base` asks the repo which shape it is
looking at — prefix already merged (skip the ids) or stranded on the step
branch (restore the worktree onto the anchor first, *then* skip them) — because
the row itself cannot say, both writers produce one, and guessing wrong either
re-runs paid work or drops it.

**Every uncertainty resolves to a full re-run**, and the probe is shaped so that
this needs no judgement: `merge-base --is-ancestor` returns its verdict in the
*exit code*, which `ExecutionPort` flattens into one indistinguishable `Err` for
"no", "unreachable machine" and "corrupt object" alike — so the plain
`merge-base` is asked instead and the answer read off stdout. Anything that is
not a printed SHA is unknown, and unknown re-runs.

Four consequences worth knowing before touching this:

- **A checkpoint covering the whole plan means the whole plan is done.** The row
  names every id from the moment the last task commits until the step completes,
  and that window spans the declared-artifact check, the verifier's agent pass
  and the final merge. `apply_landed_checkpoint` used to read all-ids-matched as
  a stale row and put the full plan back — correct under V32, where the only
  writer left a task unlanded by construction, and a 25-task re-run under V35. It
  now leaves nothing to run and the step resumes into its own tail. Two things
  there assume a task ran and are skipped accordingly: the `never_produced` gate
  (nothing emitted anything this attempt) and the in-memory artifact refs, which
  are recovered from the store instead.

- **The checkpoint rolls back with the branch.** Since the row now names a commit
  the next attempt will `reset --hard` onto, a rollback that left it standing
  would hand the retry the very work it discarded — a verifier's rejection
  reinstated, a cancel undone. `cleanup_and_rollback` therefore rewinds the row
  and the ref to the state the attempt *started* from, which keeps an earlier
  attempt's merged prefix (that work is on the feature branch and this rollback
  never touched it) while dropping this attempt's claim. The single exception is
  the mid-list failure whose prefix *merge* failed: those tasks finished, their
  commits are pinned, and restoring them beats re-paying for them.
- **`step_retry` keeps the prefix while `replay_from_step` drops it** — a retry
  resumes, an explicit redo starts over.
- **Only planner-sourced steps prefer their cached plan on resume.** The cache
  exists to keep task *ids* stable, and a planner pass re-decomposing from
  scratch is the only thing that breaks them. A `task_list_from` step re-reads
  its artifact, because that artifact is both the id source and the thing a gate
  redirect revises — answering from cache there would silently run a superseded
  task list.

---

## 3. Open

> Paths below are `crates/demeteo-core/src/`. Re-verify with `git grep`.

### S1. Stale sessions stay in cache — **[Partial]**

**Where:** `adapters/ssh/session.rs` (`SessionPool`)

The session cache evicts on the next probe failure. A half-open connection (TCP
up, SSH dropped) still serves commands that time out rather than reconnecting
immediately. The keepalive-ack no-progress abort addressed the *symptom* of a
wedged read; the cache itself is still not health-checked.

S4's retry loop now evicts before each attempt, so a half-open session is no
longer *reused* by the call that tripped over it — but that is a reaction to a
failure that already happened, not the health check this asks for. The cost of
the missing check is unchanged: the first caller still pays the timeout.

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

### S4. Retry on transient SSH drops — **[Fixed]**

**Where:** `adapters/ssh/retry.rs`, `adapters/ssh/client.rs`

One dropped network = full pipeline stop. The driver only sees the subprocess
return and marks the step `Failed` — so a run forty minutes and twenty dollars
in died on a blip that lasted three seconds.

This entry originally proposed re-establishing on `Err(SshError::ConnectionLost)`.
**There is no `SshError` and no `ConnectionLost`**: every `ExecutionPort` method
returns `Result<T, String>` by design (D3), and a dropped connection is
identifiable only by the `TRANSPORT_ERROR_PREFIX` that `transport_err` puts on
the message. Establishing what a drop *is*, at a boundary that has only strings,
was most of the work — and the answer turned out not to be a variant at all.

**Fixed by** `with_ssh_retry`, wrapping the blocking half of each port method in
`client.rs`, under one rule:

> **A call is retried only when the remote was handed nothing.**

That rule, not a per-method idempotency table, is the safety argument.
`run_command_with` executes arbitrary user shell — a `git commit`, an `npm
publish`, a merge — and the port carries no idempotency signal, so nothing at
this layer can decide whether re-running one is safe. What it *can* decide is
whether the command reached the wire, and `command::run_blocking` now reports
that: the boundary is `channel.exec`. Before it, the session or the channel
failed and the remote shell was handed nothing, so re-running is side-effect
free **whatever the operation does**. From it onward the request may have been
delivered without its reply, and once the command is running the remote process
outlives the channel — so a retry would run a second copy alongside the first.

The three-way grading (`NeverReachedRemote` / `RemoteMayHaveRun` / `Answered`)
is therefore not decoration. Only the first retries; the second is a genuine
transport failure that is deliberately *not* retried; the third is a verdict.

Six things worth knowing before touching this:

- **An exhausted retry returns the last failure verbatim.** Nothing is rewrapped,
  so `transport: ` still leads, `classify_exec_failure` still answers
  `Transport`, the verifier still routes to a non-retryable `Infrastructure`,
  and `preflight` still declines to read it as a missing binary. A wrapper that
  rewrote a persistent drop into any other class would silently reopen every
  hole C0.2/D3 closed — which is why the guard for it asserts through the
  classifier, not the string.
- **A retry that succeeds leaves no trace.** The caller's contract is unchanged;
  a call that recovered on attempt two is indistinguishable from one that worked
  first time. A caller that could tell would start branching on it.
- **`ShellOptions::timeout` is spent across the whole call, not per attempt.**
  S10's harness deadline survives intact rather than becoming `attempts ×
  ceiling`. When there is time left but not enough for another backoff, the
  transport failure already in hand is surfaced rather than a manufactured
  timeout — it is the more specific answer and the one D3 routes correctly.
- **Cancellation needs no machinery.** The backoff is a plain `.await`, so a
  caller racing this against `cancel_watch` (as `run_harness_command` does)
  drops the future and the wait ends at once. A blocking sleep there would have
  resurrected S10 in a new place.
- **The pooled session is evicted between attempts.** Without it a retry reuses
  the corpse: `SessionPool::get`'s liveness probe is an SFTP `readdir`, which a
  half-open connection can still answer while `channel_session` fails.
- **Authentication failures are excluded by name.** They arrive as
  `NeverReachedRemote` and would otherwise be retried; repeating a rejected
  credential cannot succeed and is how an account gets locked. A *handshake*
  failure is deliberately not on that list — it is what a restarting sshd
  produces, and it is the exact error a real dropped session presents on
  reconnect (`Failed getting banner`), so listing it would disable the feature.

`test_connection`, `control_rpc` and `spawn_interactive` are **not** retried.
The first two are reachability probes whose answer is "can this connect right
now", and `control_rpc` additionally carries side-effecting runner methods and
backs the Machines-view status probe S7 already flags as slow. `spawn_interactive`
cannot be retried at all: a live PTY cannot be transparently re-established
under a caller already holding the handle — which is also the honest limit of
this fix, since an agent turn dies with its session either way.

What this deliberately does **not** cover: a drop in the middle of a long
command. That is `RemoteMayHaveRun`, and absorbing it would mean re-running
work that may still be in flight.

Proved against a real sshd rather than a double — the retry depends on
`SessionPool` evicting a corpse and `ssh_util::connect` re-handshaking, neither
of which exists in a fake. `run-ssh-conformance.sh` now drives a TCP relay in
front of the container that can drop every live connection: a 300ms outage must
be absorbed transparently, and one that outlasts the whole budget must still
classify as `Transport`. The two halves pull in opposite directions, so neither
a wrapper that never retries nor one that swallows the distinction can pass both.

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

### S9. `replay_from_step` leaks the sequence checkpoint ref — **[Open]**

**Where:** `adapters/step_executor/impl_traits/replay.rs`, `steps/sequence/mod.rs`

A replay drops the `sequence_checkpoints` row but not the
`refs/demeteo/seq/<feature>/<step>` ref that pins the prefix's commits, because
deleting the ref needs a resolved execution context (repo path + machine) purely
to tidy up. The row is the authority — with it gone the resume reads "no
checkpoint" and re-runs — so this is correctness-neutral, and the step's own
completion path deletes the ref next time it runs.

The gap is the feature that is **abandoned** rather than re-run: nothing ever
runs that completion path, so the ref pins the discarded attempt's commits for
the life of the repo and `git gc` can never reclaim them. Repo growth, not
incorrect behaviour — but unbounded, and invisible to the user because the ref
lives outside `refs/heads` and so never appears in `git branch`.

**Fix:** sweep `refs/demeteo/seq/<feature>/*` where the feature is deleted or in
a terminal state with no checkpoint row — either at feature deletion (which has
the project context the replay path lacks) or as a periodic reaper alongside
whatever eventually collects abandoned worktrees.

### S10. The prepare/test harness has no deadline and cannot be cancelled — **[Fixed]**

**Where:** `adapters/step_executor/driver/verifier.rs` (`run_harness_first`, `harness_shell_options`)

`harness_shell_options` builds from `ShellOptions::login_interactive()`, whose
`..Self::default()` leaves `timeout: None`, and `run_harness_first` then awaits
`exec.run_command_with(..)` directly — no `tokio::select!` on `cancel_watch`. The
one user-authored command the orchestrator ever executes therefore runs unbounded
*and* uninterruptible: the step shows no progress, spends nothing, and survives
Stop until the app restarts.

Nothing upstream covers it. The 1800s `wall_cap_s`
(`adapters/agent/event_stream/turn.rs`) bounds an agent *turn*, and the harness
runs before the turn starts. The `command` node type — same user-authored shell,
built from the same `harness_shell_options` — *does* wrap its call in a `biased`
select on `cancel_watch` (`adapters/step_executor/steps/command.rs`), so today the
two callers of one primitive disagree about whether Stop works.

The trigger is the default, not an exotic config: `detect_worktree_strategy`
(`adapters/worktree/git_ops/strategy.rs`) maps a root `package.json` to a bare
`npm test`, which resolves to watch mode on a large class of projects — the
Stratosbar project in the dev DB has `"test": "vitest"`.

**Fixed by** three changes that only work together — shipping the deadline alone
would have made things strictly worse than the hang:

1. `harness_shell_options` now carries the run's `wall_cap_s` as an explicit
   `ShellOptions::timeout`, reusing the existing preferences knob rather than
   adding a second one. The `command` node keeps overriding it with
   `spec.timeout`, so that path is unchanged.
2. Both `run_command_with` calls in `run_harness_first` go through
   `run_harness_command`, which races them against `cancel_watch` in a `biased`
   select — the same mechanism `steps/command.rs` uses, so the two callers of
   `harness_shell_options` no longer disagree about whether Stop works. Dropping
   the future is what stops the work; the local adapter kills the process group.
   A new `VerifierError::Cancelled` carries that outcome to
   `StepOutcome::Cancelled` instead of dressing a cancel as a failure.
3. `classify_exec_failure` replaces `is_transport_failure`. It is pure over the
   error string and returns `Transport` / `Timeout` / `NonZeroExit`, which is the
   distinction that matters: **only a non-zero exit reached a verdict.** Before
   it, `verifier.rs` never mentioned `TIMEOUT_ERROR_PREFIX` at all, so a
   newly-added deadline would have fallen through to `classify_harness_failure`
   and opened a rework loop against code that was never tested. A `Timeout` now
   terminates as `Environment` with remediation naming watch mode, mirroring the
   exit-127 path; `Transport` keeps its existing `Infrastructure` routing, which
   already avoided a verdict.

The classifier's unit tests include the case that was watched fail first — a
`timeout:`-prefixed error classifying as `NonZeroExit` — plus a guard that the
prefixes must *lead* the string, so a suite that prints the word `timeout: ` in
its own output cannot rewrite its own classification.

### S11. A green harness's stderr is discarded before it reaches the validate agent — **[Fixed]**

**Where:** `adapters/step_executor/driver/verifier.rs` (`run_harness_first`), `adapters/local/execution.rs` (`command_result`), `adapters/ssh/command.rs` (`run_blocking`)

Both transports return stdout alone on success and merge stderr only on failure.
That is the D3 contract, identically honoured, and not a bug in either adapter —
the bug is that `run_harness_first` does not compensate for it. The `command` node
type does, wrapping its command as `( … ) 2>&1` with a comment recording exactly
why: a green `cargo test` or `npm run build` — both of which "report almost
entirely on stderr" — would otherwise file an empty artifact.

The validate path has the same problem with a worse consequence. An empty output
block is not filed as an artifact nobody reads — it is injected into the agent's
prompt under a heading asserting the harness already ran, followed by a claim
that the results are authoritative and a ban on re-running anything. The agent is
handed nothing and told it is evidence.

**Fixed by** extracting `merge_stderr_into_stdout` and routing *both* callers
through it — `run_harness_first` and `steps/command.rs`, which previously carried
its own copy of the same literal. Sharing it is the point: these two are the only
places a user-authored command's output is shown to somebody, and they had
already drifted once. The exit status is the subshell's last command's, so the
pass/fail gate is unaffected, and the harness section is the same shape green or
red. Its tests cover the two shapes that actually break: a command ending in a
`#` comment (which would swallow the closing paren without the newlines) and the
`set +e; …; exit $rc` accumulator `detect_worktree_strategy` emits for a polyglot
repo.

### S12. The no-harness fallback renders under an "already executed" heading — **[Fixed]**

**Where:** `adapters/step_executor/driver/verifier.rs` (`run_harness_first`), `adapters/step_executor/steps/agent/mod.rs`

When `test_command` is unset, `run_harness_first` returns the "No test harness was
configured or detected for this project" string on the `Ok` path, indistinguishable
to its caller from a real result. `agent/mod.rs` then injects it under
`## Harness Results (already executed by the orchestrator)`, followed by "the
results above are authoritative" and "Do NOT re-run the build or test suite".

An agent told that nothing ran, that the nothing is authoritative, and that it may
not check for itself has one coherent move left, and it certifies a feature nobody
tested.

Commit `2257ffb` fixed the *prompt* side of this — the shipped verifier
instructions now require the agent to read what the block actually says — and
recorded the engine-side hole as an explicit follow-up. This entry is that
follow-up; the prompt fix is a mitigation resting on the agent obeying prose that
the surrounding template contradicts.

**Fixed by** `HarnessOutcome` — `run_harness_first` now returns `Ran { name,
cmd, output }` or `NotConfigured` instead of a pre-rendered string, and the
heading moved *into* `render_section` on the type. That coupling is the fix, not
a detail: a caller that cannot choose the heading cannot put "already executed by
the orchestrator" above an absence. The `NotConfigured` block names what did not
happen, refuses the inference explicitly ("an absence of evidence, not a passing
result"), drops the do-not-re-run ban that only makes sense for a real result,
and points at the `environment` verdict — which is what S13 made reachable.

### S13. The validate verdict schema omits `environment`, which its own instructions require — **[Fixed]**

**Where:** `adapters/step_executor/steps/agent/mod.rs`, `adapters/step_executor/driver/verifier.rs` (`parse_verdict_text`)

`parse_verdict_text` accepts `"environment"` and routes it to a terminal
`NonRetryable` — the deliberate escape hatch for criteria no amount of
re-implementation can satisfy, because the project is not configured to run the
command they demand. The shipped `s-validate` verifier instructions tell the agent
to use it. But the JSON contract appended to the prompt offers only
`{ "<key>": "pass" }` and `{ "<key>": "fail", … }`, and the re-ask correction
issued on a `Missing` verdict repeats the same two-option schema. The escape hatch
is described in prose and absent from the shape the agent is told to emit.

It is not a theoretical gap. In feature `f-1785157902856` the validate agent's own
report named criterion 1 unprovable — "The supplied harness proves only
`cargo test`" — and returned `fail`, because that was the only option the schema
gave it. The resulting `verdict.redirect` rework cycle cost **$14.63 / 11.0M
tokens**, with a second cycle still running when this was written, re-implementing
a feature whose defect was a project setting.

**Fixed by** `verdict_contract`, a pure function that renders the menu with all
three verdicts plus the discriminator for choosing `environment` over `fail`, and
by giving the re-ask correction the same three options — a correction that
silently dropped `environment` would push a correct judgement into `fail` on the
retry. Engine-side parsing already accepted it and is unchanged. The contract
honours a custom `verdict_key`, which a test pins: hard-coding `"verdict"` there
would emit a contract `parse_verdict_text` cannot satisfy.

### S14. A failing verdict skips the declared-artifact check — **[Fixed]**

**Where:** `adapters/step_executor/steps/agent/mod.rs`

`missing_artifacts` is computed alongside the artifact paths early in the step and
consumed much later, in the merge-result match. The verdict block sits between
them and returns `StepOutcome::VerdictFailed` directly, so a step that fails on a
verdict never reaches the check — and never records the artifact paths it did
produce either.

Observable in the dev DB: `step_executions.artifact_paths` is `[]` for
`s-validate` on `f-1785157902856` even though `validation-report.md` exists on
disk. The two failure modes the check exists to separate — "the agent judged the
work and rejected it" and "the agent never produced its deliverable" — are
indistinguishable from the row, which is precisely the signal the check was added
to give.

**Fixed by** persisting `artifact_path`/`artifact_paths` on the verdict-failure
path and appending an undelivered-artifact note to the verdict's reason via
`note_undelivered_artifacts`.

Deliberately *not* by reordering the two checks, which is what this entry
originally proposed. Making a missing artifact win would replace the verdict's
reason — the actionable part, and the thing the rework step decomposes into
tickets — with a generic "declared artifact never produced". The verdict still
leads; the missing report is appended, because the consuming step attaches the
report by name and needs to know there is nothing there. A *passing* verdict
still falls through to the ordinary check, which already covered it.

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

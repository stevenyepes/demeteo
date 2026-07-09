# Demeteo: Remote Unattended Execution

> **Design doc for running workflows on remote machines with the desktop app
> closed.** See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the hexagon and port
> surface this builds on, [`DDD_MODEL.md`](DDD_MODEL.md) for the ubiquitous
> language (Machine, Feature, Gate, Provider Instance, Permission Profile),
> [`RELIABILITY_PLAN.md`](RELIABILITY_PLAN.md) for the durable-state machinery
> reused here, and [`DECISIONS.md`](DECISIONS.md) for the master decision table.
> This doc is the source of truth for the **Remote Runner** feature.

## Phase Placement Key

- **v1.x** — any 1.x release; no specific commit promised.
- **v2+** — major version; not before the remote runner is stable.

This feature targets **v2+**: it moves the control plane, which is a larger
change than the incremental remote-*execution* plumbing that already exists.

---

## 1. The core problem: today the laptop *is* the control plane

The `ExecutionDriver` (`adapters/step_executor/driver.rs`) runs as an
**in-process `tokio` task inside the desktop app**. It holds `Arc`s to the
SQLite repos, the in-memory `GateWaiter` (`tokio::sync::Notify`), the agent
registry, and the execution ports. Remote **Machines** today are only
*execution hands*: the driver stays on the laptop and reaches out per-command
over SSH (`ExecutionPort::run_command(machine_id, cmd)`, sftp for files).

Consequence: **close the laptop → the tokio task dies → the run stops.** The DB
persists state and the driver is respawned + reconciled on next launch
(`driver_registry`, `impl_traits/replay.rs`; the `GateWaiter` doc explicitly
handles "app restart"), but **nothing advances while the app is closed.**

So "run workflows without demeteo always open" is *not* an increment on top of
the remote-machine plumbing. It requires **moving the brain, not just the
hands.** The hexagonal ports/adapters layout is almost purpose-built for this,
and the hard parts — durable state, gate reconciliation, driver respawn —
already exist.

---

## 2. Locked decisions

| #  | Decision | Answer |
|----|----------|--------|
| R1 | Where the brain runs | A headless `demeteo-runner` binary on the **remote machine**, reusing the same engine crate. Laptop becomes a client/observer. |
| R2 | Platform scope | **Linux only.** The runner is supervised by a **systemd `--user` unit with lingering enabled** (`Restart=always`). No macOS/launchd or bare-`tmux` tier — a non-Linux machine is simply ineligible for remote runs. |
| R3 | Result transport | **Git.** The runner pushes the feature branch to `origin`; the laptop `git fetch`es on return. Only run *metadata/events* use the control channel. |
| R4 | Control channel | Small RPC over a **unix-domain socket on the remote, reached through the existing SSH port-forward** (`forward.rs`). No new listening port; authz inherited from SSH login. |
| R5 | Credentials | **Split model.** *Coding-agent auth is a machine precondition* — the remote machine must already be authenticated and configured for the selected agent (matches today's host requirement). *Git-provider push access is provided by Demeteo* — it injects the Provider Instance PAT it already holds, **memory-only** and run-scoped, so the machine carries no standing git secret. See §6. |
| R6 | "Unattended" scope | Auto-approves **Gates only**. The per-command permission/intercept layer and the worktree filesystem fence **stay on**. See §5. |
| R7 | Gate policy shape | Not a boolean. Gates are classified by blast radius; unattended auto-approves the safe class and **parks** the dangerous class for the human. See §5. |
| R8 | Cancellation | Explicit `cancel_run` RPC only. Laptop disconnect / app close **never** cancels a run. |
| R9 | Source of truth | The **remote DB** owns a remote run. The laptop keeps a mirror keyed by `(machine_id, run_id)` and reconciles via an append-only event log by offset. |
| R10 | Terminal "PR ready" | On successful unattended completion the runner **auto-opens the PR/MR** via the existing idempotent `MrPublisher`. Opening a PR is safe/reviewable/reversible, so it is *not* a parked gate — only the *merge to default* is (§5). "PR ready" is the success terminal state; its notification carries the PR URL. See §8. |
| R11 | Reboot behavior | A machine reboot restarts the runner (systemd lingering, R2), which reconciles from SQLite, treats orphaned mid-step `running` rows as **interrupted**, and **auto-resumes** by re-running the interrupted step from its per-step checkpoint (Decision 14), under a bounded reboot-retry budget. **Laptop-unreachable ≠ run-failed.** See §7.1. |

> **Multi-client authz (extends R4).** R4's "authz inherited from SSH login"
> assumes *one* trusted laptop. When one shared runner serves **several**
> clients, SSH login alone puts every client inside the same trust boundary
> with mutual access to each other's runs. The runner therefore enforces
> **per-run ownership**: each client stamps a stable `client_id` (a persisted
> `install_id` UUID) into every control-RPC's `params`; `submit_run` records
> it as the run's `owner_client_id`; and every run-scoped RPC checks it,
> returning the *same* "no such run" error on a mismatch as for an absent run
> (no existence leak). `list_runs` is filtered to the caller. This is **soft**
> multi-tenancy (`client_id` is not a secret — it protects honest clients from
> cross-talk and bugs, not a malicious co-tenant) and is fully back-compatible
> (an old client sends no `client_id` and a new runner reads it as `""`, the
> single legacy tenant; an old runner ignores the field). Full design + build
> phases: [`MULTI_CLIENT_RUNNER.md`](MULTI_CLIENT_RUNNER.md).

---

## 3. Architecture

```
Laptop (client / observer)                 Remote Linux machine (control plane for its runs)
┌───────────────────────────┐  SSH tunnel  ┌───────────────────────────────────────────┐
│ Demeteo desktop app         │◄───────────►│ demeteo-runner (headless, same engine)     │
│  - launch UI (unattended)   │ control RPC │  - ExecutionDriver (identical code path)    │
│  - observer / return inbox  │ over unix   │  - own SQLite (source of truth for its runs)│
│  - mirror DB (cache)        │ socket      │  - agent CLI runs LOCALLY (pre-authed)      │
│  - holds git-provider PAT   │ + git PAT   │  - git PAT held in memory, run-scoped       │
└───────────────────────────┘  injection   │  - systemd --user unit, Restart=always      │
        ▲  git fetch                        └───────────────────────────────────────────┘
        └──────────────── origin (results travel as a pushed feature branch) ────────────
```

Two properties make this cheaper than it sounds:

1. **When the brain moves onto the machine, "remote execution" becomes "local
   execution."** The runner *is* the machine, so agent turns run local to it —
   no nested SSH, no per-command round-trips. The runner reuses the simpler,
   more reliable *local* `ExecutionPort` path.
2. **Results ride git, not the control channel** (R3). The durable transport for
   the actual work product is git itself; the control channel only carries
   metadata/events, which shrinks the sync-reliability surface dramatically.

`demeteo-runner` is a second binary in the same cargo workspace with a
**non-Tauri composition root**: it swaps the Tauri UI `NotificationPort` for a
webhook/email adapter and keeps the local `ExecutionPort`. Everything under
`domain/`, `ports/`, and the step-executor engine is shared verbatim.

---

## 4. Handoff protocol & lifecycle

Control RPC methods (over the SSH-tunneled unix socket, R4):

| Method | Semantics |
|--------|-----------|
| `submit_run(spec) -> run_id` | **Idempotent**, keyed by a laptop-generated UUID. Re-submit = no-op. Turns at-least-once delivery into exactly-once effect. |
| `inject_credentials(run_id, git_pat)` | Push the run-scoped git-provider PAT into runner memory (§6). Separate from `submit_run` so it can be re-supplied after a runner restart. (Coding-agent auth is *not* sent — it is a machine precondition.) |
| `list_runs()` / `get_status(run_id)` | Reconcile state on app open. |
| `stream_events(run_id, from_offset)` | Tail an **append-only per-run event log**; the laptop catches up on everything missed by offset — never relies on a live socket having been connected. |
| `decide_gate(run_id, gate_id, decision)` | Clear a **parked** gate remotely from the laptop. |
| `cancel_run(run_id)` | The only way to stop a run (R8). |
| `health()` | Heartbeat + version + capacity. |

**Lifecycle:** laptop composes the spec (description, workflow, agent/model,
unattended flag + gate policy + budget) → `submit_run` → `inject_credentials` →
runner persists the spec to its own DB → spawns the driver → returns the handle.
**The user may close the app immediately.** The remote DB is now source of truth
(R9); the laptop mirrors by `(machine_id, run_id)` and reconciles by event
offset.

---

## 5. Unattended & the two-layer security model

The most important insight from the current code: **there are two independent
approval layers, and "unattended" must relax only one of them.**

1. **Gates** — human checkpoints *between workflow steps* (`gate_decisions`
   table, `GateWaiter`). This is what the user means by "gates always approved."
2. **Per-command permission intercept** — the policy/allowlist layer
   (`domain/permission.rs`, `domain/intercept.rs`; `DirectExecutionPort` is the
   documented *no-policy* variant, and Machines already carry
   `auto_approved_rules` command allowlists). This plus the
   `external_directory: deny` worktree fence is the sandbox.

**Rule (R6):** unattended relaxes **gates**; it keeps the per-command policy
layer and the filesystem fence **on**. Auto-approving *gates* is not
auto-approving *arbitrary shell*. If a run dropped to the no-policy execution
path, the agent would be effectively unsandboxed on a box holding credentials —
that must never be the meaning of "unattended."

**Gate policy is not a boolean (R7).** Classify gates by blast radius:

- **Safe class** — review / informational gates, and merge to the *feature*
  branch. Unattended **auto-approves** these.
- **Dangerous class** — merge to the **default branch**, push to protected refs,
  deploy, delete, or **any action over the run's budget**. Unattended **parks**
  these for the human (surfaced in the return inbox, §7; cleared via
  `decide_gate`).

This is the pressure valve that keeps unattended *safe* without breaking the UX
promise ("close the laptop, come back to results"). It extends the existing
`auto_approved_rules` model from commands to gate classes. **Default policy:**
auto-approve review gates + auto-merge to the feature branch; park
merge-to-default and over-budget.

**Budget ceilings.** Hard per-run and per-machine caps on token cost and
wall-clock. Exceeding → park or stop; **never** auto-approve more spend. The
usage/cost accumulator already exists, so the hook is present.

**Immutable audit.** Every auto-approved gate, command, diff, and cost is
recorded on the remote and synced back, so the user can review exactly what ran
while away. Non-repudiation is the price of admission for unattended runs; logs
and the synced event stream must scrub secrets.

---

## 6. Credentials (R5)

The two credential kinds are handled **differently**, and the split is
deliberate.

### 6.1 Coding-agent auth — machine precondition (not injected)

The remote machine **must already be authenticated and configured** for the
selected coding agent (API key, model, CLI installed and logged in), exactly as
the README requires of any host today. Demeteo does **not** transmit coding-agent
secrets. Rationale: these are long-lived, per-machine identities that belong to
the operator of the machine; keeping them on the box (and out of the control
channel) is both simpler and a smaller blast radius than shuttling them per run.
A machine that is not agent-ready is ineligible for remote runs — surfaced at
launch as a readiness check.

### 6.2 Git-provider push access — provided by Demeteo (injected)

The runner needs git-provider access to fetch the repo and push the feature
branch to `origin` (R3). The machine carries **no standing git secret**; instead
Demeteo injects the **Provider Instance PAT it already holds**, per run.

This is not new behavior — it relocates an existing pattern. Today
`GitOpsHelper::clone_repository` (`adapters/worktree/git_ops/clone.rs`) reads the
PAT from the keyring on the laptop and ships it to a remote machine embedded in
the clone URL (`https://x-access-token:{pat}@host/repo`) over the SSH-executed
command. The runner design keeps this injection model but tightens one thing:
**the PAT must not land in the remote command line or URL** (where it is visible
in the runner's `ps`, shell history, and logs). Use an in-memory per-run
credential helper / askpass instead. Per run:

- Transmitted over the **SSH-tunneled control channel** (R4) via
  `inject_credentials` — never over a plaintext or separately-exposed channel.
- Held **in runner memory only**, scoped to the run. **Never** written to the
  runner's disk, SQLite, artifacts, git credential store, or logs.
- Used only for this run's fetch/push (e.g. via an in-memory credential helper
  or a per-run askpass), and **wiped at run end** (success, failure, or cancel).
- A compromised *idle* runner therefore has no git secret to steal — it exists
  only for the lifetime of an active run.

**Restart tradeoff.** Because the PAT is memory-only, a runner crash mid-run
loses it. If the run is at a point that needs git access (its terminal push, or
a mid-run fetch), it **parks in a `needs-credentials` state** and is re-injected
on the next laptop connection (or re-supplied via `inject_credentials`). The
work state survives in the runner DB (R9); only the ephemeral PAT needs
re-supply. Note the window is naturally narrow — push is a run-end operation —
so most of a run executes with no git secret resident at all. Prefer
short-lived / scoped tokens where the provider supports them (§10.3).

---

## 7. Reliability

- **Survive runner restart, not just laptop disconnect.** Supervise with a
  **systemd `--user` unit + lingering** (R2), `Restart=always`. On restart the
  runner reconciles from its own SQLite — reuse the existing driver-respawn +
  gate-reconcile machinery, run headless.
- **Decouple cancellation from disconnect (R8).** Closing the app or dropping
  the SSH tunnel must **not** cancel a run. This deletes the current assumption
  that a dead connection kills work.
- **Idempotent submit + event-log offsets (R9)** give clean catch-up and no
  double-starts.
- **Heartbeats / last-seen** let the laptop distinguish "running on a healthy
  runner" from "runner dead/unreachable."
- **Caps:** per-machine max concurrent runs (queue overflow) plus the existing
  per-step timeouts.

### 7.1 Machine shutdown & reboot

This is the load-bearing reliability case, and the reason R2 pins **systemd
`--user` with lingering** rather than a login-scoped supervisor.

- **Graceful reboot / `systemctl` stop.** systemd sends `SIGTERM` to the unit.
  The runner handles it: mark in-flight steps `interrupted`, flush the event
  log, checkpoint, exit cleanly. Lingering means the unit comes back **at boot
  without anyone logging in**, and the runner resumes from SQLite.
- **Hard power loss / OOM kill.** No chance to checkpoint. SQLite runs in **WAL
  mode** so the DB is crash-consistent. On restart the runner reconciles: any
  step still marked `running` with no live child PID is an **orphan** → marked
  `interrupted`.
- **Auto-resume (R11).** An interrupted step is re-run **from its per-step
  checkpoint** (Decision 14: "per-step checkpoints; synthetic gate on mid-step
  interrupt"). Because steps operate in a git worktree, the safe recovery is to
  reset the worktree to the step's base and re-run that step — the same
  machinery the retry loop already uses. A **bounded reboot-retry budget**
  prevents a crash-looping machine from re-running a run forever; exhausting it
  parks the run as `failed (unstable host)`.
- **The git PAT is gone after reboot** (memory-only, §6.2). Agent work that
  needs no git access resumes immediately; a step that needs fetch/push parks in
  `needs-credentials` until the laptop reconnects and re-injects. Since push is a
  run-end operation, most of a resumed run proceeds without it.
- **Laptop's view is not authoritative.** When the laptop can't reach the runner
  (machine off, network down), it shows **`unreachable · last seen <t>`** — it
  must **never** report the run as failed. Only the runner's own reconciled
  state is truth (R9); a machine that is merely off is a *paused*, not *failed*,
  run.
- **Permanent machine death (dead hardware).** Anything already pushed to
  `origin` is safe (R3) — completed steps and opened PRs survive. **Unpushed
  in-worktree WIP is lost with the machine.** Mitigation lever (optional):
  checkpoint-push the feature branch to `origin` at each completed step boundary,
  so partial progress is recoverable even if the box never returns. See §10.6.

---

## 8. UX / journeys

See [`UX_JOURNEYS.md`](UX_JOURNEYS.md) for the existing journey format.

- **Launch.** In the Start Feature modal, add "Run on machine ▾" (Machine
  already exists) + an **Unattended** toggle that reveals the gate policy (which
  classes auto-approve vs park) and a budget cap. After launch: a run badge
  *"Running on `<machine>` · unattended"* and an explicit reassurance —
  *"You can close Demeteo. This run continues on `<machine>`."*
- **Fire-and-forget many.** A fleet overview so the user can trigger several
  runs across machines and walk away.
- **Return experience (the "report back").** On app open, reconcile all runs
  across all machines into a **return inbox** grouped by terminal/actionable
  status. The taxonomy:

  | Status | Meaning | Notification on reopen |
  |--------|---------|------------------------|
  | **PR ready** | Completed; branch pushed; PR/MR auto-opened (R10) | ✅ raise, deep-linked to the PR URL + diff |
  | **Failed** | A step exhausted its retry budget, or a hard budget/policy stop tripped, or `failed (unstable host)` (§7.1) | ✅ raise, deep-linked to the failure reason + logs |
  | **Parked (needs you)** | Hit a dangerous gate (e.g. merge-to-default), or `over-budget` | ✅ raise; user clears via `decide_gate` |
  | **Needs credentials** | Runner restarted and lost the PAT (§6.2/§7.1) | ✅ raise; laptop re-injects on connect |
  | **Running** | Still executing on a healthy runner | silent (live-viewable) |
  | **Unreachable** | Machine off / network down (§7.1) | silent badge `last seen <t>`, *not* a failure |

  Parked/failed items surface at the top of the inbox.

- **Two notification channels — this answers "notify me when I reopen."**
  1. **Runner-push while the laptop is off.** The moment a run reaches a
     terminal/actionable state, the runner fires email/Slack/webhook/ntfy via the
     existing `NotificationPort` — so the user learns *before* reopening.
  2. **Laptop reconcile-on-reopen.** On app open, the laptop pulls each machine's
     runs, diffs against its mirror DB, and raises a **desktop notification** for
     everything that became **PR-ready** or **failed** (and parked /
     needs-credentials) *since last seen*. This is the "receive a notification
     when opening Demeteo again" behavior, driven off the reconciliation diff so
     nothing that happened while away is missed.

  "PR ready" leans on machinery that already exists: `MrPublisher::publish_mr`
  (idempotent) opens the PR at run end, and `mr_monitor` + `fetch_mr_state`
  already refresh MR state on launch and flip the feature to `completed` when it
  merges — so the return inbox shows live PR state, not a stale snapshot.
- **Live view when connected.** Tail the remote event log over the tunnel —
  identical UI to a local run.

---

## 9. Reuse map & build phases

| Phase | Deliverable |
|-------|-------------|
| P1 | `demeteo-runner` headless bin: non-Tauri composition root, local `ExecutionPort`, webhook/email `NotificationPort`. Runs a workflow end-to-end when invoked directly on a Linux box. |
| P2 | Control RPC module over `forward.rs` tunnel → unix socket. `submit_run` (idempotent) + `list_runs` + `stream_events` + `health`. Laptop mirror DB keyed by `(machine_id, run_id)`. |
| P3 | Git-PAT injection (§6.2): `inject_credentials`, memory-only run-scoped store, per-run git credential helper/askpass, wipe-on-end, `needs-credentials` park state. Plus an **agent-readiness check** (§6.1) at launch that rejects machines not pre-authed for the selected agent. |
| P4 | Unattended gate policy (§5): gate blast-radius classification, auto-approve safe / park dangerous, budget caps. Keep per-command policy + fence on. Auto-open PR at success via `MrPublisher` (R10). |
| P5 | Deployment: reuse SSH machine setup (`setup_commands`, agent install) to install/upgrade the runner + its systemd-user unit (lingering enabled), with a version handshake. |
| P6 | Reboot resilience (§7.1, R11): SIGTERM checkpoint handler, WAL mode, orphan-`running` reconciliation + auto-resume from checkpoint, bounded reboot-retry budget. |
| P7 | UX: launch toggle + policy/budget controls, run badge, return-inbox taxonomy, dual-channel notifications (runner-push + reconcile-on-reopen), live remote view. |

---

## 10. Decisions on the former open questions (M7.3)

All eight were resolved once M0–M7.1 actually shipped and the tradeoffs
stopped being hypothetical. Kept numbered to match the original list;
each entry says what's actually implemented today and why, or what's
deliberately deferred and under what condition it should be revisited.

1. **Runner ↔ engine versioning — no strict handshake; version-match is
   structural, not enforced.** `demeteo-runner --version` and the
   `health` RPC both expose a version string, but nothing refuses a
   mismatch today. The reason this is safe in practice: the only
   provisioning path (M7.1's `remote_enable_runs`) always SFTPs *this
   laptop's own build* over the existing binary and restarts the unit —
   there is no path today where a laptop talks to a runner it didn't
   just provision itself, so drift can't occur. CI now publishes a
   version-matched `demeteo-runner` release asset and the laptop can
   auto-fetch one when it doesn't already have a matching local build
   (`remote_runner_local_check`/`remote_runner_download`), but this is
   still laptop-side only — the remote box never `curl`s anything itself,
   and the fetched binary is checksum-verified against the exact version
   tag the running app reports, so the invariant above still holds.
   **Revisit when** a remote-side `curl`-on-the-box provisioning path
   ships instead (deliberately not built — remote machines aren't assumed
   to have internet access) — that's the path where laptop and runner
   versions could actually diverge, and that's when a real
   refuse-on-mismatch (or N-1 tolerance) check earns its complexity.
2. **Two laptops, one runner — no explicit arbitration lock; the
   existing idempotent operations already prevent the bad outcomes.**
   `submit_run` is keyed by a client-generated `run_id` (get-or-create,
   never a duplicate). `decide_gate` targets one `step_execution_id`,
   which the underlying `GateWaiter`/gate-decision machinery (shared
   with the desktop app) only accepts once — a second laptop deciding
   the same gate after it's resolved gets an error, not a silent
   double-apply. `cancel_run` uses `cancel_if_active`, a single
   conditional `UPDATE ... WHERE status NOT IN (terminal)` — not
   read-then-write — so two laptops racing a cancel can't stomp a real
   `awaiting_mr` outcome back to `cancelled` (this exact race was
   caught and fixed by testing during M3.3). The one real gap is
   **display** staleness: two laptops' mirror DBs can disagree about
   what they've last seen, but the runner's own DB stays the single
   source of truth (R9) and both mirrors reconcile against it
   independently — never against each other. No further work planned
   here.
3. **Scoped tokens — not implemented; standing PAT injection is what
   ships.** Minting short-lived, repo-scoped tokens (GitHub fine-grained
   PATs, GitLab project access tokens) is provider-specific plumbing
   with its own auth flow per provider, which is a bigger lift than this
   milestone's slice. Deferred as a follow-up; §6.2's in-memory-only,
   run-scoped, wipe-on-terminal-state handling already bounds the
   exposure window even with a standing PAT.
4. **Disk/DB exhaustion — not proactively handled; a write failure
   surfaces as an ordinary run failure.** There's no disk-space
   pre-check and no GC of old `runner_runs`/`run_events` rows. A SQLite
   write failure from a full disk propagates as a `Result::Err` through
   the normal error path and the run ends up `failed`, which is at
   least not silent corruption — but it's not a clean `failed (disk
   exhausted)` diagnosis either, and nothing reclaims space
   automatically. Suggested follow-up if this becomes a real problem in
   practice: detect the specific I/O error class and report it as such,
   plus a time-based retention GC for terminal rows older than N days.
5. **Default-branch parking granularity — global only, via the
   workflow, not per-project.** `StepConfig.gate_class` (M5.1) is a
   per-*workflow-step* field, not a per-*project* setting — every
   unattended run parks whatever the workflow's `s-gate-ship` step (or
   equivalent) is marked, and the three bundled workflows all mark it
   `dangerous`. This is coarser than per-project but strictly more
   flexible than "park everything, no exceptions" — a custom workflow
   can already mark its ship gate `safe` if a project genuinely wants
   full auto-merge. A dedicated per-project override is deferred until
   someone actually wants a project-specific policy independent of which
   workflow it runs.
6. **Checkpoint-push durability — run-end only; no per-step
   checkpoint-push.** `push_feature_branch` (in `await_terminal_and_push_inner`)
   fires exactly once, after the feature reaches a terminal state — not
   at every completed-step boundary. Chosen over the alternative
   because extra per-step pushes litter `origin` with WIP refs on every
   run, for a benefit (surviving *permanent* machine death mid-run) that
   only matters for hardware that never comes back — the far more common
   failure modes (reboot, `kill -9`, OOM) are already covered by M2.2/M2.3's
   checkpoint-and-resume, which needs no push at all. Revisit only if
   permanent-machine-death data loss actually happens to a user.
7. **Reboot-retry budget — a global default of 5, not yet per-run
   configurable.** Implemented as `REBOOT_RETRY_BUDGET` in
   `crates/demeteo-runner/src/reconcile.rs`: a run that has been
   auto-resumed 5 times parks as `failed (unstable host)` rather than
   retrying a sixth time. Per-run overrides are straightforward to add
   to `RunSpec` later if a workload needs a different tolerance, but
   nothing has needed it yet.
8. **Linger without privilege — (a), install anyway and warn.**
   Implemented exactly as scoped: `remote_enable_runs` (M7.1) always
   installs and starts the systemd `--user` unit, then best-effort
   attempts `loginctl enable-linger`; a failure there returns a
   `warning` string surfaced in `MachinesView` instead of failing the
   whole install. The run works for the current SSH session either way
   — only survival across logout/reboot depends on lingering.

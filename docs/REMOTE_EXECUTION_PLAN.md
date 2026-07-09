# Demeteo: Remote Unattended Execution — Implementation Plan

> **The build plan for [`REMOTE_EXECUTION.md`](REMOTE_EXECUTION.md).** That doc
> is the design (decisions R1–R11, sections §1–§10); this doc is the
> milestone-by-milestone execution plan. Task format follows
> [`BACKEND_REFACTOR_TASKS.md`](BACKEND_REFACTOR_TASKS.md): each task states
> **What / Where / Why / Definition of Done**. Cross-refs to design decisions
> use the `Rn` tags and to design build-phases the `Pn` tags.

## Guiding principles

1. **Reuse the engine verbatim.** The runner is the *same* `DagStepExecutor` /
   `ExecutionDriver`, wired by a different composition root. No forked logic.
2. **Every milestone is independently valuable and demoable.** We can stop after
   any `M` and have shipped something real; the feature degrades gracefully.
3. **Land behind a flag.** All remote-run UI/entrypoints sit behind a
   `remote_runs` feature flag (app setting) until M6 is proven.
4. **Ground truth is code, not this doc.** File paths below are the current
   integration points; verify against `master` when picking up a task.

## Milestone map

| M | Theme | Demoable outcome | Design refs |
|---|-------|------------------|-------------|
| M0 | Engine extraction | App still works; a headless bin can construct the engine without Tauri | R1 |
| M1 | Headless runner MVP | `demeteo-runner` runs a workflow to completion on a Linux box over SSH | R1, R3, P1 |
| M2 | Supervision & reboot | Runner survives `kill -9` and reboot, resumes mid-run | R2, R11, §7.1, P5/P6 |
| M3 | Control channel | Laptop submits + streams + reconciles a remote run | R4, R8, R9, P2 |
| M4 | Credentials | Remote run clones/pushes with injected PAT; PAT never on disk/argv | R5, §6, P3 |
| M5 | Unattended policy | Auto-approve safe gates, park dangerous, budget caps, auto-open PR | R6, R7, R10, §5, P4 |
| M6 | Laptop UX | Launch unattended, close app, get notified, return inbox | R10, §8, P7 |
| M7 | Deploy & harden | One-click enable-remote-runs; version handshake; audit; security review | §10 |

Ordering is mostly linear. M4/M5 can proceed in parallel once M3 lands. M6
depends on M3+M5. M2 can be developed in parallel with M3 (both depend on M1).

> **Beyond M7:** multi-client isolation (one shared runner serving several
> Demeteo installs, each client's runs owned and invisible to the others) is
> designed separately in [`MULTI_CLIENT_RUNNER.md`](MULTI_CLIENT_RUNNER.md),
> with a detailed **P0** (per-run ownership + project-settings fidelity) plan.

---

## M0 — Engine extraction (the enabling refactor)

**Context.** Today everything lives in one crate `demeteo` (`src-tauri/`, lib
name `demeteo_lib`, `crate-type = ["cdylib","rlib","staticlib"]`). The lib
hard-depends on Tauri and the composition root is inlined in `lib.rs::run`
(the `tauri::Builder` `setup` closure, ~L206–461). A headless runner cannot
reuse this without linking Tauri. **This is the highest-risk, highest-leverage
milestone — do it first, land it clean, then build on it.**

### M0.1 — Create a Cargo workspace with a `demeteo-core` library crate

- **What:** Convert to a workspace. New member `demeteo-core` holds the
  Tauri-free engine: `domain/`, `ports/`, `application/`, `shared/`, and the
  non-Tauri `adapters/` (database, agent, worktree, step_executor, artifact_store,
  attachment_store, local, router, merge, conflict, mr_publisher, pricing,
  provider_http, memory_*, scheduler, mr_monitor, ssh, create_project). Keep the
  Tauri app crate (`demeteo`) depending on `demeteo-core`.
- **Where:** New root `Cargo.toml` `[workspace]`; new `src-tauri/core/` (or
  `crates/demeteo-core/`); move modules; `src-tauri/` keeps `commands/`,
  `adapters/tauri_ui/`, `forward.rs`, `terminal.rs`, `sftp.rs`, `state.rs`,
  `lib.rs`, `main.rs`.
- **Why:** The runner binary must link the engine without a webview/Tauri
  runtime (R1). Ports/adapters are already the seam — this makes it physical.
- **DoD:** `cargo build` + full test suite (473 lib + 21 orchestration + lint +
  doctests) green from the app crate; `cargo tree -p demeteo-core` shows **no
  `tauri` dependency**; `npx tsc --noEmit` unaffected.

### M0.2 — Extract a reusable composition function

- **What:** Pull the adapter-construction body out of `lib.rs::run`'s setup
  closure into `demeteo-core::composition::build_core_context(cfg) -> CoreContext`,
  parameterized by a `CoreConfig` (db path, artifact dir, execution mode
  local-only vs router, notification sink). The Tauri app calls it and adapts the
  result into its `AppContext`; the runner calls it directly.
- **Where:** `composition/mod.rs` (currently just re-exports `AppContext`),
  `lib.rs` (shrinks to a call), `state.rs`.
- **Why:** One construction site for both binaries; the design's "same engine,
  different composition root" (R1) becomes literally one shared function plus two
  thin shells.
- **DoD:** App boots and runs a local feature end-to-end through
  `build_core_context`; no behavior change; `NotificationPort` and
  `ExecutionPort` are injected params, not hardcoded.

**Risk / mitigation:** big move-only diff. Do it as move-only commits (no logic
edits) so review is mechanical; run the suite after each module moves. Budget
this milestone generously — everything downstream rides on it.

---

## M1 — Headless runner MVP

Goal: a `demeteo-runner` binary that, invoked on a Linux box, runs a workflow to
completion locally and pushes the result branch. No laptop control channel yet —
driven by a spec file. Proves the brain relocates (R1, R3).

### M1.1 — `demeteo-runner` binary + composition root

- **What:** New bin crate `demeteo-runner`. `main` parses `submit <spec.json>` /
  `resume` / `status`, builds a `CoreContext` via `build_core_context` with:
  execution = **local-only** (`LocalSubprocessAdapter`, *not* the SSH router —
  the runner *is* the machine, so nested SSH collapses away), its own SQLite
  under `~/.local/share/demeteo-runner/`, and a `NotificationPort` = noop for now.
- **Where:** new crate; reuses `adapters::local::execution::LocalSubprocessAdapter`,
  `adapters::step_executor::DagStepExecutor`, `AgentRegistry`.
- **Why:** P1. Local execution is simpler and more reliable than per-command SSH
  (design §3, point 1).
- **DoD:** `demeteo-runner submit spec.json` on a Linux host with a pre-authed
  agent runs a real workflow to a terminal state; feature branch is created in a
  worktree.

### M1.2 — Run spec + git result push

- **What:** Define `RunSpec` (project/repo, workflow JSON, description, agent,
  model, loop budget). The runner clones/fetches via `GitOpsHelper`, runs the DAG,
  and **pushes the feature branch to `origin`** at completion (R3). PAT for M1 is
  read from an env var (real injection is M4).
- **Where:** `RunSpec` in `demeteo-core::domain`; reuse `git_ops/clone.rs`,
  `git_ops/sync.rs`, `feature_start` on `DagStepExecutor`.
- **Why:** R3 — results ride git; only metadata rides the control channel later.
- **DoD:** After a successful headless run, `git ls-remote origin` shows the
  pushed `feature/<slug>` branch; laptop can `git fetch` and review the diff.

---

## M2 — Supervision, shutdown & reboot resilience

Goal: the runner survives process death, host reboot, and hard power loss, and
auto-resumes (R2, R11, §7.1).

### M2.1 — systemd `--user` unit + install

- **What:** Ship a `demeteo-runner.service` user unit (`Restart=always`,
  `RestartSec`, journald logging) and an installer that enables it and runs
  `loginctl enable-linger <user>` so it starts at boot with no login session.
- **Where:** `demeteo-runner/dist/systemd/`; installer script invoked by M7's
  deploy path (manual for now).
- **Why:** R2 — lingering is the whole reason the run survives a closed laptop
  *and* a reboot.
- **DoD:** `systemctl --user status demeteo-runner` active after reboot with no
  interactive login.

### M2.2 — Crash-consistent state + graceful SIGTERM

- **What:** Put the runner SQLite in **WAL mode**. Install a SIGTERM handler that
  marks in-flight steps `interrupted`, flushes the event log, and exits cleanly.
- **Where:** `adapters/database/connection.rs` (WAL pragma — verify app DB
  setting and mirror it), runner `main` signal handling.
- **Why:** §7.1 — graceful reboot checkpoints; hard loss stays consistent.
- **DoD:** `systemctl --user stop` mid-run leaves the DB consistent and the step
  marked `interrupted`, not `running`.

### M2.3 — Restart reconciliation + bounded auto-resume

- **What:** On startup, scan for runs with `running` steps and no live child →
  mark `interrupted`; re-run the interrupted step from its per-step checkpoint
  (Decision 14 machinery) under a **reboot-retry budget**; exhaustion →
  `failed (unstable host)`.
- **Where:** runner startup path; reuse `impl_traits/replay.rs`,
  `driver_registry.rs`, existing retry loop in `driver.rs`.
- **Why:** R11. Reuses the interrupt→checkpoint→retry logic the engine already
  has.
- **DoD:** `kill -9` the runner mid-step → systemd restarts it → the run resumes
  and reaches a terminal state; a scripted reboot mid-run does the same; a
  crash-loop trips the budget and parks as `failed (unstable host)`.

---

## M3 — Control channel (laptop ⇄ runner)

Goal: submit, observe, and reconcile remote runs from the app (R4, R8, R9, P2).

### M3.1 — Secure listener (decided: Unix socket, `0600`)

**Decision (was an open fork, now closed):** the runner listens on a
**Unix-domain socket** at `~/.local/share/demeteo-runner/control.sock` with
`0600` perms, forwarded to the laptop via **OpenSSH Unix-socket forwarding**
(`ssh -L <local>:<remote.sock>`). Protection comes from OS file permissions —
no other local user can open the socket, and nothing is exposed to the network.

*Rejected alternative (kept only as fallback):* loopback TCP `127.0.0.1:<port>`
via the existing `forward.rs`. It reuses more existing code, but loopback is
reachable by **any local user on a shared host**, so it would require a per-runner
**bearer token** on every request to be safe. Only fall back to this if
Unix-socket forwarding proves impractical, and then the token is mandatory.

- **What:** RPC server on the `0600` Unix socket; laptop client dials it through
  the SSH Unix-socket forward.
- **Where:** new `demeteo-runner` RPC server module; laptop SSH transport
  (extend `forward.rs` / `adapters/ssh/client.rs` for Unix-socket `-L`).
- **Why:** R4 — no new public port; authz inherited from SSH; hardened against
  other local users *for free* via file perms.
- **DoD:** Laptop reaches the runner RPC only through the SSH tunnel; a second
  local user on the remote **cannot** reach the socket (verify with a test that
  attempts to connect as another uid and is denied).

### M3.2 — RPC surface + idempotent submit

- **What:** Implement `submit_run(spec) -> run_id` (idempotent by
  laptop-generated UUID), `list_runs`, `get_status`, `health` (heartbeat +
  version + capacity). Persist submitted specs before spawning the driver.
- **Where:** RPC module; runner DB `runs` table.
- **Why:** R9 + at-least-once→exactly-once (design §4).
- **DoD:** Re-submitting the same `run_id` returns the existing run, never a
  duplicate; `list_runs` reflects state across a runner restart.

### M3.3 — Append-only event log + streaming + laptop mirror

- **What:** Runner writes an append-only per-run event log with monotonic
  offsets; `stream_events(run_id, from_offset)` tails it. Laptop keeps a **mirror
  DB** keyed by `(machine_id, run_id)` and reconciles by pulling events since its
  last offset. Cancellation is `cancel_run` only — **disconnect never cancels**
  (R8).
- **Where:** runner `run_events` table; laptop mirror repo (new migration on the
  app DB, following the V-numbered chain in `adapters/database/migration.rs`).
- **Why:** R9 — catch up on everything missed without relying on a live socket.
- **DoD:** Submit a run, watch events live; kill the tunnel mid-run (run keeps
  going); reconnect and the mirror catches up with zero gaps; `cancel_run` is the
  only thing that stops it.

---

## M4 — Credential injection

Goal: the runner clones/pushes with a PAT that never touches disk or argv (R5,
§6). Coding-agent auth stays a machine precondition (§6.1).

### M4.1 — Agent-readiness probe (precondition enforcement)

- **What:** Before accepting a run for an agent, the runner verifies that agent
  is installed and authed on the host; the laptop surfaces failures at launch.
- **Where:** reuse `commands/agent_config_probe.rs` / `application/agent_probe.rs`
  logic, exposed as an RPC `probe_agent(kind)`.
- **Why:** §6.1 — a non-ready machine is ineligible; fail loud at launch, not
  mid-run.
- **DoD:** Launching against a machine missing the selected agent is blocked with
  a clear message; a ready machine passes.

### M4.2 — `inject_credentials` + memory-only run-scoped store

- **What:** `inject_credentials(run_id, git_pat)` over the tunnel; runner holds
  it **in memory only**, keyed by run, wiped on terminal state. Never written to
  DB, artifacts, git config, or logs.
- **Where:** runner in-memory cred store; laptop pulls PAT from keyring
  (`GitOpsHelper::get_provider_pat`, `git_ops/clone.rs:7`) and sends it.
- **Why:** R5/§6.2 — no standing git secret on the machine.
- **DoD:** PAT is present only for the run's lifetime; `grep` of runner DB +
  logs + `journalctl` shows no PAT.

### M4.3 — Per-run git askpass (keep PAT out of argv)

- **What:** Replace the current URL-embedded-PAT clone
  (`https://x-access-token:{pat}@host/...`, `clone.rs:41`) with a per-run
  `GIT_ASKPASS`/credential-helper that reads from the in-memory store, so the PAT
  never appears in `ps`, shell history, or the remote URL.
- **Where:** `git_ops/clone.rs`, `git_ops/sync.rs`, `git_ops/merge.rs` — a small
  seam so the runner path uses askpass while the laptop path is unchanged (or
  migrate both).
- **Why:** §6.2 hardening — the design explicitly calls out this tightening over
  today's behavior.
- **DoD:** `ps aux` on the remote during a clone/push shows no token;
  fetch/push succeed; the `needs-credentials` park state triggers if the store is
  empty (e.g. after reboot, §7.1).

---

## M5 — Unattended gate policy

Goal: unattended relaxes **gates only**, classified by blast radius, with budget
caps and auto-PR (R6, R7, R10, §5).

### M5.1 — Gate blast-radius classification + policy config

- **What:** Classify gates into **safe** (review/informational, merge-to-feature)
  vs **dangerous** (merge-to-default, push-to-protected, deploy, delete). Add a
  `GatePolicy` (per-run, chosen at launch) that auto-approves safe and **parks**
  dangerous. Keep the per-command permission/intercept layer + worktree fence
  **on** (R6 — do *not* switch to `DirectExecutionPort`).
- **Where:** gate model + `steps/gate.rs`; policy hook in the gate wait path
  (`gate_waiter.rs` / `steps/gate.rs`); extend the existing `auto_approved_rules`
  concept from commands to gate classes.
- **Why:** R7 — the pressure valve that keeps unattended safe without breaking
  the UX promise.
- **DoD:** Unattended run auto-approves a review gate and parks a
  merge-to-default gate; a `park` is visible over the control channel.

### M5.2 — Budget caps + hard-stop semantics

- **What:** Per-run and per-machine caps on token cost and wall-clock. Exceeding →
  park or stop; never auto-approve more spend.
- **Where:** reuse `domain/usage.rs` accumulator; enforce in the driver loop.
- **Why:** §5 — abuse/cost ceiling for unattended.
- **DoD:** A run with a $ cap parks at `over-budget` when exceeded instead of
  continuing.

### M5.3 — `decide_gate` RPC + auto-open PR

- **What:** `decide_gate(run_id, gate_id, decision)` lets the laptop clear parked
  gates remotely. On successful completion, the runner **auto-opens the PR** via
  the idempotent `MrPublisher::publish_mr` — "PR ready" is the success terminal
  state (R10). Opening a PR ≠ merging.
- **Where:** RPC module; reuse `adapters/mr_publisher/`, `ports/mr_publisher.rs`.
- **Why:** R10 + design §8. `publish_mr` is already idempotent (checks
  `features.mr_url`).
- **DoD:** Unattended success → PR opened, URL recorded; a parked gate is
  clearable from the laptop and the run proceeds.

---

## M6 — Laptop UX

Goal: the full "close the laptop, come back to results" journey (R10, §8, P7).

### M6.1 — Launch surface

- **What:** In `StartFeatureModal`, add "Run on machine ▾" + an **Unattended**
  toggle that reveals gate policy (safe/dangerous classes) + budget cap. Post-
  launch badge *"Running on `<machine>` · unattended"* and the reassurance
  *"You can close Demeteo. This run continues on `<machine>`."*
- **Where:** `src/components/StartFeatureModal.tsx`, `ProjectHome.tsx`; new Tauri
  commands to reach the control channel.
- **Why:** §8. Machine concept + `MachinesView` already exist.
- **DoD:** User launches an unattended remote run and can immediately close the
  app.

### M6.2 — Return inbox + status taxonomy

- **What:** On app open, reconcile all runs across all machines into a **return
  inbox** grouped by: **PR ready / Failed / Parked / Needs-credentials / Running /
  Unreachable** (design §8 table). `unreachable ≠ failed`. Parked/failed float to
  the top; deep-link to PR URL / failure logs / diff.
- **Where:** new inbox view; laptop mirror DB; reuse `mr_monitor` +
  `fetch_mr_state` for live PR state.
- **Why:** §8 — the "report back."
- **DoD:** After runs complete while the app was closed, reopening shows each in
  the correct bucket with the right deep link.

### M6.3 — Dual-channel notifications

- **What:** (1) **Runner-push while away** — email/Slack/webhook/ntfy via a real
  `NotificationPort` adapter on the runner, fired on terminal/actionable state.
  (2) **Reconcile-on-reopen** — laptop diffs the mirror and raises a **desktop
  notification** for anything newly **PR-ready** or **failed** (+ parked /
  needs-credentials) since last seen.
- **Where:** runner `NotificationPort` adapter (replaces M1 noop); laptop reuses
  `NotificationBell` / `notifications.ts` / `adapters/tauri_ui/notification.rs`.
- **Why:** §8 — the explicit "notify me when I reopen" requirement, diff-driven
  so nothing is missed.
- **DoD:** Complete a run with the app closed → receive the away-channel alert →
  reopen → desktop notification + inbox entry.

### M6.4 — Live remote view

- **What:** When connected, tail the remote event log over the tunnel into the
  existing feature/step UI — identical to a local run.
- **Where:** `FeatureDetail.tsx` + `stream_events`.
- **DoD:** A remote run is watchable live with the same UI as local.

---

## M7 — Deployment & hardening

### M7.1 — Runner auto-install / upgrade (user-space, no root)

- **What:** Detect → provision → supervise, entirely as the SSH user, no `sudo`.
  1. **Detect** over SSH: `command -v demeteo-runner` + `demeteo-runner --version`.
     Missing or version-mismatched → (re)install.
  2. **Provision the binary** — the runner ships as a **static musl `x86_64`
     binary**, built and published by CI under the exact same version tag as the
     app (`build.yml`). Provisioning is always **push over SFTP** (`sftp.rs`) to
     `~/.local/bin/demeteo-runner` — the remote box is never assumed to have
     internet access, so it never `curl`s anything itself. The *laptop* is what
     may need to fetch: `remote_runner_local_check` looks for a usable local copy
     (dev override, a dev's own build next to the app binary, or a previously
     downloaded version match) with no network call; if none matches the app's
     own version, the UI prompts the user and `remote_runner_download` fetches
     the matching GitHub release asset (+ verifies its published checksum) to a
     laptop-local temp cache before the existing SFTP push runs.
  3. **Install user-space:** binary → `~/.local/bin/`, data/DB →
     `~/.local/share/demeteo-runner/`, unit → `~/.config/systemd/user/`,
     then `systemctl --user enable --now demeteo-runner`.
  4. **Enable persistence:** `loginctl enable-linger $USER` so the runner
     survives SSH logout and reboot (R2). **Caveat:** on some distros this needs
     admin/polkit; if it fails, install still succeeds but warn the user that
     runs won't survive logout/reboot (see design note + §10.8).
  5. **Upgrade** is the same path re-run when `--version` mismatches, followed by
     `systemctl --user restart demeteo-runner`.
- **Where:** reuse `adapters/agent/install.rs::run_official_install` pattern (runs
  a command locally/remotely via `ExecutionPort`), `sftp.rs`, `commands/machine.rs`,
  `commands/ssh.rs`; new `demeteo-runner.service` template from M2.1;
  `MachinesView.tsx` for the "Enable remote runs" action + status/version badge.
- **Why:** the machine should become run-ready with one click and no privileged
  process; user-space + systemd `--user` is exactly why R2 chose a user unit.
- **DoD:** On a machine with no runner, one "Enable remote runs" action yields a
  running, correctly-versioned **user** process (`systemctl --user status` active,
  process owned by the SSH user, nothing owned by root); a later version bump
  upgrades it in place; a machine where linger can't be enabled installs but
  surfaces the persistence caveat.

### M7.2 — Audit, secret scrubbing, security review — **DONE**

- **What:** Immutable audit trail per unattended run (every auto-approved gate,
  command, diff, cost) synced back; event-log/notification secret scrubbing; run
  `/security-review` on the credential + control-channel + unattended-policy code.
- **Why:** §5 non-repudiation + §6 hygiene.
- **DoD:** Audit visible in the inbox; no secret leaks in synced data; security
  review actioned.
- **Shipped:**
  - *Audit trail = the append-only `run_events` log, surfaced in the inbox.*
    `run.rs` already emits the control-plane decisions (gate auto-approve, park,
    over-budget, needs-credentials, push, PR) plus a new **`cost`** event at
    terminal state. The return inbox's log viewer (`RemoteRunInbox.tsx`,
    relabelled **"Audit log"**) is now reachable from *every* bucket — including
    successful `pr_ready` runs, whose auto-approved-gate trail is exactly what
    non-repudiation needs. **Scope note:** per-*command* rows stay in the mirrored
    step/feature view ("View feature", C4.3) rather than being duplicated into
    `run_events` — capturing the agent's own commands would need an engine-level
    audit sink, out of scope for "surface the existing log". The event log covers
    the runner's control-plane commands (clone/push/PR) and all gate/budget
    decisions.
  - *Secret scrubbing at the sink.* New `demeteo-core::shared::secret_scrub`
    (dependency-free; masks GitHub/GitLab PATs and URL-embedded basic-auth) is
    applied in the two adapter choke points that persist laptop-visible text —
    `RunEventsPort::append` (payload) and `RunnerRunPort::update_status` (error)
    — so the direct `rpc.rs` failure-path writers that bypass `run::emit` are
    covered too, plus the away-notification webhook body.
  - *Security review actioned.* Two findings fixed: (1) the error/event sinks
    above bypassed scrubbing; (2) the `0600` control socket had a bind-then-chmod
    TOCTOU — closed by creating the runner data dir `0700` before the socket/DB
    exist (`main.rs::ensure_private_data_dir`), which also protects the event-log
    DB from other local users.

### M7.3 — Resolve remaining open questions

- Multi-observer arbitration (§10.2), checkpoint-push durability (§10.6),
  reboot-retry budget default (§10.7), disk/DB exhaustion behavior (§10.4),
  default-branch parking granularity (§10.5), scoped tokens (§10.3).

---

## Testing strategy (cross-cutting)

- **Unit** — gate classification, budget enforcement, reconciliation logic,
  idempotent submit, event-log offsets live in `demeteo-core` with no I/O.
- **Integration harness** — spin a `demeteo-runner` bound to a temp unix socket
  in-process; a fake origin (bare git repo in a tempdir) and a stub agent
  (existing `agent/test_stubs.rs`) exercise submit → run → push → reconcile
  without a real remote host. Add as `tests/remote_runner.rs`.
- **Reboot simulation** — a test that kills the runner mid-step and asserts
  resume-from-checkpoint + reboot-retry-budget exhaustion.
- **Security tests** — assert no PAT in DB/logs/argv (M4); assert a second local
  user cannot reach the RPC (M3).
- Keep the existing green bars: 473 lib + 21 orchestration + workflow-lint +
  doctests + `clippy -D warnings` + `tsc --noEmit`.

## Sequencing summary

```
M0 ─► M1 ─┬─► M2 ─────────────┐
          └─► M3 ─┬─► M4 ─┐   │
                  └─► M5 ─┴─► M6 ─► M7
```

M0 is the gate for everything. M2 and M3 both fan out from M1 and can be built in
parallel. M4 and M5 both need M3 and can be parallelized. M6 needs M3+M5. M7 last.

## Top risks

1. **M0 scope creep.** The engine/Tauri split is invasive; keep it move-only,
   land it before anything else, don't mix in logic changes.
2. **Control-channel authz on shared hosts** (M3.1) — *resolved*: Unix socket
   `0600` + SSH Unix-socket forwarding, so file perms keep other local users out.
   The only residual risk is if we're ever forced onto the loopback-TCP fallback,
   where the bearer token becomes mandatory — don't ship that variant tokenless.
3. **Reboot resume correctness** (M2.3) — re-running an interrupted step must be
   safe against a dirty worktree; lean entirely on the existing reset-to-base
   retry path, don't invent a new one.
4. **PAT hygiene regressions** (M4.3) — the askpass seam must cover clone, fetch,
   push, and sync; a missed call site re-embeds the token in a URL.

# Demeteo: Multi-Client Runner — Design & P0 Plan

> **Extends [`REMOTE_EXECUTION.md`](REMOTE_EXECUTION.md)**, which covers a
> *single* trusted laptop driving one `demeteo-runner` and is fully built.
> This doc covers the next step: **one shared runner serving multiple Demeteo
> clients concurrently, with each client's runs isolated from the others.**
>
> **Status: designed, not built.** Nothing below has shipped.
>
> Design decisions are tagged `MC-Dn`; build phases `P0`–`P3`. Each task states
> **What / Where / Why / Definition of Done**. Ground truth is code — verify
> file paths against the branch when picking up a task.

## Guiding principles (inherited, non-negotiable)

1. **Reuse the engine verbatim.** The runner is the *same* `DagStepExecutor` /
   `ExecutionDriver` as the desktop app, wired by a different composition root
   (`build_core_context` + `ExecutionMode::LocalOnly`). No forked run logic.
2. **Don't break the local-vs-remote port seam.** `ExecutionPort` (local
   subprocess vs SSH) and `control_rpc(machine_id, method, params)` stay generic
   and transport-agnostic. Multi-client concerns live in the *application/command*
   layer and the *runner RPC* layer, never in the port trait signatures.
3. **One code path for local and remote.** `run.rs::execute_run` is shared by the
   one-shot CLI `submit` and the `submit_run` RPC. Whatever we add must keep that
   single path.

---

## 1. Problem statement

We want to run the Demeteo desktop app on **multiple machines** (Client A, Client
B, …) that all drive a **single** `demeteo-runner` on a shared remote host,
without interfering with each other. Concretely:

- A workflow triggered by Client A must **never** be reported back to Client B.
- Neither client can read, cancel, gate-decide, or otherwise touch the other's
  runs.
- The runner stays a shared service: **no client may unilaterally restart or
  upgrade it** out from under another client's in-flight runs.
- Per-run behavior honors **the launching client's project settings** (test
  harnesses, prepare command, default test command, extra writable paths,
  feature lifecycle), not runner-side re-detected defaults.

## 2. Current-state analysis

### 2.1 What already isolates (structurally)

| Mechanism | Where | Effect |
|-----------|-------|--------|
| `run_id` is client-minted (`laptop-{uuid}`) | `commands/remote_runner.rs` | Globally unique; no cross-client collision |
| `feature_id` is client-minted (`f-{uuid}`) | `commands/remote_runner.rs` | Unique; laptop & runner share one id |
| Each run creates its **own project** | `runner/src/run.rs` (`projects::create`) | Separate workspace dir, clone, and settings row per run |
| One tokio task per run | `runner/src/rpc/lifecycle.rs` (`submit_run` → `tokio::spawn`) | Runner is already structurally concurrent |
| Credentials / event log / status keyed by `run_id` | `runner/src/run.rs`, `runner/src/rpc/` | Per-run, not global |
| Client **never calls `list_runs`** | verified: only doc-comment references | Reconcile is *mirror-driven*, not enumeration-driven — B has no row for A's run and never polls it |

**Takeaway:** the *happy-path* "A's run isn't reported to B" already holds,
because each client only reconciles `run_id`s in its own local
`remote_run_mirror`. What's missing is **enforcement** — the difference between
"works when everyone behaves" and "robust."

### 2.2 What does NOT isolate (the gaps)

The runner's entire authz model is *"exactly one trusted laptop"* (`rpc/mod.rs`
header + `main.rs::ensure_private_data_dir`: security = `0600` socket + `0700`
dir, "no *other OS user* can reach it").

| # | Gap | Why it bites with N clients |
|---|-----|------------------------------|
| **a** | **No ownership check on any RPC.** `get_status`, `list_steps`, `read_artifact`, `list_messages`, `stream_events`, `cancel_run`, `decide_gate`, `inject_credentials`, `get_worktree` all trust a bare `run_id`/`gate_id`. `list_runs` returns everyone's runs. | Any client reaching the socket can read/cancel/gate another client's run if it learns the id. Cross-tenant hole. |
| **b** | **Shared OS user + one socket.** All clients SSH in as the *same* user and forward the *same* `control.sock`. | Everyone is *inside* the trust boundary with full mutual access. No per-client boundary exists. |
| **c** | **One shared SQLite DB + engine singletons** (scheduler, MR-monitor, memory-worker, driver). | N concurrent features from N clients multiply write contention — the #1 *reliability* risk. |
| **d** | **Client-owned daemon lifecycle** (M7.1 "enable remote runs" upgrades/restarts the runner on version mismatch; SIGTERM `mark_all_running_interrupted` hits *all* rows). | Client A upgrading/restarting the shared runner **interrupts every in-flight run of Client B.** |
| **e** | **Away-notifier is process-global** (one webhook from env). | Push notifications don't route to the owning client. |
| **f** | **Project settings re-derived on the runner.** `RunSpec` carries none of `prepare_command`/`harnesses`/`test_command`/`extra_writable_paths`/`feature_lifecycle`; `run.rs` uses `fetch_default_settings()` + bootstrap-detected `worktree_strategy`. | With N clients, A and B may hold *different* settings for the *same* repo. Re-detection can't honor either. |

Gaps **a/b** are the *isolation* story; **c/d** the *reliability* story; **f**
the *fidelity* story (folded in per the earlier finding).

## 3. Design decisions

### MC-D1 — Client Identity
Each Demeteo install generates a stable **`client_id`** (a persisted `install_id`
UUID in the client's app-data), plus an optional human label. Every control RPC
carries it.

### MC-D2 — Ownership enforcement is the core of isolation
`runner_runs` gains an `owner_client_id` column, stamped at `submit_run`. **Every
read/mutate RPC resolves the run and checks `owner_client_id == request.client_id`;
a mismatch returns the *same* "no such run" error** (no existence leak). This
converts `run_id` from a **bearer capability** into an **owned resource** — the
enforcement layer, not "B just doesn't know the id."

This is **soft** multi-tenancy: `client_id` is not a secret, so it protects
honest clients from accidental cross-talk and bugs, not against a *malicious*
co-tenant forging an id. That is the right cost/benefit for the target scenario
(your own machines / one team). MC-D5 leaves the seam for hardening.

### MC-D3 — Thread `client_id` without touching the port trait
`control_rpc(machine_id, method, params)` **does not change signature.** The
`client_id` rides **inside `params`**, stamped centrally by a single
command-layer helper (`remote_rpc`/`control_rpc_owned` in `commands/remote_runner.rs`)
so no per-call-site drift. The runner extracts it generically from
`req.params["client_id"]` in `dispatch` *before* method routing. The local
`ExecutionPort` still returns its "remote runs unsupported locally" error
unchanged. *(Alternative considered: put `client_id` on the envelope inside the
SSH adapter — rejected because it forces transport code to know the install_id,
coupling the port seam to a multi-client concern.)*

### MC-D4 — Honor the launching client's project settings (fidelity)
`RunSpec` gains an **optional** `project_settings` payload (serde-default `None`).
`run.rs::execute_run` overlays it onto what it persists via `save_settings`
*before* the shared `feature_start` reads it — **no change to `feature_start` or
any port.** Merge rule:
- **Client wins** for all tunables: `branch_prefix`, `test_command`,
  `build_command`, `coverage_command`, `conventions_file`, `pr_template`,
  `harnesses`, `prepare_command`, `extra_writable_paths`, `conflict_policy`,
  `feature_lifecycle`, `default_*`, `artifact_subdir`, `commit_artifacts`.
- **Runner detection wins** for `default_branch` (it read `origin/HEAD` on the
  *actual* clone; ground truth for the checkout). Fall back to the spec's
  `default_branch`, then `"main"`.
- `None` (old client) → today's behavior exactly (detected strategy + defaults).

Because each run gets its own project row, there is **no cross-run settings
contamination** — this composes cleanly with per-run isolation.

### MC-D5 — Authorization strength ladder (design now, build later)
- **(A) Soft** — `client_id` ownership only (MC-D2). Same OS user/socket.
  **← P0 target.**
- **(B) Token** — a `register_client` RPC issues a per-client bearer token bound
  to `client_id`, stored in the client keyring; RPCs reject a mismatched token.
  Defends against a co-tenant forging an id. **← P2.**
- **(C) Hard OS isolation** — one runner instance per OS user (own socket/data
  dir). Strongest, but loses the shared-runner benefits; only for untrusted
  co-tenants. **← P3, optional.**

### MC-D6 — Shared-service lifecycle
The runner becomes a service nobody unilaterally restarts. `health` advertises
`runner_version` + `min_supported_client`; a too-old client *warns* instead of
force-upgrading. Upgrades become an explicit **drain** (stop accepting new runs,
let in-flight finish, then restart). Per-client `last_seen` makes retention GC
safe (never reap a terminal run an absent owner hasn't reconciled). **← P1.**

### MC-D7 — Concurrency & reliability
Confirm the engine tolerates N concurrent features (the desktop already runs
several against one `AppContext`; verify no global single-run assumption). Audit
SQLite **WAL + `busy_timeout`** (N clients multiply write contention). Add a
**concurrency cap** (semaphore in `submit_run`; surplus reported `queued`) so one
client can't starve others. Drop runner-side push-notify in favor of client
pull-reconcile. **← P1.**

## 4. Phase roadmap

| Phase | Theme | Delivers |
|-------|-------|----------|
| **P0** | **Isolation + settings fidelity** | Per-run ownership enforced on the runner; `client_id` threaded; project settings honored. **Delivers the core ask with no regressions.** |
| P1 | Shared-service lifecycle & concurrency | Drain/upgrade coordination, `last_seen`, safe GC, concurrency cap, SQLite audit |
| P2 | Hardening | Per-client bearer token (MC-D5 B) |
| P3 | Optional hard isolation | Per-OS-user runner instances (MC-D5 C) |

---

## 5. P0 — Isolation + settings fidelity (detailed plan)

> **Status (2026-07-09): P0 implemented** on branch `unified-run-experience`.
> P0.1–P0.6 landed: `client_id` threaded through one stamping site
> (`remote_rpc`), migration `V26__runner_runs_owner.sql` + `owner_client_id`
> on `RunnerRun`/`get_or_create`, the `require_owner`/`require_owner_of_gate`
> guards enforced on every run-scoped RPC, `list_runs` filtered,
> `RunSpec::project_settings` with the MC-D4 merge. Unit-tested: the
> leak-nothing ownership property (`check_owner`), owner stamping + no-rehome
> at the port, `client_id` param injection, and the settings merge.
> **Deferred follow-up:** the full two-`client_id` in-process-socket
> integration test iterating the whole RPC surface (P0.4 DoD) — the runner
> crate has no socket-level harness yet; the pure `check_owner` test covers
> the load-bearing "wrong-owner == absent" guarantee in the meantime.

**Outcome demoed:** two Demeteo installs drive one runner; each sees/controls
only its own runs; `list_runs` is filtered; a run honors the launching client's
project settings; an old client (no `client_id`) still works unchanged.

### P0.1 — `client_id` on the client, threaded through every remote RPC

- **What:** Generate + persist a stable `install_id` (UUID) in the client's
  app-settings on first use. Add a single command-layer helper that stamps
  `"client_id"` into the `params` of **every** `ctx.exec.control_rpc(...)` call
  the remote commands make.
- **Where:** `src-tauri/src/commands/remote_runner.rs` (new `remote_rpc(ctx,
  machine, method, params)` wrapper; route all `remote_submit_run`,
  `remote_get_status`, `remote_reconcile_runs`→`reconcile_one_run`,
  `remote_refresh_run`, `remote_stream_events`, `remote_get_feature`,
  `remote_list_steps`, `remote_read_artifact`, `remote_list_messages`,
  `remote_get_worktree`, `remote_decide_gate`, `remote_cancel_run`,
  `remote_reinject_credentials` through it). `install_id` source: app-settings
  (new getter) or a dedicated keyring entry.
- **Why:** MC-D1/MC-D3 — one stamping site, no per-call drift, and the
  `ExecutionPort` trait is untouched (transport stays generic).
- **DoD:** every outbound control RPC includes `client_id`; unit test asserts the
  wrapper injects it and preserves existing params keys; `install_id` is stable
  across app restarts.

### P0.2 — `owner_client_id` on `runner_runs`

- **What:** Migration `V26__runner_runs_owner.sql` adds
  `owner_client_id TEXT NOT NULL DEFAULT ''`. Extend `RunnerRun` +
  `RunnerRunPort::get_or_create` to accept/persist it. Existing rows backfill to
  `''` (the "legacy/unknown" tenant).
- **Where:** `crates/demeteo-core/migrations/V26__runner_runs_owner.sql`;
  `ports/runner_run.rs`; the sqlite adapter impl of `RunnerRunPort`;
  `runner/src/rpc/lifecycle.rs::submit_run` (stamp from the request).
- **Why:** MC-D2 — durable ownership is the enforcement substrate.
- **DoD:** migration applies idempotently; `get_or_create` round-trips
  `owner_client_id`; legacy rows read back `''`; existing runner tests green.

### P0.3 — Extract `client_id` in the runner dispatch + ownership guard helper

- **What:** In `dispatch`, read `req.params.get("client_id")` once (default `""`).
  Add `fn require_owner(svc, run_id, client_id) -> Result<RunnerRun, String>`
  that loads the run and returns the **"no such run"** error on owner mismatch
  (never distinguishing "exists but not yours" from "absent").
- **Where:** `crates/demeteo-runner/src/rpc/mod.rs` (dispatch);
  `crates/demeteo-runner/src/rpc/ownership.rs` (the guard).
- **Why:** MC-D2/MC-D3 — a single choke point so no method forgets the check, and
  no existence leak.
- **DoD:** unit tests: owner match → `Ok`; mismatch → same error string as
  missing; empty-vs-empty (two legacy clients) documented behavior (see Risks).

### P0.4 — Enforce ownership on every run-scoped RPC

- **What:** Route `get_status`, `get_feature`, `list_steps`, `read_artifact`,
  `list_messages`, `stream_events`, `get_worktree`, `cancel_run`,
  `inject_credentials` through `require_owner`. For `decide_gate`, resolve
  `gate_id → step_exec → feature → run` then owner-check (closes the bare-gate
  trust). Filter `list_runs` by `owner_client_id`.
- **Where:** `crates/demeteo-runner/src/rpc/` (each method + `dispatch` in
  `mod.rs`).
- **Why:** MC-D2 — this is the isolation guarantee, enforced server-side.
- **DoD:** integration test (in-process runner on a temp socket, two `client_id`s):
  client B gets "no such run" for A's `run_id` on **every** RPC; `list_runs`
  returns only the caller's; `decide_gate` on A's gate from B is rejected.

### P0.5 — Honor project settings via `RunSpec`

- **What:** Add `RunSpec::project_settings: Option<ProjectSettings>` (serde
  default). Client fills it from `ctx.projects.get_settings(project_id)` in
  `remote_submit_run`. Runner overlays it per MC-D4's merge rule when composing
  the row for `save_settings`, keeping bootstrap-detected `default_branch`.
- **Where:** `crates/demeteo-core/src/domain/run_spec.rs` (new field);
  `src-tauri/src/commands/remote_runner.rs` (populate);
  `crates/demeteo-runner/src/run.rs` (merge before `save_settings`, ~L226).
- **Why:** MC-D4 / gap **f** — parity with local runs; no `feature_start` or port
  change (only *what row exists* before the shared code reads it).
- **DoD:** unit test on the merge (client tunables win, detected `default_branch`
  wins, `None` → legacy behavior); an integration run asserts a spec-supplied
  `prepare_command` + `extra_writable_paths` are present in the runner's persisted
  settings and reach the verifier.

### P0.6 — Docs, flag, and no-regression sweep

- **What:** Note the `client_id`/ownership contract in `REMOTE_EXECUTION.md` §
  authz; confirm an **old client (no `client_id`)** and an **old runner (ignores
  the field)** both still work (serde defaults both ways). Keep all changes
  behind the existing `remote_runs` flag.
- **Where:** `docs/REMOTE_EXECUTION.md`; compatibility tests.
- **DoD:** old-client→new-runner and new-client→old-runner paths both pass; the
  existing green bars hold (lib + orchestration + workflow-lint + doctests +
  `clippy -D warnings` + `tsc --noEmit`).

## 6. Testing strategy (P0)

- **Unit** — `client_id` param injection; `require_owner` match/mismatch;
  settings-merge rule; migration backfill.
- **Integration** — in-process runner on a temp Unix socket, a fake bare-git
  origin, the stub agent; drive **two** `client_id`s and assert full RPC-surface
  isolation + `list_runs` filtering + `decide_gate` cross-client rejection +
  settings honored end-to-end.
- **Compatibility** — old-client (no field) and old-runner (ignores field)
  matrices stay green.

## 7. Risks & mitigations

1. **Two "legacy" clients share `owner_client_id = ''`.** Until every client
   ships P0.1, unknown-owner runs collapse into one tenant (today's behavior).
   *Mitigation:* ship P0.1 before advertising multi-client; treat `''` as a
   single legacy tenant, documented, not a security boundary.
2. **Soft isolation only.** `client_id` is forgeable by a malicious co-tenant.
   *Mitigation:* explicitly P2 (token). P0's threat model is trusted machines.
3. **SQLite contention under real concurrency.** *Mitigation:* P1 WAL/busy_timeout
   audit + concurrency cap; P0 doesn't increase concurrency beyond what
   `submit_run`'s existing `tokio::spawn` already allows.
4. **Missed RPC in the ownership sweep re-opens the hole.** *Mitigation:* the
   single `require_owner` choke point + an integration test that iterates the
   *entire* method list for client B.
5. **Settings merge clobbers `default_branch`.** *Mitigation:* explicit rule +
   unit test that detected branch wins over a stale spec value.

## 8. Sequencing

```
P0.1 (client_id) ─┐
P0.2 (schema) ────┼─► P0.3 (extract+guard) ─► P0.4 (enforce) ─┐
P0.5 (settings) ──┘                                           ├─► P0.6 (docs+compat)
```

P0.5 is independent of the ownership chain and can land in parallel. P0.4 is the
isolation guarantee and gates the multi-client demo.

# Task Plan — Terminal Sessions (v1 slice)

**Source spec:** [`TERMINAL_SESSIONS_SPEC.md`](TERMINAL_SESSIONS_SPEC.md) — read
§11 (decisions) first; every task below cites the spec section it builds.

Nothing here is built. The plan covers **v1**: journeys J-S1 (gate fix), J-S2
(failed step), J-S4 (plain shell) and J-S5 (index and attach). Phase 2 lists
what was designed and deferred, so nobody re-decides it.

## How to run a task

1. Read this file's header + the single task section. Read the spec sections
   the task references — not the whole spec.
2. Load only the files under **Context** (respect line ranges; several source
   files are >1k lines).
3. Stay inside **Touch**. If the task turns out to require edits outside it,
   stop and report — that's a decomposition bug in this plan; fix the plan first.
4. Run the task's **Done when** checks, plus `npm run checks`. A task that
   touches an `ExecutionPort` impl or a `WorktreeOpsPort` method also runs
   `crates/demeteo-core/tests/conformance/run-ssh-conformance.sh`.
5. Commit per task (`feat(terminal): T0.2 session worktree kind`) and flip the
   checkbox below.

**Sizing rule:** ≤ ~2,000 lines of required reading, one coherent diff, no task
depends on holding two subsystems in context at once.

**Migrations:** the highest migration today is **V53**. This plan adds two, both
additive: **V54** `terminal_sessions` (T0.1) and **V55**
`gate_decisions.approved_head_sha` (T0.5). Neither is a §6 gate item.

**Gate items already approved (AGENTS.md §6):** changing agent spawn logic — the
interactive permission model in spec §4.5 was approved on 2026-09-11 (spec §11).
T0.3 implements exactly that model and nothing wider. No dependency is added
by this plan; if a task finds it needs one, that is a fresh §6 question.

### Key code coordinates (shared reference — don't re-discover these)

> Line numbers drift. Re-verify with `git grep` before relying on one.

| What | Where |
|---|---|
| Terminal session spawn (local pty via `portable-pty`, remote via a fresh ssh2 channel, branch bootstrap line) | `src-tauri/src/terminal/start.rs`, `src-tauri/src/terminal/transport.rs` |
| Live session registry (`ActiveSession`, `SessionState`, 256 KiB scrollback ring) | `src-tauri/src/terminal/model.rs` |
| Activity resolver + OSC 777 scanner + claude hook settings | `src-tauri/src/terminal/activity.rs`, `activity_scanner.rs`, `hooks.rs` (`build_agent_launch_command`) |
| Terminal Tauri commands (`list_terminal_sessions`, `report_terminal_screen_activity`) | `src-tauri/src/terminal/commands.rs` |
| Frontend session state, reload reconciliation, single-write launch contract | `src/context/TerminalPanelProvider.tsx` (reconcile ~657-697, launch write ~432-452) |
| `TerminalTabDescriptor`, `TerminalActivity` | `src/types.ts` (~1255-1285) |
| "Code with Agent" routing, Browse Code remote resolution | `src/components/FeatureDetail/useWorktreeRouting.ts` |
| `resolve_feature_worktree` (returns the **main clone**, not a worktree) | `crates/demeteo-core/src/application/worktree.rs` |
| `remote_get_worktree` → runner `get_worktree` | `src-tauri/src/commands/remote_runner.rs`, `crates/demeteo-core/src/application/remote_runs/transport.rs`, `crates/demeteo-runner/src/rpc/reads.rs` |
| Worktree naming (`_wt_<id>`, `_wt_sync_<slug>`, `_cache_`) | `crates/demeteo-core/src/paths.rs` (~531-584) |
| Sync worktree provisioning + stale `_wt_sync` scan | `crates/demeteo-core/src/adapters/worktree/git_ops/sync.rs` (`provision_sync_worktree`, ~740-830) |
| Subtask worktree provision / teardown / branch delete | `crates/demeteo-core/src/adapters/worktree/git_ops/worktree.rs` (~516-595, ~756-860) |
| **Merge target selection** (the §2.2 hazard) | `crates/demeteo-core/src/adapters/worktree/git_ops/merge.rs` (`merge_subtask`, `abort_inflight_merge`) |
| Terminal-worktree location rule (the precedent for a sync `domain/` rule) | `crates/demeteo-core/src/domain/terminal_worktree.rs` |
| Worktree conformance precedent (local + loopback sshd) | `crates/demeteo-core/tests/conformance/terminal_worktree.rs` |
| Gate park / approve / redirect | `crates/demeteo-core/src/adapters/step_executor/steps/gate/mod.rs`, `redirect_reset.rs`; policy in `crates/demeteo-core/src/domain/gate/` |
| Gate decision command, port, repo | `src-tauri/src/commands/features.rs` (`gate_decide`), `crates/demeteo-core/src/ports/step_executor.rs` (`GatePresenter`), `crates/demeteo-core/src/adapters/database/repos/gate.rs` |
| Step retry / replay | `crates/demeteo-core/src/adapters/step_executor/impl_traits/step_executor.rs`, `replay.rs`; UI `src/components/FeatureDetail/useRerunActions.ts` |
| `DomainEvent` → `run_events` bridge (one pure function) | `crates/demeteo-core/src/ports/notification.rs`, `crates/demeteo-core/src/adapters/run_event_log.rs` (`run_event_record`); labels in `src/components/RunEventFeed.tsx` |
| OS notification routing | `src-tauri/src/adapters/tauri_ui/notification.rs` |
| Harness capability matrix | `crates/demeteo-core/src/ports/agent_runtime.rs` (`AgentCapabilities`, ~409-475); per-harness in `crates/demeteo-core/src/adapters/agent/*/mod.rs` |
| `PermissionProfile` + translators (`OPENCODE_PERMISSION`, `--disallowedTools`) | `crates/demeteo-core/src/domain/permission.rs`, `crates/demeteo-core/src/ports/agent_runtime.rs` (~162-196), `crates/demeteo-core/src/adapters/agent/claude_code/mod.rs` (~570-620) |
| Client install id (for tmux names) | `crates/demeteo-core/src/application/remote_runs/client_id.rs` |
| Runner install / status probes | `src-tauri/src/commands/remote_install.rs`, `crates/demeteo-core/src/infrastructure/runner/status.rs`, `crates/demeteo-runner/packaging/install.sh` |
| Machine model (`name` is the friendly label) | `crates/demeteo-core/src/domain/models/machine.rs` |
| Feature cleanup | `crates/demeteo-core/src/application/lifecycle.rs` |
| Shortcuts + global keydown handler | `src/lib/shortcuts.ts`, `src/hooks/useKeyboardShortcuts.ts` |
| UI prefs store (`get_app_session`/`set_app_session`) | `src/lib/uiPrefs.ts` |
| Rail / header terminal entries | `src/components/ProjectRail.tsx`, `src/components/TopBar.tsx` |
| Full-screen terminals view (to retire) | `src/components/TerminalsView.tsx` |
| Feature page composition | `src/components/FeatureDetail/FeatureDetail.tsx`, `FeatureHeader.tsx`, `GateStrip.tsx`, `InspectorColumn.tsx`, `StepCard.tsx`, `RunPanes.tsx` |
| Launchers today (two agent lists) | `src/components/StartSessionButton.tsx`, `src/components/NewTerminalMenu.tsx`, `src/lib/agents.ts` |

---

## Phase 0 — Foundations (no visible UI change)

Dependency order: `T0.1 ─▶ T1.1, T1.6` · `T0.2 ─▶ T1.2, T1.4` · `T0.3 ─▶ T1.2,
T1.4` · `T0.4 ─▶ T1.1, T1.6` · `T0.5 ─▶ T1.4` · `T0.6` is independent. Inside
Phase 0, `T0.4` needs `T0.1` (it writes the tmux name onto the row); the rest
are independent of each other.

### T0.1 — Session rows: `terminal_sessions` table, full descriptor on reload

- [ ] **Goal:** A session is a row (spec §2.1). Every field in the §2.1 table
  persists, `tabId` is the persisted id, reload rebuilds tabs with
  `projectId`/`featureId`/`origin`/`worktree`/`brief`/`baseSha` intact, and a
  Feature can list its sessions from the DB. Today the Rust registry keeps
  `work_dir`/`work_branch` but `list_terminal_sessions` drops them and the
  frontend mints a fresh `tabId`.
- **Size:** medium.
- **Context:** spec §2.1; `src-tauri/src/terminal/model.rs` (`ActiveSession`,
  `SessionInfo`), `commands.rs` (`list_terminal_sessions`), `start.rs`;
  `src/context/TerminalPanelProvider.tsx` reconcile block; `src/types.ts`
  descriptor; one existing repo under
  `crates/demeteo-core/src/adapters/database/repos/` as the shape to copy;
  `crates/demeteo-core/src/ports/db.rs` for how a repo is exposed.
- **Touch:** `crates/demeteo-core/migrations/V54__terminal_sessions.sql`
  (columns: id, project_id, feature_id NULL, machine_id, origin_kind,
  origin_step_execution_id NULL, worktree_kind, worktree_path, attached,
  brief NULL, base_sha NULL, persistent, tmux_name NULL, agent_kind NULL,
  created_at, ended_at NULL, last_exit_code NULL); a `TerminalSessionRepo` +
  port; `start_terminal_session` takes the new fields and writes the row;
  `SessionInfo` carries the full descriptor; `TerminalPanelProvider` reconciles
  with it; `src/lib/terminal.ts` wrappers; `src/types.ts`.
- **Done when:** a test creates a session with a feature and origin, drops the
  frontend state, re-lists, and gets the same `tabId`, `featureId`, `origin`,
  `worktree.path` and `brief` back (watched fail first by reverting the
  reconcile change). Migration applies on a fresh DB and on a V53 DB.

### T0.2 — Session worktree kind (`_wt_session_<slug>`) and the two merge fences

- [ ] **Goal:** Spec §2.2 in full: `paths::session_worktree_dir`,
  `WorktreeOpsPort::provision_session_worktree(machine, repo_dir, branch,
  attach)` (attach = checkout branch; not attach = detached at the branch tip),
  `session_worktree_detach`, `session_worktree_attach`, removal from the
  branch-delete path, exclusion from the sync stale scan, and — the hazard —
  `merge_subtask` and the sync flow never choosing a `_wt_session_` worktree as
  a merge target. The "is this a session worktree" rule is a sync `fn` in
  `domain/` (same shape as `domain/terminal_worktree.rs`), so both fences and
  the scan share one predicate.
- **Size:** medium-large. The only task touching git_ops merge semantics.
- **Context:** spec §2.2; `paths.rs` ~531-584 (`sync_worktree_dir` and its
  doc on why one spelling matters); `git_ops/sync.rs` ~740-830; `git_ops/merge.rs`
  whole file (short); `git_ops/worktree.rs` ~516-595 and ~756-860;
  `domain/terminal_worktree.rs`; `domain/worktree_listing.rs` (the parser
  `merge_subtask` uses to find the checked-out path);
  `tests/conformance/terminal_worktree.rs` as the test template;
  [`EXECUTION_PARITY.md`](EXECUTION_PARITY.md) §"Platform is not transport".
- **Touch:** `paths.rs`, new `domain/session_worktree.rs`, `git_ops/{sync,merge,
  worktree}.rs`, `ports/worktree_ops.rs`, `tests/conformance/session_worktree.rs` (local + loopback
  sshd, observing only through `ExecutionPort`).
- **Done when:** (a) conformance: provision attached and detached on local and
  SSH produce byte-identical `git worktree list --porcelain` shapes; (b) a unit
  test builds a listing where the feature branch is checked out **only** in a
  `_wt_session_` worktree and asserts `merge_subtask`'s target selection falls
  through to the subtask-worktree arm — watched fail before the fence exists;
  (c) the sync stale scan leaves a `_wt_session_` entry alone; (d)
  `branch_delete` removes it. `run-ssh-conformance.sh` green.

### T0.3 — Interactive launch capability, deny list, exit trailer

- [ ] **Goal:** Spec §4.5 "Rules" and §5.1's exit signal. A new
  `AgentCapabilities.interactive_launch: Option<InteractiveLaunch>` with:
  prompt argv shape, model argv, effort argv (or none), deny delivery
  (`Argv(...)` for claude-code/codex, `Env("OPENCODE_PERMISSION")` for
  opencode), and the quit key sequence. Declared for claude-code, codex and
  opencode; `None` for hermes and pi. A launch-line builder in
  `src-tauri/src/terminal/` composes `cd`, env prefix, binary, args (all
  `shell_single_quote`d), then the OSC 777 trailer with `state=exit;code=$?`.
  The scanner parses `code`, the resolver emits `exit` with it, the frontend
  stops collapsing `exit` to `null`, and the row's `last_exit_code` is written.
  `build_agent_launch_command` (claude `--settings` hooks) stays and is
  composed, not replaced.
- **Size:** medium.
- **Context:** spec §4.5 Rules, §5.1, §11 "Spawning"; `ports/agent_runtime.rs`
  (`AgentCapabilities`, the permission translators); each adapter's
  `capabilities()` (five files, ~40 lines each); `src-tauri/src/terminal/
  hooks.rs` (`build_agent_launch_command`, `shell_single_quote`),
  `activity_scanner.rs`, `activity.rs` (~100-140 precedence + record),
  `model.rs` (`activity_state`); `TerminalPanelProvider.tsx` ~89-91 and
  ~432-452; `src/lib/agents.ts`; [`AGENT_INTEGRATION.md`](../AGENT_INTEGRATION.md)
  for where the per-harness mechanism is documented.
- **Touch:** `ports/agent_runtime.rs`, the five `adapters/agent/*/mod.rs`,
  `src-tauri/src/terminal/{hooks,activity_scanner,activity,model,start}.rs`,
  `TerminalPanelProvider.tsx`, `src/types.ts` (`TerminalActivity` gains
  `'exited'` with a code), `AGENT_INTEGRATION.md` (one section per harness:
  the exact interactive argv). The Windows local arm builds the same line with
  `cmd.exe` quoting; the trailer uses `%ERRORLEVEL%` there (put it behind the
  existing `cfg(target_os = "windows")` in `transport.rs`, keep the builder
  itself `cfg`-free per AGENTS.md §7). Run `scripts/check-windows.sh`.
- **Done when:** a unit test renders the launch line for each of the three
  harnesses with a brief containing a single quote and a `$`, and asserts the
  quoting and the trailer; a scanner test feeds `state=exit;code=3` and
  asserts the resolver records exit with code 3 and the frontend maps it to
  `'exited'` — watched fail by feeding the old sequence. A deny list of
  `["Bash"]` appears in the claude line as `--disallowedTools` and in the
  opencode env as JSON. hermes and pi render `None` and the builder refuses a
  brief for them.

### T0.4 — tmux: runner prerequisite, host detection, wrapped sessions, lazy reconcile

- [ ] **Goal:** Spec §2.4. Runner install and the status probe check
  `command -v tmux` and surface a loud warning when absent. Per-machine
  detection for local and desktop-over-SSH hosts, cached beside the existing
  platform cache. When persistent, the spawned pty runs
  `tmux new-session -A -s <name> -c <dir>` before the launch line; `name` =
  `dm-<client8>-<feature8>-<origin>` from `client_install_id`. On launch,
  rows with a `tmux_name` are reconciled against `tmux ls -F
  '#{session_name} #{session_created} #{session_attached}'` (alive, age,
  attached elsewhere) with no channel opened; a tab attaches when shown.
  `End` = `tmux kill-session`. Windows local never wraps.
- **Size:** medium.
- **Context:** spec §2.4, §4.8; `src-tauri/src/terminal/{start,transport}.rs`;
  `crates/demeteo-core/src/infrastructure/runner/status.rs` and
  `src-tauri/src/commands/remote_install.rs` (the probe pattern);
  `crates/demeteo-core/src/adapters/ssh/session.rs` (`platform_cache`, the
  pool — note terminals deliberately open a fresh session; keep that);
  `application/remote_runs/client_id.rs`; T0.1's repo.
- **Touch:** the two runner probe sites + `packaging/install.sh` (warn, do not
  install); a `domain/tmux_name.rs` pure fn (name from client id, feature id,
  origin — tested); `start.rs`/`transport.rs` (wrap); a `reconcile_persistent_
  sessions` free fn over `ExecutionPort` + the session repo; `commands.rs`
  (`end_terminal_session` for tmux); the attach-command strings for §4.8
  (`ssh -p <port> <user>@<host> -t 'tmux attach -t <name>'`, built from the
  `Machine` row, never from an ssh-config alias).
- **Done when:** name fn tested (length, charset tmux accepts, distinct
  clients → distinct names); a reconcile test with a fake `ExecutionPort` that
  **errors on any command it was not told** (AGENTS.md §7) returns
  alive/age/attached for two rows and marks a third `ended`; a session started
  on a tmux-less machine has `persistent = false` and no `tmux_name`.
  Runner status shows the tmux warning when the probe fails.

### T0.5 — `gate_decisions.approved_head_sha`, recorded on every approval

- [ ] **Goal:** Spec §2.3. V55 adds `approved_head_sha TEXT NULL`. On
  `GateVerdict::Approve` the gate step reads `git rev-parse <feature_branch>`
  through `ExecutionPort` on the machine that owns the clone and stores it
  with the decision; `GateDecided` carries it; the `gate_decided` run event
  payload and the inspector's attempt history show it. For detached runs
  nothing crosses the RPC: the runner's own gate step does the read.
- **Size:** small.
- **Context:** spec §2.3, §4.10; `steps/gate/mod.rs` ~198-245; `repos/gate.rs`;
  `ports/notification.rs` (`GateDecided`); `run_event_log.rs` ~186;
  `RunEventFeed.tsx` ~140-190; `AttemptTable.tsx`.
- **Touch:** `migrations/V55__gate_approved_head_sha.sql`, `repos/gate.rs`,
  `domain/models/` gate decision struct, `steps/gate/mod.rs`,
  `ports/notification.rs`, `run_event_log.rs`, `RunEventFeed.tsx`, the
  inspector's gate attempt row.
- **Done when:** the e2e gate test asserts the stored SHA equals the branch tip
  at approval (with a fake exec that answers `rev-parse` and errors on anything
  else); the run-event bridge test covers the new payload field.

### T0.6 — Keyboard: xterm keeps the keyboard, `⌘.` to hand-back, `⌘\`` repurposed

- [ ] **Goal:** Spec §6. The global keydown handler ignores every non-Shift
  Ctrl chord while an xterm has focus (Linux/Windows), with no delay on Esc.
  `⌘.` and `⌘⇧F` stop aliasing the palette; `⌘.` is reserved for hand-back
  (wired in T1.4). `⌘\`` becomes "toggle this Feature's drawer" (wired in
  T1.1); until then it is a no-op rather than opening the retired view.
  `⌘⇧\`` stays registered; its context-sensitivity lands in T1.1/T1.8.
- **Size:** small.
- **Context:** spec §6; `src/lib/shortcuts.ts` (~123-125, ~185-243, ~518-529
  `isEditableTarget`); `src/hooks/useKeyboardShortcuts.ts` (~60-160);
  `src/components/TerminalSurface.tsx` (no custom key handler today).
- **Touch:** those three files and their tests
  (`useRunShortcuts.test.tsx` pattern).
- **Done when:** a test dispatches Ctrl+K with focus inside an xterm element
  and asserts the palette does not open, and the same chord outside does —
  watched fail first. The chord table in the palette's help matches spec §6.

---

## Phase 1 — Surfaces (J-S1, J-S2, J-S4, J-S5)

Dependency order: `T1.1 ─▶ T1.2, T1.3, T1.4, T1.6, T1.8` · `T1.2 ─▶ T1.4` ·
`T1.4 ─▶ T1.5` · `T1.7`, `T1.9`, `T1.10` are independent once Phase 0 is in.

### T1.1 — Drawer, focused layout, collapsed bar; retire `TerminalsView`

- [ ] **Goal:** Spec §4.6, §4.7 (minus the hand-back bar and context-rail
  contents that T1.4 fills), §4.3 collapsed bar, §4.4 "while the drawer is
  open". One session, two layouts. All `TerminalSurface`s stay mounted in a
  global host and are portalled into whichever drawer shows them (this
  replaces `TerminalsView`'s keep-mounted-and-hide trick). The `terminals`
  route is removed; the rail entry temporarily opens the drawer of the current
  page until T1.6 gives it the index. Location chip and violet frame (§5.2),
  tab bar with the `+` menu (`Shell here`, `Another agent here`, `Run the
  harness gate`, `Dev server`), copy attach (§4.8, strings from T0.4).
- **Size:** large. UI only.
- **Context:** spec §4.6-4.8, §4.3 (collapsed bar), §4.4 last paragraph, §5.2;
  `TerminalsView.tsx`, `TerminalPanelProvider.tsx`, `TerminalSurface.tsx`,
  `FeatureDetail.tsx`, `RunPanes.tsx`, `src/lib/uiPrefs.ts` (drawer height per
  window size is a new pref), `src/App.css` tokens (AGENTS.md §4).
- **Touch:** new `src/components/sessions/{SessionDrawer,SessionFocus,
  SessionTabBar,CollapsedSessionBar,AttachPopover,TerminalHost}.tsx` (one per
  file, <400 LOC each), `FeatureDetail.tsx`, `RunPanes.tsx`, router, delete
  `TerminalsView.tsx` and its tests, `uiPrefs.ts`. Every new `className` must
  resolve (`scripts/check-classes.mjs` is in `npm run checks`).
- **Done when:** navigating away from a Feature and back keeps the xterm
  instance (no reconnect, scrollback intact); `⌘↵` swaps layouts without
  remounting the surface; a runner session renders the violet frame, a local
  one none; hiding the drawer restores the run view exactly.

### T1.2 — Brief card (`gate`, `failed_step`) and "Fix with agent"

- [ ] **Goal:** Spec §4.5 for the two v1 origins. Location card without a
  toggle; agent row from the interactive-capable harnesses (T0.3) with
  per-project last-used memory in `uiPrefs`; CONTEXT from the critic verdict
  (gate) or the failing command + last 20 lines (failed step, from
  `error_message` / the attempt row); GOAL drafted; DON'T; footer. Enter
  starts: provision the session worktree attached (T0.2), start the session
  with the brief and deny list (T0.3), write the row with `baseSha` (T0.1),
  open the drawer on it. Esc starts without a brief. "Recreating from the
  branch" note when the worktree was missing.
- **Size:** medium.
- **Context:** spec §4.5, §4.3 header table; `src/components/ui/HarnessModelPicker.tsx`
  (harness/model/effort picker to reuse); `src/lib/harnessVerdict.ts` (error parsing
  for the failed-step context); `useWorktreeRouting.ts` (to replace `handleOpenTerminal
  Tab`); `StepInspector.tsx` for where the critic verdict is already rendered.
- **Touch:** new `src/components/sessions/BriefCard.tsx` + a
  `useStartSession.ts` hook; `useWorktreeRouting.ts` (delete the main-clone
  checkout path); `src/lib/terminal.ts`; `uiPrefs.ts`.
- **Done when:** with a stubbed start command, Enter produces one
  `start_terminal_session` call whose brief equals the edited text and whose
  deny list comes from the Feature's compiled profile; Esc produces one call
  with `brief: null`; the harness select never lists hermes or pi.

### T1.3 — Header split button, status strip, inspector decision block

- [ ] **Goal:** Spec §4.3 header table (v1 rows), caret menu, the strip's
  **Gate waiting / Gate approved / Step failed** variants (PR variant
  deferred), and §4.4's inspector Overview blocks: `WHAT YOU'RE DECIDING` with
  the session row, live tail for a running step, attempt history otherwise.
  Session pill on `StepCard` and the node chip on `WorkflowNode` (§4.4, the
  only run-view additions in v1). Environment-class failures show the
  reproduce line and `Retry`, no `Fix with agent`.
- **Size:** large. UI only.
- **Context:** spec §4.3, §4.4 (Timeline and Inspector paragraphs only); `Feature
  Header.tsx`, `GateStrip.tsx`, `FeatureStatusBanners.tsx`, `InspectorColumn.tsx`,
  `StepInspector.tsx`, `StepCard.tsx`, `src/components/canvas/WorkflowNode.tsx`,
  `EnvironmentNotReadyPanel.tsx`, `src/lib/runStatus.ts`.
- **Touch:** those files; `GateStrip.tsx` becomes `StatusStrip.tsx` with the
  precedence gate → failed; new `SessionPill.tsx` shared by card, step and node.
- **Done when:** existing `GateStrip`/`FeatureHeader` tests are migrated, not
  deleted; one test per strip variant; the split button label follows the
  state table for all five rows.

### T1.4 — Hand-back panel and bar, after-hand-back effects

- [ ] **Goal:** Spec §4.9 and §4.10 for `gate` and `failed_step`, plus the
  bar in §4.6. Git state (head, base, commits since, `git status --porcelain`)
  read through the pooled `ExecutionPort` on the session's machine, refreshed
  on activity transitions and on a poll while the drawer is visible.
  Uncommitted block with the template commit message + commitlint check +
  Stash. Agent working → `Stop the agent` (quit sequence, then offered kill) /
  `Wait`. Choices: gate → approve (records via T0.5) / redirect-to-critic
  (existing redirect with the critic step id as feedback) / redirect with a
  note; failed → `step_retry` / `replay_from_step` behind the disclosure. On
  confirm: detach the worktree (T0.2), then the pipeline call. After: strip
  flips, toast, `handed_back` run event (new `DomainEvent` + bridge arm +
  label), pill → idle, bar reads `detached · pipeline owns the branch` with
  `Re-attach` once parked. The hand-back **policy** (which choices for which
  origin, what blocks) is a sync fn in `domain/handback.rs`; the panel and an
  adapter perform it.
- **Size:** large. The one task that touches domain, adapter and UI together;
  split the domain fn + adapter into a first commit if the diff passes ~1,500
  lines.
- **Context:** spec §2.3, §4.9, §4.10; `domain/gate/` (the policy shape to
  copy), `domain/run_control.rs` (refusal fns); `gate_presenter.rs`;
  `impl_traits/step_executor.rs` (`step_retry`, `replay_from_step`);
  `run_event_log.rs`; `useRerunActions.ts`; `src/lib/features.ts`
  (`decideGate`); T0.2's port methods.
- **Touch:** new `domain/handback.rs` (+ tests), a `handback` application fn
  over the ports it needs (never construct an `ExecutionDriver` in a test —
  AGENTS.md §3), new Tauri commands (`session_git_state`, `session_commit`,
  `session_stash`, `session_handback`), `src/lib/sessions.ts`, new
  `src/components/sessions/{HandBackPanel,HandBackBar}.tsx`,
  `ports/notification.rs` + `run_event_log.rs` + `RunEventFeed.tsx`.
- **Done when:** domain tests: dirty tree blocks every choice; working agent
  blocks until stop/wait; untracked files excluded unless ticked; the
  commit template passes this repo's commitlint. Adapter test with an erroring
  fake exec: hand-back at a gate calls detach **before** `gate_decide`, and a
  failed detach aborts without deciding — watched fail first. UI: `⌘.` opens
  the panel and never confirms.

### T1.5 — Route every pipeline-advancing action through the panel

- [ ] **Goal:** Spec §2.3 last paragraph. Inspector `Approve`/`Redirect…`, strip
  `Retry`, replay-from-step, and the step card's `Decide Gate →` open the
  hand-back panel when the Feature has a session; unchanged otherwise.
- **Size:** small.
- **Context:** T1.4's panel API; `ActionsTab.tsx` (~70-96), `GateStrip`/
  `StatusStrip`, `StepCard.tsx`, `useRerunActions.ts`.
- **Touch:** those call sites and a single `useHandBackGate()` hook they share.
- **Done when:** one test per call site: with a session present the panel
  opens and the pipeline command is **not** called; without one, the command is
  called as before.

### T1.6 — Sessions index popover, rail count, pipeline-card pill, header item removed

- [ ] **Goal:** Spec §4.2 and §4.1. The rail's Terminals entry counts sessions
  **waiting on you** (awaiting input/approval + finished-dirty) and opens the
  index popover (Project → Feature rows, `No feature` rows, `Still alive on
  <runner>` from T0.4's reconcile, `End`, keyboard ↑↓↵). `PipelineCard` gets
  the session pill with loudest-state precedence and a tooltip listing all.
  The `TopBar` Terminals item is removed (UI_REDESIGN_PLAN D3, decided).
- **Size:** medium. UI only.
- **Context:** spec §4.1, §4.2, §5.1; `ProjectRail.tsx` (~27-33, ~108-112,
  ~190-199), `TopBar.tsx` (~50, ~148-157), `PipelineCard.tsx`, `SessionRow.tsx`
  (the row to evolve), `src/lib/runStatus.ts`; T0.1's per-feature listing.
- **Touch:** `ProjectRail.tsx`, `TopBar.tsx`, `PipelineCard.tsx`, new
  `src/components/sessions/SessionsIndex.tsx`, `SessionRow.tsx`,
  `SessionPill.tsx` from T1.3; `docs/UI_REDESIGN_PLAN.md` D3 marked decided.
- **Done when:** count test: two waiting + one working + one idle → `2 waiting`;
  Enter on a row navigates to the Feature with the drawer on that tab; a
  finished-dirty session appears in the count and the `End` on a dirty runner
  row asks first.

### T1.7 — Notifications for awaiting-input and finished-dirty

- [ ] **Goal:** Spec §5.1's last column. `DomainEvent::TerminalAwaitingInput`
  and `TerminalFinishedDirty` (once per session), routed like
  `TerminalAwaitingApproval`, gated by the same background/focus rules.
- **Size:** small.
- **Context:** `ports/notification.rs` (~193), `src-tauri/src/terminal/activity.rs`
  (~206-225), `src-tauri/src/adapters/tauri_ui/notification.rs`,
  `src-tauri/tests/tray_notification.rs`.
- **Touch:** those four.
- **Done when:** the tray notification test covers both new arms and asserts
  finished-dirty fires once for repeated dirty polls.

### T1.8 — Project home `Start session`: last-used location, caret, Feature-less drawer

- [ ] **Goal:** Spec §4.1 "Start session" and J-S4. Primary = shell at the
  project's last-used location (`uiPrefs`, keyed per project; main clone on
  first use). Caret = location picker + one entry per registry harness (one
  list, shared with the drawer's `+` menu). Sessions open in a Feature-less
  drawer on Project home. `⌘⇧\`` off a Feature page opens this shell.
- **Size:** small-medium.
- **Context:** spec §4.1, §6; `StartSessionButton.tsx`, `NewTerminalMenu.tsx`,
  `TerminalWorktreeLocationPicker.tsx`, `src/lib/terminalRecents.ts`,
  `src/lib/agents.ts`; T1.1's drawer.
- **Touch:** `StartSessionButton.tsx` (split button), `NewTerminalMenu.tsx`
  (reduce to the shared list), `uiPrefs.ts`, Project home composition.
- **Done when:** the two launchers render the same harness list from one
  source (test asserts equality); the primary click opens without a picker.

### T1.9 — Cleanup guard

- [ ] **Goal:** J-S7's v1 guard. `feature_cleanup` refuses while the Feature has
  any open session row (names them); `branch_delete` removes the session
  worktree (T0.2 did the git side; this wires the refusal).
- **Size:** small.
- **Context:** `application/lifecycle.rs` (~24-118); T0.1's repo.
- **Touch:** `lifecycle.rs`, its test, the Cleanup dialog copy.
- **Done when:** a test with one open session row gets the refusal naming it;
  with none, cleanup proceeds as before.

### T1.10 — Docs that the code now contradicts

- [ ] **Goal:** AGENTS.md §2: the `PermissionProfile` bullet reads "allow/deny
  for unattended runs; interactive sessions enforce deny and let the harness
  ask (TERMINAL_SESSIONS_SPEC §4.5)", and the `PermissionPolicyPort` name is
  replaced by the real one. [`TERMINAL_ACTIVITY.md`](TERMINAL_ACTIVITY.md)
  gains the `exited` state and the OSC trailer. [`UX_JOURNEYS.md`](UX_JOURNEYS.md)
  points J9/J10 at the drawer. [`UI_REDESIGN_PLAN.md`](UI_REDESIGN_PLAN.md) D3
  decided. Spec §3 surface table: `TerminalsView` marked retired with the
  commit. `docs/KNOWN_ISSUES.md` if any Windows gap surfaced in T0.3.
- **Size:** small. Docs only; `scripts/check-doc-refs.sh` and the cargo doc
  link gate still run.
- **Done when:** `npm run checks` green; no doc names `TerminalsView` as a
  place to work.

---

## Phase 2 — Designed, deferred (do not re-decide; see spec §11)

| Item | Spec | Needs first |
|---|---|---|
| J-S3 `published` origin: PR review comments in the brief, `Push to PR…`, `Check & push`, force-push confirmation, "manually changed" flag, "bring up to date" as a **merge** | §2.3, §4.5, §7 J-S3 | An MR-port method that reads review threads (GitHub + GitLab), a `features.manually_changed` column, T1.4 |
| J-S6 subtask peek with the live-step banner and keystroke holding | §4.11, §7 J-S6 | T1.1 |
| J-S7 full Cleanup confirmation (per-session tree state, ending tmux sessions) | §7 J-S7 | T1.9, T0.4 |
| Run-view slice: open on what needs you at 100%, `Needs you \| Whole run`, minimap, dashed `on_failure` loops with cycle counts, edge colouring | §4.4 Graph | none — independent of sessions |
| Interactive cost | §5.3 | a harness that reports it |
| Runner leg of the session-worktree conformance test | §10 Parity | the `remote_reconcile_runs` refactor named in [`EXECUTION_PARITY.md`](EXECUTION_PARITY.md) |

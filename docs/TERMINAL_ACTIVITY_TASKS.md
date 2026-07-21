# Terminal Agent Activity — Task Breakdown

Status: **Execution checklist**
Companions: `docs/TERMINAL_ACTIVITY_PLAN.md` (the how), `docs/TERMINAL_ACTIVITY_UX.md` (the what).

Purpose: slice the plan into **small, self-contained tasks** each of which a single
agent can complete without loading the whole terminal subsystem into context. Every
task names its files, its anchor symbols, its acceptance criteria, and its
dependencies. Tasks marked **‖ parallel** can run at the same time as their siblings.

Conventions:
- **Context budget** = the files an agent must read to do the task. Kept small on
  purpose — if a task needs more than ~3 files, it's split.
- **Anchors** = existing symbols to reuse, so no one re-derives the foundation.
- A task is *done* when its acceptance criteria pass **and** `cargo test` /
  `npm test` stay green.

---

## Phase 1 — Universal working/waiting floor + the whole UI surface

Ships working-vs-waiting live for every agent, plus the full UI wiring (nav count
included, reading 0 until Phase 2/3 feed `awaiting_approval`). No injection, no
transport risk. **Start here.**

### T1.1 — Backend: per-session last-output timestamp
- **Files:** `src-tauri/src/terminal.rs`
- **Anchors:** `struct ActiveSession` (~L29), `drain_local` (~L568), `drain_ssh`
  (~L530, already has a local `last_activity` Instant at L540/L549).
- **Do:** add `pub last_output_at: Arc<Mutex<Instant>>` (or `AtomicU64` millis) to
  `ActiveSession`; on every chunk read, both `drain_local` and `drain_ssh` update it.
  Generalize `drain_ssh`'s existing `last_activity` to write this shared field
  instead of a thread-local, so both transports feed one source of truth.
- **Accept:** field updates on output for both local and SSH; existing drain tests
  still pass; no change to forwarded bytes.
- **Context budget:** 1 file. **Deps:** none. **‖ parallel** with all frontend tasks.

### T1.2 — Backend: activity event + cadence sweep
- **Files:** `src-tauri/src/terminal.rs`
- **Anchors:** `spawn_agent_detector` / `detect_agents_once` (~L1257–1334) — copy the
  sleeping-thread + snapshot-then-emit-outside-lock pattern. `SessionInfo` (~L169) and
  the `terminal-session-agent` emit — clone the envelope shape.
- **Do:** define the event `terminal-session-activity → { session_id, state }` where
  `state ∈ "working" | "awaiting_input" | "exit"`. Add `spawn_activity_sweep` (~250ms
  tick): for each session **with an agent present** (`agent` is `Some`), resolve
  `working` if `last_output_at` is within the cadence window (~1s), else
  `awaiting_input`. Track last-emitted state per session; **emit only on change**.
  Plain-shell sessions (agent `None`) never emit.
- **Accept:** byte ⇒ `working` within one tick; ~1s quiet ⇒ `awaiting_input`; no
  re-emit of an unchanged state; agent-gating holds (plain shell silent).
- **Context budget:** 1 file. **Deps:** T1.1.

### T1.3 — Backend: instant `working` on idle→byte (optional optimization)
- **Files:** `src-tauri/src/terminal.rs`
- **Do:** when a drain reads the first byte after the session was idle, mark
  `working` inline (don't wait for the sweep tick). Guard so it only emits on the
  idle→active edge (no per-byte IPC).
- **Accept:** working shows sub-tick after silence; no emit storm during steady output.
- **Context budget:** 1 file. **Deps:** T1.2. *Skippable for MVP.*

### T1.4 — Frontend: activity type ‖ parallel
- **Files:** `src/types.ts`
- **Do:** `export type TerminalActivity = 'working' | 'awaiting_input' | 'awaiting_approval' | null;`
  add `activity?: TerminalActivity` to `TerminalTabDescriptor`.
- **Accept:** typechecks; no runtime behavior change.
- **Context budget:** 1 file. **Deps:** none.

### T1.5 — Frontend: reducer + event subscription
- **Files:** the terminal panel provider/context (grep `SET_AGENT` +
  `useTauriEvent('terminal-session-agent'`), plus `src/hooks/useTauriEvent.ts`).
- **Anchors:** mirror `SET_AGENT` exactly — same idempotent per-session update.
- **Do:** add `SET_ACTIVITY` (idempotent; `exit → null`) and
  `useTauriEvent('terminal-session-activity', …)` dispatching it.
- **Accept:** reducer is idempotent (same state twice = no-op); `exit` clears to null.
- **Context budget:** 2 files. **Deps:** T1.4.

### T1.6 — Frontend: `ActivityIndicator` component ‖ parallel
- **Files:** new `src/components/ui/ActivityIndicator.tsx` (+ colocated test).
- **Anchors:** `src/components/ui/AgentBadge.tsx` for the violet spinner idiom and
  styling conventions; UX §2 for the visual language (violet spinner = working,
  amber dot = waiting, pulsing red-amber = needs a decision, nothing = null).
- **Do:** pure presentational component keyed on `TerminalActivity`; renders nothing
  for `null`.
- **Accept:** one snapshot/render assertion per state incl. `null`.
- **Context budget:** 2 files. **Deps:** T1.4.

### T1.7 — Frontend: render the indicator in both surfaces
- **Files:** `src/components/SessionRow.tsx`, `src/components/TerminalSurface.tsx`.
- **Anchors:** where `AgentBadge` renders in each — place `ActivityIndicator` as its
  sibling (row = primary glance; surface header = focused tab).
- **Do:** read `activity` off the descriptor and render the indicator beside the badge.
- **Accept:** row and header show the correct mark as state changes; null shows only
  the agent badge.
- **Context budget:** 2 files + read T1.6's props. **Deps:** T1.5, T1.6.

### T1.8 — Frontend: nav attention count wiring
- **Files:** the Terminals nav item (grep the nav/rail item that shows terminals;
  candidates `src/components/TerminalsView.tsx`, `RailNavItem.tsx`).
- **Do:** derive a count of sessions in `awaiting_approval` and render it as a badge.
  In Phase 1 this is structurally always 0 — **the wiring is what ships**; it lights
  up for free once Phase 2/3 emit `awaiting_approval`.
- **Accept:** count renders from state; is 0 with no approval sessions; increments in
  a unit test that injects an `awaiting_approval` descriptor.
- **Context budget:** 1–2 files. **Deps:** T1.5.

### T1.9 — Backend tests
- **Files:** `src-tauri/tests/infrastructure/terminal.rs`
- **Do:** cadence transitions (byte⇒working; quiet⇒awaiting_input); agent-gating
  (plain shell never emits); dedup (unchanged state not re-emitted).
- **Deps:** T1.2.

### T1.10 — Frontend tests
- **Files:** colocated with T1.5/T1.6.
- **Do:** reducer idempotency + `exit→null`; `ActivityIndicator` per state; nav count
  from injected state.
- **Deps:** T1.5, T1.6, T1.8.

**Phase 1 dependency order:** T1.1 → T1.2 → (T1.3 opt); T1.4 → {T1.5, T1.6} → T1.7;
T1.5 → T1.8; tests trail their targets. Frontend chain (T1.4→) runs fully in parallel
with the backend chain (T1.1→).

---

## Phase 2 — Claude "needs a decision" + OS notification

Precise `awaiting_approval` for Claude via injected hooks, plus the OS notification.
**Gated on T2.1 — do the spike before building anything else in this phase.**

### T2.1 — Transport spike (GATE) ✅ DONE
- **Result:** transport = **hook-JSON `terminalSequence`** (`/dev/tty` is dead — no
  controlling terminal in Claude ≥ v2.1.139). Private OSC 777 payloads pass verbatim
  on post-init hook events (`Stop`/`Notification`/`PreToolUse`/…), **not**
  `SessionStart`. `--settings '<inline JSON>'` fires injected hooks. Full write-up:
  `docs/spikes/terminal-activity-transport.md`; plan §2a/§7 updated.
- **Impact on downstream tasks:** T2.2 drain scanner is unchanged (parses the same
  namespaced OSC). T2.3 emits the OSC via the hook's `terminalSequence` output (a tiny
  reporter script that `printf`s the sequence and `jq`-wraps it as JSON), bound to the
  §2d post-init events — drop any `/dev/tty` reporter idea. T2.5 nonce goes in the OSC
  payload as planned.

### T2.2 — Drain OSC scanner
- **Files:** `src-tauri/src/terminal.rs` (a new stateful scanner unit + its own tests).
- **Do:** stateful scanner between PTY read and broadcast. Engages **only on `ESC`**;
  on the namespaced prefix, buffer into a bounded residual (≤128 B); parse `state` on
  `ST` (`ESC \` or `BEL`); **strip** the whole sequence from forwarded bytes; emit the
  activity event; flush untouched on overflow; handle chunk-splits. Verify
  `nonce` (from T2.5) before accepting.
- **Accept:** the full test matrix in plan §6 (full / split across 2 & 3 chunks /
  `ESC\` vs `BEL` / overflow-flush / interleaved normal output / **other real OSCs
  passed through** — OSC 0/8/11 / two-in-one-chunk / nonce accept+reject).
- **Context budget:** the scanner lives in one module; test-first. **Deps:** T2.1.

### T2.3 — Backend owns the launch line
- **Files:** `src-tauri/src/terminal.rs` (`build_agent_launch_command`), and the
  frontend provider that today writes `claude\r` (grep `launchCommand`).
- **Do:** `build_agent_launch_command(claude-code)` returns
  `claude --settings '<hooks>'` with reporter hooks inline. **Frontend must stop
  writing `launchCommand` for hooked agent kinds** or Claude launches twice. Document
  the `--settings` merge caveat (deep-merges `hooks`, replaces the per-event array —
  ephemeral, per-session, never touches user files).
- **Accept:** no double-launch; user's `~/.claude` untouched; a test asserts the
  frontend suppresses its own write for hooked kinds.
- **Context budget:** 2 files. **Deps:** T2.1 (transport decides hook payload shape).

### T2.4 — Event→state map
- **Files:** the hook reporter payload (colocated with T2.3) + scanner state mapping.
- **Do:** wire the matcher table from plan §2d: `UserPromptSubmit`/`PreToolUse`/
  `PostToolUse`⇒`working`; `Notification.permission_prompt`⇒`awaiting_approval`;
  `Notification.idle_prompt`/`Stop`⇒`awaiting_input`; `SessionEnd`⇒`exit`. Implement
  the §2 precedence latch: `awaiting_approval` survives the cadence floor's
  `awaiting_input`, clears only on `working` or its source retracting.
- **Accept:** precedence-latch test (approval survives a cadence tick; clears on
  resume); ordering `working→awaiting_approval→(approve)→working` resolves.
- **Context budget:** 2 files. **Deps:** T2.2, T2.3.

### T2.5 — Per-launch nonce
- **Files:** `src-tauri/src/terminal.rs` (launch line + scanner).
- **Do:** inject `nonce=<random>` into the sequence at launch; scanner accepts it only
  for that session. Closes spoofed-OSC spam and cross-session TTY bleed.
- **Accept:** scanner rejects a sequence with a wrong/absent nonce (covered in T2.2's
  matrix). **Deps:** T2.3 (owns launch line); consumed by T2.2.

### T2.6 — OS notification via existing port
- **Files:** `src-tauri/src/adapters/tauri_ui/notification.rs` (`NotificationPort` /
  `DomainEvent`) + the activity emit site.
- **Do:** add a `DomainEvent` variant for `awaiting_approval` and route it through the
  existing pipeline so **focus / permission / `run_in_background` gating is reused**,
  not duplicated. This is the one real integration seam — bridge, don't copy the gate.
- **Accept:** notification fires only when unfocused + opted-in; deduped; existing
  notification tests green. **Deps:** T2.4.

**Phase 2 order:** T2.1 (gate) → T2.2 ‖ T2.3 → T2.5 (into T2.2) → T2.4 → T2.6.

---

## Phase 3 — "Needs a decision" for every agent (on-screen recognition)

Herdr-style recognition against the rendered xterm.js grid. Promotes only
`awaiting_input → awaiting_approval`; never invents working/idle.

### T3.1 — Rule-pack format & loader
- **Do:** decide format (inline TOML-like vs small JSON pack; bundled vs remotely
  updatable — plan §7.3) and write a loader. One rule set per agent: text patterns +
  ANSI/OSC evidence. Hot-reloadable.
- **Accept:** a sample pack loads; malformed pack fails loudly. **Deps:** none (can
  start after Phase 1).

### T3.2 — Recognition engine (bottom-rows matcher)
- **Files:** frontend, near `TerminalSurface` / xterm buffer access.
- **Do:** run the compiled rule set against the **bottom N rows of the rendered grid**
  (never scrollback). Strict approval-only: match ⇒ promote to `awaiting_approval`.
  Throttled to render-idle (rAF/debounced), agent-sessions only.
- **Accept:** matches a known prompt ⇒ approval; silence never yields approval; never
  blocks paint. **Deps:** T3.1, Phase 1 UI.

### T3.3 — Debounce + precedence feed
- **Do:** presence/confirmation debounce against transient frames; feed the same
  `activity` state under §2 precedence (screen-sourced approval == hook-sourced).
- **Accept:** no flap on a one-frame match; behaves identically to Phase 2 approval.
  **Deps:** T3.2, T2.4 (precedence latch).

---

## Phase 4 — Reach & hardening

Longest tail, lowest marginal value. Each is independent.

- **T4.1 — Remote hooked agents:** ✅ DONE. The Phase-2 drain scanner already runs on
  the SSH stream unchanged (`drain_ssh` builds an `ActivityScanner` when a nonce is
  present) — no runner needed. The missing half was *delivery*: a local `--settings`
  temp path is meaningless on the far host, so the reporter-hooks JSON is now SFTP'd
  onto the remote (`write_remote_settings_file`, run inside `start_ssh_session` while
  the session is still blocking and before the drain thread starts, so it never races
  the interactive read) to a nonce-keyed `/tmp/demeteo-claude-activity-<nonce>.json`
  (`remote_activity_settings_path`); the launch line references that remote path. An
  SFTP failure degrades to an unhooked launch. Remote file is left in `/tmp`
  (harmless — no live channel at teardown). Scope: remote *menu-launched* Claude only;
  remote hand-started agents still get nothing (no over-SSH presence detection — a
  separate deferred piece). **Deps:** Phase 2.
- **T4.2 — Stuck-`working` backstop:** ✅ DONE (cross-check, not TTL). Correction to the
  plan's premise: the cadence floor does NOT cover hooked sessions — the sweep skips
  them (`sweep_activity_once`: a TUI agent repaints continuously, so its byte stream
  never falls quiet), and the hook tier outranks cadence in `resolve`. So a lost
  `SessionEnd`/`Stop` would spin `working` forever, and a silence TTL can't fire.
  Implemented instead as a **process-detector cross-check** (`detect_agents_once` →
  `should_clear_activity_on_agent_exit`): when the local `ps` detector sees a session's
  agent leave (Some→None), it folds `exit` into the activity record (clearing it) so the
  badge can't strand on a spinner after the agent is gone. Guarded on an existing record
  (no spurious `exit` for plain shells). The alive-but-idle case (lost `Stop`, agent
  still running) is recovered by Claude's own `idle_prompt` Notification (§2d →
  `awaiting_input`) and the next `UserPromptSubmit`. **Documented gap:** remote hooked
  sessions have no `ps` cross-check (matching remote presence detection's constraint) and
  rely solely on those hook signals. **Deps:** Phase 2.
- **T4.3 — Merge user's own hooks** into the `--settings` payload (removes the 2c
  per-event replace caveat). **Deps:** T2.3.
- **T4.4 — Nonce hardening finalize + Windows.** **Deps:** T2.5.

---

## Suggested first sprint (agent-dispatchable now)

Two independent chains, safe to run in parallel:

1. **Backend floor:** T1.1 → T1.2 → T1.9  (one agent, one file + tests)
2. **Frontend surface:** T1.4 → T1.6 ‖ T1.5 → T1.7 → T1.8 → T1.10

They meet only at the event contract string `terminal-session-activity` and the
`state` values, which are fixed above — so neither chain blocks the other.

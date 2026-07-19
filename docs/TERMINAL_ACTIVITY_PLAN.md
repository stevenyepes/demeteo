# Terminal Agent Activity — Technical Plan

Status: **Draft for implementation**
Owner: terminals
Companion: `docs/TERMINAL_ACTIVITY_UX.md` (the experience this delivers).

Phasing rule: **least work / broadest impact first, then increasing value.**
Every phase is defined in terms of what it *reuses* from the existing
foundation — we add signal sources and one UI surface, not a new subsystem.

---

## 1. Foundation we build on (already shipped)

| Piece | Where | What it gives us |
|---|---|---|
| Drain threads | `terminal.rs` `drain_local` / `drain_ssh` | See **every byte** of every session (local PTY today, SSH too). `drain_ssh` already tracks a `last_activity` instant. |
| Background detector | `terminal.rs` `spawn_agent_detector` (3s loop) | The pattern for a cheap sleeping thread that diffs per-session state and emits. Presence via `ps` DFS (`find_agent_under`) — **sees through tmux/nesting**. |
| Event envelope | `SessionInfo` + `terminal-session-agent` | Additive Tauri event shape to clone for activity. |
| Frontend reducer | `TerminalPanelProvider` `SET_AGENT`, `AgentBadge` | Idempotent per-session state + a badge to render a sibling next to. |
| xterm.js | `TerminalSurface` | Holds the **rendered screen grid** — the input for on-screen approval recognition (Phase 3), with no server-side VT parser needed. |
| Notification port | `adapters/tauri_ui/notification.rs` (`NotificationPort` / `DomainEvent`) | Focus/permission/`run_in_background`-gated OS notifications — **reuse, don't reinvent** the gating. |

---

## 2. State model

```
null              → agent present, no activity signal yet (or plain shell)
working           → turn in progress / tool running
awaiting_input    → idle at prompt, waiting for the user's next message
awaiting_approval → blocked on a permission/confirmation gate
```

- **Layered on presence.** `agentKind` answers *which*; `activity` answers *what*.
- **Sourced precedence — not naive last-writer-wins.** Sources have different
  authority, so we resolve by priority, not arrival order:

  ```
  awaiting_approval  (explicit: hook or on-screen prompt)   ← highest
  working            (explicit hook, OR output cadence)
  awaiting_input     (output cadence: gone quiet)           ← lowest
  ```

  `awaiting_approval` is a **latch**: once set by its source it is *not*
  overridden by the cadence floor's `awaiting_input`. It clears only on a
  *working* signal (the agent resumed) or its own source retracting (hook
  `Stop`/`PreToolUse`, or the on-screen prompt disappearing). This is what lets
  the cheap universal floor and the precise approval layer coexist without
  fighting.

- **Ephemeral.** Never persisted; reset to `null` on reload/reconnect; re-derived
  from live signals (a reconnected tab is honest, not frozen).

Event contract (additive, deduped — emit only on real change):

```
terminal-session-activity → { session_id, state: "working" | "awaiting_input" | "awaiting_approval" | "exit" }
```
`exit` clears to `null`.

---

## 3. Two signal sources, split by which state they own

The core design decision (see the review that produced this plan): **layer by
state, not by transport.** A cheap universal signal owns the common states; a
precise per-agent signal owns the one high-value state where guessing is
unacceptable.

- **Signal A — output cadence (backend, universal, cheap).** Owns
  `working ↔ awaiting_input`. Bytes arriving ⇒ `working` (instant). ~1s of
  silence ⇒ `awaiting_input`. Gated to sessions with an agent present. Works for
  every agent, local or remote, hooked or hand-started, because it only needs
  the byte stream the drain already sees.
- **Signal B — explicit approval detection (precise).** Owns
  `awaiting_approval`. Two implementations, both feeding the same state:
  - **B1 — Claude hook** (Phase 2): the agent self-reports via injected hooks.
  - **B2 — on-screen recognition, Herdr-style** (Phase 3): match the rendered
    approval prompt for any agent.

---

## 4. Phases

### Phase 1 — Universal working/waiting floor + the whole UI surface
**Least work, broadest impact. Zero injection, zero transport risk.**

This alone delivers the §UX "glance across the panel" goal for *every* agent.

Backend:
- Track a per-session **last-output timestamp** (extend the drain; `drain_ssh`
  already has `last_activity` — generalize it and add to `drain_local`, stored
  on `ActiveSession`).
- One lightweight **sweep** (~250ms; reuse the `spawn_agent_detector` sleeping-
  thread pattern, or fold into it at a shorter tick): for each session **with an
  agent present**, resolve `working` (output within the window) vs
  `awaiting_input` (quiet), and emit `terminal-session-activity` **only on
  change**.
- Instant path: on the first byte after quiet, the sweep flips to `working`
  within one tick; for true snappiness we can also mark `working` inline in the
  drain when a session was idle (optional optimization).

Frontend:
- `src/types.ts`: `type TerminalActivity = 'working' | 'awaiting_input' | 'awaiting_approval' | null` on `TerminalTabDescriptor`.
- `SET_ACTIVITY` reducer (idempotent, `exit → null`), `useTauriEvent('terminal-session-activity', …)` — mirror `SET_AGENT`.
- `ActivityIndicator` (`src/components/ui/`) rendered in **SessionRow** (beside `AgentBadge`) and the **TerminalSurface** header.
- **Nav attention count** — sessions in `awaiting_approval` (extends naturally once Phase 2/3 land; in Phase 1 the count is simply always 0, the wiring is what ships).

Performance: O(sessions) per tick; dedup emits; no per-byte IPC.

Ships: **working vs waiting, live, for all agents, everywhere.**

---

### Phase 2 — Claude "needs a decision" + OS notification
**Moderate work, highest-value single state.** Precise `awaiting_approval` for
Claude, plus the notification that reaches you when you've looked away.

- **2a — Transport spike (GATE, do this first).** Verify empirically that a
  Claude hook subprocess can write an OSC to the PTY. The naive path
  (`printf '\033]…' >/dev/tty`) depends on hooks having a controlling terminal —
  **this is unverified and there is evidence it may not hold** (hooks may run
  with stdin as a pipe and no `/dev/tty`). Spike it in ~30 min before building
  2b. If it fails, switch the transport to the **hook JSON `terminalSequence`
  output** (have Claude emit the sequence into the real PTY on the hook's
  behalf) — this sidesteps `/dev/tty` entirely. Decide here.
- **2b — Drain OSC scanner.** A small stateful scanner between PTY read and
  broadcast: on the namespaced prefix, buffer into a bounded residual (≤128 B),
  parse `state` on `ST` (`ESC \` or `BEL`), **strip** the whole sequence from
  forwarded bytes, emit the event; flush untouched on overflow; handle
  chunk-splits. Engages only on `ESC`. Lives in the drain ⇒ **remote reuses it
  unchanged** (Phase 4). Wire format namespaced + versioned + **nonce** (below).
- **2c — Backend owns the launch line.** `build_agent_launch_command(claude-code)`
  → `claude --settings '<hooks>'` with the reporter hooks inline. Crucial:
  **frontend must stop writing `launchCommand` for hooked agents** (today
  `TerminalPanelProvider` writes `claude\r` itself) or Claude launches twice —
  the backend returns the full line and the frontend suppresses its own write
  for those kinds. `--settings` is per-session and **never mutates the user's
  files**. Note the real merge semantics: `--settings` deep-merges the `hooks`
  object but **replaces the array** under each event key — so our
  `Notification`/`Stop`/`PreToolUse`/… hooks replace the user's own hooks *on
  those same events* for this session only (ephemeral, acceptable; document it).
- **2d — Event→state map** (matchers verified against current Claude Code):

  | Claude hook (matcher) | state |
  |---|---|
  | `UserPromptSubmit`, `PreToolUse`, `PostToolUse` | `working` |
  | `Notification` `permission_prompt` | `awaiting_approval` |
  | `Notification` `idle_prompt` | `awaiting_input` |
  | `Stop` | `awaiting_input` |
  | `SessionEnd` | `exit` |

  `PreToolUse` fires **before** the permission prompt, so the ordering
  `working → awaiting_approval → (approve) → working` resolves correctly under
  the §2 precedence latch. (Optional richer events to evaluate:
  `PermissionRequest`/`PermissionDenied` as dedicated signals; `SubagentStop` /
  `TaskCompleted` to keep `working` accurate through background/subagent work.)
- **2e — OS notification, via the existing port.** Route `awaiting_approval`
  through the `NotificationPort` pipeline — add a `DomainEvent` variant so the
  **focus / permission / `run_in_background` gating is reused**, not duplicated.
  (The activity path emits raw `terminal-session-*` Tauri events today; this is
  the one real integration seam — bridge it, don't copy the gate.)
- **Nonce (ship here, not later).** Once notifications fire, a spoofed OSC from a
  repo script becomes notification spam / approval-fatigue — more than cosmetic.
  A per-launch `nonce=<random>` in the sequence, accepted only for that session,
  is nearly free (we own both ends) and also closes cross-session TTY bleed.

Performance: scanner is a few byte-compares per `ESC`, bounded residual, strip is
in-place; emits deduped.

Ships: **precise "needs a decision" for Claude + the notification that matters.**

---

### Phase 3 — "Needs a decision" for every agent (on-screen recognition)
**More work, broadens the high-value state to all agents.** This is the
Herdr-style heuristic, done the way Herdr actually does it (not naive cadence).

- **Match the rendered screen, frontend-side.** demeteo already has the xterm.js
  buffer — run recognition against the **bottom N rows** of the *rendered* grid
  (never the scrollback the user can scroll), so no server-side VT parser is
  needed.
- **Per-agent rule packs as data.** A small rule set per agent (text patterns +
  ANSI/OSC evidence like title/progress sequences), kept as **data** (TOML-like),
  hot-reloadable — adding an agent is authoring a file, not a subsystem. This is
  the lesson from Herdr: "universal" is universal in *mechanism*, still per-agent
  in *configuration*, but the config is cheap and centrally patchable.
- **Strict approval-only.** Recognition only ever promotes `awaiting_input →
  awaiting_approval` when the bottom buffer matches a known approval/permission
  UI. It never invents `working`/`idle` (the cadence floor owns those) and never
  guesses approval from silence.
- **Debounce.** A small presence/confirmation debounce prevents flap on
  transient frames.
- Feeds the same `activity` state under the §2 precedence (screen-sourced
  `awaiting_approval` behaves exactly like the hook-sourced one).

Performance: throttled to render-idle (rAF/debounced), bottom-rows-only,
**agent-sessions only**, compiled rule set — must never block paint or add
scroll jank.

Ships: **"needs a decision" for Codex, OpenCode, and hand-started agents.**

---

### Phase 4 — Reach & hardening
**Longest tail, lowest marginal value.**

- **Remote hooked agents.** The injected hook runs on the remote and emits the
  same OSC → the **same Phase-2 drain scanner** parses it out of the SSH stream.
  No runner needed.
- **Stuck-`working` liveness backstop.** If a `Stop`/`SessionEnd` is ever missed,
  `working` could persist while the agent actually idles (the `Stop` payload has
  **no** background-tasks field to lean on). The cadence floor already mostly
  covers this (silence ⇒ `awaiting_input`), but add a defensive TTL/cross-check
  against the process detector for the hooked path.
- **Merge the user's own hooks** into the `--settings` payload (removes the
  per-event replace caveat from 2c).
- **Nonce hardening finalize, Windows.**

---

## 5. Performance budget (hard constraints)

- Terminal **input/scroll latency is untouched** — no work on the keystroke path.
- Scanner engages **only on `ESC`**, bounded residual (≤128 B), in-place strip.
- Cadence sweep is **O(sessions) at ~250ms**; no per-byte IPC; **`working` is
  instant** (inline/one-tick), `awaiting_input` settles ≤ ~1s.
- On-screen recognition is **throttled, bottom-rows-only, agent-gated**, and runs
  off the render-idle path — never blocks a frame.
- All activity events are **deduped** (emit only on state change).

---

## 6. Testing

- **Phase 1:** cadence transitions (byte ⇒ working; quiet ⇒ awaiting_input);
  agent-gating (plain shell never emits); dedup (no re-emit of same state);
  reducer idempotency; `ActivityIndicator` per state; nav count wiring.
- **Phase 2:** scanner — full/split(2,3 chunks)/`ESC\`-vs-`BEL`/overflow-flush/
  interleaved-normal-output/**other real OSCs passed through** (OSC 0 title, 8
  hyperlink, 11 bg — the gap in the old spec)/two-in-one-chunk/nonce accept+reject;
  state map; precedence latch (approval survives a cadence tick, clears on
  resume); notification gating reuse; no double-launch.
- **Phase 3:** rule-pack matching per agent; strict-approval (silence never
  yields approval); debounce; throttle never blocks paint.
- **Manual:** launch Claude → trigger a permission prompt (needs-a-decision +
  notification when backgrounded) → approve (working) → finish (waiting) → exit
  (clear). Confirm **no OSC artifact** ever renders.

---

## 7. Open decisions

1. **Phase-2 transport spike** — `/dev/tty` from hooks vs hook-JSON
   `terminalSequence`. *Gates Phase 2; resolve first.*
2. **Cadence window** — the silence threshold for `awaiting_input` (start ~1s;
   tune against real agent idle prompts, watching for agents that animate while
   idle — mitigated by agent-gating + the Phase 2/3 approval overlay correcting
   the one critical case).
3. **Rule-pack format & location** — inline TOML-like data vs a small JSON pack;
   bundled vs remotely updatable (Herdr updates remotely because agent UIs
   drift).

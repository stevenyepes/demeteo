# Terminal Agent Activity Detection — Spec (v1a: Claude Code, local, hook-based)

Status: **Draft for implementation**
Owner: terminals
Related: `src-tauri/src/terminal.rs` (drain + `spawn_agent_detector`), `src/context/TerminalPanelProvider.tsx`, `src/lib/agents.ts`, `remote-terminals-agent-labels` (agent *presence* work this builds on).

---

## 1. Goal & scope

Users must be able to tell, at a glance, whether a terminal:

1. is in an interactive session with a **supported coding agent** (already shipped: agent *presence* via `agentKind`);
2. has that agent **working** (processing a turn / running a tool);
3. has that agent **awaiting user action** — split into **awaiting input** (idle at its prompt) and **awaiting approval** (blocked on a permission gate, notification-worthy).

This spec covers the **precise, hook-based path** (the `tmux-agent-status` model) for **Claude Code**, on **local** terminals, that **demeteo launches itself**. It is deliberately the high-fidelity slice; the Herdr-style heuristic fallback and remote/runner delivery are explicit non-goals here (§11).

### 1.1 In scope (v1a)
- Claude Code sessions started via the `+ New` menu / workflow launch on the **local** machine.
- Activity states: `working`, `awaiting_input`, `awaiting_approval` (+ cleared).
- Transport: injected Claude **hooks** that emit a private **OSC escape** into the PTY; the existing **drain thread** parses it. No files, no sockets, no new deps.
- A per-tab activity indicator (list + surface) and an OS notification on `awaiting_approval`.

### 1.2 Out of scope (tracked as follow-ups, §11)
- OpenCode & Codex activity — **specified** as profiles (§4.3–§4.5) but **built in P4**; Claude Code (P1) ships first. Codex is only partially hook-covered (§4.5).
- **Remote** terminals (delivered later via the runner's `notify_bridge`, which already carries `agent_spawned`).
- **Hand-started** agents (user types `claude` themselves) — no hooks to inject; stays presence-only. Heuristic fallback is future work.
- Windows.
- Anti-spoofing nonce, merging the user's own hooks (§9, §11).

---

## 2. State model

```
none            → no supported agent in the session (existing agentKind == null)
present         → agent detected, activity unknown (agentKind set, no activity event yet)
working         → turn in progress / tool running
awaiting_input  → agent idle at its prompt, waiting for the user's next message
awaiting_approval → agent blocked on a permission/confirmation gate
```

- **Activity is layered on top of presence.** `agentKind` (already shipped) answers *which* agent; `activity` answers *what it's doing*. A tab can be `present` with `activity == null` until the first hook fires.
- **Level-based, last-writer-wins.** Each hook sets an *absolute* state, never toggles. `working` is set by turn/tool events and only cleared by a `Notification`/`Stop`. This makes transitions monotonic and thrash-free (no debounce needed — §7.3).
- Activity is **ephemeral**: never persisted, not restored on reload/reconnect. It re-establishes on the next hook (§8).

---

## 3. Transport: private OSC over the PTY

The drain thread already sees every byte of every session (local PTY today, SSH tomorrow). We reuse it: the injected hook writes a **private, namespaced OSC** to `/dev/tty` (the controlling terminal = PTY slave), which lands in demeteo's drain stream. This needs **no files, no sockets, no new dependency, and no session-correlation** — the byte channel is already session-scoped.

### 3.1 Wire format (the contract)

```
ESC ] 5379 ; demeteo ; v=1 ; state=<STATE> ST
```

- `ESC` = `0x1B`, `]` = OSC introducer, `5379` = private/unassigned code, params `;`-delimited.
- `ST` (String Terminator) = `ESC \` (`0x1B 0x5C`) **or** `BEL` (`0x07`) — parser accepts both.
- `demeteo` = mandatory namespace. Parser ignores any OSC lacking it.
- `v=1` = format version, for forward evolution.
- `<STATE>` ∈ `{ working, awaiting_input, awaiting_approval, exit }`. `exit` clears activity (agent ended).

A stray demeteo-OSC in some *other* terminal is a harmless unknown OSC (ignored by every emulator). demeteo **strips** the full sequence from the bytes forwarded to xterm, so it never renders.

### 3.2 Hook command template (POSIX sh, pure ASCII)

```sh
printf '\033]5379;demeteo;v=1;state=%s\033\\' <STATE> >/dev/tty 2>/dev/null || true
```

- `\033` (octal ESC) and `\033\\` (ESC + `\`, the ST) keep the whole thing ASCII, so it embeds cleanly in JSON and a shell command line — no raw control bytes.
- `>/dev/tty` targets the PTY slave → demeteo's drain reads it. A hook subprocess of the agent shares the session's controlling terminal, so `/dev/tty` resolves correctly.
- `|| true` + `2>/dev/null`: a reporting failure must **never** fail or block the agent turn.

---

## 4. Launch injection & agent profiles

One shared OSC contract (§3); each agent adds a **profile** = *(injection mechanism + event→state map)*. Everything else — §3 wire format, §5 scanner, §6 event, §7 UI — is agent-independent, so a new agent is a profile, not a subsystem. **§4.1–§4.2 are the Claude Code profile (P1)**; **§4.3–§4.4** add OpenCode and Codex (P4); **§4.5** is the fidelity each can promise.

demeteo controls the launch command (it already writes `launchCommand` into the fresh PTY, §`TerminalPanelProvider.openInternal`). For a Claude launch we launch with an **inline `--settings`** payload carrying the reporter hooks:

```
claude --settings '<HOOKS_JSON>'
```

`--settings` overrides matching keys **for that session only** — the user's `~/.claude/settings.json` is never modified and nothing needs cleanup.

### 4.1 Ownership: the backend builds the launch command

Move construction of the agent launch command for **hooked** agents into the backend so the OSC/hook contract lives in exactly one place (Rust), next to the parser that consumes it — mirroring how `agent_kind_for_binary` is the backend twin of `src/lib/agents.ts`.

- Add `build_agent_launch_command(agent_kind) -> Option<String>` (or fold into `start_terminal_session`, which already receives `agent_kind`). Returns e.g. `claude --settings '{…}'` for `claude-code`, `None` for agents without a hook profile (frontend then falls back to the bare binary, as today).
- The frontend keeps writing the returned string to the PTY (unchanged mechanics), or the backend writes it directly after the branch bootstrap (preferred — it already writes the bootstrap line; keeps the whole hook story server-side).

### 4.2 HOOKS_JSON (Claude hooks schema)

One reporter per lifecycle event; the state is hardcoded per hook (no logic in the hook):

| Claude hook (matcher) | Emitted `state` |
|---|---|
| `UserPromptSubmit` | `working` |
| `PreToolUse` | `working` |
| `PostToolUse` | `working` |
| `Notification` `permission_prompt` | `awaiting_approval` |
| `Notification` `idle_prompt` | `awaiting_input` |
| `Notification` `agent_needs_input` | `awaiting_input` |
| `Stop` | `awaiting_input` |
| `SessionEnd` | `exit` |
| `SessionStart` (optional) | `awaiting_input` (agent boots at its prompt) |

Shape (escaping abbreviated — `\033` becomes `\\033` inside the JSON string):

```jsonc
{
  "hooks": {
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "printf '\\033]5379;demeteo;v=1;state=working\\033\\\\' >/dev/tty 2>/dev/null || true" }] }],
    "PreToolUse":       [{ "hooks": [{ "type": "command", "command": "…state=working…" }] }],
    "PostToolUse":      [{ "hooks": [{ "type": "command", "command": "…state=working…" }] }],
    "Notification": [
      { "matcher": "permission_prompt", "hooks": [{ "type": "command", "command": "…state=awaiting_approval…" }] },
      { "matcher": "idle_prompt",       "hooks": [{ "type": "command", "command": "…state=awaiting_input…" }] },
      { "matcher": "agent_needs_input", "hooks": [{ "type": "command", "command": "…state=awaiting_input…" }] }
    ],
    "Stop":       [{ "hooks": [{ "type": "command", "command": "…state=awaiting_input…" }] }],
    "SessionEnd": [{ "hooks": [{ "type": "command", "command": "…state=exit…" }] }]
  }
}
```

### 4.3 Profile — OpenCode (P4)

OpenCode has no per-session settings flag; it loads **plugins** from `~/.config/opencode/plugin/` (global) or `.opencode/plugin/` (project). demeteo ships a small, idempotent plugin (`demeteo-activity.js`), installed/refreshed on the first OpenCode launch. Plugins are **additive**, so it coexists with the user's own. The plugin runs inside the opencode process (in the PTY) and writes the same OSC (§3.1) to `/dev/tty` (Node: `fs.writeSync(fs.openSync('/dev/tty','w'), osc)`).

| OpenCode event | state |
|---|---|
| `tool.execute.before` | `working` |
| `message.updated` / `message.part.updated` | `working` |
| `permission.asked` | `awaiting_approval` |
| `permission.replied` | `working` |
| `session.idle` | `awaiting_input` |
| `session.created` (optional) | `awaiting_input` |
| `session.deleted` / `session.error` | `exit` |

Caveat: install is **persistent** (not per-session like Claude). Verify the plugin dir name (`plugin` vs `plugins`), the exact event names, and whether a per-run plugin/`--config` flag exists (§10).

### 4.4 Profile — Codex CLI (P4 — partial)

Codex's only external hook is the `notify` program (one command, run with a single JSON arg). To stay non-invasive, set it via a per-run CLI override instead of editing `~/.codex/config.toml`:

```
codex -c 'notify=["/bin/sh","<demeteo-reporter>"]'
```

The reporter reads the JSON arg's `type` and writes the OSC to `/dev/tty`. What `notify` delivers today:

| Codex `notify` `type` | state |
|---|---|
| `agent-turn-complete` | `awaiting_input` |

That is the **only** external event Codex emits, so `working` and `awaiting_approval` are **not hook-derivable** — they come from the **heuristic backstop** (§11): output cadence → `working`; the TUI's approval-prompt pattern → `awaiting_approval`. Note `notify` is a **single** program (editing `config.toml` would clobber the user's; the `-c` per-run override avoids that). Verify whether a newer Codex exposes `approval-requested` externally (§10).

### 4.5 Fidelity contract

| Agent | working | awaiting_input | awaiting_approval | injection |
|---|---|---|---|---|
| **Claude Code** | hook | hook | hook | per-session `--settings` (non-invasive) |
| **OpenCode** | hook | hook | hook | persistent plugin (coexists) |
| **Codex CLI** | heuristic | hook | heuristic | per-run `-c notify` (verify) |

The UI **must not over-promise** — render only the states an agent's profile can source: a Codex tab shows `awaiting_input` precisely, but `working`/`awaiting_approval` only when the heuristic backstop is enabled.

---

## 5. Backend: drain-thread OSC scanner

A small, stateful scanner sits between the PTY read and the `Broadcast`, per session.

### 5.1 Behaviour
1. Pass bytes through to the `Broadcast` (scrollback + subscribers) **unchanged**, except:
2. When it sees the prefix `ESC ] 5379 ;`, it starts buffering into a bounded per-session residual (cap **128 bytes**).
3. On `ST` (`ESC \` or `BEL`): parse `;`-params, require the `demeteo` namespace, read `state`, map to activity (§4.2 inverse), **remove the whole sequence** from the forwarded bytes, and emit `terminal-session-activity` (§6).
4. On cap-overflow without an `ST`: it was not our sequence — **flush the buffered bytes through** to the Broadcast unchanged (never swallow real output) and reset.
5. Handle **splits across chunks**: the residual buffer persists between `read()`s so a sequence straddling two chunks is still matched.

### 5.2 Invariants
- Never forwards a recognised demeteo-OSC to xterm (no visible artifact).
- Never drops or reorders non-demeteo bytes.
- Bounded memory: residual ≤ 128 bytes per session.
- Only engages on an `ESC` byte — the common path is a cheap byte-compare.

### 5.3 Placement
In `drain_local` / `drain_ssh` (or a shared `scan_and_forward` helper wrapping `send_chunk`). Because it lives in the drain, the **same code path serves remote later** with zero changes (§11).

---

## 6. Event contract

New Tauri event, additive:

```
terminal-session-activity  →  { session_id: string, state: "working" | "awaiting_input" | "awaiting_approval" | "exit" }
```

- `exit` (or a `SessionEnd`) clears activity to `null`.
- Emitted only on an *actual* state change (scanner dedupes: don't re-emit the same state).
- Reuses the `SessionInfo`-style payload envelope already used by `terminal-session-agent`.

---

## 7. Frontend

### 7.1 Types & state
- `src/types.ts`: `type TerminalActivity = 'working' | 'awaiting_input' | 'awaiting_approval' | null;` add `activity?: TerminalActivity` to `TerminalTabDescriptor`.
- `TerminalPanelProvider`: `SET_ACTIVITY { sessionId, state }` reducer action (maps `exit` → `null`); `useTauriEvent('terminal-session-activity', …)` dispatches it. Idempotent (skip re-render when unchanged), matching the `SET_AGENT` pattern.

### 7.2 UI
- New `ActivityIndicator` (`src/components/ui/`):
  - `working` → animated spinner (violet, matching `AgentBadge`).
  - `awaiting_input` → steady amber dot + "needs input" (title).
  - `awaiting_approval` → pulsing red/amber + "approval" (title) — highest salience.
  - `null` → nothing.
- Render it in **`SessionRow`** (next to `AgentBadge`, line 1) and **`TerminalSurface`** header.
- **Global attention**: the Terminals nav item shows a count of sessions in `awaiting_*` (so a backgrounded terminal that needs you is visible app-wide).

### 7.3 No debounce needed
State is level-based (§2): `working` events are sticky and only cleared by a `Notification`/`Stop`. Rapid `PreToolUse`/`PostToolUse` bursts all set `working` → no flicker.

---

## 8. Lifecycle & edge cases

- **Reconnect / webview reload**: activity is ephemeral → reset to `null`; re-derives on the next hook. (`startup reconcile` sets `activity: null`.)
- **Agent exits**: `SessionEnd` → `exit` → `null`. The existing process detector also clears `agentKind`.
- **Hand-started `claude`**: no injected hooks → `activity` stays `null` (presence-only). Documented limitation.
- **Stale `working`**: if a `Stop`/`SessionEnd` is somehow missed, `working` could persist. Mitigation: when the presence detector reports the agent gone (`agentKind` → null), the provider also clears `activity`. (Belt-and-suspenders; hooks are the primary signal.)
- **Background tasks**: Claude may `Stop` while background work continues (couldn't confirm the payload field — §10). If it fires `Stop` → we'd show `awaiting_input` early. Acceptable for v1a; revisit with the payload check if it proves wrong.

---

## 9. Security

- The injected hook only `printf`s a **fixed** OSC to `/dev/tty` — no data read, no exfiltration, no blocking; `|| true` guarantees it can't fail a turn.
- The OSC is **namespaced** (`demeteo`) and **stripped** before render.
- Spoofing (a repo script emitting a fake state OSC) is **cosmetic-only** and out of scope for v1a; a per-launch nonce in the sequence is the hardening follow-up (§11).
- `--settings` is inline and per-session; **no user file is written or mutated**.

---

## 10. Open questions to resolve during implementation

1. **Does `--settings` *merge* or *replace* the `hooks` key?** If it replaces, a demeteo-launched session would suppress the user's own hooks. v1a ships demeteo hooks only (reporters are non-blocking, and most users have none); **follow-up**: read + merge the user's resolved hooks into the payload. *Verify first.*
2. **`Stop` payload & background tasks** — confirm whether `Stop` carries a `background_tasks`/`stop_hook_active` field so we don't flip to `awaiting_input` while work continues.
3. **Hook firing cadence** — confirm `PreToolUse`/`PostToolUse` fire often enough that long tool runs stay `working` (the level-based model already tolerates gaps, but verify no silent window).
4. **OpenCode plugin injection** — confirm the plugin dir (`~/.config/opencode/plugin` vs `plugins`), the §4.3 event names, and whether a **per-run** plugin/`--config` flag exists (would make injection non-persistent like Claude).
5. **Codex `-c notify=…` per-run override** — confirm it sets `notify` for a single run without editing `~/.codex/config.toml` (§4.4).
6. **Codex external event surface** — confirm whether the current Codex exposes anything beyond `agent-turn-complete` (e.g. `approval-requested`) to the *external* `notify` program; today only turn-complete is external, forcing the heuristic backstop.
7. **OSC from non-shell runtimes** — verify the OpenCode plugin (Node `fs` write) and the Codex reporter both reach `/dev/tty` and land in the drain exactly like Claude's `printf` hook.

---

## 11. Follow-ups (post-v1a)

- **Heuristic fallback** (Herdr-style): output cadence + spinner/prompt patterns + the existing process detector. Two roles: the state source for hand-started agents / agents without hooks, **and the backstop that fills Codex's un-hooked `working`/`awaiting_approval`** (§4.4–§4.5). Universal, lower fidelity.
- **Remote**: the injected hook runs on the remote and emits the same OSC → the **same drain scanner** (§5.3) parses it out of the SSH stream — *no runner needed*. (Optionally also surface `notify_bridge`'s `agent_spawned`/status for runner-managed runs.)
- **OpenCode & Codex profiles (P4)**: implement the §4.3–§4.5 profiles (OpenCode plugin; Codex `-c notify`) next to `AGENTS`, resolving the §10 verifications first.
- **Merge user hooks** into the `--settings` payload (open question 1).
- **Anti-spoof nonce**: `nonce=<per-launch>` param; scanner accepts only the session's nonce.
- **Windows**.

---

## 12. Testing

### 12.1 Rust unit (drain scanner) — the core risk
- Full sequence in one chunk → state emitted, bytes stripped.
- Sequence split across 2 and 3 chunks → still matched.
- `ST` = `ESC \` vs `BEL` → both parse.
- Unterminated prefix past the 128-byte cap → buffered bytes flushed through unchanged (nothing swallowed).
- demeteo-OSC interleaved with normal ANSI/output → normal bytes preserved byte-for-byte, only ours removed.
- Two sequences in one chunk → both emitted.
- Non-namespaced `ESC ] 5379` (no `demeteo`) → passed through, not acted on.
- State→activity mapping table.

### 12.2 Frontend
- `SET_ACTIVITY` reducer (incl. `exit` → `null`, idempotent no-op).
- `ActivityIndicator` renders correctly per state.
- Nav attention count reflects `awaiting_*`.

### 12.3 Manual / integration
Launch Claude via `+ New` → trigger a permission prompt (`awaiting_approval`) → send a message (`working`) → let it finish (`awaiting_input`) → exit (`null`). Confirm no OSC artifact renders in xterm.

---

## 13. Rollout phases

- **P0** — Contract + backend OSC scanner + `terminal-session-activity` event. Validate with logs, no UI.
- **P1** — `build_agent_launch_command` + inline `--settings` injection for local Claude.
- **P2** — Frontend `ActivityIndicator` in `SessionRow` + `TerminalSurface`; nav attention count.
- **P3** — OS notification on `awaiting_approval` (reuse the notification adapter; only when the terminal isn't the active/focused view; debounced).
- **P4a** — OpenCode profile (§4.3): idempotent plugin install + event→OSC map. Full fidelity.
- **P4b** — Codex profile (§4.4): per-run `-c notify` reporter (`awaiting_input`) + **heuristic backstop** for `working`/`awaiting_approval` (§11).
- **P5+** — remaining follow-ups (§11): remote via the drain scanner, heuristic fallback for hand-started agents, anti-spoof nonce, merge user hooks, Windows.

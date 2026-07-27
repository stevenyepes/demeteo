# Terminal Agent Activity — Design Notes

> **Shipped** (Phases 1–3, plus T4.1/T4.2 of the hardening tail). This replaces
> the UX spec, technical plan, and task breakdown that drove the work. It keeps
> only what reading `src-tauri/src/terminal/` and `src/lib/terminalActivity/`
> will not tell you: why the layering is what it is, the transport findings that
> cost a spike to establish, and the two constraints that will bite anyone
> extending it.

## The problem it solves

Running several coding agents across terminal tabs, you can't tell which one is
thinking, which is done and waiting, and which is **stuck asking permission** —
without clicking into each tab. The one that needs a decision is exactly the one
you can't afford to miss. **The glance test:** open the terminal panel, look
once, know the state of every agent.

## State model

```
null              → agent present, no activity signal yet (or plain shell)
working           → turn in progress / tool running
awaiting_input    → idle at prompt, waiting for the user's next message
awaiting_approval → blocked on a permission/confirmation gate
```

Activity is layered **on top of presence**: `agentKind` answers *which* agent is
in a tab, `activity` answers *what it's doing*. State is **ephemeral** — never
persisted, reset to `null` on reload/reconnect, re-derived from live signals, so
a reconnected tab is honest rather than frozen.

Event contract (additive, deduped — emitted only on real change):

```
terminal-session-activity → { session_id, state: "working" | "awaiting_input" | "awaiting_approval" | "exit" }
```

### Precedence is sourced, not last-writer-wins

Sources carry different authority, so `resolve` picks by priority, not arrival
order:

```
awaiting_approval  (explicit: hook or on-screen prompt)   ← highest
working            (explicit hook, OR output cadence)
awaiting_input     (output cadence: gone quiet)           ← lowest
```

`awaiting_approval` is a **latch**: once set it is not overridden by the cadence
floor's `awaiting_input`. It clears only on a `working` signal (the agent
resumed) or its source retracting (hook `Stop`/`PreToolUse`, or the on-screen
prompt disappearing). **This latch is the whole reason the cheap universal floor
and the precise approval layer can coexist without fighting** — remove it and
every approval flickers away after ~1s of silence.

### Two signals, split by which state they own

The core decision: **layer by state, not by transport.**

- **Signal A — output cadence** (backend, universal, cheap). Owns
  `working ↔ awaiting_input`. Bytes arriving ⇒ `working`; ~1s silence ⇒
  `awaiting_input`. Gated to sessions with an agent present. Works for every
  agent, local or remote, hooked or hand-started, because it needs nothing but
  the byte stream the drain already sees.
- **Signal B — explicit approval detection** (precise). Owns
  `awaiting_approval`, from two implementations feeding the same state: the
  injected Claude hook, and Herdr-style on-screen recognition against the bottom
  N rows of the rendered xterm grid (never scrollback) for any agent. Recognition
  is strictly promote-only: it can raise `awaiting_input → awaiting_approval` and
  never invents `working` or idle.

## Transport: hook-JSON `terminalSequence`, not `/dev/tty`

Established empirically against Claude Code v2.1.211. Both halves matter:

- **`/dev/tty` is dead.** Claude runs hooks in their own session with **no
  controlling terminal** (v2.1.139+); every `printf … >/dev/tty` fails. A control
  PoC that *did* share a controlling terminal succeeded, so the mechanism is
  fine — Claude deliberately detaches the hook. Don't re-litigate this.
- **`terminalSequence` works.** The hook returns
  `{"terminalSequence":"<ESC>]777;…<BEL>"}` and Claude writes the bytes to the
  PTY on its behalf. Arbitrary OSC 777 payloads are **not** sanitized down to a
  notification grammar, so a private namespaced signal
  (`\x1b]777;demeteo;…;state=working\x07`) rides through verbatim.

Three caveats that are easy to rediscover the hard way:

1. `terminalSequence` renders on post-init events (`Stop`, `Notification`,
   `PreToolUse`/`PostToolUse`, …) but **not on `SessionStart`** — dropped before
   the UI is live. Every activity signal rides a post-init event, so this is
   harmless *as long as it stays that way*.
2. It does **not** render in headless `claude -p` mode. Only matters for tests;
   Demeteo runs Claude interactively.
3. `--settings` deep-merges the `hooks` object but **replaces the array** under
   each event key. Our hooks therefore replace the user's own hooks *on those
   same events*, for that session only. Ephemeral and accepted — merging the
   user's hooks into the payload is the open T4.3 item below.

The backend owns the launch line for hooked agents. The frontend must **not**
also write `launchCommand` for those kinds or Claude launches twice.

## Two constraints that will bite you

**Remote hooked agents need the settings file on the far host, written before
the drain starts.** A local `--settings` temp path is meaningless over SSH, so
the reporter-hooks JSON is SFTP'd to a nonce-keyed
`/tmp/demeteo-claude-activity-<nonce>.json`. It must be written inside
`start_ssh_session` **while the session is still blocking and before the drain
thread starts**, or it races the interactive read. SFTP failure degrades to an
unhooked launch. The remote file is left in `/tmp` (harmless — no live channel at
teardown). Scope is remote *menu-launched* Claude only; remote hand-started
agents get nothing, because there is no over-SSH presence detection.

**A stuck `working` cannot be fixed with a silence TTL.** The cadence floor does
not cover hooked sessions — the sweep skips them, because a TUI agent repaints
continuously and its byte stream never falls quiet — and the hook tier outranks
cadence in `resolve`. So a lost `SessionEnd`/`Stop` would spin `working` forever
and no TTL could fire. The backstop is instead a **process-detector cross-check**
(`detect_agents_once` → `should_clear_activity_on_agent_exit`): when the local
`ps` detector sees a session's agent leave (Some→None), it folds `exit` into the
activity record. The alive-but-idle case (lost `Stop`, agent still running) is
recovered by Claude's own `idle_prompt` Notification and the next
`UserPromptSubmit`. **Documented gap:** remote hooked sessions have no `ps`
cross-check and rely solely on those hook signals.

## Windows support

Local terminals work on Windows with activity degraded to best-effort. The split
is scoped to the **local** shell — the SSH path always targets a POSIX shell and
is untouched regardless of client OS.

- **Shell is `%COMSPEC%` / `cmd.exe`**, chosen by `select_local_shell()`. The
  work-branch bootstrap line has a cmd.exe variant with its own
  `cmd_double_quote` escaper. The SSH path never calls that selector — it always
  emits `branch_bootstrap_line_posix`, so a Windows client opening a remote
  terminal still sends POSIX to the remote shell.
- **Local agents run unhooked.** `is_hooked_agent_kind` stays OS-agnostic;
  whether the hook *transport* is usable is a separate session-scoped decision in
  `hook_transport_supported(is_local)`. The transport emits `printf '%s'` and
  POSIX-single-quoted `--settings`, none of which survive cmd.exe — so it returns
  `false` for **local** Windows sessions only. There, `awaiting_approval` comes
  only from the on-screen scanner, which is platform-neutral. SSH sessions keep
  their hooks on any client.
- **Process-tree detector** has a Windows implementation
  (`Get-CimInstance Win32_Process` with `CREATE_NO_WINDOW`) returning `None` on
  any failure. `detect_agent_in_command` strips `.cmd`/`.exe`/`.bat` as well as
  `.js`/`.mjs` so `claude.cmd` maps to `claude-code`.

The Windows/POSIX split is keyed on the **target shell** (local vs remote), never
on the client's compile-time OS alone.

## Still open

- **T4.3 — merge the user's own hooks** into the `--settings` payload, removing
  the array-replace caveat above.
- **T4.4 — nonce hardening finalize.**
- **Cadence window tuning** — the silence threshold for `awaiting_input` is ~1s.
  Watch for agents that animate while idle; mitigated by agent-gating and the
  approval overlay correcting the one critical case.

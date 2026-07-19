# Spike: Terminal Activity — Phase-2 hook→PTY transport (T2.1 GATE)

Date: 2026-07-19 · Claude Code **v2.1.211** · POSIX (Linux) · **Decision: use the
hook-JSON `terminalSequence` output.**

Gates Phase 2 of `docs/TERMINAL_ACTIVITY_PLAN.md`. Question: how does an injected
Claude hook get a namespaced OSC signal into the PTY that demeteo's drain reads —
the naive `printf '…' >/dev/tty`, or the hook-JSON `terminalSequence` field?

## Method

All harness scripts under the session scratchpad (`spike/`). Three probes:

1. **Mechanical PoC** (`mech_poc.py`) — a sub-subprocess with **piped** stdin/stdout
   (mimicking how Claude feeds hook JSON) writing `\033]777;demeteo;…\033\\` to
   `/dev/tty`, under a PTY it shares as controlling terminal. Baseline for "does the
   POSIX mechanism work at all."
2. **Real Claude, `/dev/tty` path** — `claude -p` under `script(1)` with `--settings`
   injecting `SessionStart`/`Stop`/`SessionEnd` hooks that write to `/dev/tty` and log
   diagnostics.
3. **Real Claude, `terminalSequence` path** — interactive `claude` under a `pty.fork`,
   hooks returning `{"terminalSequence":"…"}` carrying five probe OSC forms (OSC 0
   title, OSC 9, `777;notify`, a **private `777;…`**, OSC 99), each with a unique ascii
   marker. Ran across `SessionStart` and (after driving a real turn) `Stop`.

## Results

| Probe | Result |
|---|---|
| Mechanical `/dev/tty` (shared controlling tty) | **OSC reached PTY master** — mechanism is sound |
| Real Claude hook → `/dev/tty` | **FAILED** on every event. Diagnostics: `controlling-tty: ?`, write to `/dev/tty` errored. Claude runs hooks in their own session with **no controlling terminal** (docs: *"Cannot open `/dev/tty` or send escape sequences directly"*, v2.1.139+). stdin **and** stdout are pipes — but that alone never removes `/dev/tty`; the missing *controlling terminal* is what does. |
| `terminalSequence` on **`SessionStart`** | Nothing rendered — all five forms dropped (UI not live yet). |
| `terminalSequence` on **`Stop`** | **All five forms rendered to the PTY verbatim**, including the private form: `\x1b]777;MKraw777;state=working\x07`. |
| `terminalSequence` in headless `claude -p` | Not rendered (no interactive terminal). Test-only concern — demeteo runs Claude interactively. |

## Decision & consequences

- **Transport = hook-JSON `terminalSequence`.** `/dev/tty` is eliminated.
- Arbitrary **OSC 777** private payloads pass through **unsanitized**, so the plan's
  namespaced + versioned + nonce wire format is viable and **Phase 2b's drain OSC
  scanner is unchanged** — it parses `\x1b]777;demeteo;…(BEL|ST)` out of the stream
  exactly as designed (and the same scanner handles remote via the SSH stream in
  Phase 4).
- **Bind signals to post-init events** (`Notification`, `Stop`, `PreToolUse`,
  `PostToolUse`, `UserPromptSubmit`), never `SessionStart`. §2d's map already does.
- `claude --settings '<inline JSON>'` fires injected hooks — no settings *file*
  required (the §2c per-event array **replace** caveat still applies).

## Notes / hygiene

- Terminator: Claude emitted **BEL** (`\x07`) for our sequences; the scanner must accept
  both BEL and ST (`ESC \`) per the plan.
- The spike set `theme`/`hasCompletedOnboarding` in `~/.claude.json` to skip first-run
  onboarding so interactive mode reached a session; reverted after (ephemeral env).

# Terminal Agent Activity — Experience Spec

Status: **Draft**
Owner: terminals
Companion: `docs/TERMINAL_ACTIVITY_PLAN.md` (the technical plan).
Builds on the shipped agent-*presence* work (`agentKind` / `terminal-session-agent`).

This document is the **experience**: what a developer using demeteo sees and can
rely on. The *how* — signals, transports, phases — lives in the plan.

---

## 1. The problem, in one sentence

When you're running several coding agents across terminal tabs, you can't tell
which one is thinking, which one is done and waiting for you, and which one is
**stuck asking permission** — without clicking into each tab. The one that needs
a decision is exactly the one you can't afford to miss, and it's invisible the
moment you look away.

**The glance test:** open the terminal panel, look once, and know the state of
every agent. Never click a tab just to find out it's been waiting for five
minutes.

---

## 2. The three states a developer cares about

Activity is layered **on top of presence**. Presence (already shipped) answers
*which* agent is in a tab. Activity answers *what it's doing right now*. A tab
with an agent but no activity signal yet simply shows the agent badge and no
activity mark.

| State | What it means to you | Visual language | Your move |
|---|---|---|---|
| **Working** | The agent is processing a turn or running a tool. | Violet **animated spinner** (matches `AgentBadge`). | Leave it. |
| **Waiting for you** | The turn finished; the agent is idle at its prompt. | Steady **amber dot** — "your turn". | Go type. |
| **Needs a decision** | Blocked on a permission / confirmation gate. | **Pulsing red-amber**, highest salience. Counted in the nav. Fires an OS notification when you've looked away. | Approve/deny — it's frozen until you do. |
| *(none)* | Plain shell, or an agent we can't read yet. | No activity mark (agent badge still shows). | — |

The visual weight is deliberately ordered: *needs a decision* must out-shout
*waiting*, which must out-shout *working*. The eye should land on the tab that
needs a human first.

---

## 3. Where you see it

1. **Session-list row** — an activity mark beside the agent badge. This is the
   primary glance: the whole panel's state in one column.
2. **Terminal surface header** — the same mark on the focused tab, so the state
   is present without leaving the terminal you're in.
3. **Terminals nav item** — a **count of terminals that need you** (needs a
   decision). This is the app-wide signal: a backgrounded terminal that hit an
   approval gate is visible from anywhere in demeteo, even with the panel
   closed.
4. **OS notification** — fired **only** for *needs a decision*, **only** when the
   demeteo window isn't the thing you're looking at (hidden or unfocused),
   deduped, and only when you've opted into background mode. It reuses the same
   notification behavior features already use — so it respects your existing
   permission grant and never double-notifies something already on screen.

---

## 4. What you can rely on (the quality bar)

These are promises, not aspirations. The plan is built to hold them.

- **Live.** Flips to *working* the instant output resumes, and settles to
  *waiting* within about a second of the agent going quiet. No spinner that
  keeps spinning after the agent stopped; no lag before "your turn" shows.
- **Honest — never a confident lie.** The indicator only shows a state it can
  actually source. *Needs a decision* is claimed only when there's a real
  approval prompt, never guessed from silence — a false "approve me" is worse
  than showing nothing, so we don't guess it.
- **Broad first, precise where we can.** The basic **working / waiting** read
  works for **every** agent — Claude, Codex, OpenCode, and any we add — whether
  it's local or remote, and whether you launched it from demeteo or typed it by
  hand. The high-value **needs a decision** state lights up wherever we can
  detect it precisely, and stays silent (rather than wrong) elsewhere.
- **Invisible mechanics.** It never prints stray characters into your terminal,
  never edits your agent's config files, never touches your `~/.claude`
  settings, and adds no perceptible typing or scrolling lag.
- **Zero setup.** Nothing to install or toggle. Open an agent and it works.
- **Self-healing.** It never shows a stale state after a reload or a dropped-and-
  reconnected session — activity re-derives from live signals, so a reconnected
  tab is honest immediately, not frozen on whatever it last showed.

---

## 5. What you'll see, per agent (the honesty contract)

The UI never over-promises. If a state can't be sourced for an agent, that mark
simply doesn't appear for it — you're never shown a guess dressed as a fact.

| Agent | Working / Waiting | Needs a decision |
|---|---|---|
| **Claude Code** (any launch) | ✅ always | ✅ precise |
| **Codex, OpenCode, others** | ✅ always | ✅ when its approval prompt is recognized; silent otherwise |
| **Hand-started / unknown agent** | ✅ always | ✅ when its approval prompt is recognized; silent otherwise |
| **Plain shell** | — | — |

The **working / waiting** column is universal from day one. The **needs a
decision** column fills in by phase (see the plan) — Claude first with the
highest fidelity, then every other agent via on-screen recognition.

---

## 6. Non-goals

- Not an agent dashboard, timeline, or per-tool breakdown — just the at-a-glance
  state.
- No persistence/history of activity — it's a live read, not a log.
- Windows is later (the current terminal foundation is POSIX-first).
- We do not modify, merge, or manage the user's own agent hooks/config as part
  of this (a future convenience, not a requirement).

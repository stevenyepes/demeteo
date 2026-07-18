# Demeteo Terminals View & Persistent Session Spec

> **Scope:** Turns the terminal experience into a first-class, full-page
> **Terminals view** reached from the project rail — with a vertical list of
> session tabs and a single active xterm surface — while guaranteeing that a
> terminal's underlying process (local PTY or remote SSH child) survives view
> navigation, panel collapse, and tab switching. Supersedes the always-at-
> bottom panel introduced in PR #58 (`feat: persistent multi-terminal panel`).
> Source of truth for the terminal UX and session-lifecycle work.
>
> **Status:** Items marked **[Shipped]** have landed. Items marked
> **[Planned]** are specified here but not yet implemented. Items marked
> **[Fix]** correct a defect in the PR #58 baseline and are prerequisites for
> the view work. Everything in this document is currently **[Planned]** unless
> noted otherwise.
>
> **File-line anchors** point at the PR #58 head
> (`demeteo/features/f-54b8ea3707ae7fc0`). Re-verify with `git grep` before
> implementing — line numbers drift as surrounding code changes.
>
> **Cross-refs:** [`AGENTS.md`](../AGENTS.md) §5 (Visual Design Rules), §4
> (Code Conventions), §6 (File Layout); [`ARCHITECTURE.md`](ARCHITECTURE.md)
> for transport invariants.

---

## 0. Invariants this spec must preserve

1. **A process dies only on explicit intent.** The PTY/SSH child is torn down
   *only* by: closing its tab, "kill all", or the tray `CloseAction::Cleanup`
   at app quit. View navigation, panel collapse/hide, tab switching, and
   surface unmount **must never** call `close_terminal_session`.
2. **Session ownership is decoupled from presentation.** Sessions live in the
   `TerminalPanelProvider` (and, authoritatively, in the backend session map).
   No routed view owns a session; views are pure renderers that attach/detach
   an output channel.
3. **One live xterm at a time.** Only the *active* session mounts a
   `TerminalSurface`. Inactive sessions keep running headless; their output
   accumulates in a backend scrollback buffer and repaints on next attach.
4. **Output is never silently lost or doubled.** Every attached subscriber
   receives every chunk exactly once; a newly attached surface repaints from
   scrollback without duplicating live output (see §3).
5. **Design tokens are fixed.** Colors, fonts, and spacing follow
   [`AGENTS.md`](../AGENTS.md) §5 — reuse the existing palette
   (`#08090c` / `#0c0d12` / cyan-400 / emerald-400 / ruby-400), no new tokens.

---

## 1. Goal & user stories

Users run multiple interactive coding-agent terminals (Claude, OpenCode,
Hermes, Codex, or a bare shell) across local and remote machines. They need to:

- **US-1** — Open several terminals and switch between them from a dedicated,
  full-height view (not a cramped bottom strip), using **vertical tabs**.
- **US-2** — Navigate to a feature pipeline, project home, or settings and
  back **without killing** any running agent. (tmux/Zellij-like persistence.)
- **US-3** — Launch a specific coding agent straight into a new terminal tab.
- **US-4** — Rename tabs, see per-session status at a glance, and close
  individual sessions or all of them.
- **US-5** — Have a long list of workspaces *and* terminals scroll cleanly
  without pushing other UI off-screen.

Non-goals (V1): split panes inside one tab, drag-to-reorder tabs, persisting
scrollback across an app restart (only across in-app navigation / webview
reload while the backend process lives).

---

## 2. Architecture at a glance

```
┌───────────────────────── TerminalPanelProvider (session owner) ───────────────────────┐
│  state: tabs[], activeTabId, collapsed   bindingRef: tabId→sess_id                      │
│  open() close() focus() setTitle() rename() getSessionId()                              │
└───────────────┬───────────────────────────────────────────────────────────────────────┘
                │ context (always mounted, app-wide)
     ┌──────────┴───────────┐                         backend (authoritative)
     ▼                      ▼                         ┌─────────────────────────────┐
 ProjectRail            TerminalsView  ── attach ───▶ │ ActiveSession               │
 (Terminals entry,      (vertical tabs +              │  Broadcast { channels,      │
  live count)            single active surface)       │             scrollback }    │
                            │                         │  PTY / SSH child            │
                            ▼                         └─────────────────────────────┘
                     TerminalSurface (xterm)  ◀── output broadcast + scrollback replay
```

- The provider is mounted once at the app root and never unmounts, so session
  state and the `tabId→sess_id` binding outlive any route change (**invariant
  2**). This is unchanged from PR #58.
- **What changes:** the bottom `TerminalPanelHost` is removed; rendering moves
  into a routed, full-page `TerminalsView`. Only the active tab mounts an
  xterm (**invariant 3**).

---

## 3. Backend — scrollback broadcast [Fix + Planned]

The PR #58 "seed channel + `consumeStartupReplay`" mechanism causes duplicate
output and leaves a permanent phantom subscriber (see §11 findings F1, F5).
Replace it with a **bounded per-session scrollback buffer** guarded by the same
mutex as the subscriber list, so attach-replay and live-broadcast are exactly
ordered.

**`src-tauri/src/terminal.rs`** (anchor: `ActiveSession` ~L17, `send_chunk`
~L159, `attach_terminal_session` ~L185, `start_terminal_session` ~L60):

```rust
struct Broadcast {
    channels: Vec<Channel<Vec<u8>>>,
    scrollback: VecDeque<Vec<u8>>, // whole chunks — never split an escape seq
    scrollback_bytes: usize,
}
const SCROLLBACK_MAX_BYTES: usize = 256 * 1024;
// ActiveSession.frontend_channel: Arc<Mutex<Broadcast>>
```

- **`send_chunk`** — lock once: append the chunk to `scrollback`, `pop_front`
  whole chunks until `scrollback_bytes <= SCROLLBACK_MAX_BYTES`, clone the
  channel list; unlock; `send()` to the snapshot outside the lock (prune a
  channel whose `send` fails, as today). Trimming on whole-chunk boundaries
  avoids corrupting the first repaint by cutting mid-escape-sequence.
- **`attach_terminal_session`** — lock: take a scrollback snapshot **and** push
  the new channel atomically; unlock; `send()` the concatenated scrollback to
  *only* the new channel. Any chunk arriving mid-attach serializes on the lock,
  so the new subscriber sees `scrollback → live` in order, no gap, no dup.
- **`start_terminal_session`** — seed an **empty** `Broadcast` (no permanent
  seed subscriber). Drop the `tauri_channel` parameter: output flows into
  scrollback until the surface's first `attach` replays it. Nothing is lost.

**Frontend deletions** (`TerminalPanelProvider.tsx`, `TerminalSurface.tsx`):
remove `startChannel`, `seedBytes`, `seedActive`, `deactivateSeed`,
`startupReplayRef`, `consumeStartupReplay` end-to-end; `startTerminalSession()`
loses its channel argument. This subsumes findings F1 and F5.

---

## 4. Frontend — the Terminals view [Planned]

### 4.1 Mount & persistence model

- **The view is mounted once** as an absolutely-positioned overlay over the
  main content area and toggled with CSS (`hidden` / `display:none`) based on
  `view.kind === 'terminals'` — the same "hide, don't unmount" idiom PR #58
  used for panel collapse. Returning to the view is instant; the active xterm
  is preserved across route changes.
- **Only `activeTab` renders a `<TerminalSurface>`.** Switching tabs remounts
  the surface for the new session, which attaches and repaints from backend
  scrollback (§3). Memory stays flat regardless of session count (**invariant
  3**). A hidden xterm reports size 0, so fire one `fitAddon.fit()` when the
  view becomes visible and when the active tab changes.
- *(Deferred upgrade: an LRU of 2–3 mounted-but-hidden surfaces for
  zero-repaint tab switching. Not worth the memory in V1.)*

### 4.2 Layout

```
 rail (w-60 / w-14)     TerminalsView (fills content area, hidden off-route)
┌──────────────┐  ┌────────────────┬──────────────────────────────────┐
│ search       │  │ Terminals    3 │ local · ~/app · main        ● run │  header
│ ┌──────────┐ │  │ [ + New ▾ ] 🗑 │                                  │
│ │ project  │↕│  ├────────────────┤                                  │
│ │ project  │ │  │ ● claude      ×│                                  │
│ │   …      │ │  │ ● opencode    ×│        <TerminalSurface/>         │
│ └──────────┘ │  │ ● local sh    ×│        (active session only)      │
│ ───────────  │  │       ↕        │                                  │
│ ⌷ Terminals 3│  │  (scrolls)     │                                  │
└──────────────┘  └────────────────┴──────────────────────────────────┘
```

- **Session list** — `role="tablist"`, vertical, roving `tabindex`; ↑/↓ move
  selection, Enter/click `focus(tabId)`. Active row: cyan left-accent + subtle
  `bg-white/[0.07]`. Hover reveals the close `×`. Double-click the title →
  inline rename (`maxLength 64`, matching the backend char cap).
- **Surface header** — breadcrumb `machineLabel · repoPath · branch` +
  `PhaseBadge`. The xterm owns its own scrollback; the page never scrolls.
- **Empty state** (`state.tabs.length === 0`) — centered card with
  `NewTerminalMenu` (machine picker + one-click agent buttons).
- **Responsive** — under a width threshold the session list collapses to
  `MachineDot`-only icons (same collapse idiom as the rail).
- **Motion** — subtle fade on active-tab switch; gate behind
  `prefers-reduced-motion`.

### 4.3 Keyboard

| Shortcut | Action |
|---|---|
| `Cmd/Ctrl + `` ` | Navigate to the Terminals view (or `goBack()` if already there) |
| `Cmd/Ctrl + T` | New terminal (opens `NewTerminalMenu`) |
| `Cmd/Ctrl + W` | Close the active tab |
| `↑` / `↓` | Move selection within the session list |

Wire into `src/lib/shortcuts.ts` + `src/hooks/useKeyboardShortcuts.ts`
(the `` ` `` case already exists — repoint it from `togglePanel` to navigate).

---

## 5. Component inventory (extract once, reuse everywhere) [Planned]

The PR #58 baseline duplicates button styling, the machine dot, and the rename
logic. Extract shared primitives under `src/components/ui/` (and a hook under
`src/hooks/`) per [`AGENTS.md`](../AGENTS.md) §6:

| Primitive | Responsibility | Reused by |
|---|---|---|
| `ScrollArea` | Styled scroll container: `overflow-y-auto overscroll-contain`, thin custom scrollbar, `min-h-0` flex child, `content-visibility:auto` rows | Rail workspaces list, session list |
| `RailNavItem` | One rail button (icon + label + optional count/pulse), expanded & collapsed variants | Projects, Terminals entry |
| `MachineDot` | Cyan (local) / emerald (remote) status dot + optional pulse | Session rows, rail |
| `PhaseBadge` | `connecting`/`running`/`closed`/`error` → chip; reuse existing `ui/StatusBadge` | Session rows, surface header |
| `useInlineRename` | Double-click→input, draft/commit/cancel, autofocus+select, Esc/Enter — extracted from today's `TerminalTab` | Session rows (future: project rename) |
| `SessionRow` | `MachineDot` + title (`useInlineRename`) + `PhaseBadge` + hover close; `React.memo` | `TerminalsView` |
| `NewTerminalMenu` | Dropdown of enabled agents per machine (reuses `get_agent_configs` + `AGENT_CLI` from the retiring `AgentTerminalDrawer`) → `open({ launchCommand, forceNew })` | View header, empty state |

Net-new files: `TerminalsView.tsx`, `NewTerminalMenu.tsx`, the primitives
above, plus the rail entry and route wiring. `TerminalSurface`, the provider,
and the rename/close/status logic are reused, not rewritten.

---

## 6. Navigation & rail integration [Planned]

- **Route** — add `{ kind: 'terminals' }` to the `AppView` union
  (`src/types.ts`) and to `shallowEqualView` (`src/context/NavigationContext.tsx`
  ~L20, alongside the other param-less kinds).
- **Rail entry below the workspaces** (`src/components/ProjectRail.tsx`) — turn
  the rail into a `flex-col` with a pinned search/header, the projects list
  inside a `<ScrollArea className="flex-1 min-h-0">`, and a pinned footer
  holding a `RailNavItem` "Terminals" (`TerminalSquare` icon). It reads
  `useTerminalPanel()` to show a live session count + emerald pulse when
  `state.tabs.length > 0`. Present in both the expanded and collapsed
  (`w-14`) variants.
- **Retire the bottom mount** — remove `<TerminalPanelHost />` from `AppInner`
  (`src/App.tsx` ~L535). Render `<TerminalsView />` as the CSS-toggled overlay
  instead. Repoint the TopBar toggle (`src/components/TopBar.tsx` ~L96) and the
  `` ` `` shortcut to `navigate({ kind: 'terminals' })`.

---

## 7. Agent launch flow [Fix + Planned]

PR #58 leaves `AgentTerminalDrawer` unmounted (finding F4) — nothing launches
an agent anymore; the new buttons open a bare shell. Restore it as a
first-class capability of `open()`:

- Extend `TerminalPanelOpenInput` with:
  - `launchCommand?: string` — e.g. `"claude"`, `"opencode"`.
  - `forceNew?: boolean` — bypass logical dedup so the `+` menu can stack
    multiple sessions on the same machine/repo (auto-openers keep deduping).
- In `open()`, after the session is `running`:
  `if (input.launchCommand) await writeTerminalSession(sessionId, input.launchCommand + '\r')`.
  The backend's `git checkout` bootstrap and this command are sequential
  stdin writes, so ordering is preserved.
- `NewTerminalMenu` reuses the agent-config loading and calls
  `open({ ...ctx, launchCommand: agent.binary, forceNew: true })`.
- Delete `AgentTerminalDrawer.tsx` once its logic is absorbed by
  `NewTerminalMenu`.

---

## 8. Performance requirements

1. **Single live xterm** (invariant 3) — never mount more than one
   `TerminalSurface` (plus the deferred optional LRU).
2. **No render on keystrokes** — terminal I/O flows through Tauri channels, not
   React state. `state` changes only on open/close/focus/rename/phase. Keep
   `SessionRow` `React.memo`'d so a focus change re-renders exactly two rows.
3. **Scroll lists** — `ScrollArea` uses `overscroll-contain` (no scroll-chaining
   into the surface) and `content-visibility:auto` on rows so offscreen
   workspace/session rows skip layout + paint. Virtualize (react-window) *only*
   behind a threshold (e.g. > 150 rows) so the common case pays nothing.
4. **Bounded scrollback** — `SCROLLBACK_MAX_BYTES = 256 KB` per session caps
   backend memory; trimming is O(dropped chunks).
5. **Stable context** — keep the provider `value`/callbacks memoized so the
   always-mounted view does not churn context consumers.
6. **Fit on show only** — one `fitAddon.fit()` when the hidden view becomes
   visible or the active tab changes; never on every render.

---

## 9. Scrolls (explicit)

- **Rail** — pinned search/header at top, `<ScrollArea flex-1 min-h-0>` for the
  workspace list, pinned Terminals entry in the footer. A long project list
  scrolls instead of pushing the Terminals entry off-screen (US-5).
- **Terminals view** — session list in its own `<ScrollArea>`; surface region
  is `flex-1 min-h-0` and contributes nothing to page scroll. The xterm's
  internal scrollback handles output history.
- Custom thin scrollbar styling lives once in `ScrollArea` (WebKit
  `::-webkit-scrollbar` + `scrollbar-width: thin`), consistent app-wide.

---

## 10. Data model

`src/types.ts` (already added in PR #58 — no change needed beyond the route):

```ts
interface SessionInfo { session_id: string; machine_id: string; created_at: number; title: string | null; }
interface TerminalTabDescriptor {
  sessionId: string | null; tabId: string; machineId: string; machineLabel: string;
  projectId?: string; repoPath?: string; workBranch?: string | null;
  title: string; phase: 'connecting' | 'running' | 'closed' | 'error'; createdAt: number;
}
interface TerminalPanelState { tabs: TerminalTabDescriptor[]; activeTabId: string | null; collapsed: boolean; }
```

`collapsed` becomes vestigial once the bottom panel is gone; keep it for the
webview-reload reconcile path or remove in a follow-up cleanup.

---

## 11. PR #58 defects folded in (prerequisites)

These were surfaced in review and are prerequisites for a correct view. Each is
a **[Fix]**:

| # | Defect | Resolution |
|---|---|---|
| F1 | Startup replay duplicates output produced during the attach IPC window | §3 backend scrollback (deletes the seed-channel design) |
| F2 | Rename during `connecting` never reaches the backend; the "`open()` replays" comment is false | Stash a pending title and flush via `renameTerminalSession` once `sessionId` resolves in `open()`; fix the comment |
| F3 | Concurrent `open()` can orphan a backend session (two-layer dedup disagreement) | Make `open()` the single dedup source; add open-coalescing keyed by `machineId\|repoPath\|workBranch` (skip when `forceNew`); drop the reducer's `isSameLogicalTab` branch |
| F4 | `AgentTerminalDrawer` is dead code after the migration | Absorb into `NewTerminalMenu` (§7), then delete |
| F5 | Seed channel never detached → phantom subscriber streams over IPC forever | Eliminated by §3 (no seed channel) |
| F6 | `STARTUP_RECONCILE` labels still-alive restored sessions as `phase: 'closed'` | Reconcile as `phase: 'running'` |

---

## 12. Test plan

**Rust** (`src-tauri/tests/infrastructure/terminal.rs`):
- Attach on a session with existing scrollback replays it to the new channel
  only; existing subscribers do not re-see it.
- Scrollback trims at the cap on whole-chunk boundaries.
- Two channels attached both receive live output exactly once.
- start → attach gap loses nothing; no duplication at the cutover.

**Frontend** (vitest):
- Rail Terminals entry navigates to the view and shows the live count.
- Vertical list renders all sessions; `focus` swaps the mounted surface;
  exactly one xterm is mounted at a time.
- Navigating away from and back to the view does **not** call
  `close_terminal_session` (invariant 1).
- `open({ forceNew })` yields a second tab on the same repo; a coalesced
  double-open starts exactly one session (F3).
- Reconciled tab renders as `running` (F6).
- `launchCommand` issues one `write_terminal_session` after attach (§7).
- `ScrollArea` keeps rail header/footer pinned while the workspace list
  scrolls; `overscroll-contain` prevents scroll-chaining into the surface.

---

## 13. Rollout

1. §3 backend scrollback + frontend seed-channel deletion (F1, F5).
2. F3 (`forceNew` + coalescing) and F6 — safe on the current bottom panel.
3. Extract §5 primitives; build `TerminalsView` + `NewTerminalMenu` (§4, §7,
   F4); add the route + rail entry (§6); remove `TerminalPanelHost`.
4. F2 and the keyboard/scroll polish (§4.3, §9).

Each step is independently shippable and keeps the app green
(`cargo check -p demeteo`, `npx tsc --noEmit`, `cargo test terminal`, touched
vitest suites) per [`AGENTS.md`](../AGENTS.md) §11.

---

## 14. Open questions

1. Should closing the *last* tab auto-navigate away from the Terminals view, or
   show the empty state in place? (Proposed: empty state in place.)
2. Do remote sessions need a reconnect affordance in the surface header when the
   SSH child drops, or is tab-level `error` phase enough for V1?
3. Should `collapsed` in `TerminalPanelState` be removed now or in a follow-up?

# Terminal Sessions — UX Spec

**Status:** designed, decisions settled 2026-09-11, not built. This spec replaces
the full-screen Terminals view as the place where hands-on work happens, and it
changes what "Code with Agent" does.

It describes every screen, its states and copy, and the journeys that cross them.
§11 records the decisions taken after the design review and what they changed;
the implementation plan is [TASKS_TERMINAL_SESSIONS.md](TASKS_TERMINAL_SESSIONS.md).
Where this spec says **v1**, it means the first slice in that plan (journeys
J-S1, J-S2, J-S4, J-S5); everything marked **deferred** is designed here and
scheduled there.

**Design canvas:** [Terminal Sessions Redesign](https://claude.ai/code/artifact/7ae8a102-de16-439c-9870-82686d22827b).
It is a clickable prototype with three parts:

- **Prototype page:** the live prototype, the sessions index, flow A (a gate by
  hand), flow B (continue work on a PR), and the Graph ↔ Timeline run views.
- **Research & system page:** findings, principles, the states sheet and the
  keyboard map.
- **Frames and notes:** each frame is named after the artboard file that §3 lists.
  The notes on the canvas summarise each flow.

The canvas is private to its owner until shared from its share menu. A local copy
is in `ui-mocks/terminal-sessions/`, which is gitignored like the other mock
folders: `terminal-sessions-redesign.html` is the bundled canvas, which opens in a
browser to view and export, and each `*.dc.html` is one artboard of it. Where this
spec and the canvas disagree, this spec wins. The canvas uses sample data.

Related documents:

- Terminal activity detection (working / awaiting input / awaiting approval):
  [TERMINAL_ACTIVITY.md](TERMINAL_ACTIVITY.md). This spec consumes those states
  and does not redefine them.
- Run view layout and the inspector: [UI_REDESIGN_PLAN.md](UI_REDESIGN_PLAN.md)
  §3–§4. This spec keeps those decisions and adds session signals to them.
- Global shell and the older journeys: [UX_JOURNEYS.md](UX_JOURNEYS.md). J9/J10
  there ("Open a terminal here" in the sync worktree) remain valid.
- Local/remote parity: [EXECUTION_PARITY.md](EXECUTION_PARITY.md).

---

## 1. Why this exists

### 1.1 What the code does today

| Fact | Where |
|---|---|
| **"Code with Agent" launches no agent.** It opens a plain shell in the project's *main clone* and runs `git checkout <feature branch>` there. That breaks the rule that the main checkout is never disturbed, and a failed checkout (branch already cleaned up) is silent. | `src/components/FeatureDetail/useWorktreeRouting.ts`, `src-tauri/src/terminal/transport.rs` |
| For detached (runner) runs, "Code with Agent" resolves the **laptop-side** path, while "Browse Code" goes through `remote_get_worktree`. | `useWorktreeRouting.ts` |
| A session knows its machine, folder and branch, and **nothing about a Project, Feature, Step or run**. After a webview reload, even the folder and branch are lost. | `src/types.ts` (`TerminalTabDescriptor`), `src-tauri/src/terminal/model.rs`, `src/context/TerminalPanelProvider.tsx` |
| There are no links between a terminal and a pipeline in either direction, no terminal signal on pipeline cards, and no deep link to a specific tab. | `src/components/PipelineCard.tsx`, `src/components/TerminalsView.tsx` |
| Step worktrees are temporary and deliberately hidden from the location picker. | `crates/demeteo-core/src/domain/terminal_worktree.rs` |
| Sessions live only in memory. They survive navigation and webview reloads, but not an app restart, and a remote shell dies with the app. | `TerminalPanelProvider.tsx`, `src-tauri/src/terminal/` |

### 1.2 What users need

This comes from a two-round interview with a user persona: a staff engineer
running 3–8 pipelines across 5 projects, often detached to a shared runner, who
steps in by hand on about one pipeline in three. **The persona was role-played by
an agent from a product brief.** Treat these as hypotheses to confirm with real
users, not as evidence.

- They leave Demeteo about eight times per feature, half of them during the "fix
  it by hand" phase (tmux on the runner, lazygit, VS Code, GitHub).
- They are only about 70% sure that approving a gate carries on from their manual
  commits, and they have lost uncommitted work to a retry.
- The agent starts cold: they retype the critic verdict that Demeteo already has.
- They once ran `git reset --hard` in a worktree while validate was running in it.
- Anything long-lived goes into their own tmux, because runner shells die with the
  app.
- "If opening a shell ever asks me for a feature, I'll leave."

### 1.3 Principles

1. **One session, two layouts.** The drawer on the Feature page and the focused
   session are the same pty, the same scrollback and the same agent process.
   There are never two kinds of terminal.
2. **Sessions belong to a Feature; a shell doesn't have to.** A session opened from
   a pipeline carries its Feature, the Step it was opened from, and the branch. A
   plain shell stays one keystroke away and never asks for a Feature.
3. **Brief the agent, not the human.** Context comes from the pipeline and is sent
   as the agent's first prompt. An agent never starts because a row was selected.
4. **Handing back is explicit and names the commit.** Before anything moves, the
   human sees the head SHA, the commits since the gate, and whether the working
   tree is clean. A dirty tree blocks the hand-back. Nothing is auto-committed or
   auto-pushed.
5. **Location is loud.** Runner sessions look different from local ones everywhere
   they appear. A worktree shared with a live Step says so.
6. **Let people leave.** A runner session survives the laptop closing and can be
   reattached from the user's own terminal, and no key chord inside the terminal is
   taken from the shell.

---

## 2. Model

### 2.1 Session

A **session** is a terminal tab, local or remote, optionally tied to a Feature.
It extends the existing `TerminalTabDescriptor` / `ActiveSession`. New fields are
marked ★.

| Field | Meaning |
|---|---|
| `machineId`, `machineLabel`, `repoPath`, `workBranch`, `agentKind`, `activity`, `phase` | As today. `machineLabel` is always the machine's friendly name, never its id. |
| ★ `projectId` | Always set when a session is opened from a project surface. Today it is optional and lost on reload. |
| ★ `featureId` | Set when opened from a Feature. `null` for a plain shell. |
| ★ `origin` | Where the session was opened from: `{ kind: 'gate' \| 'failed_step' \| 'published' \| 'running' \| 'none', stepExecutionId? }`. It drives the brief, the hand-back choices and the breadcrumb. `published` is deferred with J-S3 (§11). |
| ★ `worktree` | `{ kind: 'session' \| 'main' \| 'terminal' \| 'sync' \| 'step', path, attached }` (see §2.2). `attached` is whether the session worktree currently has the feature branch checked out. |
| ★ `brief` | The text sent as the first prompt, stored with the session so it can be re-read later. `null` when the session was started without a brief or is a plain shell. |
| ★ `baseSha` | The feature head when the session was opened. It is the "was …" in every commit count. |
| ★ `persistent` | `true` when the pty lives in a tmux session on the host (§2.4). |
| ★ `costUsd` | Interactive agent spend, or `null` when it can't be measured. Unknown is shown as unknown, never as $0. In v1 it is always `null` (§5.3). |

Sessions are rows in a new `terminal_sessions` table in Demeteo's own SQLite
(desktop side; the same migration lands unused on the runner, which shares the
crate). The Rust session registry stays the live source for the pty and its
scrollback; the row is what survives. A session's `tabId` **is** its persisted
id, so a deep link to a tab survives a reload. That fixes today's loss of path
and branch on reload, and it is what lets a persistent session (§2.4) be found
again after an app restart.

### 2.2 The session worktree

"Fix with agent", "Continue work" and "Shell in the feature worktree" never
touch the main clone. They create or reuse **one session worktree per Feature**
checked out on the feature branch, named on the same pattern as the sync
worktree (`crates/demeteo-core/src/paths.rs`): `<repo_dir>_wt_session_<slug>`.

- **Created on the machine where the Feature's branch lives.** For a detached run
  that is the runner. The resolution goes through the same path that "Browse
  Code" uses for remote runs (`resolve_feature_worktree`, which returns the main
  clone the session worktree is derived from). There is no choice of machine:
  the location card (§4.5) states where the branch lives, it does not offer to
  move it.
- **Recreated from the branch** if Cleanup removed it. The brief card says when
  this happens (§4.5).
- **Attached only while the pipeline is parked.** `merge_subtask` merges into
  whichever worktree has the feature branch checked out, after `git merge
  --abort` and `git reset --hard HEAD` there. A session worktree attached to the
  branch while a step runs would receive that merge and lose uncommitted work.
  Two fences, neither optional: while the Feature is `running` the session
  worktree sits at the branch tip with **detached HEAD**, and the merge and sync
  code never selects a `_wt_session_` worktree as a target. A session opened at
  a gate, at a failed step or on a completed Feature is attached.
- **Releasing the branch on hand-back.** Git refuses to check out a branch that is
  already checked out in another worktree ("already checked out at …"). So when a
  hand-back is confirmed, the session worktree detaches (`git switch --detach`)
  and the pipeline is free to advance the branch. The terminal stays open, and
  the hand-back bar reads `detached · pipeline owns the branch` until the
  worktree is attached again. **Re-attaching is explicit**: once the pipeline
  parks again, the bar offers `Re-attach`, and any new "Fix with agent" or
  "Open session" on the Feature re-attaches on open. Nothing re-attaches while
  the Feature is running.
- **Teardown.** The session worktree is removed by the Feature's branch-delete
  path (the one that removes step worktrees) and is excluded from the sync
  flow's stale-worktree scan, which today removes any `_wt_sync` entry before
  provisioning.
- **Step worktrees** (`_wt_<subtask_id>`) can be opened only read-mostly, with the
  live-step banner (§4.11). Taking over a subtask is out of scope (§9).

### 2.3 Hand-back

A hand-back is the moment the human returns control to the pipeline. What it
does depends on `origin`:

| Origin | Choices (the first is the default) | Effect |
|---|---|---|
| `gate` | Approve with my changes · Re-run critic on the new head, then return here · Redirect with a note | Records the gate decision; the executor records **the approved head SHA** on every approval (a new column on `gate_decisions`, read on the side that owns the clone), then continues from that head. "Re-run critic" **is the existing redirect** with the critic step as its target: it rewinds the critic and everything downstream of it, the gate included, and re-parks with a fresh decision. The panel says so. |
| `failed_step` | Retry `<step>` at `<sha>` (shows the command it will run) · Re-run from an earlier step (behind a disclosure, with a plain warning about what happens to commits) | Retries the step against the worktree head. "Mark passed with a reason" was considered and left out (§11). |
| `published` (deferred, J-S3) | Check & push (runs the harness gate, then pushes) · Push without the gate · Keep local for now | Pushes to the PR. The Feature stays `completed`, marked as manually changed, and never flips back to running. |

It is blocked while the tree is dirty (§4.9). It never stops the agent without
asking.

**Every pipeline-advancing action goes through it when a session exists.** The
inspector's `Approve` and `Redirect…`, the strip's `Retry`, and replay-from-step
open the same panel when the Feature has a session, so the dirty-tree block and
the SHA summary apply no matter which button was nearest. With no session they
behave as today.

### 2.4 Persistence and attach

- **Runner sessions** run inside a tmux session on the runner. **tmux is a runner
  prerequisite**: the runner install and status probes check for it and warn
  loudly when it is missing, so a runner session is always persistent. The
  runner exposes no session API; the desktop opens its usual ssh2 pty and runs
  `tmux new-session -A -s <name> -c <worktree>` in it, and the session row
  (§2.1) is the registry.
- **Naming.** `dm-<client8>-<feature8>-<origin>`, for example
  `dm-3f9a1c2e-1788403486-gate`. `client8` is a short hash of this install's
  client id (the one detached runs are already stamped with), because the
  runner is shared by several Demeteo clients under one Unix user and therefore
  one tmux server: without the client segment, another client's sessions would
  appear in the index. `origin` is the §2.1 kind (`gate`, `failed`, `running`,
  `shell`).
- **Reattach is lazy.** On launch, session rows are reconciled against `tmux ls`
  (alive, age, attached elsewhere) without opening a channel. A channel attaches
  when the tab is shown. Until then a persistent session's activity is unknown
  and it is shown as `alive <age>` (§5.1), never as waiting or working. "Copy
  attach command" gives the same session to the user's own terminal, and
  Demeteo shows `attached elsewhere` while that is so.
- **Local sessions** use tmux **when the host has it** (detected once per
  machine, `command -v tmux`), with the same naming, lifetime and reattach as
  runner sessions. Windows, and any host without tmux, keeps today's lifetime:
  the session survives navigation and reloads, not an app restart, and is marked
  `not persistent` everywhere the location shows. Nothing ever claims to
  survive when it will not. Host-side code still never assumes tmux (AGENTS.md
  §3, Cross-OS); it detects it.
- **Desktop-over-SSH hosts** that Demeteo does not provision follow the local
  rule: tmux when detected, `not persistent` otherwise.
- The pipeline's own agent runs are one-shot and can't be resumed from a terminal.
  Nothing in this UI offers `--resume` for them.

---

## 3. Map of surfaces

The "Canvas frame" column names artboards in the
[design canvas](https://claude.ai/code/artifact/7ae8a102-de16-439c-9870-82686d22827b).
Every frame except the two reference sheets is the same live prototype, opened at
a different step, and each one can be clicked through from there.

| # | Surface | Canvas frame | Replaces or extends |
|---|---|---|---|
| 4.1 | Project home: pipeline cards and rail | `Main.dc.html` (start `pipelines`) | Extends `PipelineCard`, `ProjectRail` |
| 4.2 | Sessions index (rail popover) | `SessionsIndex.dc.html` | **Replaces** `TerminalsView` as the list of sessions |
| 4.3 | Feature page: header and status strip | `GateNeedsYou.dc.html` | Extends `FeatureHeader`; the strip generalises `GateStrip` |
| 4.4 | Run view: Graph ↔ Timeline and the inspector | `RunView`, `RunTimeline`, `RunWholeGraph` | Extends `RunViewToggle`, `WorkflowNode`, `StepCard`, `InspectorColumn` |
| 4.5 | Brief card (modal) | `BriefAgent`, `ContinueBrief` | New |
| 4.6 | Session drawer | `SessionDrawer` | New. Hosts `TerminalSurface` |
| 4.7 | Focused session | `SessionFocus`, `ContinueSession` | New. The same session as 4.6 |
| 4.8 | Attach popover | reachable from 4.6 / 4.7 | New |
| 4.9 | Hand-back panel | `HandBack`, `PushToPr` | New |
| 4.10 | After hand-back | `CarriedOn` | Extends 4.3 / 4.4 |
| 4.11 | Live-step banner | `Signals.dc.html` | New |
| — | Findings and principles, states sheet | `Findings.dc.html`, `Signals.dc.html` | Reference only |

The full-screen `TerminalsView` is retired as a place to work. Its session list
becomes the index (4.2). A session opened from anywhere lands in the drawer of
its Feature, or, for a plain shell, in a Feature-less drawer on Project home
(§4.1).

---

## 4. Views

Tokens are the ones in `src/App.css`, and colour semantics follow AGENTS.md §4.
In this spec: **amber** means a human is needed · **emerald** means healthy or
done · **cyan** means interactive, terminal or running · **violet** means an
active connection (and here, specifically, *not your machine*) · **ruby** means an
error. Chips are the `Chip` primitive (uppercase mono, `TONE_CHIP`). Cards are
`glass-panel`. Icons are lucide.

### 4.1 Project home: pipeline cards and rail

**Purpose.** Show at a glance which Feature has a session and whether that session
needs you.

**Pipeline card.** `PipelineCard` is unchanged except for one addition to the
context tier: a **session pill**, after the tokens and preceded by a 1 px divider.
It appears only when the Feature has at least one open session. It is a small
`Chip` with the `TerminalSquare` icon and a label of the form
`<agent|shell> · <state>`, toned by state:

| Session state | Pill | Tone |
|---|---|---|
| Agent waiting for input or approval | `claude · waiting for you` | amber |
| Agent exited, work uncommitted | `claude · finished · 2 uncommitted` | amber |
| Exited with an error | `claude · exited · error` | ruby |
| Agent working | `claude · working` (pulsing dot) | emerald |
| Detached, alive on the runner | `runner · npm run dev · alive 2d` | violet |
| Idle shell | `shell · idle` | slate |

When a Feature has several sessions, the pill shows the loudest state. Its
tooltip lists them all.

**Rail.** The `Terminals` entry keeps its place in the rail footer, but its count
changes meaning: it becomes **sessions waiting on you** (amber pill,
`N waiting`), counting awaiting-input, awaiting-approval and finished-with-dirty-
tree states. It no longer counts open tabs; "7 terminals" means nothing, while "2
waiting" is actionable. Clicking it opens the index (4.2), not a full-screen view.
The hint under it reads `⌘1-5 jump · ⌘K palette · ⌘\` sessions`.

**Start session** (Project home header) keeps its promise that a plain shell is
one click away with no Feature and no brief. The copy beside it is: *"A plain
shell stays one click away — no feature, no brief."* The primary click opens a
shell at the **last location used for this project** (machine and folder), the
main clone on first use. The canvas's `Session location` select goes; its
picker moves behind the button's caret, which lists: the location picker
(today's machine + folder chooser), and one entry per harness in the agent
registry (`Agent: claude-code`, …) — the same list every launcher uses, so the
two launchers stop disagreeing. An agent started here has no Feature, no brief
and no deny list, exactly as today. The session opens in a drawer at the bottom
of Project home, with the same tab bar as 4.6 and no hand-back bar.

### 4.2 Sessions index (rail popover)

**Purpose.** Find any session and jump to it at its pipeline.

**Placement.** A `glass-panel` popover, 500 px wide, anchored to the rail's
Terminals entry, opening upwards. Esc or a click outside closes it.

**Anatomy.**

- **Header:** `Sessions` (Outfit 15/600) · amber chip `N waiting on you` · a
  ghost button `+ Shell ⌘⇧\`` · a close button.
- **Groups**, in this order:
  1. One group per **Project**, in rail order. Inside it, each row is a Feature
     session, plus one **`No feature`** row per plain shell.
  2. **`Still alive on <runner>`**: detached runner sessions whose Feature is done
     or whose pty isn't attached. Each row shows its age and has an `End` button.
- **Row** (7 px vertical padding): a state dot (the colours of §5.1) · the Feature
  title (12 px, truncated) with a second line of
  `<agent · purpose> · <other tabs>` (mono 10 px, truncated) · a location chip
  (`runner-01` violet / `local` slate) · the state text (mono 10 px,
  right-aligned, 92 px). A row whose worktree is shared with a live Step
  appends `· step running here` in amber.
- **Footer:** `Only your sessions — runner-01 is shared` · `↑↓ move · ↵ open at its
  pipeline`.

**Behaviour.**

- Enter or a click opens the Feature page with the drawer showing that tab. For a
  `No feature` row it opens Project home with the drawer.
- Only the current user's sessions are listed. The runner is shared, and other
  people's tmux sessions never appear.
- `End` on a runner row kills its tmux session. It asks for confirmation when the
  worktree is dirty.

### 4.3 Feature page: header and status strip

**Header** (`FeatureHeader`, padding 16 px 24 px). Everything stays the same except
the first action, which becomes a **contextual split button** (cyan, `Terminal`
icon):

| Feature state | Primary label | Primary action |
|---|---|---|
| Waiting at a gate | `Fix with agent` | Opens the brief card (4.5) with `origin: gate` |
| A step failed | `Fix with agent` | Brief card with `origin: failed_step`. For an environment-class failure the primary is `Open shell` and the strip shows the reproduce line, because there is nothing to fix in the code. |
| Completed or published | `Open shell` (v1) · `Continue work` (deferred, J-S3) | v1: a shell in the session worktree, attached to the branch, no brief and no hand-back bar. Later: brief card with `origin: published`. |
| Running | `Open shell` | Opens a shell in the session worktree at the branch tip, detached HEAD (§2.2) |
| A session already exists | `Open session` | Opens the drawer on that session |

`Fix with agent` appears only for harnesses whose capabilities declare an
interactive launch (§4.5). For the others the primary is `Open shell` and the
brief is still drafted and copyable from the drawer, so nothing pretends.

The **caret menu** (330 px, `glass-panel`) lists:

- `<primary>`: *Briefed agent in a session worktree* `↵`
- `Shell in the feature worktree`: *No brief, no agent* `⌘⇧\``
- `Shell on <runner>`: *At the Demeteo root — disk, logs, deps* (detached runs only)
- a divider
- `Copy attach command`: *Continue in your own terminal*
- `Open in VS Code`: *Remote-SSH to <runner>* on a detached run, or the local folder

**Status strip.** A single row directly under the header, before the run view. It
generalises today's `GateStrip` (`rounded-xl border px-4 py-2.5`, toned by what it
says):

| Variant | When | Content |
|---|---|---|
| **Gate waiting** (amber) | An open gate | Pulsing dot · `ShieldAlert` · `1 GATE NEEDS YOU` · mono step name (`Approve Merge / Publish`) · muted reason (`· Critic Review asked for rework`) · right: `Fix with agent` (cyan, small) · `Decide Gate →` (solid amber, as today). `Decide Gate` selects the gate step in the run view. |
| **Gate approved** (emerald) | Just after a hand-back at a gate | `✓ Gate approved with your changes` · mono `handed back at c4d5e6f · +4 commits · finalize squashes them into one` · right: cyan chip `Finalize running` |
| **PR** (emerald or cyan) | A published Feature | `PR #163 · OPEN` · a state line · right: one action. Before a session: `Review asked for changes — 3 comments` + `Continue work`. With unpushed commits: `2 commits not on the PR yet` + `Push to PR…`. After pushing: `Pushed 2 commits · reviewers notified` + chip `checks passed · pushed`. |
| **Step failed** (ruby) | A failed step | `✕ <step> failed` · error class · right: `Fix with agent` · `Retry`. (Described here, not mocked.) |

Only one strip shows at a time, following the precedence gate → failed → PR.

**Collapsed session bar** (38 px, at the bottom of the Feature page). It is shown
when the Feature has sessions and the drawer is hidden:
`TerminalSquare` · `N sessions on this feature` · `<tab> · <state> · <tab> · <state>`
(mono 11 px) · location chip · `⌘\`` · a chevron. A click or `⌘\`` opens the
drawer.

### 4.4 Run view: Graph ↔ Timeline and the inspector

This keeps UI_REDESIGN_PLAN §3.1: a run surface on the left and the one step
inspector on the right, side by side (`RunPanes`), in the order status strip →
chrome row → panes.

**Chrome row.** `RunViewToggle` (`SegmentedControl` md: `Graph` with `Network`,
`Timeline` with `List`; the selected segment uses `TONE_CHIP.cyan`) · a caption in
mono 11 px, `Selected · <step name>` · on the right, either the
`Comfortable | Compact` `DensityToggle` (**Timeline only**, as today) or, in
Graph, a run summary in mono 11 px
(`10 steps · 8 done · 1 needs you · 1 waiting`).

**Switching views.**

1. **The toggle picks a view, never a source.** Both views show the same steps,
   the same `run_events` overlay and the **same selection**.
2. **Only the left pane changes.** The status strip, the inspector (including its
   open tab) and the collapsed session bar stay where they are.
3. On entering Timeline, the list **scrolls the selected card into view**
   (smooth scroll, about 70 px above it).
4. On entering Graph, the viewport **centres on the selected node**, at the zoom
   last used for this Feature.
5. The incoming surface fades in (opacity 0→1 with a 4 px rise, 220 ms). Nothing
   else animates, which keeps the rule that only opacity animates in the webview.
6. The choice persists per Feature, in the existing UI prefs.

**Graph.** This is the existing run-mode `WorkflowCanvas` with `WorkflowNode`
cards: the kind icon box (`TONE_CHIP` by kind), title, status label, the
`AssignmentChips`, and cost and duration once completed. Additions:

- **It opens on what needs you, at 100%.** On first open the viewport frames the
  node that needs you (an open gate, a failed step, otherwise the latest active or
  finished node) with one step of context either side. It does not fit every node,
  because ten nodes fitted into the column come out unreadably small. A segmented
  control at the top left switches between `◎ Needs you` (or `Latest` when nothing
  needs you) and `⤢ Whole run`. `Whole run` animates to fit (transform, 500 ms)
  and is the only way the viewport zooms out automatically.
- **Retry loops are drawn.** An `on_failure` edge is a dashed violet curve on the
  node's side, labelled with a small mono chip (`on failure → Decompose`). Loops
  are dimmed until taken. A taken loop is drawn solid and its target shows the
  cycle count.
- **Session chip on the node.** The node a session was opened from shows a third
  chip row: `>_ claude · waiting for you` (amber while waiting, slate when idle).
- **Chrome:** `Auto-layout` at the top right (as today), the `CanvasZoomControls`
  pill at the bottom left, and a minimap at the bottom right (112 × 150) with the
  viewport outlined in cyan.
- Edges into a waiting gate are amber, edges into a pending step are dashed slate,
  and edges into a running step are cyan.

**Timeline.** This is the existing `StepTimeline` / `StepCard`: a spine, a number
circle and a status-tinted card, with retry (`↻ 1x`) and assignment chips, and
metrics on the right. A waiting gate keeps its amber ring and its
`PIPELINE PAUSED. AWAITING MANUAL REVIEW.` `Decide Gate →` row. The one addition
is the **session pill** in the title row, with the same text and tone as on the
node.

**Inspector** (`InspectorColumn` → `NodePanel`: `Step | Sync`, then Overview · Live ·
Output · Actions). It is 420 px wide, as today. Header: kind icon box · the step
name in Outfit 16/700 uppercase · a status chip and the kind in mono · close. The
Overview tab opens with three tiles (Attempts · Total cost · Duration), followed by
**one block chosen by what the selected step is**:

| Selected step | Block |
|---|---|
| A **waiting gate** | `WHAT YOU'RE DECIDING`: one sentence on what approving does (*"If you approve, Summarize & open PR pushes the branch and opens the PR. This gate is marked dangerous."*) · the upstream verdict with a chip (`CRITIC REVIEW SAID` `rework · 2`) and its findings as a numbered list · buttons `Approve` (emerald), `Redirect…` (ghost) and `Fix with agent` / `Open session` (cyan) · and, when a session exists, a **session row**: `claude · critic fix`, state text `waiting for you · +3 since gate · 2 modified · 1 untracked`, a runner chip and `⌘\``. Clicking the row opens the drawer. |
| The **finalize step of a published Feature** | `REVIEW ON PR #163` with a count chip · one card per review comment (`file:line`, then the text) · `Continue work` / `View PR` while no session exists. |
| A **running step** | Live tail (a mono well), newest line with a blinking cursor. |
| **Anything else** | `ATTEMPT HISTORY`: one row per attempt, `#n`, a status dot, the title (e.g. `failed · environment`) and a mono meta line. For a sequence step it is `SUBTASKS · MERGED INTO THE FEATURE BRANCH` instead. Then `ARTIFACT` with a link to the step's artifact path. |

**While the drawer is open** (4.6), the chrome row and the inspector hide. The
run surface becomes a band about 200 px tall above the drawer, centred on the node
that needs you, with no canvas chrome, so the gate stays visible above the
terminal. Hiding the drawer brings the full run view back exactly as it was left.

### 4.5 Brief card (modal)

**Purpose.** Give the agent the pipeline's context before it starts, editable,
for the cost of one keystroke.

**Frame.** A 680 px `glass-panel` over a 62% black scrim. It is opened by the
header primary, the strip's `Fix with agent`, the inspector's `Fix with agent`, or
the caret menu.

**Anatomy.**

1. **Title row:** a violet `Sparkles` box · the title (`Fix with agent` /
   `Continue work`) · a subtitle naming the origin
   (`Approve Merge / Publish gate · <feature>`, or `PR #163 · <feature>`) · close.
2. **Location card** (nested card): a `Server` icon · `<runner> · where this run
   lives` · mono `new session worktree on <branch>` · a note, either *"A worktree of
   its own, off the feature branch — the main checkout is never switched."* or
   *"Cleanup removed the old worktree — recreating it from the branch."* · on the
   right, the location chip (`runner-01 · tmux` violet, `local` slate, or
   `local · not persistent`). There is no machine toggle: the branch lives in one
   place (§2.2). The *"Branch is 3 behind master — bring it up to date before
   the agent starts"* row is **deferred with J-S3**, and when it arrives it is a
   **merge** of the base into the branch, never a rebase: a rebase rewrites the
   SHAs a published PR already carries and forces a force-push. It is never
   offered for `gate` or `failed_step`, where finalize squashes anyway.
3. **Agent row:** three compact selects: harness (`claude-code`), model
   (`sonnet-5`), effort (`effort · high`). Then the muted text *"project default ·
   last used here"*. The defaults come from the project, and the last choice is
   remembered per project in the UI prefs store (it is UI memory, not run
   shape, so it does not go on `ProjectSettings`). The harness list holds only
   harnesses that declare an interactive launch.
4. **CONTEXT** with a source chip:
   - `gate`: *"from the critic verdict"*. The findings as an editable text field.
   - `failed_step`: *"from the failed step"*. The failing command, then the last 20
     lines of its output, in a mono well.
   - `published`: *"PR review comments · pick what the agent handles"*. One
     checkbox row per comment (`@author · file:line`, then the text). Unticked rows
     are dimmed, and the agent is told which ones the human will handle.
5. **GOAL:** one drafted, editable sentence.
6. **DON'T:** an empty input with the placeholder *"Anything the agent must leave
   alone — e.g. don't rewrite the test fixtures"*.
7. **Footer:** *"Sent as the agent's first prompt and kept with the session.
   Nothing is written into the worktree."* · `Start without brief Esc` (ghost) ·
   `Start agent ↵` (solid cyan).

**Rules.**

- The brief is **never written into the worktree or the harness's config**
  (AGENTS.md §2). It is passed per invocation, as an argument of the launch line
  typed into the pty. Each harness declares how in a new
  `AgentCapabilities.interactive_launch` (the prompt, model and effort
  arguments, the deny-list argument or env, and the quit key sequence); the
  exact per-harness mechanism belongs in
  [AGENT_INTEGRATION.md](../AGENT_INTEGRATION.md). claude-code, codex and
  opencode take an initial prompt as an argument. **A harness that declares no
  interactive launch never gets "Fix with agent"**: typing a brief blind into a
  TUI after a guessed delay was rejected as fragile. It gets a shell, and the
  brief stays copyable.
- **Permissions.** An interactive session has a human at the keyboard, so it is
  not an unattended run: the compiled `PermissionProfile`'s **deny** list is
  enforced per invocation (an argument for claude-code and codex, the
  `OPENCODE_PERMISSION` env for opencode, on the pty that launches the agent),
  and everything else is left to the harness's own interactive asking. A shell
  tab in the same worktree carries nothing; an agent typed there by hand runs
  with harness defaults, as today. AGENTS.md §2's "allow/deny, never ask"
  therefore reads: allow/deny for unattended runs, deny plus the harness's own
  prompts for interactive ones. Changing spawn logic is a §6 gate item; this
  design was approved on 2026-09-11 (§11).
- Enter starts the agent, and Esc starts it with no brief. Esc never cancels
  silently; the close button cancels.

### 4.6 Session drawer (layout 1)

**Purpose.** Quick, in-context work under the pipeline, as with VS Code's
integrated terminal.

**Placement.** At the bottom of the Feature page, 430 px tall by default,
resizable from its top edge, and remembered per window size. It can go to full
height, which is the focused layout (4.7).

**Anatomy, top to bottom:**

1. **Hand-back bar** (44 px, `bg-panel`): `GitBranch` in cyan · mono branch name ·
   `head a1b2c3d` · cyan chip `+3 since gate` · a working-tree chip (amber
   `2 modified · 1 untracked`, or emerald `clean`) · spacer · a muted status line
   (*"The pipeline is waiting at Approve Merge / Publish"*,
   *"Nothing reaches the PR until you push"*) · emerald `Hand back…` or
   `Push to PR…`. The bar is absent for plain shells.
2. **Tab bar** (36 px, `#0c0d12`): one tab per session on this Feature. Each tab
   shows a kind icon (violet `Sparkles` for an agent, cyan `>_` for a shell),
   mono 11 px text such as `claude · critic fix`, and an activity dot (amber for
   waiting, emerald pulsing for working, none for idle). The active tab has a cyan
   2 px bottom border. Next comes `+`, which opens a menu with `Shell here ⌘⇧\``,
   `Another agent here`, `Run the harness gate (npm run checks:code)` and
   `Dev server (<project dev command>)`. At the far right: a location chip
   (`runner-01 · tmux` violet, or `local` slate) · copy attach (4.8) · Open in
   VS Code · focus `⌘↵` · hide `⌘\``.
3. **Terminal:** `TerminalSurface`. It is on `bg-app`, with 12 × 16 px padding and
   the xterm theme as today. **A runner session gets a violet frame** (a 2 px
   inset top edge in `#8b5cf6` and a 1 px ring at 22% alpha). A local session has
   no frame.

### 4.7 Focused session (layout 2)

**Purpose.** A long fix (10–40 minutes) where the pipeline doesn't matter until
it's done. It is the same session as 4.6, reached with `⌘↵` or the maximise icon.

**Anatomy.**

1. **Header** (52 px): `← Pipeline` (ghost; returns to the drawer layout with the
   run view restored) · a breadcrumb in mono 12 px:
   `<Project> › <Feature> › <origin step, amber> › <tab, white>`. Each segment
   except the last navigates. Then a spacer · location chip · copy attach · VS Code
   · restore `⌘↵`.
2. **Tab bar:** as in 4.6.
3. **Body:** the terminal (flex) and a **context rail** (360 px, `#0b0c10`, left
   border). The rail has these sections, separated by 1 px rules:
   - `WHY YOU'RE HERE`: the origin in a heading row (`Approve Merge / Publish` plus
     an amber `waiting 14h` chip, or `PR #163 review` plus `2 of 3 picked`) · the
     findings, or the picked comments with ticks and unpicked ones dimmed · a mono
     link `Brief sent to claude · view`.
   - `BRANCH`: the branch · `head <sha> · was <base> when the gate opened` · the
     commits since the base (mono 11 px, SHA in amber) · `WORKING TREE` with a
     chip, then the files (`M` in amber, `??` in slate).
   - `PIPELINE`: one 6 px bar per step, coloured by status (the gate glows amber),
     with a mono caption (`finalize waits on this gate`).
   - A **sticky footer:** `this session` · `$1.84 · in feature cost` (or
     `cost unknown`), then a full-width solid emerald `Hand back…` /
     `Push to PR…`.

The rail shows no activity log, no per-turn token counts and no DAG. Those belong
on the Feature page.

### 4.8 Attach popover

A 470 px `glass-panel`, anchored to the copy icon.

- **`SAME SESSION — IT LIVES IN TMUX ON THE RUNNER`**: a mono well containing
  `ssh <runner> -t 'tmux attach -t <tmux-name>'`, and an emerald `copied` chip
  once copied.
- **`JUST THE DIRECTORY`**: `ssh <runner> -t 'cd <session worktree> && exec $SHELL -l'`,
  or `cd <path>` for a local session.
- A footnote: *"Closing Demeteo or the laptop leaves this session running. The
  pipeline's own agent runs are one-shot and never resumable from here."*
- For a local or non-persistent session, the first section is replaced by the
  single line *"This session lives in Demeteo — closing the app ends it."*

### 4.9 Hand-back panel (modal)

**Purpose.** Show exactly what is about to happen, name the commit, and block on
unsaved work.

**Frame.** A 640 px `glass-panel` over the scrim, with an emerald icon box
(`CornerDownLeft`). Title: `Hand back to the pipeline`, or `Push to PR #163`.
Subtitle: the origin and the Feature.

**Body.**

1. **Summary well** (mono 12/20, `bg-well`):
   ```
   branch demeteo/features/f-1788403486284
   head   a1b2c3d (was 9f8e7d6 when the gate opened)
   since  +3 since gate · 2 modified · 1 untracked
   agent  claude is idle at its prompt — safe to hand back
   ```
   If the agent is working, the agent line turns amber and offers
   `Stop the agent` / `Wait`. Neither the hand-back nor the push is enabled until
   one is chosen. `Stop the agent` sends the harness's own quit sequence into
   the pty (from `interactive_launch`, §4.5) and waits for the exit signal; if
   that does not arrive within a short timeout it offers a hard kill (the
   foreground process locally, `tmux kill-session` for a persistent session).
   The same path on every transport.
2. **Uncommitted work block** (only when dirty; nested card with an amber
   border). Heading: *"Uncommitted work — commit or stash before the pipeline
   moves"*. One checkbox row per file, with its status letter and path.
   **Untracked files start unticked** and are labelled `untracked · left out`.
   Then a commit message input, pre-drafted from a **template**, not an agent:
   `<type>(<scope>): <summary>` with `fix` as the type for `gate` and
   `failed_step`, the scope derived from the touched paths, and the result run
   through the target repo's commitlint when it has one. Then `Stash` and
   `Commit N files`. Footnote: *"Untracked files are never included unless you
   tick them."*
3. **Choices** (radio cards; the selected card has an emerald border and tint):
   the list for the origin in §2.3. Each card has a title, one line of detail,
   and a mono hint on the right (`default`, `≈ $0.40`, `checks:code → push`).
4. **Footnote**, which depends on the origin:
   - gate: *"The session worktree releases the branch so the pipeline can move it.
     Your terminal stays open, and the gate records a1b2c3d as the commit you
     approved."*
   - published: *"No force-push needed: nothing was rewritten. The feature stays
     Completed, marked as manually changed. It is not flipped back to running."*
   - When a squash rewrote history, the push button reads `Force-push to PR`
     and needs a second confirmation.
5. **Footer:** `Keep working` (ghost) · the primary button (solid emerald, labelled
   by the choice: `Approve with my changes` / `Re-run critic` / `Redirect…` /
   `Check & push` / `Push to PR #163` / `Keep local`). **It is disabled while the
   tree is dirty.**

The approve choice's detail says what finalize does with the manual commits
(*"Finalize squashes the branch into one commit, so your wip commits never reach
the PR."*), so nobody has to leave to squash in lazygit.

### 4.10 After hand-back

- The panel closes, the session goes back to the collapsed bar (`claude · critic
  fix · idle`), and the terminal stays alive.
- The status strip flips to **Gate approved** or **PR pushed** (4.3).
- The run view shows the gate as completed, with `approved with your changes` and
  the SHA in its attempt history, and the next step as running (cyan). With
  nothing else selected, the inspector moves to the running step and shows its
  live tail.
- A toast (400 px, emerald border, bottom right, dismissible) reads
  `Handed back at c4d5e6f` · *"merge gate approved with your changes · finalize
  running · session kept open"*.
- The Activity feed gets `Handed back · c4d5e6f · approved with manual changes ·
  finalize started`.
- The pipeline card: the amber ring goes, the status becomes
  `Running · finalize`, and the session pill becomes `claude · idle`.

### 4.11 Live-step banner

When a session's worktree is being used by a Step that is running (for example a
shell opened into a subtask's worktree), an amber band sits across the top of the
terminal:

> ⚠ **implement · subtask 2** is running in this worktree. Anything you type can
> change what it sees. `[Type anyway]` `[Open a separate worktree]`

Keystrokes are **held** until one of the two is chosen. The banner clears itself
when the Step ends. The same condition appends `· step running here` to the row in
the index (4.2).

---

## 5. Signals

### 5.1 Session states

| State | Source | Colour | Card pill | Rail count | Index | OS notification |
|---|---|---|---|---|---|---|
| Waiting for input or approval | `activity` (TERMINAL_ACTIVITY.md) | amber | yes | yes | yes | yes |
| Finished, tree dirty | agent exited and `git status` is non-empty | amber | yes | yes | yes | once |
| Exited with an error | agent exit code ≠ 0 | ruby | yes | no | yes | no |

"Agent exited" and its exit code come from the launch line itself: the line
typed into the pty ends with a `printf` of the same OSC 777 sequence the
activity scanner already parses, carrying `state=exit` and `$?`. That works for
every harness, local or remote, hooks or not; the claude-code `SessionEnd` hook
remains a second source. Today the frontend collapses `exit` to "no signal" and
no exit code exists; both change.
| Working | `activity = working` | emerald (pulsing) | yes (quiet) | no | yes | no |
| Alive on the runner, detached | tmux alive, no attached surface | violet (with age) | yes | no | "Still alive" group | no |
| Idle shell | none of the above | slate | yes (dim) | no | yes | no |

### 5.2 Location

- A **runner** session is shown in violet everywhere: the tab-bar chip
  (`runner-01 · tmux`), the terminal frame, the index chip and the breadcrumb
  chip. It is deliberately not ruby: this is not an error, it is just not your
  machine.
- A **local** session gets a slate `local` chip and no frame.
- The host name is always the machine's friendly name.

### 5.3 Cost

An interactive session's spend counts towards its Feature's cost everywhere a
Feature's cost is shown (header, card, fleet total). When a harness can't report
interactive spend, the context rail shows `cost unknown` and the Feature cost gets
a `+ unmeasured session` tooltip. It never shows $0.

**In v1 no harness reports it**, so every session shows `cost unknown` and every
Feature with a session carries the tooltip. Measuring claude-code by reading its
transcript under `~/.claude` was rejected: it is a dependency on harness-private
files that nothing versions. Feature cost today is summed only from step rows;
a session cost will need its own source when a harness can report it.

---

## 6. Keyboard and focus

"Primary" means ⌘ on macOS and Ctrl on Linux and Windows, as `src/lib/shortcuts.ts`
uses it.

| Action | macOS | Linux / Windows | Note |
|---|---|---|---|
| Show or hide this Feature's session drawer | ⌘\` | Ctrl+\` | Repurposes today's "open Terminals view" chord |
| New shell here | ⌘⇧\` | Ctrl+Shift+\` | The existing `NEW_TERMINAL_CHORDS`. **Context-sensitive:** on a Feature page it is a shell in the session worktree, tied to the Feature, no brief (the caret menu's `Shell in the feature worktree`); anywhere else it is a Feature-less plain shell at the project's last-used location |
| Switch the session between drawer and focused | ⌘↵ | Ctrl+Shift+Enter | |
| Go from a session to its pipeline, and back | ⌘J | Ctrl+Shift+J | |
| Hand back / push | ⌘. | Ctrl+Shift+. | Opens the panel; never confirms by itself. Today ⌘. and ⌘⇧F are aliases of the palette; both aliases go, the palette keeps ⌘K and ⌘P |
| Next / previous session tab | ⌘⇧] / ⌘⇧[ | Ctrl+Shift+PgDn / PgUp | |
| Palette | ⌘K | Ctrl+K **only when the xterm is not focused** | |

**The xterm keeps the keyboard.** While a terminal has focus on Linux or Windows,
Demeteo claims only Ctrl+Shift chords and Ctrl+\`. Ctrl-a (the tmux prefix),
Ctrl-w, Ctrl-r, Ctrl-o, Ctrl-k and Esc always reach the shell, with no delay on
Esc. On macOS, ⌘ chords are safe because terminals don't use them. **Confirmed
bug, fixed in v1:** the global shortcut handler is a bare `window` keydown
listener with no editable-target guard on its modifier path, so today Ctrl+K
(and Ctrl+B, Ctrl+W, Ctrl+G, Ctrl+T, Ctrl+N, Ctrl+1..9) are stolen from a
focused shell.

Focus rules:

- Opening a session focuses its terminal.
- Closing the brief or the hand-back panel returns focus to the terminal it came
  from.
- `Decide Gate` in the strip moves focus to the inspector's decision block.

---

## 7. User journeys

Each journey lists its trigger, its steps as the user experiences them, the
outcome, and the branches off the main path. "Maya" is the persona from §1.2.

### J-S1 Fix a gate by hand

*Canvas frames: `GateNeedsYou` → `BriefAgent` → `SessionDrawer` → `SessionFocus` →
`HandBack` → `CarriedOn`.*

**Trigger.** A detached run on the runner stops at `Approve Merge / Publish`. The
critic has asked for rework with two findings. The pipeline card is ringed amber
and the rail shows `1 waiting`.

1. Maya clicks the card. The Feature page opens with the amber **gate strip**. The
   **graph** is framed on the gate, with Validate and Critic Review above it and
   Summarize below. The **inspector** already shows `WHAT YOU'RE DECIDING` with
   the critic's findings.
2. The fix doesn't fit into a one-line redirect, so she presses `Fix with agent`
   (from the strip, the inspector or the header, whichever is nearest). The
   **brief card** opens with the location already set (runner-01, a new session
   worktree on the feature branch), the agent set (claude-code · sonnet-5 · high),
   and the findings as context. She types *"don't rewrite the ticket fixtures"*
   into DON'T and presses Enter.
3. The **drawer** opens under the gate strip. The graph shrinks to a band showing
   the gate node, which now carries `>_ claude · waiting for you`. Claude starts
   with the brief as its first prompt. The hand-back bar reads
   `head a1b2c3d · +3 since gate · 2 modified · 1 untracked`.
4. The fix takes a while, so she presses `⌘↵`. The **focused session** fills the
   column, and the context rail shows why she's here, the branch and the
   pipeline. She opens the `shell` tab, checks `git log` and `git status`, and
   runs the tests.
5. She presses `Hand back…`. The **panel** shows the SHA summary and blocks on the
   dirty tree. `scratch.md` starts unticked. She commits the two files with the
   drafted message. The head becomes `c4d5e6f` and the tree is clean. `Approve
   with my changes` is the default, and she presses it.
6. The panel closes. The strip flips to **Gate approved**. The graph shows the gate
   completed and finalize running, and the inspector follows it to the live tail.
   A toast confirms the SHA. The session stays open and idle in the collapsed bar.

**Outcome.** The gate decision records `c4d5e6f`. Finalize squashes the manual
commits and opens the PR. Maya never left Demeteo and was never unsure what the
pipeline would carry on from.

**Branches.**

- *She wants a second opinion.* Choosing `Re-run critic on the new head, then
  return here` re-runs the critic against `c4d5e6f` and parks at the same gate
  again (see §10).
- *The agent is still working when she presses Hand back.* The agent line in the
  panel turns amber, and nothing is enabled until she picks `Stop the agent` or
  `Wait`.
- *The one-line fix really is one line.* `Redirect…` from the inspector, with no
  session at all.
- *She closes the laptop mid-fix.* The runner session lives on in tmux. The next
  day, the rail shows `1 waiting`; the index row takes her straight back into the
  drawer with the scrollback intact.

### J-S2 Fix a failed step

**Trigger.** Validate fails on a local run. The strip turns ruby: `✕ Validate,
Test & Security Scan failed · regression`. (If environment triage says
*environment*, the strip shows the reproduce line and `Retry` instead, because
there is nothing to fix in the code.)

1. `Fix with agent` opens the brief with `origin: failed_step`. The context is
   `npm run checks:code` and the last 20 lines of its output.
2. The agent fixes the code in the session worktree, and Maya re-runs the failing
   command from the `+` menu (`Run the harness gate`) in a second tab.
3. `Hand back…` offers `Retry Validate at <sha>` (default, showing the command it
   will run). `Re-run from an earlier step` sits behind a disclosure with the
   warning *"Steps before Validate re-run from their own worktrees; commits made
   here stay on the branch."*

**Outcome.** Validate retries against the committed head. The session tab stays
open in case it fails again.

### J-S3 Continue work on a published Feature (deferred)

*Canvas frames: `ContinueBrief` → `ContinueSession` → `PushToPr`.*

Deferred out of v1 (§11): reading review comments back is greenfield (the MR
port publishes, refreshes state and posts comments outbound; it reads no
threads), there is no "manually changed" flag on a Feature, and the push and
force-push paths are their own surface. In v1 a completed Feature offers `Open
shell` (§4.3). The canvas names the first push choice both `Check & push` and
`Push to PR #163`; when built, it is `Check & push` everywhere.

**Trigger.** A reviewer leaves three comments on PR #163. The Feature is
`Completed`, and its strip reads `PR #163 · OPEN · Review asked for changes — 3
comments`.

1. On the Feature page, the inspector opens on `Summarize & open PR` and lists the
   three comments. She presses `Continue work`.
2. The **brief** says the old worktree was removed by Cleanup and will be
   recreated, and offers *rebase onto master first* (ticked, 3 behind). The context
   is the three comments as checkboxes. She unticks the nit she means to argue
   with and starts the agent.
3. The agent makes one commit per comment. The strip now reads `2 commits not on
   the PR yet · Push to PR…`, and the card shows `claude · review fixes`.
4. `Push to PR…` offers `Check & push` (default): the harness gate runs against
   the head, then the push. She confirms.

**Outcome.** The PR is updated and reviewers are notified. The Feature stays
`Completed`, marked as manually changed, and the strip reads `Pushed 2 commits ·
checks passed · pushed`.

**Branches.** `Keep local for now` leaves the card showing `2 unpushed`. If
unpushed work sits for more than a day, the Feature appears under *Needs you*. If
she squashed, the button becomes `Force-push to PR` and needs a second
confirmation.

### J-S4 Just a shell

**Trigger.** Maya wants to check disk on the runner, or run `terraform plan` in
the infra repo.

1. `Start session` on Project home, or `⌘⇧\`` anywhere. A plain shell opens in a
   drawer: no brief, no Feature, no hand-back bar.
2. It is listed under `No feature` in the index.

**Outcome.** No ceremony. Nothing about it can touch a pipeline.

### J-S5 Come back to a session later, or from another terminal

1. The rail shows `2 waiting`. Maya opens the **index**, where rows are grouped by
   Project → Feature with location and state. Enter opens the row's Feature page
   with the drawer on that tab.
2. On a busy day she presses copy attach instead and runs
   `ssh runner-01 -t 'tmux attach -t dm-f1788403486284-critic'` in Ghostty. It is
   the same session, and Demeteo shows it as attached elsewhere while she is
   there.
3. From the focused session, `⌘J` returns her to the pipeline and `⌘J` again
   comes back to the session.

### J-S6 Look inside a running subtask (deferred)

1. While `Implement Tickets` runs, the inspector's subtask list offers
   `Open shell` on a subtask row. A shell opens in that subtask's worktree.
2. The **live-step banner** holds her keystrokes until she chooses. She picks
   `Type anyway` to run that subtask's tests in isolation.
3. The banner clears when the subtask ends. The worktree is torn down with the
   step, as it is today, and the session closes with a note in its scrollback.

### J-S7 Cleanup with live sessions (confirmation deferred; guard in v1)

1. `Cleanup` on a Feature that has sessions opens a confirmation listing each
   session, its location and its tree state.
2. Cleanup refuses while any session's tree is dirty, and names the files.
3. On confirm, runner sessions (tmux) are ended, and the session worktree is
   removed after the branch has been archived or kept according to the project's
   `feature_lifecycle`.

**v1 ships only the guard:** Cleanup refuses while the Feature has any open
session and names them, and the branch-delete path also removes the
`_wt_session_` worktree so it is never orphaned. Today `archive` (the default)
does no filesystem work at all, `auto_delete` removes step worktrees only, and
neither ends a session.

---

## 8. What changes from today

| Today | After |
|---|---|
| The full-screen `TerminalsView` is where terminals live | Sessions live on their Feature (drawer or focused). The rail entry opens an index. |
| `Code with Agent` starts a shell in the main clone | A contextual `Fix with agent` / `Continue work` / `Open shell` / `Open session`. A briefed agent in a session worktree on the host where the branch lives. |
| The session list says `demeteo · local · Running` | Each row shows Feature · purpose · location · state |
| The rail count is open tabs, badged for approvals | The rail count is sessions **waiting on you** |
| A large gate block inside the step card, plus the gate strip | One status strip and the inspector's decision block. The step card keeps its `Decide Gate →` row. |
| The graph opens fitted to every node | The graph opens on what needs you, at 100%. `Whole run` fits. |
| Hand-back is implicit (approve and hope) | An explicit panel that names the SHA and blocks on a dirty tree |

---

## 9. Out of scope

- Taking over a running subtask (stop it, fix it by hand, hand it back as "subtask
  done"). It is valuable but rare, and it needs a new step transition.
- A split-pane or layout manager inside Demeteo. People use tmux for that.
- Sessions attached to Discovery tickets. A launched ticket links to its pipeline,
  and sessions hang off the pipeline.
- Terminals opened from Ask.
- A cross-project "Needs you" queue. It belongs to the Runs inbox, and its rows
  will open these sessions.
- The §4.4 run-view changes that do not depend on sessions (opening on what
  needs you at 100%, `Needs you | Whole run`, the minimap, drawn `on_failure`
  loops, edge colouring). They are designed here and ship as their own slice;
  v1 adds only the session chip and pill and the band-above-drawer behaviour.

---

## 10. Open questions and gates

Every item below was settled on 2026-09-11; §11 has the outcome. They are kept
so the reasoning stays attached to the question.

- **Gate Policy (AGENTS.md §6): changing agent spawn logic.** "Fix with agent"
  spawns an agent from Demeteo into a pty. Building it needs explicit approval,
  and it must go through `PermissionPolicyPort` (AGENTS.md §2). Today's terminal
  launch path types a command into a shell, so how an interactive session
  satisfies that invariant has to be settled before this is built.
- **Migration:** recording the approved head SHA adds a column to
  `gate_decisions`. That is additive and so not a §6 gate item, but it needs a
  new migration file.
- **"Re-run critic, then return here"** needs a step transition that parks the run
  at the same gate again. Does the executor support re-entering a gate after a
  non-failure re-run?
- **"Mark passed with a reason"** for a failed step was asked for by the persona,
  for flaky tests. It conflicts with the harness-truthfulness stance in
  [HARNESS_BASELINE.md](HARNESS_BASELINE.md). This is a product call; the default
  is to leave it out.
- **Interactive cost:** which harnesses can report interactive spend. Until one
  does, show `cost unknown` (§5.3).
- **tmux on the runner:** is it a runner prerequisite (installed by
  `demeteo-runner` setup), or detected per host with honest degradation (§2.4)?
- **UI_REDESIGN_PLAN D3** (Terminals in both the header and the rail): this spec
  keeps the rail entry as the index. The header item could open the same index or
  be removed.
- **Parity:** creating and attaching a session worktree must behave identically on
  local, desktop-over-SSH and runner transports (AGENTS.md §2, ExecutionPort).
  Run the SSH and topology conformance suites when the implementation touches an
  `ExecutionPort` impl.

---

## 11. Decisions (2026-09-11)

Taken in a design review of this spec against the code, one round of questions
at a time. Each line is the decision and the reason it went that way; where it
changed a section above, that section already reads the new way.

**Scope**

- v1 is J-S1, J-S2, J-S4 and J-S5, with origins `gate`, `failed_step`,
  `running` and `none`. They share the brief card, the drawer, the hand-back
  panel and the session worktree. J-S3 (`published`, PR comments, push) is a
  separate integration; J-S6 needs keystroke holding; J-S7's confirmation
  touches Cleanup; the §4.4 run-view framing is independent of sessions.
- Completed Features offer `Open shell` until J-S3. Feature-less agent sessions
  stay, behind the `Start session` caret, from one agent registry.
- "Mark passed with a reason" is out: retry at the new head covers the flaky
  case, and a manual pass reads as green in the attempt history however it is
  labelled (HARNESS_BASELINE.md).
- Interactive cost is `cost unknown` everywhere in v1.

**Spawning the agent (the §6 gate item)**

- Deny list enforced, harness asks for the rest (§4.5). The status quo was
  "harness defaults, no Demeteo payload at all"; the full allow/deny profile
  would have made an attended session silently fail on denied tools.
- Brief delivery is capability-gated by `AgentCapabilities.interactive_launch`.
  Typing a brief into a TUI after a guessed delay was rejected.
- Agent exit and exit code via an OSC trailer on the launch line (§5.1).
- `Stop the agent` = the harness's quit sequence, then an offered hard kill.
- There is no `PermissionPolicyPort` in the tree (the earlier draft of §10
  assumed one); the real pieces are `PermissionProfile` and the per-harness
  translators in the agent-runtime port.

**Sessions and persistence**

- tmux is a runner prerequisite; local Linux/macOS use it when detected;
  Windows and tmux-less hosts are `not persistent` (§2.4).
- Transport is the existing ssh2 pty wrapping `tmux new -A`; the runner gains
  no session RPC. Session rows live in a new desktop SQLite table (§2.1).
  Reattach is lazy. Names carry the client id (§2.4).

**Worktree and hand-back**

- A new `_wt_session_<slug>` kind rather than reusing the sync worktree, whose
  provisioning removes stale `_wt_sync` entries first.
- Detached HEAD while the pipeline runs, and the merge/sync code never targets
  a session worktree (§2.2, the `merge_subtask` hazard). Re-attach is explicit.
- The approved head SHA is recorded on every approval, read by the executor on
  the side that owns the clone; for detached runs it lives in the runner DB
  like the rest of `gate_decisions`. This is what gives the "head moved since
  the gate opened" tell without a session.
- "Re-run critic, then return here" is the existing gate redirect (§2.3). A
  single-node re-run that does not rewind the cone does not exist and was not
  worth a new transition.
- Every pipeline-advancing action routes through the hand-back panel when a
  session exists (§2.3).
- The commit message is a template plus commitlint, no agent turn (§4.9).
- No machine toggle in the brief; no rebase, and the eventual "bring up to
  date" is a merge, `published` only (§4.5).
- Cleanup v1: refuse with open sessions, remove the session worktree (J-S7).

**Surfaces and keys**

- The header `Terminals` item is removed (UI_REDESIGN_PLAN D3); the rail entry
  is the one entry point and its count is "waiting on you".
- `⌘.` moves from the palette to hand-back; `⌘⇧\`` is context-sensitive; the
  Ctrl+K theft is fixed (§6).
- Plain shells open at the project's last-used location, picker behind the
  caret (§4.1).
- OS notifications for awaiting-input and finished-dirty ship in v1; the
  notification port already fires for awaiting-approval.

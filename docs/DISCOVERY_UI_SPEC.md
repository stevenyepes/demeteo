# Discovery — UI Implementation Spec

Implementation-facing transcription of the five design artboards in
`ui-mocks/discovery/`. It is the single source implementers read: the mocks are
static `.dc.html` artboards written against their own private stylesheet, not
against this app's. Everything below is either copied verbatim from a mock or
resolved against `src/App.css` and the existing components.

Product behaviour behind these screens is [docs/PRD_DISCOVERY.md](PRD_DISCOVERY.md);
section references (§4.6, §5.3, §6.3, §7.2) point there. This document covers
**only** layout, markup, copy and tokens.

## How to read the mocks

| Mock file | Artboard | Canvas size |
|---|---|---|
| `ProjectHome.dc.html` | Project Home — Discovery section | 1440 × 900 |
| `Main.dc.html` | Discovery — interview, graph and board | 1560 × 980 |
| `NewDiscovery.dc.html` | New discovery | 620 × 760 |
| `Decompose.dc.html` | Decompose — proposed changes | 1040 × 880 |
| `TicketEdit.dc.html` | Ticket — edit and force start | 760 × 1560 |

`demeteo-discovery.html` is a 2 MB bundled canvas of the same five artboards and
carries nothing extra. `canvas.json` is layout plus the five design annotations,
which are reproduced under each artboard below.

**Three transcription rules that apply to every artboard:**

1. **Sizes are artboard sizes, not viewport sizes.** Fixed widths that are
   *structural* (the 72/260 rail+sidebar grid) are called out as such and must
   be kept; the outer `width:`/`height:` on the artboard root is scaffolding
   and must be dropped. The one exception is the workspace row's own three
   columns (§3.2): the 560 px interview / 360 px inspector widths are this
   artboard's widest-case layout, not an unconditional rule — the row degrades
   through three modes as it narrows, per `src/components/discovery/discoveryLayout.ts`.
2. **The mock stylesheets are throwaway.** Each mock re-declares its own `.glass`,
   `.chip`, `.btn-p`, `.fld` etc. inside a `<helmet><style>` block. **None of
   those class names exist in `src/App.css`** — writing `class="glass"` or
   `class="chip c-cyan"` in this app renders an unstyled element and no gate
   catches it (`scripts/check-classes.mjs` only reports a token whose *namespace*
   the sheet emits, and `glass`/`chip`/`c-cyan` have no namespace here). Every
   mock class is mapped to its real equivalent in **§6 Shared components & tokens**.
3. **The `<link>` to fonts.googleapis.com must not be reproduced.** Fonts are
   self-hosted through `@fontsource/*` imports in `src/main.tsx`. Inter, Outfit
   and Fira Code are all already loaded at the weights the mocks use.

The mocks are driven by a tiny `DCLogic` component runtime: `<sc-if value="{{x}}">`
is a conditional, `<sc-for list="{{xs}}" as="x">` a loop, `{{x}}` an interpolation.
Treat those as the state model — every branch is a state the real screen must have.

---

## 1. ProjectHome — Discovery section

**Annotation (canvas.json):** *"§9 — Discovery is a section of Project Home,
beside Pipelines. Flip the tabs; the `empty` tweak above this artboard swaps in
the empty state."*

### 1.1 Purpose and placement

Discovery is a **third tab in the existing Project Home tab strip**, beside
Pipelines and (remote-only) Terminal. It is not a new route and not a new rail
entry. In code that is `src/components/ProjectHome.tsx`, the `tabs: TabDef<ProjectSection>[]`
array at line ~197 — add `{ value: 'discovery', label: 'Discovery', icon: <Compass className="w-3.5 h-3.5" /> }`
between `pipelines` and the conditional `terminal` entry, and widen
`ProjectSection`.

The mock reproduces the whole three-column app shell around it purely for
context. **Only the workspace column changes.** Levels 1 and 2 are the existing
`ProjectRail` / sidebar and need no edit.

### 1.2 Shell layout (context only — do not rebuild)

Root grid: `grid-template-columns: 72px 260px 1fr`, `background-color: #08090c`,
plus two ambient radial gradients:

```css
background-image:
  radial-gradient(circle at 70% 30%, rgba(139,92,246,0.04) 0%, transparent 60%),
  radial-gradient(circle at 10% 80%, rgba(6,182,212,0.03) 0%, transparent 50%);
```

- **Level 1 rail** (72 px): `background:#0d0f14`, right border `rgba(255,255,255,0.05)`,
  column, centred, `padding: 20px 0; gap: 16px`. Logo `D` — Outfit 800, 24px,
  `#06b6d4`, `text-shadow: 0 0 12px rgba(6,182,212,0.25)`, `margin-bottom:20px`.
  Then four 44×44 `rail` buttons (radius 10px): **Projects** (active — lucide
  `git-compare`-ish two-circle glyph), **Machines** (`cpu`), **Providers**
  (`plug`/`unplug`), spacer, **Settings** (`settings`). `title` attributes:
  `"Projects"`, `"Machines"`, `"Providers"`, `"Settings"`.
- **Level 2 sidebar** (260 px): `background: rgba(11,13,18,0.95)`. Header
  "Projects" with a `+` glyph (`#64748b`, 18px). Rows: `demeteo` (active, emerald
  dot, count `4`), `stv-cloud-api` (slate dot, no count), `runner-images` (cyan
  dot, count `1`).
- **Workspace** (`1fr`): `background:#0a0c10`, `padding: 28px 32px`,
  `overflow-y:auto`, `min-width:0`.

### 1.3 Workspace header row

Two-up, `justify-content: space-between; align-items: flex-end; gap: 24px`.

Left:
- Title row (`gap:8px; margin-bottom:8px`): `<h1>` Outfit 700 / 30px / `#fff` /
  `letter-spacing:-0.02em` reading **`demeteo`**, followed by a 20px lucide
  `settings` icon stroked `#94a3b8` (click target → project settings).
- Subtitle `<p>` 13px `#94a3b8`: **"1 repository on this machine · 2 providers configured"**

Right — a metric strip (`MetricStrip` in this codebase): border
`rgba(255,255,255,0.05)`, background `rgba(255,255,255,0.03)`, radius 12px,
`padding: 8px 16px`, `gap: 20px`. Three stacked label/value pairs:

| Label | Value | Value colour |
|---|---|---|
| `Fleet Active` | `2` | `#34d399` (emerald-400) |
| `Cost` | `$18.42` | `#34d399` |
| `Tokens` | `3.9M` | `#22d3ee` (cyan-400) |

Labels are the `.lbl` treatment: Fira Code, 10px, weight 700, uppercase,
`letter-spacing: 0.08em`, `#64748b`. Values: Fira Code, 13px, weight 700.

### 1.4 Tab strip row

`margin-top: 24px`, `align-items: flex-end; justify-content: space-between; gap: 12px`.

Tabs (`.tabs`: `display:flex; gap:4px; border-bottom:1px solid rgba(255,255,255,0.05); padding-bottom:1px`).
Each `.tab`: `padding: 10px 16px`, Outfit 14px weight 500, `#94a3b8`,
`border-bottom: 2px solid transparent`, 14px leading icon, `gap: 8px`. Hover
`#e2e8f0`. Active (`.tab.on`): `border-bottom-color:#06b6d4; color:#22d3ee`.

| Tab | Icon (lucide) |
|---|---|
| `Pipelines` | `sliders-vertical` (three vertical lines with knobs) |
| `Discovery` | `compass` — `<circle r=10>` + `<polygon points="16.24 7.76 14.12 14.12 7.76 16.24 9.88 9.88 16.24 7.76">` |
| `Terminal` | `square-chevron-right`-style: `<polyline points="4 17 10 11 4 5">` + `<line x1=12 y1=19 x2=20 y2=19>` |

**Compass is the Discovery glyph everywhere in this feature** — tab, hero card,
empty state. Use lucide `Compass`.

Beside the strip, not in it: a **Code Review** button (`btn-s`, `margin-bottom:4px`,
`gap:8px`, lucide `git-pull-request-arrow` icon). This already exists; it is
deliberately outside the `tablist` (see the comment in `ProjectHome.tsx` at
line ~192 — arrow keys in a tablist both move and select).

### 1.5 Discovery section body (`onDiscovery === true`)

`margin-top: 24px; display:flex; flex-direction:column; gap: 20px`.

#### 1.5.1 Start-a-discovery hero card

`.glass` with `border-radius: 16px; padding: 16px; position:relative; overflow:hidden`.
Inner row: `align-items: flex-start; gap: 16px`.

- Leading compass icon, 20px, `color:#a78bfa` (violet-400), `margin: 6px 0 0 4px; flex:none`.
- Middle column (`flex:1; min-width:0`):
  - Borderless transparent `<input type="text" maxlength=80>`, `padding:8px`,
    14px, `#fff`, Inter, no outline. Placeholder verbatim:
    **`Name something you want to think through...`**

    It carries into the modal as the Discovery's **name**, and the cap is
    `domain::models::TITLE_MAX_CHARS` — §2.3 carries why the field is bounded
    at all.
  - Meta row under it (`padding: 0 8px 4px; gap:8px`): label `Interviewer`
    (`.lbl`), then chips **`claude-code`** (cyan), **`opus`** (violet),
    **`effort high`** (slate), then 11px `#64748b` text:
    **"chosen per discovery, not from the project default"**
- Trailing `btn-p` **`New discovery`** (`flex:none; margin-top:4px`). Opens the
  New discovery modal (§2).

Compare to the Pipelines tab's equivalent hero (also in this mock, under
`onPipelines`): same card, lucide `zap` icon, placeholder
**`Draft and delegate a new feature pipeline...`**, and *no* trailing button.

#### 1.5.2 Discovery list (`hasDiscoveries`)

Column, `gap: 16px`. Each row is a `.glass` card: `border-radius:12px; padding:20px;
cursor:pointer; position:relative; overflow:hidden`, with a **4 px left accent
bar** absolutely positioned `left:0; top:0; bottom:0; width:4px` carrying
`background: <accent>` and `box-shadow: 0 0 10px <accentGlow>`.

Card anatomy, top to bottom:

1. Title row (`justify-content: space-between; gap:16px`): `<h3>` Outfit 600 /
   18px / `#fff` / `flex:1; min-width:0`; then right cluster (`gap:12px`) of a
   state chip (with a 6px dot, pulsing when live) and an age string in Fira Code
   12px weight 500 `#fff`.
2. Meta row (`margin-top:10px`, `flex-wrap`, `gap:12px`, 11px `#94a3b8`): agent
   chip (cyan), model chip (violet), `"{turns} turns"` and `"{cost}"` in
   `#cbd5e1`, then a token count preceded by a 12px lucide `zap` stroked `#22d3ee`.
3. Progress row (`margin-top:14px`, `gap:14px`): a `.bar`
   (`height:4px; border-radius:999px; background:rgba(255,255,255,0.06); overflow:hidden`)
   that is itself a flex row of two spans — landed (`#10b981`) and in-flight
   (`#06b6d4`) — sized by percentage; then the progress text in Fira Code 11px
   `#94a3b8`.
4. Detail line: `margin-top:10px`, 12px, `#64748b`, `line-height:1.6`.

The three rows in the mock, verbatim — these are the three lifecycle states:

| | Row 1 | Row 2 | Row 3 |
|---|---|---|---|
| Title | `Runner serves more than one client` | `What a gate should let you edit` | `Windows: what a login shell can still reach` |
| State chip | `Interviewing`, violet, **pulsing dot** | `Decomposed`, cyan, static dot | `Closed`, slate, static dot |
| Accent bar | `#8b5cf6` / glow `rgba(139,92,246,0.8)` | `#06b6d4` / `rgba(6,182,212,0.8)` | `#475569` / `rgba(100,116,139,0.6)` |
| Agent · model | `claude-code` · `opus` | `claude-code` · `sonnet` | `opencode` · `sonnet` |
| Turns · cost · tokens | `4 turns` · `$2.14` · `486k` | `11 turns` · `$3.90` · `1.2M` | `6 turns` · `$0.88` · `210k` |
| Age | `9m ago` | `2d ago` | `3w ago` |
| Bar | landed 14 %, in flight 14 % | landed 67 %, in flight 33 % | landed 100 %, in flight 0 % |
| Progress | `1 of 7 landed · 1 in flight` | `2 of 3 landed · 1 in flight` | `5 of 5 landed` |
| Detail | `DSC-4 is startable now. DSC-3 waits on PR #134.` | `Open for another pass — one ticket dropped after DSC-2 landed.` | `Closed. Everything it proposed has a feature; nothing was dropped.` |

Note the em dash and middot characters are literal in the copy.

#### 1.5.3 Empty state (`isEmpty`)

This is **exactly `src/components/EmptyStateCard.tsx`** — the mock reproduces
that component's markup line for line. Reuse it, do not re-author:

```
glass-panel p-8 rounded-2xl border border-white/5 text-center bg-black/20
flex flex-col items-center justify-center space-y-4 relative overflow-hidden
```

with the two blurred orbs (`absolute -top-10 -left-10 w-40 h-40 bg-violet-600/5
rounded-full blur-2xl`, and `-bottom-10 -right-10 … bg-cyan-600/5`), the 48 px
icon medallion (`w-12 h-12 rounded-full bg-violet-500/10 border
border-violet-500/25 … text-violet-400 mb-2`) holding a 24 px Compass, then:

- Title (`font-heading text-white font-medium text-base`): **`No discoveries yet`**
- Body (`text-xs text-slate-400 max-w-sm mx-auto leading-relaxed`, mock caps at
  420 px / `line-height:1.7`):
  **"A discovery is a conversation you can leave and come back to. It reads this
  repository, runs commands in its own worktree, and ends by proposing tickets
  you can start one at a time."**

The mock's empty state has **no CTA button** — the hero card above it is the
call to action.

#### 1.5.4 Pipelines tab, for reference

The mock also renders the Pipelines section so the two read as siblings. Two
pipeline cards, same 4 px accent-bar card shape, each carrying a chip
**`from DSC-2`** / **`from DSC-1`** (violet) beside `Standard Feature` and
`local`. That provenance chip is the visible half of §8.2 and is new work on the
Pipelines side: a Feature born from a Ticket shows which one.

### 1.6 States on this artboard

| State | Trigger | Rendering |
|---|---|---|
| Discovery tab, populated | default | hero card + list |
| Discovery tab, empty | `empty` prop | hero card + EmptyStateCard |
| Pipelines tab | tab switch | existing pipelines section |

No loading or error state is drawn. Use the existing `PipelineListSkeleton`
shape for the list-loading case and `ErrorToast` for failures.

---

## 2. New discovery (modal)

`NewDiscovery.dc.html` · 620 × 760.

### 2.1 Purpose and placement

Modal launched by **New discovery** on the Project Home hero card (§1.5.1). It
collects the Discovery's **name** and the interviewer's run shape, then starts
the Discovery.

**The name is a label, not the opening move.** It titles the row in §1.5.2 and
the workspace header, and no prompt reads it — the idea is said in the
interview's first message. A free textarea claimed the opposite by its shape: a
user with an idea in hand typed the idea into it, and opened an interview that
had never been told any of it. The cap
(`domain::models::TITLE_MAX_CHARS`) is what makes the field say what it is
before anyone reads the label. Build it on the existing `Modal` primitive
(`src/components/ui/Modal.tsx`); the artboard's outer `620 × 760` box with its
`radial-gradient(circle at 50% 0%, rgba(139,92,246,0.06) 0%, transparent 55%)`
is the backdrop, not part of the dialog.

### 2.2 Frame

Column flex, `glass-panel` surface (`rgba(18,22,30,0.92)`, `backdrop-filter:
blur(12px)`, border `rgba(255,255,255,0.05)`, `box-shadow: 0 8px 32px 0
rgba(0,0,0,0.4)`, radius 12px, `overflow:hidden`).

- **Header** `padding: 18px 20px`, bottom border, `flex:none`:
  - Eyebrow: Outfit, 11px, weight 600, uppercase, `letter-spacing:0.15em`,
    `#22d3ee`, `margin-bottom:6px` — **`demeteo`**
  - `<h2>` Outfit 700 / 20px / `#fff` — **`New discovery`**
- **Body** `flex:1; padding:20px; column; gap:18px; overflow-y:auto; min-height:0`
- **Footer** `padding: 16px 20px`, top border, `background: rgba(13,15,20,0.9)`,
  right-aligned, `gap:10px`, `flex:none`: `btn-s` **`Cancel`**, `btn-p`
  **`Start discovery`**

### 2.3 Fields, in order

Every label uses `.lbl` as a **block** label (`display:block; margin-bottom:8px`).

1. **`Name this discovery`** — `<input class="fld" type="text" maxlength=80>`,
   placeholder verbatim: **`Ask-the-repo chat`**. Beneath it, an 11px `#64748b`
   hint: **"A label for your discovery list — the interviewer never reads it.
   Say the idea itself in the first message; that is what it is asked about."**
   Right-aligned beside the hint, a Fira Code 11px counter of characters
   remaining, drawn only in the last quarter of the cap — a number that is
   always on reads as a limit being pressed rather than a name being written.
   Ruby past the cap, where **`Start discovery`** is also disabled: the seed
   carried in from §1.5.1 is set without a keystroke, so `maxlength` never sees
   it and it is the one value that can arrive over.
2. **`Interviewer`** — a row (`gap:8px`) of three `.pill` buttons:
   `claude-code`, `opencode`, `hermes`. Below, an 11px `#64748b` hint:
   **"Chosen here, not inherited. Interviewing and implementing want different
   things from a model."**
3. **Two-column grid** (`grid-template-columns: 1fr 1fr; gap:14px`):
   - **`Model`** — wrapping pill row, contents driven by the selected agent.
   - **`Effort`** — pill row `low` / `medium` / `high`.
4. **Effort-unsupported note** (`sc-if effortUnsupported`) — amber note, verbatim:
   **"Hermes exposes reasoning effort only through its own config file, which
   Demeteo does not write. Effort is unavailable for this interviewer — it will
   run at whatever that file already says."**
5. **`Machine`** — `<select class="fld">` with options `local` and
   `runner-01.stv.cloud` (real list comes from the machines store).
6. **`Attachments`** — a wrapping row: chip `MULTI_CLIENT_RUNNER.md` (slate),
   chip `runner-topology.png` (cyan), then a small `btn-s`
   (`padding:5px 10px; font-size:11px`) labelled **`+ Attach`**.
   - **No-vision note** (`sc-if noVision`), amber, verbatim:
     **"{model} cannot read images. `runner-topology.png` will be attached and
     ignored."** — the model name interpolates; the filename is rendered in
     `.m` (mono).
7. **Worktree note** — neutral note (`border rgba(255,255,255,0.05)`, background
   `rgba(255,255,255,0.02)`, `#94a3b8`), two paragraphs, verbatim:
   - (in `#64748b`) **"This discovery gets its own worktree, created on the first
     turn that needs the repo and reclaimed while idle."**
   - **"It reads files and runs commands there. It is given no write tools, and it
     leaves nothing behind — no branch, no committed spec. Whatever it writes
     rides to a feature as an attachment."**

### 2.4 The agent/model/effort catalog (drives every branch)

```js
'claude-code': { models: [opus (vision), sonnet (vision)],            effort: true  }
'opencode':    { models: [sonnet (vision), qwen3-coder (no vision)],  effort: true  }
'hermes':      { models: [hermes-4 (no vision)],                      effort: false }
```

Picking an agent resets the model to that agent's first entry. Initial state:
`claude-code` / `opus` / `high`.

### 2.5 Pill states

`.pill` base: radius 8px, border `rgba(255,255,255,0.05)`, background
`rgba(255,255,255,0.02)`, `#cbd5e1`, `padding: 9px 14px`, 12px, weight 500.

| State | Class | Treatment |
|---|---|---|
| Idle | — | as above; hover → border `rgba(255,255,255,0.15)`, `#fff` |
| Selected | `.on` | border `rgba(139,92,246,0.4)`, bg `rgba(139,92,246,0.12)`, text `#c4b5fd`, `box-shadow: 0 0 10px rgba(139,92,246,0.2)` |
| Unsupported | `.dis` | `opacity:0.35; cursor:not-allowed`, click is a no-op |

Effort pills go `.dis` **as a group** when the selected agent's `effort` is
false (Hermes), and the amber note appears. This is AGENTS.md §2's
"declare the capability unsupported and degrade honestly" made visible.

### 2.6 Field styling (`.fld`)

`width:100%`, background `rgba(10,12,16,0.6)`, border `1px solid
rgba(255,255,255,0.05)`, radius 6px, `padding:10px 12px`, `#f3f4f6`, Inter 13px,
no outline. Focus: border `rgba(139,92,246,0.4)` + `box-shadow: 0 0 8px
rgba(139,92,246,0.25)` — i.e. **violet focus**, which is exactly `.input-field`
in `src/App.css`. `textarea.fld { resize:none; line-height:1.65 }`.
`select.fld { appearance:none; background-color:#08090c; cursor:pointer }`.

### 2.7 States

| State | Rendering |
|---|---|
| Default | claude-code / opus / high, no notes |
| Effort unsupported | hermes selected → effort pills dimmed + amber note |
| No vision | model without vision + an image attached → amber note |
| Both | hermes + image → both notes stack, effort note first |

No loading, submitting or validation-error state is drawn. `Start discovery`
has no disabled treatment in the mock — add one for an empty prompt.

---

## 3. Discovery workspace (Main)

`Main.dc.html` · 1560 × 980. The largest artboard and the centre of the feature.

**Annotation (canvas.json):** *"§4 + §6 — click any ticket to select it, or type
a turn and send one. Flip Graph/Board above the tickets: the graph says what
depends on what, the board says how much is done. Both read the same buckets,
recomputed from the edges on every render; there is no stored column to drift."*

### 3.1 Purpose and placement

Full-workspace view of one Discovery, opened by clicking a row in §1.5.2. It
replaces the Project Home workspace column (the rail and project sidebar stay).
Root: column flex, `background:#0a0c10`, `position:relative; overflow:hidden`
(the toast anchors to it).

### 3.2 Region map

This is the region map at full width — the artboard's own size, and the
`'three-up'` layout mode below. The row is not always this wide; §3.2.1
describes the two narrower modes.

```
┌───────────────────────────────────────────────────────────────┐
│ Workspace header (flex:none, 14px 24px, bottom border)        │
├────────────────┬───────────────────────────┬──────────────────┤
│ Interview      │ Ticket graph / board      │ Inspector        │
│ width 560 fixed│ flex:1, min-width:0       │ width 360 fixed  │
│ flex:none      │                           │ flex:none        │
│ right border   │  38px sub-header          │ left border      │
│                │  ──────────────────────   │ sticky sub-header│
│                │  body (position:relative) │ scrolls          │
└────────────────┴───────────────────────────┴──────────────────┘
                                              toast: absolute
                                              right:24 bottom:24
```

Middle band is `flex:1; display:flex; min-height:0`. All three columns are
`min-height:0` so their internal scrollers work.

#### 3.2.1 Layout modes

`DiscoveryWorkspaceRow` (`src/components/discovery/DiscoveryWorkspaceRow.tsx`)
measures its own width via `useDiscoveryColumnLayout` and picks one of three
modes with `pickDiscoveryLayout`, both in
`src/components/discovery/discoveryLayout.ts` — that module is the source of
truth for the exact width thresholds; they are not restated here.

- **`'three-up'`** — the row is wide enough for all three columns at once.
  This is the layout diagrammed above: 560 px interview, flexible ticket
  graph/board pane, and the 360 px inspector (or the 760 px `TicketEditorDrawer`
  when a ticket is being edited) in-row as the third column.
- **`'overlay-inspector'`** — the row is too narrow for a third column but
  still fits Interview and the ticket graph/board pane side by side. Those two
  stay in-row; the inspector or editor no longer takes a column and instead
  renders in `TicketOverlayPanel` (`src/components/discovery/TicketOverlayPanel.tsx`),
  a portalled panel docked to the right edge of the workspace, floating over
  the ticket pane rather than displacing it.
- **`'stacked'`** — the row is too narrow to hold Interview and the ticket
  graph/board pane side by side. A segmented control lets the user toggle
  which one is visible; both remain mounted (`InterviewColumn`/`TicketColumn`
  take a `hidden` prop rather than being unmounted, so an in-progress
  interview draft and the graph's zoom state survive a toggle). The inspector
  or editor still renders in the same `TicketOverlayPanel` as
  `'overlay-inspector'`, docked over whichever pane is currently visible.

In both `'overlay-inspector'` and `'stacked'`, `TicketEditorDrawer`'s 760 px
width (§5.1) is presented inside `TicketOverlayPanel` as a permanent overlay,
never as an in-row column — only `'three-up'` gives the editor or inspector
its own column.

`TicketOverlayPanel`'s backdrop is `pointer-events-none` so it never blocks
clicks aimed at Interview or the ticket graph/board pane behind it — which
means there is no backdrop click-to-dismiss. The overlay is closed by pressing
Escape, or by the visible `Close` button in the panel's own sub-header
(`TicketEditorDrawer` shows `Close`/`Discard` there; `TicketInspector` shows
`Close`) — never by clicking outside it.

### 3.3 Workspace header

`display:flex; align-items:center; justify-content:space-between; gap:24px;
padding:14px 24px; border-bottom:1px solid rgba(255,255,255,0.05);
background: rgba(13,15,20,0.6); flex:none`.

Left column (`gap:6px`):
- Breadcrumb, Fira Code 11px `#64748b`, `gap:6px`:
  **`demeteo`** · separator `/` in `#334155` · **`Discovery`**
- Title row (`gap:12px`): `<h1>` Outfit 700 / 20px / `#fff` /
  `letter-spacing:-0.01em` — **`Runner serves more than one client`**; chip
  **`Interviewing`** (violet, **pulsing dot**); chip **`7 tickets · 2 started`** (slate).

Right cluster (`gap:18px; flex:none`):
- Metric strip, same treatment as §1.3: **`Turns`** (`{{turnCount}}`, `#fff`),
  **`Spend`** (`$2.14`, `#34d399`), **`Tokens`** (`486k`, `#22d3ee`).
  `turnCount` is the rendered block count, i.e. every bubble and question card.
- `btn-s` **`Close discovery`**
- `btn-p` **`Decompose`** with a 14px lucide `code`/`chevrons-left-right` icon
  (`<path d="M12 3v18">` + `<path d="m8 7-4 4 4 4">` + `<path d="m16 7 4 4-4 4">`),
  `gap:8px`.

### 3.4 Interview column (560 px in `'three-up'`)

`width:560px; flex:none; column; border-right:1px solid rgba(255,255,255,0.05);
background: rgba(11,13,18,0.4); min-height:0`. This is the `'three-up'` width
(§3.2.1); in `'stacked'` mode `InterviewColumn` instead takes the row's full
width (its `widthMode="full"` prop), and in either narrower mode it may carry
`hidden` rather than being unmounted.

#### 3.4.1 Column sub-header (38 px)

`padding: 0 16px; height:38px; background: rgba(18,22,30,0.6); border-bottom:1px
solid rgba(255,255,255,0.05); flex:none`. Left: **`Interview`** in Outfit 12px
weight 500 `#9ca3af`. Right: chip row (`gap:6px`) — **`claude-code`** (cyan),
**`opus`** (violet), **`effort high`** (slate), **`local`** (slate).

The same 38 px sub-header bar appears on the graph column and the inspector.
It is a shared piece (§6.2).

#### 3.4.2 The confinement banner (amber, always present)

`margin: 12px 16px 0; radius 8px; border 1px solid rgba(245,158,11,0.20);
background: rgba(245,158,11,0.05); padding: 8px 12px; font-size:11px;
line-height:1.6; flex:none`. Verbatim:

> (p, `#64748b`) **"Its own worktree, reclaimed while idle. What holds
> claude-code to it:"**
> - (li, `#64748b`) **"*Reading files* — claude-code's own file tools refuse to
>   open a file outside this worktree."**
> - (li, `rgba(253,230,138,0.9)`) **"*Changing files* — the interview is given no
>   write tools. Nothing below the harness refuses one."**

`*…*` marks a `<span style="font-weight:500">` on the leading phrase. The `<ul>`
is `margin: 4px 0 0; padding-left:16px`.

The second bullet is deliberately brighter than the first: the harness fence is
a *statement of intent*, never a platform guarantee (AGENTS.md §2, "no surface
may promise more than it carries"). **The wording is load-bearing — do not
soften it, and the bullet text must be keyed off the actual interviewer harness,
not hard-coded to claude-code.**

#### 3.4.3 Transcript scroller

`.scroll` with `flex:1; overflow-y:auto; padding:16px; column; gap:16px;
min-height:0`, pinned to the bottom on every mount/update
(`scrollTop = scrollHeight`).

Each block is a wrapper `<div style="display:flex; flex-direction:column;
align-items:{{b.align}}">` where align is `flex-end` (user), `flex-start`
(agent), or `stretch` (question card).

**(a) Text bubble.** `.bub` = `max-width:88%; padding:12px 16px; radius:10px;
line-height:1.55; font-size:13px; white-space:pre-wrap`.
`.bub.agent` = bg `rgba(139,92,246,0.08)`, border `rgba(139,92,246,0.15)`,
`align-self:flex-start`, `border-top-left-radius:2px`.
`.bub.user` = bg `rgba(6,182,212,0.10)`, border `rgba(6,182,212,0.20)`,
`align-self:flex-end`, `border-top-right-radius:2px`.
Inside, a `.sender` line: 11px, weight 600, uppercase,
`letter-spacing:0.05em; margin-bottom:6px`, `gap:6px`, with a 6 px dot;
`.s-agent { color:#8b5cf6 }` reading **`Interviewer`**, `.s-user { color:#06b6d4 }`
reading **`You`**.
Optional meta line beneath the bubble: `margin-top:6px; padding:0 4px;
font-size:10px; color:#475569`, Fira Code — e.g.
**`read 6 files · ran git log, rg · 12.4k tokens · $0.31`**.

**(b) Question card** (`.qcard`) — see §3.4.4.

**(c) Streaming placeholder** (`sc-if pending`): an agent bubble whose sender dot
**pulses**, containing the partial text plus a `.caret`:
`display:inline-block; width:7px; height:13px; background:#a78bfa;
vertical-align:-2px; animation: blink 1s steps(1) infinite` where
`@keyframes blink { 0%,50% {opacity:1} 50.01%,100% {opacity:0} }`.
Mock streaming text: **`Reading crates/demeteo-runner/src/auth.rs`**.

**As built, the mock's meta line beneath is not carried over.** It read
`streaming · one-shot turn, resumed from the stored transcript`, and as a
constant it claimed a resume on every turn including the first, where there was
no session to resume. What actually happened is known only when the turn ends
(`DiscoveryTurnCompleted.reseeded`) and is rare, so it is reported there, on the
settled bubble, and nothing is claimed while the turn streams.

In its place the streaming bubble carries an activity strip *inside* it: the
call in flight in human words with its icon, a ticking elapsed time, and the
same summary the settled meta line renders. The mock's `Reading …` is that
strip rather than mock prose — a reasoning turn emits no text for minutes, so
the indicator has to stand without one.

**The wait starts before the agent does.** A turn is claimed and announced
(`setting_up`) before it is resolved, so the placeholder is up from the press
rather than from the first agent event — the gap it covers is a worktree
re-provision after an idle reclaim (§4.6), which is minutes on a large repo and
was the whole of what a pressed Decompose used to show. What the strip may say
there is bounded by what the phase knows: setup provisions a worktree only when
there is none, so the copy names the turn and never the worktree.

**(d) Empty state** — a Discovery that has been opened and not yet spoken to
has an empty transcript, because §2.1's name is not a turn. Centred in the
column, `max-width:380px`, 13px `#64748b` with an Outfit weight-600 `#cbd5e1`
first line:

- **`Nothing has been said yet`**
- **"The name you filed this under is a label for your list. Describe the idea
  here — the first thing you send is the whole of what the interviewer is asked
  about, and it reads the repository before answering."**

This is the second half of §2.1's cap: the modal stops the idea going into the
name, and this says where it goes instead. Neither works alone — a bounded
field with no destination just loses the idea somewhere else.

The seeded transcript, verbatim (use as fixtures):

- **You:** "I want the runner to serve more than one desktop client.
  docs/MULTI_CLIENT_RUNNER.md is the old sketch and I no longer trust it."
- **Interviewer:** "The sketch settles the transport and nothing else. Two things
  it leaves open:\n\n1. How a client proves who it is. It says \"a shared token\",
  which means any desktop ever set up can drive any run.\n2. What happens when
  two clients want the same worktree. Nothing in it says."
  · meta `read 6 files · ran git log, rg · 12.4k tokens · $0.31`

#### 3.4.4 Question card — the interview's primary affordance

`.qcard`: `width:100%; radius:12px; border:1px solid rgba(139,92,246,0.22);
background: rgba(139,92,246,0.05); padding:14px 16px`.
Answered: `.qcard.done` → border `rgba(255,255,255,0.06)`, background
`rgba(255,255,255,0.02)` — the card recedes once settled.

Header row (`justify-content:space-between; gap:10px; margin-bottom:10px`):
left = `.sender.s-agent` **`Interviewer`** with dot (`margin-bottom:0`) plus a
violet chip carrying the question's short header; right = a state chip:

| Card state | Chip | Tone |
|---|---|---|
| live | **`Needs you`** | violet (`c-violet`) |
| settled | **`Answered`** | emerald (`c-emerald`) |

Question text: `<p>` `margin: 0 0 12px; font-size:13.5px; line-height:1.55;
color:#f1f5f9`.

Options list: column, `gap:8px`. Each `.opt` is a **`<button>`**:
`display:flex; gap:10px; align-items:flex-start; width:100%; text-align:left;
radius:8px; border:1px solid rgba(255,255,255,0.06); background: rgba(5,6,8,0.5);
padding:10px 12px; color:inherit; font-family:inherit`.

| Modifier | Applied when | Treatment |
|---|---|---|
| `.live` | card is the open question | `cursor:pointer`; hover → border `rgba(139,92,246,0.45)`, bg `rgba(139,92,246,0.08)`, `transform: translateX(2px)` |
| `.rec` | option is the recommended one | border `rgba(16,185,129,0.22)` |
| `.chosen` | settled + this option was picked | border `rgba(16,185,129,0.45)`, bg `rgba(16,185,129,0.06)` |
| `.faded` | settled + not picked | `opacity:0.38` |

Class composition in the mock: live → `live` (+ ` rec`); chosen → `chosen`;
otherwise → `faded` (+ ` rec`).

Inside each option:
- `.key` — a 19×19 keycap: `radius:4px; border:1px solid rgba(255,255,255,0.12);
  centred; Fira Code 10px; color:#94a3b8; flex:none; margin-top:1px`. Contents
  are `1`, `2`, `3` … and `↵` (`&crarr;`) for the free-text option.
  `.opt.chosen .key` → border `rgba(16,185,129,0.45)`, color `#34d399`.
- Label line (`flex; gap:8px; flex-wrap`): `.opt-label` (13px, weight 500,
  `#f1f5f9`), optionally an emerald chip **`Recommended`** (live cards only), or
  on a settled+chosen option an emerald chip **`Chosen`** with a 10 px lucide
  `check`.
- `.opt-desc` — `margin: 4px 0 0; font-size:11.5px; line-height:1.6;
  color:#94a3b8`, rendered `display:block`.

**Free-text option** (live cards only), rendered after the options:
`.opt.live` with key `↵`, label **`Something else`**, description verbatim:
**"Answer in your own words below. The interviewer takes it as written rather
than fitting it to the nearest option."**
Clicking it **focuses the composer input** — it does not submit anything.

**Custom-answer record** (settled cards where the user typed instead of picking):
`margin-top:10px; radius:8px; border:1px solid rgba(6,182,212,0.20);
background: rgba(6,182,212,0.06); padding:10px 12px`, containing a
`.sender.s-user` line **`You answered in your own words`** (`margin-bottom:4px`)
and the text at `font-size:12.5px; line-height:1.6; color:#e2e8f0;
white-space:pre-wrap`.

Sequencing rule from the mock logic: a question renders only if it is settled or
it is the next unsettled one and nothing is pending — **one open question at a
time**. Each settled question is followed by the interviewer's reply bubble; when
the user answered in free text instead of picking, the reply is the fallback,
verbatim:
**"Taking that as written rather than fitting it to the nearest option.\n\nIt
changes what the ticket has to prove, so it lands in the acceptance rather than
the description — and I will say in the ticket that it came from you, not from
me."** · meta `read 3 files · 7.0k tokens · $0.18`

##### The three seeded questions (copy verbatim — use as fixtures)

**Q1 — header chip `First move`.**
"The sketch leaves two things open, and they are not independent. Which do you
want to settle first?"

1. **`Identity, then leases`** *(Recommended)* — "A lease needs an owner, so
   identity comes first either way. This is the order the work already has."
   · reply: "Then the registry has to land before authentication does: a keypair
   is only useful once there is a row to attach it to, and the revoke you want is
   a write against that row.\n\nOne constraint that decides a ticket, so it is
   worth saying out loud: key material cannot go in SQLite. AGENTS.md is
   unconditional about that." · meta `read 4 files · ran cargo metadata · 9.8k tokens · $0.24`
2. **`Leases, then identity`** — "Workable only if you will lease a worktree to a
   client you cannot yet name — which means the lease cannot be revoked."
   · reply: "Then the first version of a lease is held by whoever asked for it,
   and released by whoever asks. That is not a lease so much as a lock with no
   owner.\n\nI will write it that way, and put the revoke gap in the ticket rather
   than leave the next reader to find it." · meta `read 3 files · 7.2k tokens · $0.18`
3. **`Both in one pass`** — "One ticket, one worktree, one PR. Faster to merge,
   and impossible to review in pieces." · reply: "That is one ticket touching the
   registry, the auth path and the worktree layer at once. It will merge as a
   single reviewable unit or not at all.\n\nSay the word if the review cost turns
   out to matter more than the merge count." · meta `read 3 files · 6.9k tokens · $0.17`

**Q2 — header chip `Identity`.** "How should a client prove who it is?"

1. **`An ed25519 keypair per client`** *(Recommended)* — "The operator installs one
   public key per laptop. Revoking one laptop is a one-line edit that touches
   nobody else. Costs a distribution step at setup." · reply: "Then DSC-2 is the
   key exchange and nothing else, and revoke becomes its own ticket — they touch
   different surfaces and want different acceptance.\n\nThe desktop side is a
   keyring read; the runner side is a public key in a file the operator installs.
   Nothing secret is ever a thing Demeteo holds."
   · meta `read 5 files · ran rg, git log · 11.2k tokens · $0.28`
2. **`One shared token`** — "What the old sketch says. Nothing to distribute, and
   revoking one laptop means rotating every laptop." · reply: "Buildable in an
   afternoon, and it forecloses the thing you opened with: the first revoke is an
   outage for everyone.\n\nI will write it as asked and put the rotation cost in
   the ticket description, where the agent implementing it will read it."
   · meta `read 3 files · 6.4k tokens · $0.16`
3. **`Reuse the SSH host key the runner already trusts`** — "No new secret
   anywhere, and it rides transport you already trust. Ties a client to a host, so
   one machine with two users is one client." · reply: "That gets identity with no
   new secret, which is real. The cost is that a client is a host: two people on
   one machine are one client, and revoking either revokes both.\n\nThat holds
   only while a laptop has one user. Worth writing into the ticket as an
   assumption rather than leaving it implied." · meta `read 4 files · 8.1k tokens · $0.20`

**Q3 — header chip `Refusal`.** "A client's key is refused halfway through a run
it started. What happens to the run?"

1. **`Let it finish; refuse the next one`** *(Recommended)* — "The run already owns
   a worktree and a branch. Killing it strands both, and the operator revoked a
   laptop, not a branch." · reply: "Then refusal gates accepting work, not work
   already accepted. The runner keeps the state machine it has — no new terminal
   status, nothing to reclaim mid-run.\n\nThat is the whole shape. Three tickets
   fall out of it, and one of them can start now."
   · meta `read 2 files · 6.1k tokens · $0.15`
2. **`Kill it and reclaim the worktree`** — "Cleanest state, and it discards
   whatever the agent already committed — recoverable from the branch, if it got
   that far." · reply: "Then the runner needs a reclaim path that is safe against a
   worktree an agent is still writing to. `git worktree remove` on a live checkout
   is where this usually goes wrong, so it is its own ticket and it is not
   small.\n\nFour tickets rather than three." · meta `read 3 files · 7.4k tokens · $0.19`
3. **`Park it and wait for a decision`** — "A gate the operator clears later. One
   more state to build, and one more thing that can sit forever." · reply: "Then it
   is a gate, and gates already exist — this is a new reason to raise one, not a
   new mechanism.\n\nIt does add a state that can sit indefinitely, so the ticket
   should name who is expected to clear it. Three tickets, one of which is a gate."
   · meta `read 3 files · 6.8k tokens · $0.17`

Q1 is **pre-answered on load** (`identity-first`) so the settled shape is visible
immediately.

#### 3.4.5 Advisory strip (`sc-if advisory`)

Shown once every question is settled and nothing is pending. `margin: 0 16px 10px;
gap:8px; radius:8px; border 1px solid rgba(255,255,255,0.05); background:
rgba(255,255,255,0.02); padding:8px 12px; font-size:11px; color:#94a3b8; flex:none`.
13 px lucide `info` (`circle` + `M12 16v-4` + `M12 8h.01`) stroked `#a78bfa`.
Copy verbatim:
**"The interviewer sees nothing left to settle. Decompose whenever you want — or
keep going."**

Per §5.1 this is **advisory only** — the agent may signal, the user ends the
interview. Nothing here disables the composer or auto-navigates.

#### 3.4.6 Composer

`padding: 14px 16px; background: rgba(13,15,20,0.9); border-top:1px solid
rgba(255,255,255,0.05); flex:none`.

- **Awaiting hint** (`sc-if awaiting`, i.e. an open question exists and nothing
  is pending): `margin: 0 0 8px; font-size:11px; color:#64748b`, verbatim:
  **"Pick an option above, or answer here — both settle the same question."**
- Input row (`gap:10px; align-items:flex-end`):
  - `<input type="text">` `flex:1`, bg `rgba(10,12,16,0.6)`, border
    `rgba(255,255,255,0.05)`, radius 8px, `padding:10px 14px`, `#f3f4f6`, Inter
    13px, no outline. Placeholder is state-dependent:
    - awaiting an answer → **`Answer in your own words...`**
    - otherwise → **`Ask, answer, or push back...`**
    - `Enter` sends (preventDefault); no Shift+Enter newline in the mock.
  - `btn-s` (`padding:9px 12px`), `title="Attach a file or image"`, lucide
    `paperclip`.
  - `btn-p` (`padding:9px 14px`), lucide `send` — no text label.
- Attachment row (`margin-top:8px; gap:8px`): `.lbl` **`Attachments`** then slate
  chips **`MULTI_CLIENT_RUNNER.md`**, **`runner-topology.png`**.

**Send semantics from the mock:** if an unsettled question remains, the typed text
becomes that question's custom answer; otherwise it is appended as a plain user
turn. Both then set `pending` (streaming) for ~1.7 s, then raise the toast.

#### 3.4.7 Toast

`position:absolute; right:24px; bottom:24px; gap:10px; padding:12px 16px;
radius:12px; border:1px solid rgba(16,185,129,0.25); background:
rgba(6,20,16,0.95); box-shadow: 0 8px 32px rgba(0,0,0,0.5); font-size:12px`.
Emerald dot, then Outfit weight 600 `#f3f4f6` **`Turn complete`** and a second
line at `#94a3b8` 11px: **`Runner serves more than one client · 41s`**
(discovery title · turn duration). Auto-dismiss after 4.5 s in the mock.

### 3.5 Ticket graph / board column (flex:1)

`flex:1; column; min-width:0`.

#### 3.5.1 Sub-header (38 px)

Same bar as §3.4.1. Left: a `.seg` segmented control. Right: progress readout.

`.seg`: `display:flex; gap:2px; padding:2px; radius:8px; border:1px solid
rgba(255,255,255,0.05); background: rgba(5,6,8,0.6)`. Buttons:
`border:none; background:transparent; color:#64748b; font-size:11px; weight:500;
padding:4px 10px; radius:6px; inline-flex; gap:5px`. Hover `#cbd5e1`.
Selected (`.on`): `background: rgba(139,92,246,0.14); color:#c4b5fd;
box-shadow: inset 0 0 0 1px rgba(139,92,246,0.25)` — **violet selection**.

| Segment | Icon (lucide) |
|---|---|
| **`Graph`** | `workflow` — two 6×6 rects at (3,3) and (15,15) plus `M6 9v3a3 3 0 0 0 3 3h6` |
| **`Board`** | `kanban`-ish — rects `6×18 @ (3,3)` and `6×11 @ (15,3)` |

Right cluster (`gap:10px`): Fira Code 10px `#94a3b8` progress text, then a
`.bar` (`display:flex; width:96px; height:4px; radius:999px; overflow:hidden;
background: rgba(255,255,255,0.06)`) with an emerald `#10b981` segment and a cyan
`#06b6d4` segment; the bar's `title` repeats the text.

Progress string is computed, never stored:
`` `${landed} of ${live} landed · ${running} in flight` `` where `live` excludes
dropped tickets. Computed from the seeded tickets this is
**`1 of 6 landed · 1 in flight`** (7 tickets, DSC-6 dropped) — note the
ProjectHome card for the same discovery reads `1 of 7 landed · 1 in flight` and
its static bar segments are `14.3%` (1/7). The mocks disagree; **the derived
value (excluding dropped) is the one to ship**, and the two surfaces must use
the same helper.

#### 3.5.2 View shell

`flex:1; position:relative; min-height:0`. **Both views are absolutely placed
(`position:absolute; inset:0`) inside this one shell** — the mock's own comment
says why: *"a branch must not have to carry flex sizing through whatever wraps
it."* Keep that structure.

#### 3.5.3 Graph view (default)

Canvas `.well`: `background-color:#050608` with a dot grid —
```css
background-image: radial-gradient(#334155 1px, transparent 1px);
background-size: 20px 20px;
```
(`#050608` is `--bg-well` / `.panel-field`; the dot grid is new.)

**Edges** — one `<svg class="edges {{selKey}}" viewBox="0 0 660 700"
preserveAspectRatio="none">` sized `660 × 700` at `left:12px; top:16px`,
`position:absolute; inset:0; overflow:visible; pointer-events:none`.
`.edges path { fill:none; stroke:#334155; stroke-width:1.5 }`.
`.edges path.met { stroke: rgba(16,185,129,0.45) }` — a **satisfied** prerequisite
edge is emerald; an unmet one is `#334155`.

Selection highlighting is pure CSS: the svg carries `sel-t3` etc., each path
carries `f-<from>` and `t-<to>` classes, and one rule lights the incident edges:

```css
.sel-t1 .f-t1, .sel-t2 .f-t2, .sel-t3 .f-t3, .sel-t3 .t-t3, .sel-t4 .t-t4,
.sel-t5 .f-t5, .sel-t5 .t-t5, .sel-t7 .t-t7 { stroke: #22d3ee; stroke-width: 2; }
```

That enumeration is a mock shortcut. In the real build, derive the highlight from
the selected node's incident edges rather than hand-listing pairs.

The six seeded paths:

```
f-t1 t-t3 met  M160 148 C160 164, 160 164, 160 180
f-t2 t-t3      M460 148 C460 168, 160 160, 160 180
f-t1 t-t4 met  M160 148 C160 168, 460 160, 460 180
f-t3 t-t5      M160 304 C160 320, 160 320, 160 336
f-t3 t-t7      M160 304 C160 400, 310 400, 310 492
f-t5 t-t7      M160 460 C160 476, 310 476, 310 492
```

**Nodes** — absolutely positioned inside a `660 × 700` layer at the same offset.
`.node`: `position:absolute; width:280px; radius:12px; border:1px solid
rgba(51,65,85,0.6); background: rgba(15,23,42,0.72); backdrop-filter: blur(4px);
padding:10px 14px; cursor:pointer; box-shadow: 0 10px 15px -3px rgba(0,0,0,0.35);
transition: border-color .15s ease, box-shadow .15s ease`. Hover → border `#475569`.

| Modifier | Border / glow | Meaning |
|---|---|---|
| `.n-emerald` | `rgba(16,185,129,0.40)`, no glow | landed |
| `.n-cyan` | `rgba(6,182,212,0.50)` + `0 0 18px rgba(6,182,212,0.18)` | running |
| `.n-violet` | `rgba(139,92,246,0.50)` + `0 0 18px rgba(139,92,246,0.18)` | ready |
| `.n-amber` | `rgba(245,158,11,0.50)` + `0 0 18px rgba(245,158,11,0.20)` | blocked, *close* to ready |
| `.n-dropped` | `opacity:0.5; border-style:dashed` | dropped |
| *(none)* | base slate | blocked, far from ready |
| `.node.sel` | `rgba(34,211,238,0.70)` + `0 0 0 1px rgba(34,211,238,0.4), 0 0 18px rgba(34,211,238,0.25)` | selected (cyan) |

Node internals:
- Row (`gap:10px`): a 30×30 verdict tile — `class="chip {{tint}}"` overridden to
  `width:30px; height:30px; padding:0; justify-content:center; border-radius:8px;
  position:relative; flex:none` — carrying **one 15 px lucide icon chosen by
  bucket**. The mock's comment: *"The tile carries the verdict, not just the
  tint: an agent-built plan is read for what is done first."*

  | Bucket | Icon | Notes |
  |---|---|---|
  | landed | `check` (`M20 6 9 17l-5-5`), stroke-width 2.5 | — |
  | running | `activity` (`M22 12h-4l-3 9L9 3l-3 9H2`) | carries `class="pulse"` |
  | ready | `arrow-right` (`M5 12h14` + `m12 5 7 7-7 7`) | — |
  | blocked | `lock` (rect 18×11 @ (3,11) + `M7 11V7a5 5 0 0 1 10 0v4`) | — |
  | dropped | `circle-minus` (`circle r=10` + `M8 12h8`) | — |

- Text block (`min-width:0; flex:1`): ticket id in Fira Code 9px `#64748b`
  `letter-spacing:0.06em`; title 13px weight 500, colour by bucket —
  dropped `#64748b` + `text-decoration: line-through`, landed `#cbd5e1`,
  everything else `#f1f5f9`.
- Footer (`margin-top:8px; padding-left:40px; gap:6px; flex-wrap`): state chip in
  the node's tint, then a Fira Code 9px `#64748b` note.

**Legend** — `.legend`: `position:absolute; left:16px; bottom:16px; gap:12px;
padding:6px 12px; radius:999px; border:1px solid rgba(255,255,255,0.05);
background: rgba(15,23,42,0.9); backdrop-filter: blur(12px); font-size:10px`.
Five dot+label pairs (`.k { gap:5px; color:#94a3b8 }`), verbatim and in order:

| Label | Dot |
|---|---|
| `Blocked` | `#fbbf24` amber |
| `Ready` | `#a78bfa` violet |
| `In flight` | `#22d3ee` cyan |
| `Landed` | `#34d399` emerald |
| `Dropped` | `#64748b` slate |

**Zoom controls** — `position:absolute; right:16px; bottom:16px; gap:6px`; three
`btn-s` at `padding:6px 10px` with `background: rgba(15,23,42,0.9)`, labelled
**`−`** (`&minus;`), **`+`**, and **`Fit`** (the last at `font-size:11px`).

**Interaction:** clicking a node selects it (drives the inspector and the edge
highlight). **There is no drag.** Node positions are fixed `x`/`y` literals in
the mock and no drag handler, drag cursor, or drop target appears anywhere in the
graph. Layout is computed, not user-arranged. Pan is likewise absent — only the
three zoom buttons hint at viewport control, and they are unwired.

#### 3.5.4 Board view

`.scroll` at `position:absolute; inset:0; overflow-y:auto; background:#050608;
padding:16px 12px; column; gap:18px`. **Five lanes, always all five, in this order**:

| Lane | Colour | Note (right of the rule) |
|---|---|---|
| `Blocked` | `#fbbf24` | `waiting on an edge` |
| `Ready` | `#a78bfa` | `you start these` |
| `In flight` | `#22d3ee` | `PR open` |
| `Landed` | `#34d399` | `merged into master` |
| `Dropped` | `#64748b` | `decided against, with a reason` |

Lane header `.lane-hd` (`gap:8px`): a dot in the lane colour · `.nm` (Outfit 11px
weight 600, `letter-spacing:0.02em`, lane colour) · `.ct` (Fira Code 10px
`#64748b`) holding the card count · `.rule` (`flex:1; height:1px; background:
rgba(255,255,255,0.05)`) · `.ct` holding the lane note.

Cards wrap in a `flex-wrap` row, `gap:10px`. `.tcard`: `width:190px; radius:10px;
border:1px solid rgba(51,65,85,0.6); background: rgba(15,23,42,0.72);
padding:10px 12px; cursor:pointer`. Hover → border `#475569`, `translateY(-1px)`.
`.tcard.sel` gets the same cyan selection ring as `.node.sel`. The bucket
modifiers (`n-emerald`, `n-cyan`, …) are reused on the card, plus `.done` for
landed. Title treatments:
`.tcard .ttl { font-size:12px; weight:500; color:#f1f5f9; line-height:1.4 }`,
`.tcard.done .ttl { color:#94a3b8 }`,
`.tcard.n-dropped .ttl { color:#64748b; text-decoration:line-through;
text-decoration-color: rgba(100,116,139,0.5) }`.

Card internals: top row = ticket id (Fira Code 9px `#64748b`
`letter-spacing:0.06em`) and an agent chip in the ticket tint at
`padding:1px 6px`; then the title; then a footer row (`margin-top:8px; gap:6px`)
of a lane-coloured dot and the ticket note in Fira Code 9px `#64748b`.

**Empty lane:** `<p style="margin:0; font-size:11px; color:#475569">` reading
**`Nothing here.`**

**No drag between lanes.** Lanes are derived from the edge graph on every render
(§6.3) — there is no stored column and nothing to drop into. The only card
interaction is click-to-select.

#### 3.5.5 The seven seeded tickets

These are the fixtures for both views and the inspector.

| key | id | bucket | tint / node class | state | note | x,y |
|---|---|---|---|---|---|---|
| t1 | `DSC-1` | landed | `c-emerald` / `n-emerald` | `Landed` | `PR #131 merged` | 20,24 |
| t2 | `DSC-2` | running | `c-cyan` / `n-cyan` | `Running` | `PR #134 open` | 320,24 |
| t3 | `DSC-3` | blocked | `c-amber` / `n-amber` | `Blocked` | `waiting on DSC-2` | 20,180 |
| t4 | `DSC-4` | ready | `c-violet` / `n-violet` | `Ready` | `every prerequisite landed` | 320,180 |
| t5 | `DSC-5` | blocked | `c-slate` / *(none)* | `Blocked` | `waiting on DSC-3` | 20,336 |
| t6 | `DSC-6` | dropped | `c-slate` / `n-dropped` | `Dropped` | `folded into RUNNER_DEV.md` | 320,336 |
| t7 | `DSC-7` | blocked | `c-slate` / *(none)* | `Blocked` | `waiting on DSC-3, DSC-5` | 170,492 |

Note the two grades of blocked: **amber** when a prerequisite is at least in
flight (DSC-3 waits on an open PR), **slate** when the prerequisite has not even
started (DSC-5, DSC-7). That distinction is visible in the tint, the node border
and the inspector verdict colour, and it is worth preserving.

Titles: DSC-1 `Session registry keyed by client id` · DSC-2 `Authenticate clients
by ed25519 keypair` · DSC-3 `Multiplex run streams over one connection` ·
DSC-4 `Desktop picks a runner identity per project` · DSC-5 `Fair-share
scheduling across clients` · DSC-6 `Operator guide for multi-client runners` ·
DSC-7 `Topology conformance for two clients`.

### 3.6 Inspector column (360 px in `'three-up'`)

`.scroll` at `width:360px; flex:none; border-left:1px solid rgba(255,255,255,0.05);
background: rgba(13,15,20,0.7); overflow-y:auto; min-height:0`. This is the
`'three-up'` width and placement (§3.2.1): the inspector renders in-row as the
row's third column only in that mode. In `'overlay-inspector'` and `'stacked'`
it renders unchanged but inside `TicketOverlayPanel` instead, docked to the
right edge of the workspace rather than occupying a column.

**Sticky sub-header** (`padding:0 16px; height:38px; background: rgba(18,22,30,0.6);
border-bottom:1px solid rgba(255,255,255,0.05); position:sticky; top:0; z-index:2`):
ticket id in Outfit 12px weight 500 `#9ca3af`, and the state chip in the ticket
tint.

Body: `padding:16px; column; gap:18px`. Sections in order:

1. **Title + description.** `<h2>` Outfit 600 / 16px / `#fff` /
   `line-height:1.35; margin: 0 0 6px`; `<p>` 12px `#9ca3af` `line-height:1.6`.
2. **Verdict card.** `radius:12px; border:1px solid {{verdictBorder}};
   background:{{verdictBg}}; padding:12px 14px`. Header row (`gap:8px`): dot in
   `{{verdictColor}}`, then Outfit 13px weight 600 in the same colour.
   Below: `<p>` `margin: 6px 0 0; font-size:11px; line-height:1.6; color:#94a3b8`.
   The mock comments this: *"computed from the edges every time this renders."*

   | Ticket | Verdict | Colour | Border | Bg | Why (verbatim) |
   |---|---|---|---|---|---|
   | DSC-1 | `Started` | `#34d399` | `rgba(16,185,129,0.20)` | `rgba(16,185,129,0.05)` | "Its PR merged into master 3 days ago. Read from the forge, not from the run." |
   | DSC-2 | `Started` | `#22d3ee` | `rgba(6,182,212,0.20)` | `rgba(6,182,212,0.05)` | "Running now. Its PR is open, so nothing waiting on it has been released yet." |
   | DSC-3 | `Blocked` | `#fbbf24` | `rgba(245,158,11,0.20)` | `rgba(245,158,11,0.05)` | "One of two prerequisites has landed. Recomputed from the edges on every read — there is no readiness column to drift." |
   | DSC-4 | `Startable` | `#a78bfa` | `rgba(139,92,246,0.20)` | `rgba(139,92,246,0.05)` | "Its one prerequisite merged. Demeteo says so; it does not start anything on its own." |
   | DSC-5 | `Blocked` | `#94a3b8` | `rgba(255,255,255,0.05)` | `rgba(255,255,255,0.02)` | "DSC-3 has not started, so it has no PR to read a verdict from." |
   | DSC-6 | `Dropped` | `#94a3b8` | `rgba(255,255,255,0.05)` | `rgba(255,255,255,0.02)` | "Dropped with a reason, which satisfies its dependents the same way a closed PR does. The record of the decision stays." |
   | DSC-7 | `Blocked` | `#94a3b8` | `rgba(255,255,255,0.05)` | `rgba(255,255,255,0.02)` | "Two prerequisites, neither started." |

3. **`Prerequisites`** (`.lbl`, `display:block; margin-bottom:8px`). Column,
   `gap:8px`. Each row: `radius:8px; border:1px solid rgba(255,255,255,0.05);
   background: rgba(255,255,255,0.02); padding:8px 10px; gap:10px;
   align-items:flex-start` — a dot (`margin-top:6px`) in the prerequisite's
   colour, a title at 12px `#e2e8f0`, a Fira Code 10px `#64748b` note
   (`margin-top:2px`), and a state chip.
   Prerequisite notes verbatim: `PR #131 merged into master` (state `Landed`,
   emerald), `PR #134 open · last read 40s ago` (state `Waiting`, cyan),
   `not started — no PR to read` (state `Waiting`, slate).
   Empty case: `<p style="margin:0; font-size:11px; color:#64748b">` verbatim
   **"None. Nothing in this discovery gates it."**
4. **`Execution`** — a wrapping chip row (`gap:6px`): workflow (violet), agent
   (cyan), model (violet), **`effort {value}`** (slate).
   Values in the fixtures: `Standard Feature` / `UI Feature` / `Docs`;
   `claude-code` / `opencode`; `opus` / `sonnet`; `high` / `medium` / `low`.
5. **`Acceptance`** — a `<ul>` at `padding-left:16px; font-size:12px;
   line-height:1.7; color:#cbd5e1`, one `<li>` per criterion. A dropped ticket
   shows a single `—`.
6. **`Files`** — a column at `gap:4px` of Fira Code 11px `#94a3b8` paths, plain
   text (not links in the mock).
7. **`What its agent will be told`** (§7.2) — a sunk well:
   `radius:12px; border:1px solid rgba(255,255,255,0.05); background:
   rgba(5,6,8,0.8); padding:12px 14px`, `.lbl` header, then Fira Code 11px
   `#94a3b8` `line-height:1.7; white-space:pre-wrap`. Mock strings verbatim:
   - DSC-1: `No prerequisites in this discovery.`
   - DSC-2 / DSC-4: `DSC-1 landed on master. Its code is in your base branch.`
   - DSC-3: `DSC-1 landed on master.\nDSC-2 has not — its PR is still open, so its code is not in your base branch.`
   - DSC-5: `DSC-3 has not landed — it has not started, so none of its work exists.`
   - DSC-6: `Not started. This ticket was dropped.`
   - DSC-7: `DSC-3 has not landed — it has not started.\nDSC-5 has not landed — it has not started.`
8. **Action row** (`gap:8px`): `btn-p` at `flex:1` carrying a state-dependent
   label and disabled flag, plus a `btn-s` **`Edit`** (opens §5).

   | Ticket | Primary label | Disabled | Force row |
   |---|---|---|---|
   | DSC-1, DSC-2 | `Open feature` | no | no |
   | DSC-3 | `Blocked by DSC-2` | yes | yes |
   | DSC-4 | `Start ticket` | no | no |
   | DSC-5 | `Blocked by DSC-3` | yes | yes |
   | DSC-6 | `Dropped` | yes | no |
   | DSC-7 | `Blocked by 2 tickets` | yes | yes |

9. **Force-start row** (`sc-if showForce`): a full-width `btn-s` with
   `border-color: rgba(245,158,11,0.25); color:#fbbf24`, label verbatim
   **`Force start with a reason…`** (`&hellip;`). Opens the ticket editor's force
   flow (§5.7).

### 3.7 States on this artboard

| State | Rendering |
|---|---|
| Question open | live `.qcard`, `Needs you` chip, awaiting hint above the composer, placeholder `Answer in your own words...` |
| Question settled | `.qcard.done`, `Answered` chip, chosen option emerald, others `.faded` |
| Answered in free text | settled card + cyan "You answered in your own words" panel + fallback reply |
| Streaming | pending agent bubble with blinking caret and pulsing sender dot; option clicks and send are ignored while pending |
| Interview exhausted | advisory strip appears; placeholder becomes `Ask, answer, or push back...` |
| Turn complete | emerald toast, auto-dismissing |
| Graph / Board | segmented control; both read the same derived buckets |
| Ticket selected | cyan ring on node/card, incident edges cyan, inspector reflects it |
| Blocked / ready / in flight / landed / dropped | see the bucket tables above |

No empty-graph state ("not decomposed yet") is drawn. It is needed: a Discovery
in `Interviewing` before its first Decompose has zero tickets. Reuse
`EmptyStateCard` in the graph shell.

---

## 4. Decompose — proposed changes

`Decompose.dc.html` · 1040 × 880.

**Annotation (canvas.json):** *"§5.3 — re-decomposing is additive. Started tickets
are locked, and applying never renumbers."*

### 4.1 Purpose and placement

Modal raised by **Decompose** in the workspace header (§3.3). It is a **review-
and-apply diff**, not a wizard: the interviewer has produced a proposal and the
user chooses which parts of it land. The outer artboard box carries
`radial-gradient(circle at 70% 10%, rgba(139,92,246,0.05) 0%, transparent 55%)`
as backdrop.

### 4.2 Frame

Same glass dialog shell as §2.2, `padding: 32px` around it in the artboard.
Column flex, `min-height:0`, `overflow:hidden`.

**Header** (`padding: 20px 24px`, bottom border, `justify-content:space-between;
align-items:flex-start; gap:20px; flex:none`):
- Eyebrow (Outfit 11px / 600 / uppercase / `letter-spacing:0.15em` / `#22d3ee`):
  **`Second pass`** — this is the *nth-pass* label; a first decomposition should
  read `First pass`.
- `<h2>` Outfit 700 / 22px / `#fff`: **`Proposed changes`**
- `<p>` (`margin: 8px 0 0; font-size:12px; line-height:1.6; color:#94a3b8;
  max-width:620px`), verbatim:
  **"Nothing here is applied until you apply it. Tickets that already have a
  feature cannot be revised, removed or renumbered — they are listed so you can
  see what the interviewer worked around."**
- A `btn-s` close button at `padding:6px 10px; flex:none` holding a 16 px lucide `x`.

**Validation bar** (`padding: 12px 24px`, bottom border, `gap:16px; flex:none;
background: rgba(16,185,129,0.04)`):
- Emerald chip with an 11 px `check` icon: **`Schema valid`**
- 11px `#94a3b8` sentence, verbatim (the mono span is `DSC-5 → DSC-3 → DSC-5` in
  `#cbd5e1`):
  **"One cycle was refused while the interviewer still had the graph in context —
  it re-authored `DSC-5 → DSC-3 → DSC-5` as a single edge. Nothing invalid reaches
  a ticket row."**

  This bar is the **only** validation surface drawn. A failing state is not
  mocked; render it in the same slot with a ruby chip and the refusal reason.

**Body** `.scroll`: `flex:1; overflow-y:auto; padding: 20px 24px; column;
gap:22px; min-height:0`.

**Footer** (`padding: 16px 24px`, top border, `background: rgba(13,15,20,0.9)`,
`justify-content:space-between; gap:20px; flex:none`):
- Left `<p>` 11px `#64748b`, verbatim:
  **"Ticket ids are stable. Applying this never renumbers DSC-1 through DSC-7."**
- Right (`gap:10px`): `btn-s` **`Keep talking`** (returns to the interview) and
  `btn-p` **`Apply {n} of 6 changes`** — the label is live and recomputes as
  checkboxes toggle (`'Apply ' + count + ' of 6 changes'`).

### 4.3 The four groups, in order

Each group is a `<div>` with a header row (`gap:10px; margin-bottom:10px`)
holding a coloured `.lbl` and a Fira Code 10px `#475569` count, then a column of
cards at `gap:10px`.

| Group | Label colour | Count copy |
|---|---|---|
| `Added` | `#34d399` emerald | `3 new tickets` |
| `Revised` | `#fbbf24` amber | `2 unstarted tickets` |
| `Removed` | `#f87171` ruby | `1 unstarted ticket` |
| `Locked` | default `#64748b` slate | `2 tickets have a feature` |

### 4.4 Card and checkbox

`.card`: `radius:12px; border:1px solid rgba(255,255,255,0.05); background:
rgba(255,255,255,0.02); padding:14px 16px; display:flex; gap:14px;
align-items:flex-start`.
`.card.pick { cursor:pointer }` + hover → border `rgba(255,255,255,0.12)`,
background `rgba(255,255,255,0.04)`.
`.card.off { opacity:0.45 }` — a deselected change.
`.card.lock { background: rgba(5,6,8,0.6) }` — sunk, non-interactive.

`.box` (the checkbox): `18×18; radius:5px; border:1px solid rgba(255,255,255,0.15);
centred; flex:none; margin-top:2px; color:transparent`, holding a 12 px `check`
at stroke-width 3.
`.box.on { background:#8b5cf6; border-color:#8b5cf6; color:#fff }` — **violet
check, the primary-action colour**.
`.box.dis { border-style:dashed; border-color: rgba(255,255,255,0.08) }` — used on
locked rows, which show an 11 px `lock` icon stroked `#475569` instead of a check.

**The whole card is the click target** (`onClick` sits on `.card.pick`), not just
the box. Every Added / Revised / Removed card starts checked.

### 4.5 Added cards

Content column: id (Fira Code 10px `#64748b`) + title (14px weight 500
`#f1f5f9`) on one row at `gap:10px`; a `why` paragraph (`margin: 6px 0 0;
font-size:12px; line-height:1.6; color:#94a3b8`); then a chip row
(`margin-top:8px; flex-wrap; gap:6px`): workflow (violet), agent (cyan), and a
dependency chip whose tint varies.

Verbatim fixtures:

| id | Title | Why | Workflow | Agent | Dep chip |
|---|---|---|---|---|---|
| `DSC-9` | `Revoke one client without restarting the runner` | "Split out of DSC-2 in this pass: the key exchange and the revoke path touch different surfaces and want different acceptance." | `Standard Feature` | `claude-code` | `blocked by DSC-2` (amber) |
| `DSC-10` | `Lease a worktree to one client at a time` | "The question the old sketch never answered. It needs the registry, which landed, and identity, which has not." | `Standard Feature` | `claude-code` | `blocked by DSC-2` (amber) |
| `DSC-11` | `Surface which runner answered, per project` | "Learned while implementing DSC-1: with two clients the desktop cannot say which runner it reached, and neither can a log." | `UI Feature` | `opencode` | `no prerequisites` (slate) |

New ids continue from the existing high-water mark (DSC-8 was proposed and is
being removed in the same pass; numbering does not reuse it).

### 4.6 Revised cards

Same header row, then a **field-diff well**: `margin-top:10px; radius:8px;
background: rgba(5,6,8,0.7); border:1px solid rgba(255,255,255,0.04);
padding:10px 12px; column; gap:6px`. Each diff row is
`display:flex; gap:10px; align-items:baseline`:
- field name — `.lbl` at `width:92px; flex:none`
- values — Fira Code 11px `line-height:1.7`, two stacked lines:
  `.was { color:#f87171; text-decoration:line-through; text-decoration-color:
  rgba(248,113,113,0.5) }` over `.now { color:#34d399 }`.

Then the `why` paragraph (`margin: 8px 0 0`, same treatment as Added).

Verbatim fixtures:

- **`DSC-5` — `Fair-share scheduling across clients`**
  - `blocked by`: was `DSC-3` → now `DSC-3, DSC-10`
  - `test command`: was `cargo test -p demeteo-runner` → now `npm run checks:code`
  - why: "Leases decide what fair-share is scheduling over, so it has to land first."
- **`DSC-7` — `Topology conformance for two clients`**
  - `acceptance`: was `2 criteria` → now `3 criteria — adds the single-client baseline from DSC-8`
  - why: "Absorbs the benchmark rather than keeping a ticket whose only job was to compare against this one."

### 4.7 Removed card

One card, verbatim:
- `DSC-8` — **`Bench the mux against a single-client baseline`**
- Why: **"Folded into DSC-7's acceptance. Removing it here is not the same as
  dropping it — a dropped ticket keeps its reason and releases what waited on it;
  this one was never started and nothing points at it."**

The distinction between *removed* (never started, no dependents, vanishes) and
*dropped* (§6.6 — keeps its reason, satisfies dependents) is the whole point of
that sentence. Do not paraphrase it.

### 4.8 Locked group

Two `.card.lock` rows, no toggle. Content is one row: id, title in `#94a3b8`
(dimmed, unlike the editable cards' `#f1f5f9`), and the ticket's live state chip.

- `DSC-1` · `Session registry keyed by client id` · chip **`Landed`** (emerald)
- `DSC-2` · `Authenticate clients by ed25519 keypair` · chip **`Running`** (cyan)

### 4.9 States

| State | Rendering |
|---|---|
| Change selected (default) | `.box.on` violet check, card at full opacity |
| Change deselected | empty box, `.card.off` (45 % opacity), Apply count decrements |
| Locked | dashed box + lock icon, sunk background, no hover, no pointer |
| Schema valid | emerald chip + explanatory sentence |
| All deselected | Apply label reads `Apply 0 of 6 changes` — **the mock does not disable the button; it should be disabled at 0** |

No loading, applying, or schema-invalid state is drawn.

---

## 5. Ticket — edit and force start

`TicketEdit.dc.html` · 760 × 1560 (a tall scrolling panel, not a dialog).

**Annotation (canvas.json):** *"§4.6 + §6.5 + §7.2 — attachments are the launch
dropzone verbatim, staged until the ticket starts. Switch the model to
qwen3-coder-480b for the vision warning, remove a chip, or type a force-start
reason: the agent briefing near the bottom picks all three up."*

### 5.1 Purpose and placement

The full editor for one Ticket, reached from **Edit** in the inspector (§3.6.8)
or **Force start with a reason…**. Its frame is an inspector-shaped panel —
`width:760px; background:#0d0f14; border-left:1px solid rgba(255,255,255,0.05);
overflow-y:auto` — i.e. a **wider replacement for the 360 px inspector**, or a
right-hand drawer over the workspace. Not centred, not a modal.

### 5.2 Sticky action header (44 px)

`padding: 0 20px; height:44px; background: rgba(18,22,30,0.6); border-bottom:1px
solid rgba(255,255,255,0.05); position:sticky; top:0; z-index:3;
backdrop-filter: blur(12px)`.

Left (`gap:10px`): id in Fira Code 12px `#9ca3af` (**`DSC-3`**), chip
**`Blocked`** (amber, with dot), chip **`Unstarted`** (slate).
Right (`gap:8px`): `btn-s` **`Discard`** and `btn-p` **`Save ticket`**, both at
`padding:6px 12px…6px 14px; font-size:12px`.

### 5.3 Body

`padding:20px; column; gap:14px`. Opens with a `.hint`
(`font-size:11px; line-height:1.6; color:#64748b`), verbatim:
**"Every field is yours while this ticket has no feature. Starting it locks the
lot — a started ticket is never revised, removed or renumbered."**

Then four `.card` groups, a briefing well, a divider, and the force-start block.

`.card` here is **lighter than a glass panel**: `radius:12px; border:1px solid
rgba(255,255,255,0.05); background: rgba(18,22,30,0.55); padding:14px 16px;
column; gap:14px`. `.card-hd` is `justify-content:space-between; gap:12px` with
its `.lbl` at `margin:0`. The mock's own comment: *"One card per group of fields.
The flat column this replaced gave a twelve-field form no reading order at all."*

### 5.4 Card 1 — `The work`

- **`Title`** — `<input class="fld">`, value `Multiplex run streams over one connection`
- **`Description`** — `<textarea class="fld" rows="4">`, value:
  "Every client watches its own runs down a single connection, with no run
  visible to a client that did not start it. The registry gives the identity to
  filter on; this ticket is the transport."
- **`Acceptance`** — a column of removable rows (`gap:8px`). Each `.row`
  (`display:flex; align-items:center; gap:8px`) is: `.idx`
  (`width:18px; flex:none; text-align:right; Fira Code 10px; color:#475569`)
  holding `1`, `2`, …; an `<input class="fld">`; and an `.x` remove button
  (`title="Remove this criterion"`, 15 px lucide `x`).
  Then an `.add` button at `margin-left:26px` with a 13 px `plus`:
  **`Add criterion`**.
  Values: `Two clients stream concurrently without interleaving`,
  `A client sees only the runs it owns`.
- **Two-column grid** (`1fr 1fr; gap:16px`):
  - **`Files`** — same removable-row pattern, `<input class="fld mono">`,
    `title="Remove this path"`, and an `.add` **`Add path`** (no indent).
    Values: `crates/demeteo-runner/src/stream/mux.rs`,
    `crates/demeteo-core/src/ports/execution.rs`.
  - **`Test command`** — `<input class="fld mono">`, value `npm run checks:code`.
    Below it a `.hint` at `margin-top:8px`, verbatim:
    **"Inside a run, the full `checks` judges commits this ticket never wrote."**
    (`checks` in mono.) This is the AGENTS.md §7 `checks:code` rule surfaced in
    the UI — keep it.

`.fld.mono` = Fira Code, 12px. `.add`: `inline-flex; gap:6px; align-self:
flex-start; background:transparent; border:1px dashed rgba(255,255,255,0.10);
color:#94a3b8; radius:6px; padding:7px 12px; font-size:12px` — hover → border
`rgba(139,92,246,0.35)`, color `#c4b5fd`, background `rgba(139,92,246,0.05)`.
`.x`: `background:transparent; border:none; color:#475569; padding:4px;
radius:6px` — hover → `color:#f87171; background: rgba(239,68,68,0.08)` (**ruby
on destructive hover**).

### 5.5 Card 2 — `Attachments`

Header: `.lbl` **`Attachments`** and a slate chip **`{n} of 10 · staged`**.

The mock reproduces **`AttachmentDropzone` in `launch` mode plus `AttachmentChip`
verbatim** — its comment says so explicitly. **Reuse the components; do not
re-author the markup.** The mock CSS ↔ real class mapping:

| Mock class | Real classes (`src/components/AttachmentDropzone.tsx`, `AttachmentChip.tsx`) |
|---|---|
| `.dz` | `rounded-xl border border-white/10 bg-[rgba(18,22,30,0.75)] backdrop-blur-md p-3 transition-all` |
| `.dz.over` | `border-cyan-400/60 bg-[rgba(18,22,30,0.85)]` |
| `.pick` | `inline-flex items-center gap-2 px-3 py-1.5 rounded-lg border border-violet-500/30 bg-violet-500/10 hover:bg-violet-500/20 text-violet-200 text-xs font-medium transition-colors` |
| `.dzhint` | `flex-1 min-w-0 flex items-center gap-2 text-[11px] font-mono text-slate-400` |
| `.att` | `group relative inline-flex items-center gap-2 rounded-lg border border-white/10 bg-[rgba(18,22,30,0.75)] backdrop-blur-md pr-2 pl-1.5 py-1` |
| `.thumb` | `shrink-0 rounded-md overflow-hidden flex items-center justify-center bg-black/40 border border-white/5 w-9 h-9` |
| `.att-name` | `truncate font-medium text-slate-100 max-w-[180px] text-xs` |
| `.att-size` | `text-[10px] font-mono text-slate-500` |
| `.mime` | `shrink-0 font-mono uppercase tracking-wider text-[9px] px-1.5 py-0.5 rounded-md border border-violet-500/30 bg-violet-500/10 text-violet-300` |
| `.warn` | `flex items-start gap-2 px-3 py-2 rounded-lg border border-violet-500/40 bg-ruby-500/10 text-ruby-200` (from `StartFeatureModal.tsx`) |

Copy, all verbatim and all already in the components:

- Pick button: **`Add files`** with a lucide `upload-cloud`.
- Hint line, with a lucide `file-plus-2`:
  **`or drop here · click to pick · paste an image · png / jpg / webp / gif / pdf / txt · max 100 MB each · 10 per feature`**
- Empty state, with a lucide `file-warning` (`text-slate-600`):
  **`No attachments yet. They will be referenced via [attachment -- <name>].`**
- Chip icons: image → lucide `image` at `text-cyan-300/80`; document → lucide
  `file-text` at `text-slate-400`. Remove button: `X`, `title="Remove {name}"`.
- Vision warning (see §5.5.1), with a lucide `eye-off` at `text-ruby-300`:
  **"Model {model} does not read images"** + **" — attachments will be referenced
  as paths only and not inlined."** and a dismiss `X`.

Seed attachments: `runner-topology.png` · `184.2 KB` · `PNG` · image;
`MULTI_CLIENT_RUNNER.md` · `11.4 KB` · `MARKDO` · doc;
`stream-trace.txt` · `62.8 KB` · `PLAIN` · doc.

Closing `.hint` under the card, verbatim:
**"A ticket has no feature to attach to yet, so these stage here and are
committed the moment it starts — the same path a launch takes. The interview's
own attachments stay with the interview."**

#### 5.5.1 Vision gate

The mock reimplements `modelSupportsImagesByName` and records why:
*"Pessimistic by design: any model that isn't a positive match is treated as
no-vision, so an image is never silently dropped."*

```
lowercase(model)
  contains 'embedding' | 'whisper'                        → false
  contains any of: gpt-4, gpt-5, gemini, claude, vision,
                   opus, sonnet, haiku, fable, minimax     → true
  otherwise                                               → false
```

Warning shows when **an image is attached AND the model fails that test AND the
warning has not been dismissed**. Changing the model resets the dismissal. Use
the real `modelSupportsImagesByName` helper, not a copy.

Model options in the mock: `opus`, `sonnet`, `qwen3-coder-480b`, `deepseek-v3.2`
— the last two trigger the warning.

### 5.6 Card 3 — `Execution`

Header: `.lbl` **`Execution`** plus a right-aligned `.hint`, verbatim:
**"Per ticket — a plan whose parts want different agents can say so."**

A `1fr 1fr` grid at `gap:10px` of four `.selwrap` selects, in order:

1. Workflow — `Standard Feature`, `UI Feature`, `Docs`
2. Agent — `claude-code`, `opencode`, `hermes`
3. Model — from the harness catalog (`opus`, `sonnet`, `qwen3-coder-480b`, `deepseek-v3.2`)
4. Effort — `effort: high`, `effort: medium`, `effort: low`

`.selwrap { position:relative }` with `select.fld { appearance:none;
background-color:#08090c; cursor:pointer; padding-right:30px }` and an absolutely
positioned 14 px lucide `chevron-down` at `right:10px; top:50%; margin-top:-7px;
pointer-events:none; color:#475569`.

### 5.7 Card 4 — `Blocked by`

Column of `.edge` rows (`gap:8px`): `display:flex; align-items:flex-start;
gap:10px; radius:8px; border:1px solid rgba(255,255,255,0.05); background:
rgba(255,255,255,0.02); padding:10px 12px` — dot (`margin-top:6px`) in the
prerequisite's colour, title at 12px `#e2e8f0`, Fira Code 10px `#64748b` note,
state chip, and an `.x` with `title="Remove this edge"`.

Rows verbatim (note the `·` separator between id and title here, unlike the
inspector's space):
- **`DSC-1 · Session registry keyed by client id`** / `PR #131 merged into master`
  / chip **`Landed`** emerald / dot `#34d399`
- **`DSC-2 · Authenticate clients by ed25519 keypair`** / `PR #134 open · last
  read 40s ago` / chip **`Waiting`** cyan / dot `#22d3ee`

Then an `.add` button: **`Add an edge`**.

Closing `.hint`, verbatim:
**"Edges point only at tickets in this discovery. Anything outside it belongs in
the description, sequenced by hand."**

### 5.8 Agent briefing well (§7.2)

`radius:12px; border:1px solid rgba(255,255,255,0.05); background:
rgba(5,6,8,0.8); padding:14px 16px`. `.lbl` **`What its agent will be told`**,
then Fira Code 11px `#94a3b8` `line-height:1.8; white-space:pre-wrap`.

**It is composed, and every editable thing above feeds it** — that is the point
of the artboard. Composition:

```
DSC-1 landed on master. Its code is in your base branch.
DSC-2 has not — its PR is still open, so its code is not in your base branch.
                                            ← (blank line, if any attachments)
Attached: [attachment -- runner-topology.png], [attachment -- MULTI_CLIENT_RUNNER.md], [attachment -- stream-trace.txt]
The image rides as a path only — this model does not read images.     ← only when an image is attached and the model has no vision
                                            ← (blank line, when force-started)
This ticket was started before DSC-2 landed, deliberately:
"<the recorded reason>"
```

Attachment names are joined with `, ` and each is wrapped as
`[attachment -- <name>]`.

### 5.9 Force-start block

Preceded by a 1 px divider (`background: rgba(255,255,255,0.05)`).

Label `.lbl` in `#fbbf24`: **`Start it anyway`**. Then a `<p>` at 12px `#94a3b8`
`line-height:1.6; margin: 0 0 12px`, verbatim:
**"DSC-2's PR is still open, so nothing has released this ticket. Force start
bypasses every edge at once — and records why, for you and for the agent that
reads its own prerequisite list."**

Three phases:

**(a) `idle`** — a row (`gap:10px`):
- `btn-p` **disabled**, label **`Blocked by DSC-2`**
  (`.btn-p[disabled] { opacity:0.35; cursor:not-allowed; box-shadow:none; filter:none }`)
- `btn-a` **`Force start…`** — the amber button:
  `background: rgba(245,158,11,0.08); border:1px solid rgba(245,158,11,0.25);
  color:#fbbf24; radius:6px; padding:10px 18px; weight:550; font-size:13px`;
  hover → `rgba(245,158,11,0.14)`; disabled → `opacity:0.35; cursor:not-allowed`.
- `btn-s` **`Drop ticket…`** at `margin-left:auto` (pushed to the far right —
  a different action from force-starting, and visually separated).

**(b) `asking`** — an amber well: `radius:12px; border:1px solid
rgba(245,158,11,0.25); background: rgba(245,158,11,0.05); padding:14px 16px`.
- `.lbl` in `#fbbf24`: **`Why are you bypassing DSC-2?`** (interpolate the
  blocking ticket id; with more than one, name them all).
- `<textarea class="fld" rows="3">` with placeholder verbatim:
  **`This project has no forge remote — DSC-2 merged out of band this morning.`**
- Action row (`margin-top:12px; gap:10px`): `btn-a` **`Force start DSC-3`**
  (disabled while `reason.trim().length < 8`), `btn-s` **`Cancel`**, and a
  right-aligned 11px `#64748b` hint that is either
  **`A reason is required.`** or **`Recorded on the ticket.`** by the same rule.

**(c) `forced`** — an emerald well: `radius:12px; border:1px solid
rgba(16,185,129,0.25); background: rgba(16,185,129,0.05); padding:14px 16px`.
- Header row (`gap:8px`): emerald dot + Outfit 13px weight 600 `#34d399`
  **`Force-started`**
- Fira Code 11px `#94a3b8` `line-height:1.8`, `margin: 8px 0 0`:
  **`Steven Yepes · 20 Aug 2026, 14:12`** then `<br>` then the reason in curly
  quotes: `“{reason}”`
- `.hint` at `margin-top:10px`, verbatim:
  **"Kept on the ticket, and repeated to the agent above."**

The briefing well (§5.8) gains the bypass paragraph the moment phase becomes
`forced`.

### 5.10 States

| State | Rendering |
|---|---|
| Unstarted (the only editable state) | every field live; header chips `Blocked` + `Unstarted` |
| Started | not drawn — per §5.3's hint, everything locks; render read-only |
| Attachments present | chips + `{n} of 10 · staged` |
| No attachments | `No attachments yet…` line, chip count `0 of 10 · staged` |
| Drag over dropzone | `.dz.over` — cyan border |
| Vision warning | image + no-vision model, dismissible |
| Force idle / asking / forced | §5.9 (a)/(b)/(c) |
| Reason too short | confirm disabled, hint `A reason is required.` |

No save-in-flight, save-error or validation state is drawn.

---

## 6. Shared components & tokens

### 6.1 Colour semantics as the mocks use them

The mocks obey AGENTS.md §4 exactly. Restated with the Discovery meanings:

| Tone | Hex family | Role in Discovery |
|---|---|---|
| **violet** `#8b5cf6` / `#a78bfa` / `#c4b5fd` | primary action & active | `btn-p` gradient, focus rings, the Discovery compass icon, the `Interviewing` state, the **Ready** bucket, agent bubbles, the selected segment, the Decompose checkbox, the mime pill |
| **cyan** `#06b6d4` / `#22d3ee` / `#67e8f9` | streams & interactive | active tab underline, user bubbles, **selection ring** on nodes and cards, **In flight** bucket, `Waiting`-on-an-open-PR prerequisites, dropzone drag-over, eyebrow labels, token metrics |
| **emerald** `#10b981` / `#34d399` | done & healthy | **Landed** bucket, satisfied edges (`path.met`), `Recommended` / `Chosen` options, `Answered`, `Schema valid`, spend metric, the force-started confirmation, the `Turn complete` toast |
| **amber** `#f59e0b` / `#fbbf24` / `rgba(253,230,138,0.9)` | caution, not failure | **Blocked** bucket (when a prerequisite is at least in flight), the worktree-confinement banner, the effort-unsupported and no-vision notes, the whole force-start affordance (`btn-a`) |
| **ruby** `#ef4444` / `#f87171` / `#fecaca` | error / removal | the `Removed` group label, `.was` strike-through in a diff, destructive hover on `.x`, the vision warning's background and text |
| **slate** `#64748b` / `#94a3b8` / `#475569` / `#334155` | inert / unstarted | **Dropped** bucket, blocked-with-nothing-started, secondary copy, edges, grid dots, `.lbl` |

Note the two amber/slate grades of *blocked* (§3.5.5) and that ruby appears
**only** for removal and error — never for a blocked or dropped ticket.

### 6.2 Reusable pieces the mocks repeat

| Piece | Where it appears | Notes |
|---|---|---|
| **38 px column sub-header** | Interview, Graph/Board, Inspector | `padding:0 16px; height:38px; background: rgba(18,22,30,0.6); border-bottom:1px solid rgba(255,255,255,0.05)`; a 12 px Outfit weight-500 `#9ca3af` title on the left, chips or controls on the right. Sticky in the inspector. **Extract once.** |
| **Metric strip** | ProjectHome header, Discovery header | Already `src/components/ui/MetricStrip.tsx` |
| **Chip** | everywhere | Already `src/components/ui/Chip.tsx` at `size="sm"` |
| **`.lbl` field label** | every artboard | Already `src/components/ui/FieldLabel.tsx` (see §6.4 for the one difference) |
| **Split progress bar** | ProjectHome card, Discovery sub-header | New; see §6.4 |
| **Verdict / briefing well** | Inspector §3.6.7, TicketEdit §5.8 | `radius:12px; border rgba(255,255,255,0.05); background: rgba(5,6,8,0.8)`; identical in both |
| **Prerequisite row** | Inspector §3.6.3, TicketEdit §5.7 | Same row; the editor adds a remove button and uses `·` between id and title |
| **Amber note panel** | NewDiscovery ×2, Interview banner, force-start well | `border 1px solid rgba(245,158,11,0.20…0.25); background: rgba(245,158,11,0.05); color: rgba(253,230,138,0.9)`; 11–12px, `line-height:1.6` |
| **Removable list row (`.row`/`.idx`/`.x`) + `.add`** | TicketEdit acceptance, files, edges | New; three uses in one screen |
| **AttachmentDropzone / AttachmentChip** | NewDiscovery, Interview composer, TicketEdit | Already built — reuse |
| **EmptyStateCard** | ProjectHome empty | Already built — reuse |
| **Bucket → tone mapping** | graph nodes, board lanes, chips, verdicts, legend, progress bar | **One function.** The mocks derive the lanes and the node tints from the same `bucket` field and the annotation is explicit that nothing is stored |

### 6.3 What already exists in `src/App.css` (use these)

| Mock declaration | Existing equivalent | Fidelity |
|---|---|---|
| `.h { font-family:'Outfit' }` | `font-heading` (via `--font-heading` in `@theme`) | exact |
| `.m { font-family:'Fira Code','JetBrains Mono' }` | `font-mono` (via `--font-mono`) | exact |
| `body { font-family:'Inter' }` | `--font-sans`, already on `body` — **UI text needs no class** | exact |
| `.glass { … }` | `.glass-panel` | **byte-identical** values (`--bg-panel`, `blur(12px)`, `--border-glass`, `0 8px 32px 0 rgba(0,0,0,0.4)`, `--radius-panel`) |
| `.glass:hover` | `.glass-panel-hover` | exact (`--bg-panel-hover`, `rgba(255,255,255,0.08)`) |
| `.btn-p` | `.btn-primary` | identical but for padding (mocks use `8px 16px`/`9px 16px`/`10px 20px` vs `10px 18px`) and `font-size:13px`, which `.btn-primary` does not set |
| `.btn-s` (Main/NewDiscovery/Decompose/TicketEdit) | `.btn-secondary` | identical but for `font-size:13px` |
| `.fld` | `.input-field` | identical values; `.input-field` sets `font-size:0.9rem` (14.4 px) where the mocks use 13 px |
| chat bubble `.bub`, `.bub.agent`, `.bub.user` | `.chat-bubble`, `.chat-bubble.agent`, `.chat-bubble.user` | same colours and radii; the mock widens `max-width` 80 % → 88 % and `line-height` 1.45 → 1.55, and sets 13 px vs `0.9rem` |
| `.sender`, `.s-agent`, `.s-user` | `.chat-bubble-sender` + its `.agent`/`.user` descendants | same idea; mock uses `letter-spacing:0.05em` / `margin-bottom:6px` vs `0.5px` / `4px` |
| composer input | `.chat-input` | same box; **`.chat-input:focus` is cyan** where the mock's inline input has no focus style |
| composer wrapper | `.chat-input-area` | exact |
| `.pulse` + `@keyframes pulse-glow` | `@keyframes pulse-glow` in App.css | **the keyframes are byte-identical** (`0%,100% {opacity:0.4} 50% {opacity:1}`, `1.2s ease-in-out infinite`) |
| `.chip` + `.c-*` | `Chip` component + `TONE_CHIP` in `src/lib/runStatus.ts` | `Chip size="sm"` renders `rounded border font-mono uppercase tracking-wide gap-1 px-2 py-0.5 text-[10px]` — matches the mock's 4 px radius / `2px 8px` / 10 px / `0.025em` |
| `.dot` (6 px) | `Chip`'s own dot (`w-1.5 h-1.5 rounded-full bg-current`), `MachineDot` | exact |
| `.tabs` / `.tab` / `.tab.on` | `src/components/ui/TabBar.tsx` size `md` | exact — `border-cyan-500 text-cyan-400` on selected, `px-4 py-2.5 text-sm font-heading font-medium` |
| `.rail` | `.nav-icon-btn` / `.nav-icon-btn.active`, `RailNavItem` | exact (44 px, radius 10, violet active with `0 0 10px` glow) |
| `.row` / `.row.on` (ProjectHome sidebar) | `.list-item` / `.list-item.active` | exact, including `translateX(4px)` hover and `--transition-spring` |
| `.scroll::-webkit-scrollbar*` | global `::-webkit-scrollbar` rules | exact — **drop the mock's local copy** |
| `.well` background `#050608` | `--bg-well` / `.panel-field` | colour matches; the dot grid does not exist (§6.4) |
| empty state | `src/components/EmptyStateCard.tsx` | markup-for-markup identical |
| `.dz`, `.pick`, `.att`, `.thumb`, `.mime` | `AttachmentDropzone.tsx`, `AttachmentChip.tsx` | verbatim reproductions of the real components |
| `.warn` | the vision-warning block in `StartFeatureModal.tsx` | verbatim |
| Tailwind palettes `amber-*`, `cyan-*`, `emerald-*`, `violet-*`, `slate-*` | Tailwind v4 defaults | present |
| `ruby-*` | `@theme --color-ruby-50…950` in `src/App.css` | present (`text-ruby-400` = `#f87171`, `ruby-200` = `#fecaca`, `ruby-300` = `#fca5a5`) |

### 6.4 What does **not** exist — the gaps

Naming these explicitly because a class with no matching rule renders as nothing
here and no gate reports it (AGENTS.md §7, "Class names are gated too"):

**(a) Every mock-local class name.** None of these resolve in this app:
`h`, `m`, `lbl`, `glass`, `chip`, `c-emerald`, `c-cyan`, `c-violet`, `c-amber`,
`c-ruby`, `c-slate`, `dot`, `pulse`, `btn-p`, `btn-s`, `btn-a`, `well`, `node`,
`n-emerald`, `n-cyan`, `n-violet`, `n-amber`, `n-dropped`, `sel`, `edges`, `met`,
`f-t*`, `t-t*`, `sel-t*`, `bub`, `agent`, `user`, `sender`, `s-agent`, `s-user`,
`caret`, `qcard`, `done`, `opt`, `live`, `rec`, `chosen`, `faded`, `key`,
`opt-label`, `opt-desc`, `seg`, `bar`, `legend`, `k`, `lane`, `lane-hd`, `nm`,
`ct`, `rule`, `tcard`, `ttl`, `scroll`, `toast`, `tabs`, `tab`, `on`, `rail`,
`row`, `pill`, `dis`, `fld`, `mono`, `note`, `card`, `card-hd`, `hint`, `pick`,
`off`, `lock`, `box`, `was`, `now`, `idx`, `add`, `x`, `selwrap`, `dz`, `over`,
`dzhint`, `att`, `thumb`, `att-name`, `att-size`, `mime`, `warn`, `edge`.
Translate each to a Tailwind utility set or an existing component per §6.3 —
**never copy a mock class string into a `className`.**

**(b) Genuinely new CSS the mocks introduce.** These have no counterpart in
`src/App.css` and must be authored (as Tailwind arbitrary values at the call
site, or as new rules — a new rule needs a `@theme` key if it is to be a utility):

| Missing thing | Declaration from the mock | Used by |
|---|---|---|
| `@keyframes blink` | `0%,50% { opacity:1 } 50.01%,100% { opacity:0 }`, applied as `blink 1s steps(1) infinite` | the streaming caret (§3.4.3). **Add it beside `pulse-glow` and add it to the `prefers-reduced-motion` block** — it is an always-on animation, exactly what that block exists for |
| Streaming caret | `display:inline-block; width:7px; height:13px; background:#a78bfa; vertical-align:-2px` | §3.4.3 |
| Dot-grid canvas | `background-image: radial-gradient(#334155 1px, transparent 1px); background-size: 20px 20px` over `--bg-well` | graph view (§3.5.3) |
| Split progress bar | flex row of two coloured spans inside `height:4px; border-radius:999px; background: rgba(255,255,255,0.06); overflow:hidden` | §1.5.2, §3.5.1 |
| Graph node surface | `background: rgba(15,23,42,0.72); border:1px solid rgba(51,65,85,0.6); backdrop-filter: blur(4px); box-shadow: 0 10px 15px -3px rgba(0,0,0,0.35)` — a **lighter blur and darker slate than `--bg-panel`** | `.node`, `.tcard` |
| Cyan selection ring | `box-shadow: 0 0 0 1px rgba(34,211,238,0.4), 0 0 18px rgba(34,211,238,0.25)` with border `rgba(34,211,238,0.70)` | node + card selection |
| Per-bucket node glow | `0 0 18px rgba(<tone>,0.18…0.20)` | `.n-cyan`, `.n-violet`, `.n-amber` |
| Amber button (`btn-a`) | `background: rgba(245,158,11,0.08); border 1px solid rgba(245,158,11,0.25); color:#fbbf24` | force start (§5.9) |
| Dashed "add" button | `border:1px dashed rgba(255,255,255,0.10)`, hover violet | §5.4, §5.7 |
| Amber note panel | `border rgba(245,158,11,0.20); background rgba(245,158,11,0.05); color rgba(253,230,138,0.9)` | §2.3, §3.4.2 |
| Diff `.was`/`.now` | `#f87171` + `line-through` over `#34d399` | §4.6 |
| Keycap `.key` | `19×19; radius:4px; border rgba(255,255,255,0.12); Fira Code 10px; #94a3b8` | §3.4.4 |
| Legend pill | `radius:999px; border rgba(255,255,255,0.05); background rgba(15,23,42,0.9); backdrop-filter: blur(12px)` | §3.5.3 |
| Segment strip | `.seg` — see §6.5 for how it differs from `SegmentedControl` |
| Sunk briefing well | `background: rgba(5,6,8,0.8)` with a border — `--bg-well` is `#050608` **opaque**; this is the translucent variant | §3.6.7, §5.8 |
| Editor card surface | `rgba(18,22,30,0.55)` — a **lighter** panel than `--bg-panel`'s `0.92`, with no blur and no shadow | §5.3 |
| Workspace/artboard backgrounds | `#0a0c10` (workspace), `rgba(11,13,18,0.4)` (interview), `rgba(13,15,20,0.7)` (inspector), `rgba(13,15,20,0.6)` (headers), `rgba(18,22,30,0.6)` (sub-headers) | no tokens exist for any of these; `--bg-app` is `#08090c` and `--bg-sidebar` is `#0d0f14` |
| Ambient page gradients | the two `radial-gradient` washes in §1.2 and the single wash on each modal artboard | no token |
| Toast | `border rgba(16,185,129,0.25); background rgba(6,20,16,0.95)` | `ErrorToast.tsx` exists but is a different shape (`border-l-4`, `min-w-[320px]`, icon + title + body + actions) — the Discovery success toast needs either a new variant or a tone prop on it |

**(c) Fonts.** The mocks' `<link rel="stylesheet" href="https://fonts.googleapis.com/…">`
must not be carried over — `src/main.tsx` self-hosts Inter (300–700), Outfit
(400–800) and Fira Code (400–600) via `@fontsource`, deliberately, so the app
works offline. All weights the mocks use are covered except `font-weight: 550`
on `btn-p`/`btn-s`, which is synthesised — and is already what `.btn-primary`
does, so it is precedent, not a new problem.

### 6.5 Divergences from existing components — decide these before building

1. **`.seg` vs `SegmentedControl`.** The mock's Graph/Board control selects in
   **violet** (`bg rgba(139,92,246,0.14)`, `#c4b5fd`, inset violet ring) while
   `src/components/ui/SegmentedControl.tsx` selects in **cyan** (`TONE_CHIP.cyan`)
   and wraps `border-white/10 bg-white/[0.02] rounded-lg`. The mock is also
   denser (`padding:2px`, `gap:2px`, `4px 10px` segments, 11 px). Either accept
   the app's cyan and reuse the component, or add a tone axis to it. **Do not
   fork it** — it is a `radiogroup` with its own arrow-key contract.
2. **`.card` in TicketEdit is not `glass-panel`.** `rgba(18,22,30,0.55)`, no blur,
   no shadow. `SectionCard` (`glass-panel p-5`) is heavier. A nested card inside
   a panel wanting a lighter surface is a real gap; name it once rather than
   inlining `bg-[rgba(18,22,30,0.55)]` five times.
3. **`.btn-s` has two spellings across the mocks.** ProjectHome's is *filled*
   (`background: rgba(255,255,255,0.05); border rgba(255,255,255,0.10); radius:8px;
   padding:7px 12px; font-size:12px; color:#cbd5e1`); every other artboard's is
   *ghost* and matches `.btn-secondary`. Use `.btn-secondary` and treat the
   ProjectHome variant as the existing app's own small-button style.
4. **`.animate-pulse-glow` is not the mock's `.pulse`.** The App.css utility adds
   a **static cyan** `box-shadow: 0 0 10px 1px rgba(6,182,212,0.5)`; the mock's
   `.pulse` is opacity only. For a violet `Interviewing` dot or an amber blocked
   dot, `animate-pulse-glow` paints the wrong glow. `Chip` already does the right
   thing — its dot uses Tailwind's `animate-pulse` with
   `motion-reduce:animate-none`.
5. **Inspector width.** The Discovery inspector is 360 px; `src/components/ui/Inspector.tsx`
   owns the app's inspector contract. Check its width before hard-coding.
6. **`FieldLabel` vs `.lbl`.** `FieldLabel` renders
   `text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider` — 12 px,
   `slate-400`, no bold. The mocks' `.lbl` is **10 px, weight 700, `#64748b`
   (slate-500), `letter-spacing:0.08em`**. The mocks use it as a section eyebrow
   as often as a form label. Either widen `FieldLabel` with a size/tone axis or
   accept the existing treatment — but pick one, because the mocks use `.lbl` in
   nineteen places across five screens.

### 6.6 Interaction inventory

| Affordance | Present? | Notes |
|---|---|---|
| Click a graph node / board card to select | yes | drives inspector + edge highlight |
| **Drag** anything | **no** | no drag on graph nodes, no drag between board lanes, no reordering of acceptance criteria or files. The board's lanes are derived from edges and have nothing to drop into. The only drag surface in the whole feature is the attachment dropzone (`.dz.over`) |
| Pan / zoom the graph | buttons only | `−` / `+` / `Fit` exist and are unwired; no pan handler, no wheel zoom |
| Hover | yes | cards lift (`translateY(-1px)`), sidebar rows slide (`translateX(4px)`), options slide (`translateX(2px)`), buttons glow |
| Keyboard | partial | `Enter` sends a turn; `.key` caps (`1`,`2`,`3`,`↵`) imply number-key answering but no handler is wired. Wire them |
| Tooltips (`title=`) | `"Projects"`, `"Machines"`, `"Providers"`, `"Settings"`, `"Attach a file or image"`, `"Remove this criterion"`, `"Remove this path"`, `"Remove this edge"`, `"Remove {name}"`, `"Dismiss"`, and the progress bar repeating its own text |
| Auto-scroll | yes | transcript pins to the bottom on every update |
| Auto-dismiss | yes | the `Turn complete` toast, ~4.5 s |

### 6.7 Copy rules the mocks hold to (§9.4)

Worth stating because several strings look like they could be tightened and must
not be:

- Demeteo **says** a ticket is startable; it never starts one. "Its one
  prerequisite merged. Demeteo says so; it does not start anything on its own."
- Readiness is always described as **recomputed**, never as stored — the graph
  sub-header, the verdict card and both annotations all say so.
- The worktree fence is stated as **intent for the harness**, with the write-tool
  gap named out loud (§3.4.2).
- *Removed* and *dropped* are different words for different things (§4.7).
- A free-text answer is **first-class**, never a fallback ("both settle the same
  question", "takes it as written rather than fitting it to the nearest option").

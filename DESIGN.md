---
name: Demeteo
description: Dark-only desktop control plane for coding agents, built as a quiet instrument: solid surface steps, one indigo "Signal" accent for action, selection and focus, and status hues spent only on meaning. Decorative blobs, uppercase tracked labels and sub-11px type remain from the retired neon look.
colors:
  accent: "#5a60c2"
  accent-hover: "#6269cc"
  accent-ink: "#838ef4"
  accent-ring: "#6e78db"
  accent-edge: "#6972c0"
  accent-tint: "rgb(90 96 194 / 0.16)"
  cyan-stream: "oklch(71.5% 0.088 215.2)"
  cyan-ink: "oklch(78.9% 0.094 211.5)"
  emerald-done: "oklch(69.6% 0.104 162.5)"
  emerald-ink: "oklch(76.5% 0.108 163.2)"
  amber-needs-you: "oklch(76.9% 0.188 70.08)"
  amber-ink: "oklch(82.8% 0.189 84.429)"
  ruby-failure: "#ef4444"
  ruby-action: "#dc2626"
  ruby-action-deep: "#b91c1c"
  ruby-ink: "#f87171"
  surface-app: "#08090c"
  surface-sidebar: "#0d0f14"
  surface-well: "#050608"
  surface-panel: "#11151d"
  surface-panel-hover: "#191e29"
  surface-raised: "#181d28"
  field-fill: "rgba(10, 12, 16, 0.6)"
  hairline: "rgb(255 255 255 / 0.05)"
  hairline-raised: "rgb(255 255 255 / 0.09)"
  hairline-strong: "rgba(255, 255, 255, 0.10)"
  token-ink-primary: "#f3f4f6"
  token-ink-secondary: "#9ca3af"
  token-ink-muted: "#6b7280"
  ink-white: "#ffffff"
  ink-slate-200: "#e2e8f0"
  ink-slate-300: "#cbd5e1"
  ink-slate-400: "#94a3b8"
  ink-slate-500: "#64748b"
  ink-slate-600: "#475569"
typography:
  display:
    fontFamily: "Outfit, sans-serif"
    fontSize: "30px"
    fontWeight: 700
    lineHeight: "36px"
    letterSpacing: "-0.025em"
  headline:
    fontFamily: "Outfit, sans-serif"
    fontSize: "24px"
    fontWeight: 700
    lineHeight: "32px"
  title:
    fontFamily: "Outfit, sans-serif"
    fontSize: "18px"
    fontWeight: 400
    lineHeight: "28px"
  section:
    fontFamily: "Outfit, sans-serif"
    fontSize: "14px"
    fontWeight: 600
    lineHeight: "20px"
    letterSpacing: "0.05em"
  body:
    fontFamily: "Inter, -apple-system, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: "20px"
  body-dense:
    fontFamily: "Inter, -apple-system, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: "16px"
  label:
    fontFamily: "Fira Code, JetBrains Mono, monospace"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: "16px"
    letterSpacing: "0.05em"
  data:
    fontFamily: "Fira Code, JetBrains Mono, monospace"
    fontSize: "11px"
    fontWeight: 400
  micro:
    fontFamily: "Fira Code, JetBrains Mono, monospace"
    fontSize: "10px"
    fontWeight: 400
    letterSpacing: "0.025em"
rounded:
  control: "4px"
  field: "6px"
  control-lg: "8px"
  panel: "12px"
  pill: "9999px"
spacing:
  hair: "4px"
  xs: "6px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  card: "20px"
  panel: "24px"
  page: "32px"
components:
  panel:
    backgroundColor: "{colors.surface-panel}"
    rounded: "{rounded.panel}"
    padding: "20px"
  panel-hover:
    backgroundColor: "{colors.surface-panel-hover}"
  raised:
    backgroundColor: "{colors.surface-raised}"
    rounded: "{rounded.panel}"
  nested-card:
    rounded: "{rounded.panel}"
    padding: "14px 16px"
  field-well:
    backgroundColor: "{colors.surface-well}"
  chip-cyan:
    backgroundColor: "rgba(6, 182, 212, 0.10)"
    textColor: "{colors.cyan-ink}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    padding: "2px 10px"
  chip-emerald:
    backgroundColor: "rgba(16, 185, 129, 0.10)"
    textColor: "{colors.emerald-ink}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    padding: "2px 10px"
  chip-amber:
    backgroundColor: "rgba(245, 158, 11, 0.10)"
    textColor: "{colors.amber-ink}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    padding: "2px 10px"
  chip-ruby:
    backgroundColor: "rgba(239, 68, 68, 0.10)"
    textColor: "{colors.ruby-ink}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    padding: "2px 10px"
  chip-slate:
    backgroundColor: "rgba(100, 116, 139, 0.10)"
    textColor: "{colors.ink-slate-400}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    padding: "2px 10px"
  chip-sm:
    typography: "{typography.micro}"
    rounded: "{rounded.control}"
    padding: "2px 8px"
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.ink-white}"
    typography: "{typography.body-dense}"
    rounded: "{rounded.field}"
    padding: "8px 16px"
  button-primary-hover:
    backgroundColor: "{colors.accent-hover}"
  button-secondary:
    backgroundColor: "rgba(255, 255, 255, 0.05)"
    textColor: "{colors.ink-slate-200}"
    typography: "{typography.body-dense}"
    rounded: "{rounded.field}"
    padding: "8px 16px"
  button-secondary-hover:
    backgroundColor: "rgba(255, 255, 255, 0.10)"
    textColor: "{colors.ink-white}"
  button-ghost:
    backgroundColor: "transparent"
    textColor: "{colors.ink-slate-400}"
    typography: "{typography.body-dense}"
    rounded: "{rounded.field}"
    padding: "8px 16px"
  button-ghost-hover:
    backgroundColor: "rgba(255, 255, 255, 0.05)"
    textColor: "{colors.ink-white}"
  button-danger:
    backgroundColor: "{colors.ruby-action}"
    textColor: "{colors.ink-white}"
    typography: "{typography.body-dense}"
    rounded: "{rounded.field}"
    padding: "8px 16px"
  button-danger-hover:
    backgroundColor: "{colors.ruby-action-deep}"
  button-danger-subtle:
    backgroundColor: "transparent"
    textColor: "{colors.ruby-ink}"
    typography: "{typography.body-dense}"
    rounded: "{rounded.field}"
    padding: "8px 16px"
  button-danger-subtle-hover:
    backgroundColor: "rgba(239, 68, 68, 0.10)"
    textColor: "#fca5a5"
  button-sm:
    typography: "{typography.body-dense}"
    padding: "4px 10px"
  button-lg:
    typography: "{typography.body}"
    padding: "10px 20px"
  input-field:
    backgroundColor: "{colors.field-fill}"
    textColor: "{colors.token-ink-primary}"
    typography: "{typography.body}"
    rounded: "{rounded.field}"
    padding: "10px 12px"
  input-inline:
    backgroundColor: "rgba(0, 0, 0, 0.4)"
    textColor: "{colors.ink-white}"
    typography: "{typography.body-dense}"
    rounded: "{rounded.control-lg}"
    padding: "10px"
  field-label:
    textColor: "{colors.ink-slate-400}"
    typography: "{typography.label}"
  tab-active:
    textColor: "{colors.ink-white}"
    typography: "{typography.body}"
    padding: "10px 16px"
  tab-idle:
    textColor: "{colors.ink-slate-400}"
  segment-selected:
    backgroundColor: "{colors.accent-tint}"
    textColor: "{colors.ink-white}"
    rounded: "{rounded.field}"
  rail-item-active:
    backgroundColor: "rgba(255, 255, 255, 0.07)"
    textColor: "{colors.ink-white}"
    rounded: "{rounded.control-lg}"
    padding: "8px 12px"
  rail-item-idle:
    backgroundColor: "{colors.surface-sidebar}"
    textColor: "{colors.ink-slate-400}"
  rail-project-active:
    backgroundColor: "{colors.surface-panel-hover}"
    textColor: "{colors.ink-white}"
    rounded: "{rounded.control-lg}"
    padding: "8px 12px"
  top-bar:
    backgroundColor: "{colors.surface-sidebar}"
    height: "56px"
    padding: "0 24px"
  top-bar-search:
    backgroundColor: "{colors.surface-well}"
    textColor: "{colors.ink-slate-400}"
    rounded: "{rounded.field}"
    padding: "6px 12px"
  modal-scrim:
    backgroundColor: "rgba(0, 0, 0, 0.6)"
---

<!--
  Record of the system AS SHIPPED on branch design/accent-buttons (master
  f0a06579 = PR #251, which carried the token move and runStatus
  re-toning; PR #252's solid surfaces, Drift group 1; and this branch's
  Signal accent and Button primitive, Drift group 2), refreshed
  2026-10-20 from code, not intentions. The direction
  it is measured against is AGENTS.md §4 ("a quiet instrument"); where the
  code still does something else, the body records what the code does and
  the "Drift" section at the end says what contradicts §4 and how to fix it.

  Ground truth: src/styles/tokens.css (values), src/App.css (rules),
  src/components/ui/*, src/lib/runStatus.ts, and the views named below.
  Counts are from ripgrep over non-test .ts/.tsx under src/ with
  comment-only lines dropped; the exact patterns are in the Drift section.

  The frontmatter holds only shipped values. The indigo "Signal" accent
  (decision 61) is shipped as `@theme static` colours in tokens.css
  (`accent`, `-hover`, `-ink`, `-ring`, `-edge`, `-tint`), and the
  frontmatter carries those six values.

  Colour values: tokens the project declares itself (tokens.css, the pinned
  utility block in App.css) are sRGB hex/rgba. Colours that reach the screen
  only through Tailwind v4 palette classes are Tailwind's OKLCH values,
  which is what actually renders; they are kept in OKLCH.
-->

# Design System: Demeteo

## Overview

**Creative North Star: "A Quiet Instrument"**

The direction is AGENTS.md §4's, adopted in 2026-10: a developer keeps
Demeteo open all day to supervise agents, so the chrome recedes and the
work and its state carry the screen. Colour is spent only on meaning,
with one accent, indigo "Signal" (decision 61), for primary actions,
selection and focus. Depth is a solid surface step with a 1px hairline
rather than an effect, and type is set in sentence case at 11px or more. The benchmarks are
developer tools (Linear, Zed, Ghostty, GitHub Primer). The retired
direction, "dark neon glassmorphism", is named in §4 as the look a
generator reaches for by default.

The shipped code is partway there. The parts that already fit are
structural: one status vocabulary (`src/lib/runStatus.ts`) with no violet
tone, a motion budget that keeps infinite animation to small liveness
marks, cascade layers in `src/App.css` so a utility beside a class takes
effect, and solid surfaces (a panel, a raised layer for whatever floats,
and a nested card, each with a hairline, one shadow token, and no blur
except on controls floating over a live canvas). Since branch
`design/accent-buttons` the accent is Signal: one `Button` primitive
whose primary fill is the accent, one global focus ring, and selection
in the accent's edge and tint, with no glow on a control or a status
mark. Much of the rest is still the old look: decorative blur blobs and
gradient tiles sit behind status screens, cyan marks some things that
are not in motion, and labels are uppercase mono with wide tracking.
The Drift section lists these as follow-up PRs.

It is an Operate tool, and density shows it. The commonest text sizes are
hand-set 11px (`text-[11px]`, 352 uses), 12px (`text-xs`, 340) and 10px (287). Machine
output (ids, costs, token counts, statuses, paths) is set in Fira Code
and headings in Outfit. There is no light theme and no theme switch.

**Key Characteristics:**
- Dark-only. Surfaces sit between the well (#050608) and panel hover (#191e29), never lighter.
- Colour is meaning. `runStatus.ts` maps every run status to one of five status hues; neither the accent nor violet names a status.
- One accent, indigo "Signal", for primary actions, selection and focus: the `Button` primary fill, a 2px focus ring, a selected edge and tint.
- Three solid surface classes, each stating its own hairline and radius: `.panel` (77 call sites in 41 files), `.raised` for menus, dialogs and toasts (33 in 27), and `.nested-card` to group content inside either.
- Mono-heavy and dense, with an ungoverned type scale (14 arbitrary text sizes).
- Depth is a solid surface step plus a hairline, and only the raised layer casts a shadow. No control or status mark glows; the last glows are in the first-run hero (Drift 3.6).
- Motion is budgeted for cost: `data-motion="paused"` freezes everything while the window is unfocused.

## Colors

Near-black neutrals, five status hues plus inert slate, and one accent
held back for action, selection and focus. Values live in
`src/styles/tokens.css`; the surfaces and the accent are Tailwind colours
there, while status hues still reach components through Tailwind palette
classes behind the `TONE_*` maps (see Drift).

### Primary
Indigo **Signal** (decision 61 in `docs/DECISIONS.md`), declared as
`@theme static` colours in `tokens.css`. It marks primary actions,
selection and focus, and never a status.
- **Signal** (`accent`, `bg-accent`): the primary fill, always with a white label (5.40:1). The `Button` primary variant and StartSessionButton's split button are the fills.
- **Signal Hover** (`accent-hover`): the primary fill under the pointer (white label 4.76:1).
- **Signal Ring** (`accent-ring`): the global 2px focus ring and the focused border of a field (`focus:border-accent-ring`, `.input-field`, `.chat-input`); 4.68:1 against the panel.
- **Signal Edge** (`accent-edge`): the selected border on tabs, segments, rows, tiles and the selected canvas node; 4.15:1 against the panel.
- **Signal Tint** (`accent-tint`): the selected fill behind segments, rows, palette highlights and the selected `<option>`; `::selection` uses the same hue at 45%.
- **Signal Ink** (`accent-ink`): accent text on a dark surface (6.22:1 against the panel), and the selected `<option>` text. Checkboxes take `accent-accent`.

Decision 61 records why stock violet was dropped: white on violet-500
measured 4.23:1 (under AA), and its focus and selection borders measured
under 2:1. Violet stays out of statuses too, because it sits too close to
the accent; what violet is left in `src/` is decoration (Drift group 3).

### Secondary
- **Stream Cyan** (`cyan-stream`): in motion. Running, bootstrapping, verifying, a published PR out for review, an "Asking" or "Interviewing" turn while it runs, and the one shimmer, the indeterminate progress bar. Some cyan still marks nothing in motion (Drift 2.9). 207 palette-class uses.
- **Cyan Ink** (`cyan-ink`): token-count icons, markdown links, in-motion chip text.

Cyan and emerald are `@theme` scales in `tokens.css` at Tailwind's lightness and about 0.6x its chroma, so they no longer outshout the accent; contrast moved by under 5% and every pairing stays above 6.5:1 on a panel. Amber keeps Tailwind's full chroma, as the one hue meant to pull the eye.

### Tertiary
- **Done Emerald** (`emerald-done`): finished well, or healthy (completed, PR ready, merged; `StatusBadge` maps machine `idle`/`active` to it, which §4's "or healthy" covers). It fills no button: approving is the gate's primary action, so it is Signal. On a workflow-graph node it is the status dot only: a completed node keeps a neutral border and a slate "Completed" label, because a finished run would otherwise turn every node green. Costs and token counts are quantities, not states, and are neutral in the run header and on nodes.
- **Needs-You Amber** (`amber-needs-you`, amber-500): a human is needed, and in `runStatus.ts` only that: the settled gate, credential and interrupted statuses, plus `REVIEW_READY_META`. `pipelineFilter.ts` derives the "Needs you" band from it. Amber has no token in `tokens.css`; it comes from Tailwind's palette. It is the only hue with its own finite animation (`pulse-attention`, three beats, then lit) and its own card treatment (`ring-1 ring-amber-500/40` on a pipeline card that needs you). Discovery's QuestionCard "Needs you", the NotificationBell `gate_pending` dot and the discovery **Ready** lane (a ticket waiting for someone to start it) are amber for the same reason; a Blocked ticket is slate, since nobody can act on it.
- **Ruby** (`ruby-failure`, `@theme --color-ruby-*`): failed or destructive. A full 50–950 scale in `tokens.css` at Tailwind v3's sRGB red values, so `ruby-500` is #ef4444 while v4's own `red-500` is a slightly different OKLCH red. **Ruby Action** (`ruby-action`, ruby-600) fills the `danger` button and steps darker to **Ruby Action Deep** (`ruby-action-deep`, ruby-700) on hover, because white on ruby-500 is under 4.5:1. Rose and raw red are also in use (Drift 2.7).

### Neutral
The surfaces and hairlines are Tailwind colours, declared in a `@theme
static` block in `tokens.css` (`bg-surface-panel`, `border-hairline`, …);
the older `--bg-*` and `--border-*` names alias them. All are opaque
except the hairlines and the field fill.

- **App Ground** (`surface-app`, `--bg-app`): body background and the root shell, which spells it as `bg-[#08090c]`.
- **Sidebar** (`surface-sidebar`, `--bg-sidebar`): the in-flow chrome: top bar, project rail and feature header. Call sites still spell it as `bg-[#0d0f14]`, some at partial alpha (Drift 5.1, 1.16).
- **Well** (`surface-well`, `--bg-well`): the darkest surface, for terminal and code wells sunk into a panel (`.panel-field`, gate artifact output) and the top-bar search field.
- **Panel** (`surface-panel`, `--bg-panel`) and **Panel Hover** (`surface-panel-hover`, `--bg-panel-hover`): the `.panel` fill and its hover, also the canvas drawers and workflow list rows. Panel hover doubles as the selected project row in the rail. The values are the old translucent panel colours composited over the app ground, so a panel on the app background looks as it did.
- **Raised** (`surface-raised`, `--bg-raised`): the `.raised` fill for menus, popovers, dialogs and toasts, a step lighter than the panel. Canvas nodes and attachment chips use it as a utility.
- **Field Fill** (`field-fill`, `--bg-input`): `.input-field` and `.chat-input`. Most call-site inputs use `bg-black/40` instead, and native fields are forced to #0a0a0e (Drift 6.1).
- **Hairline** (`hairline`, `--border-hairline`) and **Hairline Raised** (`hairline-raised`, `--border-raised`): the edge every surface states, brighter on the raised layer because it sits on a panel. **Hairline Strong** (`hairline-strong`) is `border-white/10`, the heavier weight call sites spell. Most call sites still spell the hairline itself as `border-white/5` (Drift 1.13).
- **Token inks** (`token-ink-*`, `--text-*`): Tailwind *gray*, read only by `body` and `App.css` rules.
- **Component inks** (`ink-white`, `ink-slate-200` … `ink-slate-600`): what components render. Headings are `text-white` (383 uses); body and meta run down the slate ladder. Slate text classes are pinned to v3 sRGB hex by the `PINNED UTILITY COLOURS` block in `App.css`.

### Named Rules

**The Colour-Is-Meaning Rule.** Neutral greys do everything that is not a status, an action, a selection or focus. A status colour always ships with a label or icon, because hue alone carries no state (WCAG 2.2 AA is the target).

**The Accent-Is-Not-A-Status Rule.** The accent marks primary actions, selection and focus, never a status, and neither does violet, which sits too close to it. `RunStatusTone` has no violet and no accent-hued tone, and no `Button` variant takes a status hue: a status colour on a control would say the action is that state.

**The Amber-Is-A-Human Rule.** Amber means a human is needed and nothing else. "Needs you" is derived from a settled amber status, so an amber status that needs nobody is a bug. Amber is the one hue allowed to call for attention after it settles: three pulses, then it rests lit.

**The Three-Ladder Rule.** Every hue is spelled at fixed strengths in `runStatus.ts`: chip (`/10` fill, `-400` ink, `/20` border, `TONE_CHIP`), rail badge (`/20` fill, `-300` ink, `/30` border, `/40` for amber and ruby, `TONE_BADGE`), foreground (`TONE_TEXT`) and a row's left accent at `/60` (`TONE_BORDER_L`).

## Typography

**Display Font:** Outfit (with sans-serif)
**Body Font:** Inter (with -apple-system, sans-serif)
**Label/Mono Font:** Fira Code (with JetBrains Mono, monospace)

Self-hosted through `@fontsource` in `src/main.tsx`: Inter 300–700, Outfit
400–800, Fira Code 400–600. JetBrains Mono is named in the stack but not
bundled. Symbols Nerd Font Mono is bundled as an `@font-face` in `App.css`
purely as the xterm glyph fallback.

**Character:** Outfit is geometric and slightly wide and carries headings.
Inter carries prose and controls, and Fira Code carries everything a
machine produced (629 `font-mono` uses against 152 `font-heading`).

The three utilities resolve through the `@theme` keys `--font-sans`,
`--font-heading` and `--font-mono` in `tokens.css`. `body` already sets
Inter, so UI text needs no class.

### Hierarchy
- **Display** (Outfit 700, 30px / 36px, -0.025em): the first-run hero heading and two other page titles (3 uses of `text-3xl`).
- **Headline** (Outfit 700, 24px / 32px): full-panel status screens such as bootstrapping and bootstrap failed.
- **Title** (Outfit 400, 18px / 28px): pipeline card titles at comfortable density (14px compact). Weight varies at other title sites.
- **Section** (Outfit 600, 14px, uppercase, 0.05em): `SectionCard`'s header shape, repeated by hand in settings and project views. Uppercase and tracked, which §4 retires.
- **Body** (Inter 400, 14px / 20px): composer, buttons, `md` tabs.
- **Body Dense** (Inter 400, 12px / 16px): the workhorse for descriptions, rail names, modal copy and inputs.
- **Label** (Fira Code 400, 12px, uppercase, 0.05em, slate-400): `FieldLabel`. Hand-rolled labels repeat it at 11px.
- **Data** (Fira Code, 11px): ids, costs, tokens and elapsed time in dense rows. 11px is the commonest arbitrary size.
- **Micro** (Fira Code, 10px, uppercase): `Chip` at `sm`, count badges, keyboard hints. Some meta text drops to 9px. Everything at this step is under §4's 11px floor.

### Named Rules

**The Machine-Speaks-Mono Rule.** Anything a machine produced (ids, paths, statuses, counts, costs, durations) is set in Fira Code. Human prose stays Inter.

**The Primitive-Owns-Its-Type Rule.** `Chip` and `FieldLabel` refuse casing and typography props, so the pills and labels read as one family and change in one place. As shipped both are uppercase and tracked, which §4 retires; fixing the primitive fixes every call site.

**The Name-The-Role Rule.** A font key names a role (`--font-heading`), never a face. A utility naming a face with no `@theme` key compiles to nothing and silently inherits Inter, so `scripts/check-classes.mjs` gates it.

**The Sentence-Case Rule.** Labels and headings are sentence case, and copy is translatable: no sentence built by concatenation, no meaning carried by letter case.

## Layout

A fixed desktop window: default 1440×960, minimum 1024×700
(`src-tauri/tauri.conf.json`). Responsive prefixes are rare (`md:` 25,
`sm:` 19, `lg:` 2). The layout adapts by measuring instead: header nav
entries drop their labels below a measured 1392px header width and regain
them at 1440px (`src/lib/headerLayout.ts`, with hysteresis), and the
project rail collapses by toggle.

**Shell.** The top bar (56px, three-track grid
`minmax(0,1fr) auto minmax(min-content,1fr)`, 24px side padding, solid
sidebar surface, hairline bottom border) sits over a row of the project
rail and the content. The rail is 240px expanded and 56px collapsed, the
same solid sidebar surface with a hairline right border. A feature's
header takes the sidebar surface too. None of the chrome blurs.

**Project view.** 32px page padding, content capped at `max-w-7xl`
(1280px) and centred. The pipeline list stacks cards at 16px
(comfortable) or 8px (compact) from `src/lib/density.ts`, with card
padding 20px or 12px to match. Density is a user setting
(`DensityToggle`), not a breakpoint.

**Rhythm.** Tailwind's 4px step. Gaps cluster at 8px (`gap-2`, 360), 12px
(174) and 6px (151). Panel padding is 12–24px (`p-3` 89, `p-4` 69, `p-6`
64, `p-5` on cards); full-page status screens use 32px.

**Atmosphere (retired).** Twenty blurred violet, cyan or emerald blobs
(`rounded-full` at 5–20% with `blur-[100–120px]` or `blur-2xl`) sit
behind status screens and the first-run hero, in 11 files. They are
static because an animated blur pinned the WKWebView GPU process. §4
forbids them (Drift 3.1).

**Scrolling.** The global WebKit scrollbar and `ScrollArea` are 10px, with
a translucent white thumb that brightens on hover and while dragged, with no hue.
Long lists use `content-visibility: auto` with measured intrinsic sizes
(`.pipeline-card`, `.timeline-step-card`) rather than a virtualisation
dependency.

## Elevation & Depth

§4: depth is a surface step, not an effect. Three solid surfaces (the app
background, the panel, and the raised layer for menus and dialogs), each
with a 1px hairline, and backdrop blur only on overlays floating over live
content.

As shipped since branch `design/surfaces`, depth is **a surface step**.
The ladder is solid: well #050608 < app #08090c < sidebar #0d0f14 <
panel #11151d < raised #181d28 (panel hover #191e29). A panel states a
5% hairline and casts nothing; a raised surface states a 9% hairline and
casts the one shadow token. Backdrop blur is left only on translucent
controls floating over a live canvas (6 uses in 4 files). No status
mark, button or field glows: status dots and accent bars are flat, focus
is a ring or a border, and `.animate-pulse-glow(-amber)` keep their names
but fade opacity only. The exceptions are the first-run hero's glows
(Drift 3.6) and 22 neutral shadows in 8 files cast by canvas nodes and
in-flow boxes (Drift 1.15).

### Shadow Vocabulary
- **Raised** (`box-shadow: 0 12px 32px -8px rgba(0,0,0,0.6)`, `--shadow-raised`): built into `.raised`, for menus, popovers, dialogs and toasts. The one shadow §4 keeps; panels cast none, and call sites do not re-spell it.
- **No glow.** The status-dot, accent-bar, primary-button, live-pulse and focus glows are gone; `shadow-[0_0_` survives only in `EmptyStateCard` (4/1, Drift 3.6).

### Named Rules

**The Surface-Step Rule.** Depth comes from which of three solid surfaces an element sits on, each with a 1px hairline. A card is the panel surface; a menu or dialog is the raised one.

**The Blur-Only-On-Overlays Rule.** Backdrop blur belongs only to a translucent control floating over live content (the canvas zoom controls and legends, the terminal status overlay), never to a card, a rail or a header, and never under an opaque raised surface, where it shows nothing and still costs WebKitGTK a frame per scroll.

**The No-Panel-In-Panel Rule.** A card inside a panel or a dialog is a `.nested-card` (no fill, a hairline, 12px radius, no blur, no shadow), never a second panel, so it groups content on either surface without reading as a third. A field filling a panel (canvas, stream log) is a `.panel-field`: well fill, no border, no radius.

**The No-Blur-On-Scrims Rule.** A full-window scrim never takes `backdrop-blur`. Every dialog scrim, `Modal`'s default included, is a flat `bg-black/60`, because a blurred scrim re-blurs the whole window every frame something animates beneath it.

## Shapes

Softly squared. Controls take small radii and panels take
`--radius-panel` (12px). Circles are for status dots, count badges and
avatar or icon tiles.

- **Control** (4px, `rounded`, 119 uses): chips, keyboard hints, `.btn-icon`, header nav items, inline code.
- **Field** (6px, `rounded-md`, 56 at call sites): every `Button` (172 call sites), `.input-field`, selected segments, the top-bar search.
- **Control Large** (8px, `rounded-lg`, 315): the de facto default for hand-spelled inputs, rail items, icon buttons and inner boxes.
- **Panel** (12px, `--radius-panel`): `.panel`, `.raised`, `.nested-card`, and so every dialog and menu on `.raised`. Elsewhere 12px is still spelled `rounded-xl` (62 uses in 41 files, Drift 1.14). No surface is rounder than the panel since `rounded-2xl` was removed.
- **Pill** (`rounded-full`, 132): status dots (8px), chip dots (6px), count badges, avatars.

Borders are 1px hairlines, almost always white at 5% or 10%; the raised
layer's is 9%. Hue borders
appear at `/20`–`/40` on chips, badges, warnings and tinted buttons. A
pipeline card's status is a 4px left bar clipped by the card
(`overflow-hidden`), not a border colour.

### Named Rules

**The One-Panel-Radius Rule.** Every panel-level surface is 12px (`--radius-panel`), and a call site never re-spells it. A card, a canvas well and a banner are the same object at different depths, and a second value reads as a mistake. Smaller radii belong to controls.

## Components

Tailwind class strings in TSX, with primitives in `src/components/ui/`
that own a decision so call sites cannot re-open it. `Button` owns every
button's fill, radius and padding. `SectionCard` exists and has no call
sites.

### Buttons
- **Character:** one accent fill for the action that moves the work forward, quiet neutrals for the rest, and no glow, gradient or hue that names a state.
- **Primitive:** `Button` in `src/components/ui/Button.tsx` (`<Button` 172 call sites in 63 files). The variant names a role, never a hue; `className` is for layout only. It defaults to `variant="secondary"`, `size="md"` and `type="button"`. `buttonClasses(variant, size)` gives the same classes to a control that cannot be a `<button>`, such as a file-input `<label>` or an `<a>` (7 uses in 5 files).
- **Shape:** 6px (`rounded-md`), 1px border, semibold, `gap-1.5` for an icon.
- **Sizes:** `sm` 4px × 10px at 12px; `md` 8px × 16px at 12px, the dialog and toolbar default; `lg` 10px × 20px at 14px, a status screen's one action.
- **Primary:** Signal fill, white label, hover to Signal Hover. Gate Approve, Send redirect, Replay, Retry and every other proceed action are primary; there is no emerald or cyan variant.
- **Secondary:** white at 5% with a `border-white/10` edge, slate-200 label; hover to 10% and white.
- **Ghost:** no fill, slate-400 label; hover to white at 5% and white.
- **Danger:** ruby-600 fill, white label, hover steps darker to ruby-700.
- **Danger subtle:** no fill, `ruby-500/30` edge, ruby-400 label; hover to a ruby-500/10 tint and ruby-300 (GateView's Abort).
- **State:** disabled is 40% opacity with a not-allowed cursor, and hover is spelled `not-disabled:hover:` so a disabled button never lights up. Focus is the global ring.
- **Icon:** `.btn-icon` (4px) remains for bare icon buttons; `.btn-primary` and `.btn-secondary` no longer exist.
- **Outside the primitive:** StartSessionButton's split button spells `bg-accent` by hand, because its two halves share one rounded outline that a `<Button>` cannot draw. Hue-tinted controls remain at TicketForceStart (an amber force-start), WorkflowList (rose Delete, yellow Revert icon buttons) and the first-run hero's violet sample-project button (Drift 2.7, 3.6).

### Chips
- **Style:** `Chip` is the one pill (64 call sites). Fira Code, uppercase, `tracking-wide`, 4px radius, 1px hue border at `/20`, hue fill at `/10`, hue ink at `-400`. `md` is 12px with 2px × 10px padding; `sm` is 10px with 2px × 8px.
- **State:** a status chip resolves its label, hue and liveness from `runStatusMeta`. A live status gets a leading 6px `bg-current` dot with `animate-pulse`. The dot pulses, never the pill, because a pulsing box holding text re-rasterises the text every frame. `maxWidth` is the only inline-style escape.
- **Names:** a model, workflow, harness or placement is a slate chip. A name is not a state, so it takes no hue.
- **Don't:** give a chip a hue for something that is not a status; `Chip` has no violet tone. Cyan `agent_kind` chips are drift (2.9).

### Cards / Containers
- **Corner Style:** 12px (`--radius-panel`).
- **Background:** `.panel`: solid `--bg-panel` and the hairline; no blur, no shadow. `.panel-hover` lifts to `--bg-panel-hover` with an 8% border.
- **Shadow Strategy:** none. A panel is set apart by its step and its hairline. See Elevation & Depth.
- **Border:** built in. Call sites do not add their own (the `.panel` comment in `App.css` says so); none of the 77 does.
- **Internal Padding:** 20px standard (`SectionCard`, comfortable pipeline card), 12px compact, 24–32px for dialogs and status cards.
- **Nested:** `.nested-card` groups content inside a panel or a dialog (the ticket editor's fields) at 14px × 16px padding. It has no fill, only a hairline and the panel radius, so it reads the same on either surface.

### Raised Surface
- **Use:** whatever floats over the page: menus, popovers, pickers, dialogs, the canvas palette and toasts (`.raised`, 33 uses in 27 files).
- **Style:** solid `--bg-raised`, the brighter `--border-raised` hairline, the panel radius and `--shadow-raised`, the system's one shadow token. It is opaque, so no blur goes under it, and a call site adds no shadow or radius of its own. Hue borders that carry meaning stay: ruby on the data-loss and delete confirms, amber on the provider-in-use confirm, a toast's left status bar.
- **Inside:** no darker header or footer band, which would read as a fourth surface; group with `.nested-card`. Four dialogs still on `.panel` keep a footer band (Drift 1.15).
- **Scrim:** a dialog sits on a flat `bg-black/60`, never blurred. `Modal` defaults to it; the hand-rolled overlays spell the same (Drift 1.8).

### Pipeline Card (signature)
The project view's row: a hoverable `.panel` with a 4px left accent
bar in the status hue (flat, no glow), laid out in three tiers
(`data-tier`):
1. **Scan:** the title (Outfit, 18px or 14px, white, two-line clamp), a status `Chip` (`sm`), elapsed time in white mono, and a chevron that appears on hover.
2. **Context:** workflow (slate chip), transport (hue chip with a CPU icon), optional staleness chip, cost and tokens in slate mono.
3. **Detail:** feature id and description in slate-500, truncated.

A card that needs a human adds `ring-1 ring-amber-500/40`, which renders
since PR #250.

### Inputs / Fields
- **Style:** `.input-field` (field fill, hairline, 6px, 10px × 12px, 14px Inter). Most inputs are hand-spelled: `bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white`. Native `select`, text-like `input`s and `textarea` are forced to #0a0a0e with slate-200 text and slate-600 placeholders by unlayered `!important` rules in `App.css`, and `select` gets a chevron image with 28px right padding. Those rules beat the call sites' own background and text utilities (Drift 6.1).
- **Focus:** fields keep a border shift to Signal Ring and no glow: `focus:border-accent-ring` at call sites (102 in 42 files), and `.input-field` and `.chat-input` in `App.css`. They drop the global ring with `focus:outline-none` (94 in 39 files). Every other focusable element gets the one ring: `:focus-visible` in `@layer base`, a 2px `accent-ring` outline at a 2px offset. A `tabindex="-1"` element, focused by code rather than Tab, gets none.
- **Labels:** `FieldLabel`: 12px uppercase Fira Code, slate-400, `tracking-wider`, 6px below.

### Navigation
- **Top bar:** brand mark plus lowercase "demeteo" (Outfit 700, 18px). A centred command-palette trigger styled as a field: well fill, hairline, 6px. On the right, `HeaderNavItem`s: 16px icons in the item's own text colour (slate-400, white when active or hovered), mono 12px labels at `labels` density, `aria-label` only at `icons` density. Active is `bg-white/[0.04]` with white text. The corner badge is a solid amber count, a cyan activity dot or an emerald pulse dot.
- **Project rail:** projects with a `StatusBadge` dot, a 12px name and a 9px mono sub-line. The active item is a selected row: the panel-hover surface with white text at the 8px control radius, with no border and no halo. Collapsed, 36px initial tiles (active: Signal Tint fill, Signal Edge border, white text). `RailNavItem` renders the Terminals entry (active `bg-white/[0.07]` plus `border-white/10`). `RailBadge` handles counts (mono 10px, capped at 9+ or 99+).
- **Tabs:** `TabBar` is a `tablist` with a 2px bottom border. Selected is a Signal Edge border and white text; idle is slate-400, turning slate-200 on hover. `md` is 14px Outfit medium, `sm` 12px semibold.
- **Segmented control:** `SegmentedControl` is a `radiogroup` in an 8px `bg-white/[0.02]` frame. The selected segment takes a Signal Edge border, Signal Tint fill and white text, and focus is the global ring; the optional count badge is tinted by tone.

### Status Dot
`StatusBadge` is an 8px solid hue dot, flat, with no halo.
It has no label of its own: the row's text is the label. Liveness marks
elsewhere are 6–8px dots with `animate-pulse` (cyan, in progress) or
`animate-pulse-glow` (emerald, live sessions), which fades opacity and casts no halo.

### Gate Modal
The human-approval checkpoint. A `.raised` body (raised fill, brighter
hairline, panel radius, the raised shadow), capped at `max-w-4xl` and
85vh, over its own `bg-black/60` scrim rather than `Modal`. The header has
an amber shield tile and an Outfit 700 18px title; the artifact output
sits in a well. Warning strips are amber ("already decided") or rose
("decision blocked"). The footer puts Abort (`danger-subtle`) on the left
and Redirect (`secondary`, then a `primary` send) plus Approve (`primary`)
on the right. Approve is the gate's primary action, so it carries the
accent, not emerald.

### Empty State
`EmptyStateCard` has a first-run **hero** (a large panel over static
violet and cyan blobs, a gradient icon tile with a sparkle, a 30px Outfit
700 headline, four action tiles that scale on hover, and a violet
sample-project button with a hover sheen) and an **inline** variant (a
panel with two small corner blobs, a violet icon circle, a 16px
Outfit title and a 12px slate-400 description). The icon never pulses.
The hero concentrates most of Drift group 3.

### Motion
Tokens in `tokens.css`: `--transition-fast` (0.15s ease) and
`--transition-normal` (0.25s standard curve) are used by `App.css`;
`--transition-spring` and `--transition-smooth` are declared and unused.
Components mostly use Tailwind `transition` with `duration-300`.
Keyframes in `App.css`: `pulse-glow` (1.2s, opacity 0.4↔1, infinite),
`pulse-attention` (1.2s × 3, amber), `blink` (1s hard step, the streaming
caret), `fade-in` (0.2s, one-shot), `progress-shimmer` (1.6s, the one
non-opacity infinite animation, on the 2px bar in
`TerminalStatusOverlay`), `skeleton-pulse` (1.6s) and `spin` (77 uses).

**The Motion Budget Rule.** An infinite animation claims something is still happening, so it belongs only on a small liveness dot or an in-flight spinner, and on WebKitGTK any running animation costs a core. It animates opacity only, never `box-shadow` or `transform`. All animation pauses while the window is unfocused (`data-motion="paused"`) and stops under `prefers-reduced-motion`. A pulse that is not liveness must be finite. The only shimmer is an indeterminate progress bar.

## Do's and Don'ts

These restate AGENTS.md §4 and the invariants the code already enforces.

### Do:
- **Do** read token values from `src/styles/tokens.css`; never hard-code hex.
- **Do** pick a status hue through `runStatusMeta` / `TONE_CHIP` / `TONE_TEXT` / `TONE_BADGE` / `TONE_BORDER_L`, render status pills with `Chip`, and ship every status colour with a label or icon.
- **Do** keep the accent for primary actions, selection and focus: `<Button variant="primary">` for the action, `border-accent-edge` with `bg-accent-tint` for a selected item, and the global ring for focus. A field shows focus with `focus:border-accent-ring`.
- **Do** build every button with `Button` (or `buttonClasses` for a `<label>` or `<a>`), choosing a variant by role, and pass only layout classes.
- **Do** use amber only for "a human is needed", and let its pulse run three beats, then rest lit.
- **Do** use `.panel` for a panel, `.raised` for anything floating (menu, popover, dialog, toast), `.nested-card` for a group inside either, and `.panel-field` for a field filling a panel, and keep their fill, border, radius and shadow unspelled at the call site.
- **Do** keep blur for translucent controls floating over live content (canvas zoom controls and legends, the terminal status overlay); an opaque raised surface takes none.
- **Do** write labels and headings in sentence case, at 11px or more, and say Project, not Workspace.
- **Do** set machine output (ids, paths, counts, costs, statuses) in Fira Code and headings in `font-heading`.
- **Do** put an infinite pulse only on a small liveness dot or spinner, and animate opacity only.
- **Do** use `Modal` for overlays: portalled, a flat `bg-black/60` scrim, Escape to close, with a `.raised` body.
- **Do** keep inline `style` for geometry (heights, offsets, measured widths).

### Don't:
- **Don't** build a light theme or place these surfaces on a light ground.
- **Don't** add glow shadows or `text-shadow`, or a gradient fill on a button or text.
- **Don't** fill a button with a status hue (an emerald Approve); proceed actions are primary.
- **Don't** add decorative blurred blobs, sparkle icons as ornament, `hover:scale`, or shimmer outside an indeterminate progress bar.
- **Don't** blur a card, a rail, a header or a raised surface, nest a panel in a panel, or blur a full-window scrim.
- **Don't** use all-caps letter-spaced labels or text under 11px.
- **Don't** pass a colour into `Chip`, or hand-write `bg-<hue>-500/10 text-<hue>-400` for a status.
- **Don't** give the accent or violet a status, use violet or cyan as decoration, or give amber a state that needs nobody.
- **Don't** put design tokens in `style=` props or raw hex.
- **Don't** name a font utility after a face (`font-outfit`); a key with no `@theme` entry silently renders Inter.
- **Don't** copy a pattern because it sits nearby; check it against the Drift list first.

## Drift

Where shipped code contradicts AGENTS.md §4, as of this branch. Each item
names the rule it breaks, the evidence and a one-line fix, grouped by the
follow-up PR that would close it, in the order to work them.

**How counts were taken.** `rg -n -P '<pattern>' src -g '*.{ts,tsx}' -g
'!*.test.*' -g '!src/test/**'`, lines whose code starts with `//`, `*`,
`/*` or `{/*` dropped, then matches counted with `grep -oP`. "N/F" means N
matches in F files. Group 2, 1.15, 3.3, 5.2, 5.4, 5.6, the violet and
glow facts in group 3, and the type and radius counts in Overview and
Shapes were re-measured on branch `design/accent-buttons`. The rest of
group 1 and 4.1, 4.2, 4.5, 5.1 and 5.3 were measured on branch
`design/surfaces`, which deleted `src/components/Sidebar.tsx` (nothing
imported it); the other counts were taken before that and may include a
match or two from it. Group 2 rewrote many class strings, so those older
counts may have moved by a few.
Where a classification needed reading the code (overlay or card, Project
or not), it says so.

### 1. Surfaces and depth

Items 1.1–1.7 were closed on branch `design/surfaces` (see Resolved).
What is left is call sites the surface classes have not reached yet.

1.8 **Overlays hand-rolled instead of `Modal`.** Breaks: one raised surface for menus and dialogs, reached one way. Evidence: `fixed inset-0` = 13/13, of which 12 are overlays outside `ui/Modal.tsx` (`<Modal\b` = 12/10): GateView, McpConsentView, CommandPalette, DocsPanel, CommandSelector, EnvModal, StartFeatureModal, ShortcutHelp, the ProvidersPage "provider in use" confirm, AddressFindingsLaunch, ProviderSettings and NewTerminalMenu's centred variant. All now have a `.raised` body over a `bg-black/60` scrim, but each spells its own scrim and dismissal. NewProjectView's repo picker and WorkflowBuilder's unsaved-changes guard use `absolute inset-0`, so they cover only their own view; `Modal` is portalled and would cover the window. Fix: route the 12 through `Modal`; decide whether the two scoped ones stay scoped.

1.9 **ShortcutHelp's group cards are a fourth surface.** Breaks: a card inside a dialog is a `.nested-card`. Evidence: ShortcutHelp.tsx renders each shortcut group as `rounded-xl border border-white/5 bg-[#0d0f14]/40` inside its `.raised` body (one class string, repeated per group). Fix: `.nested-card`.

1.10 **DocsPanel's panes are darker bands.** Breaks: no band darker than the surface it sits on. Evidence: inside DocsPanel's `.raised` body, the nav is `bg-[#08090c]/40` and the content pane `bg-[#08090c]/60`. Fix: drop both fills; the nav keeps its hairline divider.

1.11 **WorkflowBuilder's root is an untokened near-black.** Breaks: three solid surfaces, values in `tokens.css`. Evidence: the WorkflowBuilder root is `bg-[#0b0d12]`, between the app and sidebar surfaces with no token of its own. Fix: `bg-surface-app`, or no fill.

1.12 **AddressFindingsLaunch's card is hand-spelled.** Breaks: one panel class. Evidence: its in-flow launch card is `rounded-xl border border-white/5 bg-black/20`, a translucent card with no class. Fix: `.nested-card` if its host is a panel, else `.panel`.

1.13 **Chrome spells the hairline by hand.** Breaks: the edge is a token. Evidence: the outer edges of TopBar, both ProjectRail asides, FeatureHeader, VersionDrawer, ConfigPanel and WorkflowBuilder's drawer spell `border-white/5` (7 sites), as do their inner dividers (10 more) and WorkflowList's rows on `bg-surface-panel` (2), read one by one. Tree-wide, `\bborder(-[trblxy])?-white/5\b` = 246/101 against `\bborder-hairline\b` = 1/1 (the top-bar search). Same value, so nothing renders differently. Fix: `border-hairline` on the chrome, then sweep the rest.

1.14 **12px spelled without the panel token.** Breaks: the panel radius is `--radius-panel`. Evidence: `\brounded-xl\b` = 62/41 (`rg -c '\brounded-xl\b' src` = 63/42 with one comment line), none on the same line as `.panel`, `.raised` or `.nested-card`. They are mostly panel-like boxes (artifact wells, notices, canvas ticket nodes, toasts, ShortcutHelp's cards from 1.9) plus some icon tiles. `--radius-panel` is a `:root` property, not a `@theme` key, so no `rounded-panel` utility exists. Fix: move each onto a surface class, or expose the radius as a utility for the ones that are not surfaces.

1.15 **Shadows cast outside `.raised`.** Breaks: only the raised surface casts a shadow. Evidence: neutral shadows at call sites, `(?<![\w:-])shadow-(sm|md|lg|xl|2xl)\b|shadow-\[0_(?!0_)[^\]]*\]` = 22/8 (no `shadow-2xl` left): `WorkflowNode` ×11, where every state now casts the resting `shadow-lg shadow-black/30` in place of its tone, diff and selection glows; `ArtifactViewer` ×3 (artifact wells `shadow-lg`); `AskCanvasNode` ×3; one each in `PinnedCanvasArtifact`, `TicketGraphNode`, `NewTerminalMenu`, `ProjectSettingsShell` and `StartSessionButton`. Fix: delete in-flow shadows; decide whether a canvas node is raised (and casts `--shadow-raised`) or a panel.

1.16 **Translucent near-black fills.** Breaks: three solid surfaces. Evidence: `bg-\[#[0-9a-fA-F]{6}\]/\d+` = 36/27, 16 distinct values, led by `bg-[#0d0f14]/90` ×6 (Ask and Discovery composers and the 1.15 footers), `bg-[#08090c]/40` ×5 (read-only fields in StrategyTab and HarnessModelPicker, DocsPanel's nav) and `bg-[#050608]/85` ×4 (artifact wells); also WorkflowList's and CodeEditorView's side panes at `/50`. It includes 1.9, 1.10, and 1.15's footers and translucent bodies. Fix: the solid `surface-*` utilities, or no fill where the host already has one.

### 2. Accent and buttons

Items 2.0–2.5 and 2.8 were closed on branch `design/accent-buttons` (see
Resolved). What is left is hue spent where §4 gives it no meaning.

2.6 **Amber for states that need nobody.** Breaks: amber = a human is needed, and only that. Evidence: `(tone|accent)\s*[:=]\s*['"]amber['"]|: 'amber'` = 32/17, 7 of them in `runStatus.ts` itself. Read one by one, these need nobody: `activitySync.ts` "reconnecting", `decomposeReview.ts` "Revised" and "blocked by …", `ticketPresentation.ts` "Blocked" ("waiting on an edge"), the `AskThreadCard` "network" chip, and `harnessVerdict.ts`'s "unrunnable" (multi-line, not in the count). Gate nodes, credential failures, PR conflicts, "Undecided — left for you", QuestionCard's "Needs you" and NotificationBell's `gate_pending` do. Fix: in-motion states to cyan, dependency and change kinds to slate; §4 needs a home for "warning".

2.7 **Three reds and off-palette hues.** Breaks: ruby = failed or destructive; colour only for meaning. Evidence: ruby classes 261/72, rose 68/17, red 21/4 (`(?<![\w-])(text|bg|border|ring|from|to|via|fill|stroke|shadow|outline)-<hue>-`). Rose carries GateView's "decision blocked" strip and WorkflowList's Delete icon button (a rose tint that fills `rose-600` on hover). `App.css` pins `text-red-400`, `bg-red-500/10` and two `border-red-500/*`. Yellow, orange and blue add 12/4 (`CodeEditorView` git letters and folder icons, `ErrorToast`'s orange "Network" accent, `GeneralTab`'s unpushed chip, WorkflowList's yellow Revert icon button). Fix: rose and red to ruby, and WorkflowList's two icon buttons to `Button` (`danger-subtle`, `ghost`); orange and yellow to amber only where a human is needed, else slate.

2.9 **Cyan as decoration.** Breaks: cyan = in motion; colour only for meaning. Evidence: cyan that marks no motion, read site by site: WorkflowList's "Custom" chip (the "Starter" chip beside it is emerald, the same mistake in another hue), VersionDrawer's "Current" badge, ErrorToast's `database` kind (`text-cyan-300`, `border-l-cyan-400/70`), CodeEditorView's branch name and icon, `agent_kind` as a cyan `Chip` (`Chip[^>]*tone="cyan"[^>]*>\{[\w.]*agent_kind\}` = 3/3: AskThreadCard, AskThreadSettingsPanel, DiscoveryCard), StartFeatureModal's remote-placement hints (`Server` icon, two `text-cyan-300/80` notes and the "Always unattended" box), and cyan left-border accents on cards (`\bborder-l-(2|4)\b[^"`]*\bborder-l-cyan-` = 2/2, ProvidersPage and MachinesView). Tree-wide `tone=["']cyan["']|tone:\s*['"]cyan['"]` = 34/21, most of it legitimate motion. Fix: names and kinds to slate, the remote hints and card accents neutral, the "Current" badge slate.

TicketForceStart's force-start is an amber-tinted hand-spelled button (`border-amber-500/25 bg-amber-500/[0.08] text-amber-400`). Forcing a ticket past its dependencies is a human decision, so the hue may stand; the hand-spelling is a `Button` miss either way.

### 3. Decoration

Violet is now only decoration, and all of it is this group's: `\bviolet-` = 35/10, in the blurred blobs (`ProjectSettingsShell` ×4, `ProjectHome`, `NewProjectView`, `PreferencesScreen`, `WizardShell`, `CreateFromZeroWizard`), the gradient avatars and tiles (`AccountMenu`, `ProvidersPage`, `PreferencesScreen`, `MachinesView`), `ProjectHome`'s hover wash, and `EmptyStateCard` ×12.

3.1 **Decorative blurred blobs.** Breaks: never decorative blurred blobs. Evidence: `rounded-full[^"`]*(?<![-\w])blur-(\[\d+px\]|xl|2xl|3xl)` = 20 lines in 11 files: `ProjectSettingsShell` ×5, `EmptyStateCard` ×4, `ProjectHome` ×2, `NewProjectView` ×2, and one each in `RemoteRunInbox`, `WizardShell`, `ProviderSettings`, `PreferencesScreen`, `CreateFromZeroWizard`, `MachinesView`, `ProvidersPage`. Fix: delete them.

3.2 **Sparkles as ornament.** Breaks: never sparkle icons as ornament. Evidence: `<Sparkles\b` = 12/10 (`ProjectRail` ×2, `EmptyStateCard` ×2, `NewProjectView`, `CreateFromZeroWizard`, `GateView`, `AgentBadge`, `StartFeatureModal`, `ShortcutHelp`, `NewTerminalMenu`, `StartSessionButton`). Most sit beside a text label that already says what the control does. `AgentBadge` in compact mode shows only the sparkle, so there it may carry meaning. Fix: use the icon of the thing (a plus, a terminal, a file) or none.

3.3 **Gradient ornament.** Breaks: no gradient fills on buttons or text (strictly, these are on tiles, avatars and washes, which §4's wording does not name; they are ornament either way). Evidence: `\bbg-gradient-to-[a-z]+` = 9/6: icon tiles (`MachinesView` ×2, `PreferencesScreen`, `EmptyStateCard`), avatars (`AccountMenu`, `ProvidersPage`), a hover wash on a `ProjectHome` card, and the `EmptyStateCard` CTA overlay and sheen. The two `radial-gradient` uses are canvas dot grids and are functional. Fix: flat fills.

3.4 **`hover:scale`.** Breaks: never `hover:scale`. Evidence: `(?<![\w-])hover:scale-` = 4/1 and `group-hover:scale-` = 4/1, all in the `EmptyStateCard` hero tiles. Fix: a border or background shift on hover.

3.5 **Shimmer outside progress.** Breaks: shimmer only on an indeterminate progress bar. Evidence: the `EmptyStateCard` sample-project button sweeps a `via-white/10` gradient across itself on hover. The one allowed shimmer, `.animate-progress-shimmer` (1/1, `TerminalStatusOverlay`), is cyan only. Fix: delete the sheen.

3.6 **The first-run hero.** `EmptyStateCard` combines 3.1–3.5, the tree's last glows (`shadow-[0_0_` = 4/1), a gradient icon tile and a hand-spelled violet-600 sample-project button with a violet glow, not a `Button`, and its copy ("a modern, premium control center…") breaks PRODUCT.md's binding voice (direct, no hype). Fix: rebuild it as one solid panel, three plain actions and plain copy; it is the first thing a new user sees.

### 4. Type and copy

4.1 **All-caps letter-spaced labels.** Breaks: never all-caps letter-spaced labels; meaning not carried by letter case. Evidence: `\buppercase\b` = 275/108, and 274 of those lines also carry positive tracking (`tracking-(wide|wider|widest|\[[0-9.]+(em|px)\])` = 295/115 overall; `tracking-wider` 178, `tracking-widest` 66, `tracking-wide` 35, six arbitrary values). The primitives multiply it: `Chip` is always `uppercase tracking-wide` (`<Chip\b` = 64/24), `FieldLabel` is `uppercase tracking-wider` (49/17), `SectionCard` too (0 uses), and `App.css`'s `.chat-bubble-sender`. Fix: sentence case in `Chip` and `FieldLabel` first (two one-line changes), then sweep the hand-rolled ones.

4.2 **Text under 11px.** Breaks: never text under 11px. Evidence: `(?<![\w-])text-\[(?:[0-9]|10)(?:\.\d+)?px\]` = 345/125: `text-[10px]` 288/113, `text-[9px]` 53/28, `text-[10.5px]` 3, `text-[9.5px]` 1. `Chip size="sm"` is 10px (`<Chip\b[^>]*size="sm"` = 47/19 on one line) and `RailBadge` is 10px. Fix: raise `Chip` `sm` and `RailBadge` to 11px, then replace 9px and 10px with an 11px step.

4.3 **Ungoverned type scale.** Breaks: no rule directly; it is why 4.2 recurs. Evidence: 14 arbitrary sizes (`text-[11px]` 371/119, `[13px]` 42, `[12px]` 16, which equals `text-xs`, `[11.5px]` 15, `[12.5px]` 4, plus one-offs) and no type-size tokens. Fix: a small size scale as `@theme` keys in `tokens.css`.

4.4 **Title Case strings.** Breaks: sentence case for labels and headings. Evidence (heuristic, so approximate): two or more capitalised words with only short connectives between them, found 71 times as same-line JSX text, 23 as standalone JSX text lines, 14 in `label`/`title`/`aria-label`/`placeholder` attributes and 24 in object-literal `label`/`title` values, about 132 in all. Examples: "Workspace Bootstrap In Progress", "Configure Worktree Strategy", "Copy Complete Output", "Save Changes", "Provider Type", and shortcut and command-palette labels in `src/lib/shortcuts.ts` and `App.tsx`. Some hits are names (product or workflow names). Fix: sentence case, file by file.

4.5 **"Workspace" where the noun is Project.** Breaks: PRODUCT.md Vocabulary (the UI says Project); §4's translatable-copy rule. Evidence: `(?<![\w./-])[Ww]orkspaces?(?![\w/-])` on user-facing lines = 57 matches on 55 lines in 19 files; every line was read. About 45 lines name a Project (settings headings, bootstrap screens, the rail's "Workspaces" header, "Delete Workspace"). The other 10 use the word for something else and need their own noun: the app-wide data directory (`PreferencesScreen` ×3), the whole install (`McpConsentView` ×2), a Git host account (`ProvidersPage`), the Discovery pane (`DiscoveryWorkspaceRow`), a feature worktree path (`NewProjectView`), the command palette placeholder (`TopBar`) and the empty-state back label (`parentView.ts`). Fix: rename to Project; give the other 10 their own words.

4.6 **"Provider" where the thing is a Git host connection.** Breaks: PRODUCT.md (in code a ProviderInstance is a GitHub or GitLab connection), where the UI noun is still an Open Decision. Evidence: a scripted pass over string literals and JSX text found 48 occurrences on 46 lines in 21 files, a lower bound because multi-line JSX strings are missed (for example `PostReviewComment`'s "using your provider"). Examples: "Source Providers" (`ProvidersPage`, `TopBar` title), "Connect a Provider", "Provider Type", "No Providers Mapped", the wizard step label "Provider". Fix: settle the noun in PRODUCT.md, then rename.

4.7 **Redundant `font-sans`.** Breaks: UI text needs no class. Evidence: `\bfont-sans\b` = 24/10, including the root shell in `App.tsx`. Fix: delete.

### 5. Raw hex and `style=` tokens

5.1 **Arbitrary hex classes.** Breaks: never raw hex for design tokens. Evidence: `\[#[0-9a-fA-F]{3,8}\]` = 131/60, 12 distinct values. 83 restate tokens: `#08090c` ×45 (`surface-app`), `#0d0f14` ×21 (`surface-sidebar`), `#050608` ×17 (`surface-well`). 48 are near-blacks with no token: `#050508` ×19, `#0b0d12` ×10, `#0a0c10` ×8, `#0c0d12` ×4, `#12161e` ×3 and four singletons. Fix: replace the 83 with the `surface-*` utilities that branch `design/surfaces` added, and fold the 48 onto the nearest surface.

5.2 **Tokens are bypassed.** Breaks: values live in `tokens.css` and are read from there. Evidence: TSX reaches a token only through 10 `bg-[var(--…)]` uses and the surface utilities (`bg-surface-*` 21/14, `border-hairline` 1/1); `--text-*` is referenced from TSX zero times. Components colour status hues through palette classes (by 2.7's pattern: cyan 207, ruby 261, amber 247, emerald 207, violet 35); only the surfaces and the accent are reached as token utilities (`bg-accent` 72/39, `border-accent-ring` 102/42, `border-accent-edge` 52/36, `bg-accent-tint` 51/34, `text-accent-ink` 23/15). The two neutral ladders disagree: `--text-*` are Tailwind gray (#f3f4f6 / #9ca3af / #6b7280), while components use slate and `text-white` (383), and the root shell sets `text-white` over `body`'s `--text-primary`. Fix: one neutral ladder as tokens; the status hues stay behind `runStatus.ts`'s `TONE_*` maps.

5.4 **Colour values outside `tokens.css`.** Breaks: token values live in `tokens.css`. Evidence: `App.css` still declares 18 hex literals in rules: 11 in the `PINNED UTILITY COLOURS` block (32 utility classes re-declared at Tailwind v3 values) and 7 in the native-control rules, plus rgba values in `.panel-hover`'s 8% border, `.bubble-code`, the agent chat bubble, the scrollbar, the progress shimmer and the skeleton. Fix: move the values to tokens; whether to keep the v3 pins is a palette decision, per the block's own comment.

5.5 **Hex in library themes.** Breaks: no raw hex (lower priority: the libraries need values, not classes). Evidence: `['"]#[0-9a-fA-F]{3,8}['"]` = 18/3: `TerminalSurface` (xterm theme) ×11, `canvas/flowGraph.ts` ×5, `WorkflowCanvas` ×2. Fix: read them from the CSS custom properties at runtime, or from one exported token map.

5.6 **Unused tokens.** Evidence: no `var()` or utility reads `--text-muted`, `--text-dark`, `--transition-spring` or `--transition-smooth`. Fix: delete them.

### 6. Form controls

6.1 **`!important` native-control rules beat call-site utilities.** Breaks: no raw hex for tokens; a call site's utilities should apply (the reason for PR #250's layers). Evidence: the unlayered rules at the end of `App.css` hold 17 `!important` declarations on `select`, `option`/`optgroup`, `input[type=text|search|email|password|url]`, `textarea` and placeholders (#0a0a0e fill, #e2e8f0 text, #475569 placeholder, an accent tint and ink checked option, and on `select` `appearance: none`, a chevron `background-image` and `padding-right: 28px`). A scripted pass over JSX tags found 128 matching fields in 54 files (`select` 45, `input type="text"` 62, `password` 7, `search` 2, `textarea` 12). 98 of them carry a `bg-*` utility and 94 a text-colour utility that never apply, and the 14 using `.input-field` lose its background too. Inputs without a `type` attribute (8) and number inputs (10) are not matched, so they render differently. Moving the rules into `@layer base` would let 31 of the 45 selects' padding utilities (`p-`/`px-`/`pr-`) override the 28px chevron gutter. That it would also lose the chevron `background-image` was reported to me and not verified here. Fix: move the rules to `@layer base` on token colours, give `select` its chevron and gutter through a class or a `pr-7` convention, then remove the dead `bg-*` and text utilities.

6.2 **Selection and body font as `!important`.** Evidence: `::selection` is an unlayered `!important` (the accent at 45%), and the "WebKitGTK / Tauri compatibility layer" re-declares `body`'s font with `!important`, duplicating the `@layer base` rule. Fix: fold both into `@layer base`.

### Outside the PR groups

- **Status vocabularies outside `runStatus.ts`.** §4 says status hues resolve through `runStatus.ts` and nowhere else. About a dozen modules keep their own label→tone tables, reusing its `RunStatusTone` type and `TONE_*` maps: `syncPanel`, `discoveryProgress`, `askLifecycle`, `ticketPresentation`, `decomposeReview`, `activitySync`, `pullRequestRow`, `harnessVerdict`, `staleness` and `pipelineCard` in `src/lib/`, plus `RemoteRunInbox`, `canvas/types`, `StorageProjectCard` and `ReviewFailureCard`. Whether reusing the `TONE_*` maps satisfies "through runStatus.ts" is for §4 to say; 2.6 and 2.9 are where these tables disagree with it.
- **No-op animation.** `NotificationBell`'s toast uses `animate-[fadeIn_120ms_ease-out]`, and no `fadeIn` keyframe exists (the keyframe is `fade-in`).
- **Stale written guidance.** `.design-sync/conventions.md` still teaches the retired system: violet = "agent working", emerald = "running agent", `style={{ background: 'var(--bg-panel)' }}` for tokens, uppercase `tracking-widest` small labels, `SectionCard` first, and `StatusBadge variant="pill"`, a prop that no longer exists.

### Resolved

- The four `Modal` dialogs whose body was a `.panel` (AskThreadSettingsPanel, NewAskThreadModal, NewDiscoveryModal, DecomposeModal) and four hand-spelled floating bodies (StartSessionButton's menu, ScheduleDrawer, TurnCompleteToast, TerminalStatusOverlay's card) are on the raised surface, and their footer bands are gone (this branch).
- The token comments that gave each accent a meaning were replaced by a pointer to AGENTS.md §4 (PR #251).
- Dead CSS: `.app-container`, `.sidebar-*`, `.list-item`, `.panel-header`, `.glow-*`, `.text-glow-*`, `.supervisor-*`, `.agent-markdown`, `.card-grid`/`.metric-*` and the `translateX` hover motion were deleted in PR #250.
- Unlayered `App.css` rules silently beating utilities: rules moved into `@layer base` / `@layer components` in PR #250, so the amber "needs you" ring, ruby confirm borders and row status stripes now render.
- The `!important` colour safelist beating `hover:`/`focus:` variants: now a non-important `@layer utilities` block in PR #250. Its v3 values remain (5.4).
- Radius and shadow re-spellings on what is now `.panel` (`rounded-lg/xl/2xl`, `shadow-2xl`), and the violet halo on the active rail project, were removed in PR #250.
- Token values moved verbatim to `src/styles/tokens.css` in PR #251, and §4 now points there.
- `runStatus.ts` re-toned in PR #251: `bootstrapping` amber→cyan, `verifying` violet→cyan, `published` violet→cyan. Violet names no run status in `META`, and amber in `META` marks only the settled needs-a-human statuses (plus `REVIEW_READY_META`).
- The App.css banner naming the system "High-Tech Dark Neon Glassmorphism" and its "Glassmorphism" section titles were replaced in PR #251 by a pointer to `tokens.css`, §4 and this file.
- Old §4 wording no longer in conflict: "emerald = running agents", four hues instead of six, "status dots pulse", and a "card-surface token" that never existed. §4 was rewritten.
- 1.1 `.glass-panel` translucent and blurred: on branch `design/surfaces` the panel is solid (`--bg-panel` #11151d, the old colour composited over the app ground) with no blur and no shadow, and is renamed `.panel` (`.panel-hover`), with `--border-glass` renamed `--border-hairline`.
- 1.2 Blur on cards, rails and headers: on branch `design/surfaces` the top bar, project rail, feature header, drawers, list rows, attachment chip and dropzone, canvas nodes and `WizardShell` stopped blurring and took solid surfaces, and the two blurred scrims went flat. `backdrop-blur…` is down from 35/27 to 7/5, all on controls over a live canvas or the terminal status overlay; `Sidebar.tsx`, which nothing imported, was deleted.
- 1.3 Panel in a blurred rail, panel as a control: on branch `design/surfaces` the active project is a selected row (`bg-surface-panel-hover`) and the top-bar search trigger a field (`bg-surface-well border-hairline`).
- 1.4 No raised token, translucent and split panel tokens: on branch `design/surfaces` a `@theme static` block in `tokens.css` defines `surface-app`, `-sidebar`, `-well`, `-panel`, `-panel-hover`, `-raised`, `hairline` and `hairline-raised`, which the `--bg-*` and `--border-*` names alias, plus `--shadow-raised`; `.nested-card` lost its rgba fill, and `bg-[rgba(18,22,30…` re-spellings are down from 6/4 to 0.
- 1.5 The raised layer spelled a dozen ways: on branch `design/surfaces` 27 files' menu, popover, dialog, palette and toast bodies use `.raised` (33 uses), every dialog scrim is a flat `bg-black/60` (`Modal`'s default included), and the gate and artifact modal halos are gone. Routing the hand-rolled overlays through `Modal` remains (1.8).
- 1.6 Radius beyond the panel token: on branch `design/surfaces` `rounded-2xl` went from 12/12 to 0, so no surface is rounder than the panel. `rounded-xl` without a surface class remains (1.14).
- 1.7 The panel hairline re-spelled: on branch `design/surfaces` the white re-spellings were dropped (hue borders with meaning kept), so no `.panel` or `.raised` call site adds a `border-white/*`, radius or shadow (was 24 of 91 panel sites).
- 5.3 `style=` props carrying design values: the surface props on `ShortcutHelp` and `WizardShell` went on branch `design/surfaces`, and EnvModal's Browse button override (padding and font size on `.btn-secondary`) went with that class on `design/accent-buttons`. `style=\{\{` = 19/12, the rest geometry as read before.
- 2.0 Stock violet as the accent: on branch `design/accent-buttons` Signal is six `@theme static` colours in `tokens.css` (`accent`, `-hover`, `-ink`, `-ring`, `-edge`, `-tint`), every action, selection and focus use moved to them, and `--accent-purple*`, `--border-glass-focus` and the unused `--accent-cyan/emerald/ruby` are deleted. `\bviolet-` is down from 446/106 to 35/10, all decoration (group 3).
- 2.1 Glow shadows: `shadow-[0_0_` is down from 94/38 to 4/1, all in `EmptyStateCard` (3.6). `StatusBadge` dots and pipeline-card accent bars are flat, `.animate-pulse-glow(-amber)` keep their names but cast no shadow, the `.input-field` and `.chat-input` focus glows are gone, canvas nodes lost their tone, diff and selection glows, and every `--accent-*-glow` token is deleted.
- 2.2 Gradient primary button: `.btn-primary` and `.btn-secondary` are deleted from `App.css`, and `AskCanvasNodeInspector`'s gradient button is a `Button`.
- 2.3 Two primary spellings, no primitive: `Button` (`src/components/ui/Button.tsx`) with primary, secondary, ghost, danger and danger-subtle variants and sm/md/lg sizes; `<Button` = 172/63, `buttonClasses(` = 7/5. No status-hue variant, so gate Approve, Replay and Retry are primary.
- 2.4 Focus and selection in cyan: one `:focus-visible` ring (2px `accent-ring`, offset 2px) in `@layer base`; fields focus to `border-accent-ring` (`focus:border-cyan-` 81/32 → 0); `TabBar`, `SegmentedControl`, rows, tiles, palette highlights and the selected canvas node select with `accent-edge` and `accent-tint`; `::selection` and `select option:checked` are the accent; the scrollbar thumb and splitter hover are neutral.
- 2.5 Violet naming states and decorating: `violet` is out of `RunStatusTone` and every `TONE_*` map, `HeaderNavItem` lost its `accent` prop, and `ActionRow`'s tone is optional with a primary or danger button. Chat bubbles are neutral and the stream caret is `--text-secondary`. "Asking" and "Interviewing" are cyan while a turn runs and slate otherwise; "Open", "Reclaimable", canvas node kinds other than gate (amber), and model, workflow and placement chips are slate; `syncPanel.ts`'s state-free actions (sync, merge, resolve, review) carry no tone, so their rows are neutral and the press is the accent.
- 2.8 Emerald for healthy machines: §4 now reads "emerald = finished well, or healthy", so `StatusBadge` mapping machine `idle`/`active` to emerald is within it.

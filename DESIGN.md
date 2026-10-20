---
name: Demeteo
description: Dark-only desktop control plane for coding agents, headed for a quiet instrument with an indigo "Signal" accent; much of the shipped chrome is still the retired translucent-glass, stock-violet look.
colors:
  violet-signal: "#8b5cf6"
  violet-action: "oklch(54.1% 0.281 293.009)"
  violet-ink: "oklch(70.2% 0.183 293.541)"
  cyan-stream: "#06b6d4"
  cyan-ink: "#22d3ee"
  emerald-done: "#10b981"
  emerald-action: "oklch(59.6% 0.145 163.225)"
  emerald-ink: "#34d399"
  amber-needs-you: "oklch(76.9% 0.188 70.08)"
  amber-ink: "oklch(82.8% 0.189 84.429)"
  ruby-failure: "#ef4444"
  ruby-action: "#dc2626"
  ruby-ink: "#f87171"
  ground-app: "#08090c"
  ground-rail: "#0d0f14"
  ground-well: "#050608"
  panel-glass: "rgba(18, 22, 30, 0.92)"
  panel-glass-hover: "rgba(28, 34, 46, 0.85)"
  panel-nested: "rgba(18, 22, 30, 0.55)"
  field-fill: "rgba(10, 12, 16, 0.6)"
  hairline: "rgba(255, 255, 255, 0.05)"
  hairline-strong: "rgba(255, 255, 255, 0.10)"
  focus-violet: "rgba(139, 92, 246, 0.4)"
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
  hero: "16px"
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
  card-glass:
    backgroundColor: "{colors.panel-glass}"
    rounded: "{rounded.panel}"
    padding: "20px"
  card-glass-hover:
    backgroundColor: "{colors.panel-glass-hover}"
  card-nested:
    backgroundColor: "{colors.panel-nested}"
    rounded: "{rounded.panel}"
    padding: "14px 16px"
  field-well:
    backgroundColor: "{colors.ground-well}"
  chip-cyan:
    backgroundColor: "rgba(6, 182, 212, 0.10)"
    textColor: "{colors.cyan-ink}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    padding: "2px 10px"
  chip-violet:
    backgroundColor: "rgba(139, 92, 246, 0.10)"
    textColor: "{colors.violet-ink}"
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
    backgroundColor: "{colors.violet-action}"
    textColor: "{colors.ink-white}"
    typography: "{typography.body-dense}"
    rounded: "{rounded.control-lg}"
    padding: "8px 16px"
  button-primary-hover:
    backgroundColor: "{colors.violet-signal}"
  button-approve:
    backgroundColor: "{colors.emerald-action}"
    textColor: "{colors.ink-white}"
    typography: "{typography.body-dense}"
    rounded: "{rounded.control-lg}"
    padding: "8px 20px"
  button-approve-hover:
    backgroundColor: "{colors.emerald-done}"
  button-destructive:
    backgroundColor: "{colors.ruby-action}"
    textColor: "{colors.ink-white}"
    typography: "{typography.body}"
    rounded: "{rounded.control-lg}"
    padding: "10px 20px"
  button-secondary:
    backgroundColor: "rgba(255, 255, 255, 0.05)"
    textColor: "{colors.ink-white}"
    typography: "{typography.body-dense}"
    rounded: "{rounded.control-lg}"
    padding: "8px 16px"
  button-secondary-hover:
    backgroundColor: "rgba(255, 255, 255, 0.10)"
  button-legacy-primary:
    backgroundColor: "{colors.violet-signal}"
    textColor: "{colors.ink-white}"
    rounded: "{rounded.field}"
    padding: "10px 18px"
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
    textColor: "{colors.cyan-ink}"
    typography: "{typography.body}"
    padding: "10px 16px"
  tab-idle:
    textColor: "{colors.ink-slate-400}"
  rail-item-active:
    backgroundColor: "rgba(255, 255, 255, 0.07)"
    textColor: "{colors.ink-white}"
    rounded: "{rounded.control-lg}"
    padding: "8px 12px"
  rail-item-idle:
    backgroundColor: "{colors.ground-rail}"
    textColor: "{colors.ink-slate-400}"
  top-bar:
    backgroundColor: "rgba(13, 15, 20, 0.8)"
    height: "56px"
    padding: "0 24px"
  modal-scrim:
    backgroundColor: "rgba(8, 9, 12, 0.85)"
---

<!--
  Record of the system AS SHIPPED on branch design/direction-and-tokens
  (master d57a564b = PR #250, plus this branch's token move and runStatus
  re-toning), refreshed 2026-10-20 from code, not intentions. The direction
  it is measured against is AGENTS.md §4 ("a quiet instrument"); where the
  code still does something else, the body records what the code does and
  the "Drift" section at the end says what contradicts §4 and how to fix it.

  Ground truth: src/styles/tokens.css (values), src/App.css (rules),
  src/components/ui/*, src/lib/runStatus.ts, and the views named below.
  Counts are from ripgrep over non-test .ts/.tsx under src/ with
  comment-only lines dropped; the exact patterns are in the Drift section.
  They include src/components/Sidebar.tsx, which nothing imports.

  The frontmatter holds only shipped values. The indigo "Signal" accent
  (decision 61) is decided and not yet shipped, so it appears only in the
  Colors section, marked as the target.

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
structural: one status vocabulary (`src/lib/runStatus.ts`) with violet
taken out of it, a motion budget that keeps infinite animation to small
liveness marks, a single panel class whose radius call sites no longer
re-spell, and cascade layers in `src/App.css` so a utility beside a class
takes effect. Most surfaces, though, are still the old look: the accent
is stock violet-500/600 rather than Signal, `.glass-panel` blurs and is
translucent, glows mark status dots, accent bars and primary buttons, decorative blur blobs sit behind status
screens, and labels are uppercase mono with wide tracking. The Drift
section lists these as follow-up PRs.

It is an Operate tool, and density shows it. The commonest text sizes are
12px (`text-xs`, 408 uses), hand-set 11px (371) and 10px (289). Machine
output (ids, costs, token counts, statuses, paths) is set in Fira Code
and headings in Outfit. There is no light theme and no theme switch.

**Key Characteristics:**
- Dark-only. Grounds sit between #050608 and #12161e, never lighter.
- Colour is meaning. `runStatus.ts` maps every run status to one of five status hues; neither the accent nor violet names a status.
- One accent for primary actions, selection and focus: stock violet as shipped, indigo "Signal" as decided (not shipped).
- One panel class (`glass-panel`, 91 call sites), still translucent and blurred as shipped.
- Mono-heavy and dense, with an ungoverned type scale (14 arbitrary text sizes).
- Depth is still mostly translucency, blur and coloured glow; §4 replaces all three with solid surface steps.
- Motion is budgeted for cost: `data-motion="paused"` freezes everything while the window is unfocused.

## Colors

Near-black neutrals, five status hues plus inert slate, and one accent
held back for action, selection and focus. As shipped the accent is
Tailwind's stock violet; the decided accent is indigo "Signal" (below). Values live in
`src/styles/tokens.css`; most components still reach colour through
Tailwind palette classes rather than those tokens (see Drift).

### Primary
- **Electric Violet** (`violet-signal`, `--accent-purple`): the shipped accent, for primary actions and some selection and focus, and no run status. It is drift to migrate to Signal (Drift 2.0). The fill on primary buttons is the deeper **Violet Action** (`violet-action`, violet-600), stepping to violet-500 on hover. `.input-field` focuses to **Focus Violet** (`focus-violet`, `--border-glass-focus`). As shipped, violet also still tints model and workflow chips, lifecycle labels outside `runStatus.ts`, canvas node kinds and the streaming caret (Drift 2.5).
- **Violet Ink** (`violet-ink`): violet text in chips and icons.

**Decided, not shipped: indigo "Signal"** (decision 61 in
`docs/DECISIONS.md`, values arriving in a follow-up PR). These are the
target, not tokens; nothing in `tokens.css` or `src/` carries them yet.
- Fill `#5a60c2`, always with a white label (5.40:1); hover `#6269cc`.
- Text `#838ef4`.
- Focus ring `#6e78db`.
- Selected border `#6972c0`; selected tint `rgba(90,96,194,.16)`.

Decision 61 records why stock violet was dropped: white on violet-500
measured 4.23:1 (under AA), and its focus and selection borders measured
under 2:1. Signal is never a status, and violet stays out of statuses too,
because it sits too close to the accent.

### Secondary
- **Stream Cyan** (`cyan-stream`, `--accent-cyan`): in motion. Running, bootstrapping, verifying, and a published PR out for review all resolve to cyan. As shipped it is also the selected tab and segment, `::selection`, the selected `<option>`, the scrollbar thumb on hover and most input focus borders, all of which §4 gives to violet (Drift 2.4). Cyan is the most-used hue (540 palette-class uses).
- **Cyan Ink** (`cyan-ink`): selected-tab text, token-count icons, markdown links.

### Tertiary
- **Done Emerald** (`emerald-done`, `--accent-emerald`): finished well (completed, PR ready, merged). `StatusBadge` also maps machine `idle`/`active` to emerald. **Emerald Action** (`emerald-action`, emerald-600) fills the gate's Approve and other confirm-and-proceed buttons.
- **Needs-You Amber** (`amber-needs-you`, amber-500): a human is needed, and in `runStatus.ts` only that: the settled gate, credential and interrupted statuses, plus `REVIEW_READY_META`. `pipelineFilter.ts` derives the "Needs you" band from it. Amber has no token in `tokens.css`; it comes from Tailwind's palette. It is the only hue with its own finite animation (`pulse-attention`, three beats, then lit) and its own card treatment (`ring-1 ring-amber-500/40` on a pipeline card that needs you).
- **Ruby** (`ruby-failure`, `--accent-ruby`, `@theme --color-ruby-*`): failed or destructive. A full 50–950 scale in `tokens.css` at Tailwind v3's sRGB red values, so `ruby-500` is #ef4444 while v4's own `red-500` is a slightly different OKLCH red. Rose and raw red are also in use (Drift 2.7).

### Neutral
- **App Ground** (`ground-app`, `--bg-app`): body background and the root shell, which spells it as `bg-[#08090c]`.
- **Rail Ground** (`ground-rail`, `--bg-sidebar`): top bar (at 80%), project rail (at 50%), gate modal body, some menus.
- **Well** (`ground-well`, `--bg-well`): the darkest surface, for terminal and code wells sunk into a panel (`.panel-field`, gate artifact output).
- **Panel Glass** (`panel-glass`, `--bg-panel`) and **Panel Glass Hover** (`panel-glass-hover`, `--bg-panel-hover`): the `.glass-panel` fill. Translucent as shipped.
- **Nested Panel** (`panel-nested`): `.nested-card`, spelled as a raw rgba in `App.css` rather than a token.
- **Field Fill** (`field-fill`, `--bg-input`): `.input-field` and `.chat-input`. Most call-site inputs use `bg-black/40` instead, and native fields are forced to #0a0a0e (Drift 6.1).
- **Hairline** (`hairline`, `--border-glass`) and **Hairline Strong** (`hairline-strong`): the two border weights.
- **Token inks** (`token-ink-*`, `--text-*`): Tailwind *gray*, read only by `body` and `App.css` rules.
- **Component inks** (`ink-white`, `ink-slate-200` … `ink-slate-600`): what components render. Headings are `text-white` (386 uses); body and meta run down the slate ladder. Slate text classes are pinned to v3 sRGB hex by the `PINNED UTILITY COLOURS` block in `App.css`.

### Named Rules

**The Colour-Is-Meaning Rule.** Neutral greys do everything that is not a status, an action, a selection or focus. A status colour always ships with a label or icon, because hue alone carries no state (WCAG 2.2 AA is the target).

**The Accent-Is-Not-A-Status Rule.** The accent marks primary actions, selection and focus, never a status, and neither does violet, which sits too close to it. `runStatus.ts` still exports a `violet` tone in `RunStatusTone` for non-status accents; every use of it is drift (2.5).

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
`minmax(0,1fr) auto minmax(min-content,1fr)`, 24px side padding, rail
ground at 80% with `backdrop-blur-md`, hairline bottom border) sits over
a row of the project rail and the content. The rail is 240px expanded and
56px collapsed, rail ground at 50% with `backdrop-blur-xl` and a hairline
right border.

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
a translucent thumb that turns cyan on hover and violet while dragged.
Long lists use `content-visibility: auto` with measured intrinsic sizes
(`.pipeline-card`, `.timeline-step-card`) rather than a virtualisation
dependency.

## Elevation & Depth

§4: depth is a surface step, not an effect. Three solid surfaces (the app
background, the panel, and the raised layer for menus and dialogs), each
with a 1px hairline, and backdrop blur only on overlays floating over live
content.

As shipped, depth is still mostly **translucency, blur and glow**. The
ground ladder exists (well #050608 < app #08090c < rail #0d0f14 < panel
rgba(18,22,30,.92)), but the panel is translucent and blurred, the raised
layer has no token and is spelled per call site, and coloured halos mark
status. There is one neutral drop shadow, on `.glass-panel`, and
`shadow-2xl` (14 uses) on dialogs and status cards.

### Shadow Vocabulary
- **Panel drop** (`box-shadow: 0 8px 32px 0 rgba(0,0,0,0.4)`): built into `.glass-panel`. The one shadow §4 keeps; call sites do not re-spell it.
- **Status dot halo** (`0 0 8px rgba(<tone>, 0.8)`): `StatusBadge` dots. Retired by §4.
- **Accent bar halo** (`0 0 10px rgba(<tone>, 0.8)`): the 4px left bar on a pipeline card (slate at 0.6). Retired.
- **Primary glow** (`0 0 15px rgba(139,92,246,0.4)` and variants): violet buttons; Approve has the emerald equivalent. Retired.
- **Live pulse glow** (`0 0 10px 1px rgba(6,182,212,0.5)`): static halo on `.animate-pulse-glow`, faded by opacity; amber variant on `.animate-pulse-glow-amber`. Retired.
- **Focus glow** (`0 0 8px` violet on `.input-field:focus`, cyan on `.chat-input:focus`). Retired.
- **Modal halo** (`0 0 50px rgba(139,92,246,0.15)`): the gate modal and `ArtifactModal`. Retired.

### Named Rules

**The Surface-Step Rule.** Depth comes from which of three solid surfaces an element sits on, each with a 1px hairline. A card is the panel surface; a menu or dialog is the raised one.

**The Blur-Only-On-Overlays Rule.** Backdrop blur belongs to an overlay floating over live content (command palette, dialogs), never to a card, a rail or a header.

**The No-Glass-In-Glass Rule.** A card inside a panel is a `.nested-card` (55% panel fill, hairline, 12px radius, no blur, no shadow), never a second panel. A field filling a panel (canvas, stream log) is a `.panel-field`: well fill, no border, no radius.

**The No-Blur-On-Scrims Rule.** A full-window scrim never takes `backdrop-blur`. `Modal` uses a flat `#08090c` at 85%, because a blurred scrim re-blurs the whole window every frame something animates beneath it.

## Shapes

Softly squared. Controls take small radii and panels take
`--radius-panel` (12px). Circles are for status dots, count badges and
avatar or icon tiles.

- **Control** (4px, `rounded`, 126 uses): chips, keyboard hints, `.btn-icon`, inline code.
- **Field** (6px, `rounded-md`, 69): `.input-field`, `.btn-primary`, `.btn-secondary`, selected segments, the top-bar search.
- **Control Large** (8px, `rounded-lg`, 422): the de facto default for buttons, inputs, rail items, icon buttons and inner boxes.
- **Panel** (12px, `--radius-panel`): `.glass-panel`, `.nested-card`. Elsewhere 12px is spelled `rounded-xl` (71 uses).
- **Hero** (16px, `rounded-2xl`, 12 uses): the gate modal, some dialogs, the composer, the first-run icon tile. Larger than the panel radius (Drift 1.6).
- **Pill** (`rounded-full`, 133): status dots (8px), chip dots (6px), count badges, avatars.

Borders are 1px hairlines, almost always white at 5% or 10%. Hue borders
appear at `/20`–`/40` on chips, badges, warnings and tinted buttons. A
pipeline card's status is a 4px left bar clipped by the card
(`overflow-hidden`), not a border colour.

### Named Rules

**The One-Panel-Radius Rule.** Every panel-level surface is 12px (`--radius-panel`), and a call site never re-spells it. A card, a canvas well and a banner are the same object at different depths, and a second value reads as a mistake. Smaller radii belong to controls.

## Components

Tailwind class strings in TSX, with primitives in `src/components/ui/`
that own a decision so call sites cannot re-open it. There is no Button
primitive, so buttons are spelled per call site. `SectionCard` exists
and has no call sites.

### Buttons
- **Character (shipped):** a saturated fill, often with a glow, for the action that moves the pipeline; quiet translucent white for the rest.
- **Shape:** 8px at most call sites (`rounded-lg`); 6px on the `App.css` `.btn-*` classes.
- **Primary:** violet-600 fill, white text, 8px × 16px at 12px semibold or bold, or 10px × 20px at 14px on status screens. Hover steps to violet-500, usually adding a violet glow. Disabled drops to violet-900 at 40%.
- **Approve / Proceed:** emerald-600 fill with a resting emerald glow, stepping to emerald-500 on hover. The gate's Approve is the canonical one.
- **Destructive:** a ruby-600 fill (ProjectHome retry) or a rose tint (rose-500/10 fill, rose-500/20 border, rose-400 text; GateView "Abort feature").
- **Secondary:** white at 5% (10% on hover), optional `border-white/10`, white or slate-300 text.
- **Tone-tint:** hue/10 fill, hue/30 border, hue-400 text, turning white on hover (GateView "Redirect / Loop").
- **Ghost / Icon:** no fill, slate-400 icon, `hover:bg-white/5 hover:text-white`, 4–8px padding.
- **Legacy class buttons:** `.btn-primary` (135° violet-500 → #7c3aed gradient, `0 0 14px` violet glow on hover, weight 550, 15 uses in the Ask and Discovery views) and `.btn-secondary` (transparent with hairline, 28 uses).

### Chips
- **Style:** `Chip` is the one pill (64 call sites). Fira Code, uppercase, `tracking-wide`, 4px radius, 1px hue border at `/20`, hue fill at `/10`, hue ink at `-400`. `md` is 12px with 2px × 10px padding; `sm` is 10px with 2px × 8px.
- **State:** a status chip resolves its label, hue and liveness from `runStatusMeta`. A live status gets a leading 6px `bg-current` dot with `animate-pulse`. The dot pulses, never the pill, because a pulsing box holding text re-rasterises the text every frame. `maxWidth` is the only inline-style escape.

### Cards / Containers
- **Corner Style:** 12px (`--radius-panel`).
- **Background:** `.glass-panel`: `--bg-panel` plus `backdrop-filter: blur(12px)`, the hairline and the panel drop. `.glass-panel-hover` lifts to `--bg-panel-hover` with an 8% border.
- **Shadow Strategy:** the panel drop only. See Elevation & Depth.
- **Border:** built in. Call sites are not meant to add their own (the `.glass-panel` comment in `App.css` says so); 24 do (Drift 1.7).
- **Internal Padding:** 20px standard (`SectionCard`, comfortable pipeline card), 12px compact, 24–32px for dialogs and status cards.
- **Nested:** `.nested-card` groups fields inside a panel (ticket editor) at 14px × 16px padding.

### Pipeline Card (signature)
The project view's row: a hoverable `.glass-panel` with a 4px left accent
bar in the status hue (with a glow, as shipped), laid out in three tiers
(`data-tier`):
1. **Scan:** the title (Outfit, 18px or 14px, white, two-line clamp), a status `Chip` (`sm`), elapsed time in white mono, and a chevron that appears on hover.
2. **Context:** workflow (violet chip), transport (hue chip with a CPU icon), optional staleness chip, cost and tokens in slate mono.
3. **Detail:** feature id and description in slate-500, truncated.

A card that needs a human adds `ring-1 ring-amber-500/40`, which renders
since PR #250.

### Inputs / Fields
- **Style:** `.input-field` (field fill, hairline, 6px, 10px × 12px, 14px Inter). Most inputs are hand-spelled: `bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white`. Native `select`, text-like `input`s and `textarea` are forced to #0a0a0e with slate-200 text and slate-600 placeholders by unlayered `!important` rules in `App.css`, and `select` gets a chevron image with 28px right padding. Those rules beat the call sites' own background and text utilities (Drift 6.1).
- **Focus:** a border shift. `.input-field` goes violet with a soft glow; call-site inputs mostly go `focus:border-cyan-500/50` (81 uses) against `focus:border-violet-*` (19). `focus:outline-none` appears 98 times and a visible `focus-visible:` style in 7 files.
- **Labels:** `FieldLabel`: 12px uppercase Fira Code, slate-400, `tracking-wider`, 6px below.

### Navigation
- **Top bar:** brand mark plus lowercase "demeteo" (Outfit 700, 18px). A centred command-palette button styled as a `glass-panel` at 6px. On the right, `HeaderNavItem`s: 16px icons tinted by an `accent` prop (violet, cyan or amber), mono 12px labels at `labels` density, `aria-label` only at `icons` density. Active is `bg-white/[0.04]` with white text. The corner badge is a solid amber count, a cyan activity dot or an emerald pulse dot.
- **Project rail:** projects with a `StatusBadge` dot, a 12px name and a 9px mono sub-line. The active item is a `glass-panel` at the 8px control radius (its violet halo was removed in PR #250). Collapsed, 36px initial tiles (active: violet-500/20 fill, violet-300 text, violet-500/30 border). `RailNavItem` renders the Terminals entry (active `bg-white/[0.07]` plus `border-white/10`). `RailBadge` handles counts (mono 10px, capped at 9+ or 99+).
- **Tabs:** `TabBar` is a `tablist` with a 2px bottom border. Selected is a cyan border and cyan-400 text; idle is slate-400, turning slate-200 on hover. `md` is 14px Outfit medium, `sm` 12px semibold.
- **Segmented control:** `SegmentedControl` is a `radiogroup` in an 8px `bg-white/[0.02]` frame. The selected segment takes `TONE_CHIP.cyan` with a cyan focus ring; the optional count badge is tinted by tone.

### Status Dot
`StatusBadge` is an 8px solid hue dot with an 8px halo (none for slate).
It has no label of its own: the row's text is the label. Liveness marks
elsewhere are 6–8px dots with `animate-pulse` (cyan, in progress) or
`animate-pulse-glow` (emerald, live sessions).

### Gate Modal
The human-approval checkpoint. A rail-ground body with a violet/30
border, 16px radius and the violet modal halo, capped at `max-w-4xl` and
85vh, over its own `bg-black/70` scrim rather than `Modal`. The header has
an amber shield tile and an Outfit 700 18px title; the artifact output
sits in a well. Warning strips are amber ("already decided") or rose
("decision blocked"). The footer puts Abort (rose tint) on the left and
Redirect (violet tint, or a violet fill when sending) plus Approve
(emerald fill with glow) on the right.

### Empty State
`EmptyStateCard` has a first-run **hero** (a large glass card over static
violet and cyan blobs, a gradient icon tile with a sparkle, a 30px Outfit
700 headline, four action tiles that scale on hover, and a violet
sample-project button with a hover sheen) and an **inline** variant (a
glass card with two small corner blobs, a violet icon circle, a 16px
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
- **Do** keep the accent for primary actions, selection and focus, with a white label on an accent fill. New code waits for the Signal tokens rather than adding stock `violet-*`.
- **Do** use amber only for "a human is needed", and let its pulse run three beats, then rest lit.
- **Do** use `.glass-panel` for a panel, `.nested-card` for a card inside one, and `.panel-field` for a field filling one, and keep the panel's radius and shadow unspelled at the call site.
- **Do** keep blur for overlays floating over live content (command palette, dialogs).
- **Do** write labels and headings in sentence case, at 11px or more, and say Project, not Workspace.
- **Do** set machine output (ids, paths, counts, costs, statuses) in Fira Code and headings in `font-heading`.
- **Do** put an infinite pulse only on a small liveness dot or spinner, and animate opacity only.
- **Do** use `Modal` for overlays: portalled, a flat 85% #08090c scrim, Escape to close.
- **Do** keep inline `style` for geometry (heights, offsets, measured widths).

### Don't:
- **Don't** build a light theme or place these surfaces on a light ground.
- **Don't** add glow shadows or `text-shadow`, or a gradient fill on a button or text.
- **Don't** add decorative blurred blobs, sparkle icons as ornament, `hover:scale`, or shimmer outside an indeterminate progress bar.
- **Don't** blur a card, a rail or a header, nest a panel in a panel, or blur a full-window scrim.
- **Don't** use all-caps letter-spaced labels or text under 11px.
- **Don't** pass a colour into `Chip`, or hand-write `bg-<hue>-500/10 text-<hue>-400` for a status.
- **Don't** give the accent or violet a status, use violet as decoration, or give amber a state that needs nobody.
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
matches in F files. Counts include `src/components/Sidebar.tsx`, which
nothing imports; deleting it is free. Where a classification needed
reading the code (overlay or card, Project or not), it says so.

### 1. Surfaces and depth

1.1 **`.glass-panel` is translucent and blurred.** Breaks: three solid surfaces; blur only on overlays. Evidence: the `.glass-panel` rule in `src/App.css` sets `background: var(--bg-panel)` (`rgba(18,22,30,0.92)` in `tokens.css`) and `backdrop-filter: blur(12px)`; it has 91/49 call sites (`\bglass-panel(?!-)`), every card in the app. Fix: make `--bg-panel` opaque and drop the `backdrop-filter` from the rule; one change reaches every card.

1.2 **Blur on cards, rails and headers.** Breaks: blur only on overlays. Evidence: `backdrop-blur(-[a-z0-9]+|-\[[^\]]+\])?` = 35/27. Read one by one: 18 in 14 files are in-flow chrome or cards (`TopBar`, `ProjectRail` ×2, `Sidebar`, `FeatureHeader`, `VersionDrawer`, `ConfigPanel`, `WorkflowList` ×3, `ArtifactViewer` ×2, `RunnerVersionNotice`, `AttachmentChip`, `AttachmentDropzone`, `WorkflowNode`, `AskCanvasNode`, `TicketGraphNode`); 9 in 5 files float over a canvas (`Palette` ×3, `WorkflowCanvas` ×3, `CanvasZoomControls`, the `AskCanvasView` and `TicketGraph` legends); 6 are real overlays (dialogs, a popover, a toast, the terminal status overlay); 2 blur a scrim (`NewProjectView` repo picker, `WorkflowBuilder` unsaved-changes guard). `WizardShell` blurs a card through `style=` as well. Fix: remove blur and translucency from the 18 and the two scrims; decide whether canvas chrome counts as an overlay.

1.3 **Glass in glass, and the panel used as a control.** Breaks: a card inside a panel is a `.nested-card`; controls take smaller radii. Evidence: the active project in `ProjectRail` is a `glass-panel` at `rounded-lg` inside a `backdrop-blur-xl` aside; the `TopBar` search button is a `glass-panel` at `rounded-md` inside a `backdrop-blur-md` header. Fix: give the rail item `RailNavItem`'s selected treatment with a Signal selection mark, and the search button a field style.

1.4 **No token for the raised layer; panel tokens are translucent and split.** Breaks: three solid surfaces, values in `tokens.css`. Evidence: `tokens.css` declares six background tokens (`--bg-app`, `--bg-sidebar`, `--bg-well`, `--bg-panel`, `--bg-panel-hover`, `--bg-input`) and none for menus and dialogs. `.nested-card` and `.bubble-code` spell their fills as raw rgba in `App.css`, and call sites re-spell the panel fill at other alphas (`bg-\[rgba\(18,\s*22,\s*30` = 6/4, plus the two `style=` props in 5.3). Fix: define app, panel and raised surface tokens plus the hairline, and map well, input and nested onto them.

1.5 **The raised layer is spelled a dozen ways.** Breaks: one raised surface for menus and dialogs. Evidence: `fixed inset-0` = 13/13, of which 12 are hand-rolled overlays outside `ui/Modal.tsx` (12 `<Modal>` uses in 10 files). Their scrims are `bg-black/70` ×6, `bg-black/60` ×2, `#08090c/85` ×1 and none ×3. Dialog and menu bodies use `bg-[#0d0f14]` (GateView), `bg-[var(--bg-sidebar)]` (ArtifactModal, OverflowMenu), `glass-panel` (CommandPalette, DocsPanel, NotificationBell, AccountMenu, DiscoveryActionsMenu), `bg-slate-900/95` (AddressFindingsLaunch, canvas `Palette`), `bg-[#0c0d12]/95` (TerminalWorktreeLocationPicker), `bg-[#0d0f14]/90` (PostReviewComment) and an inline rgba (ShortcutHelp). Fix: a raised-surface class on the token from 1.4, and route the 12 through `Modal`.

1.6 **Panel-level radius beyond the panel token.** Breaks: the panel radius is `--radius-panel`. Evidence: `\brounded-2xl\b` = 12/12 (16px on the gate modal, dialogs, the composer, the hero icon tile); `\brounded-xl\b` = 71/46 spells 12px without the token. Fix: dialogs and raised surfaces take `--radius-panel`; expose it as a utility.

1.7 **The panel hairline is re-spelled.** Breaks: the panel states its own border (§4, and the `.glass-panel` comment). Evidence: 24 of the 91 `glass-panel` call sites add `border-white/*` (`/10` ×17, `/5` ×6, `/20` ×2; one site has two). `/5` is a no-op, `/10` and `/20` are heavier panels. Fix: delete them; a panel that needs emphasis takes a hue border, not a heavier white.

### 2. Accent and buttons

2.0 **Stock violet is the accent.** Breaks: the accent is indigo "Signal" (decision 61), and violet names nothing. Evidence: every `violet-` utility is to migrate or remove: `\bviolet-` = 450/107. Split by role: solid fills `(?<![\w/-])bg-violet-(500|600|700)(?![\w/])` = 41/21 plus state fills `(hover|active|disabled):bg-violet-` = 44/30 (primary buttons); focus `(focus|focus-visible|focus-within):[\w-]*violet` = 19/11; text `text-violet-` = 136/74; borders `border-(l-)?violet-` = 84/57; tints `bg-violet-\d+/` = 84/50; `rgba(139,92,246…)` literals in TSX = 34/25 (mostly glows, 2.1). In CSS: `--accent-purple` and `--accent-purple-glow` and `--border-glass-focus` in `tokens.css`; in `App.css`, `var(--accent-purple)` ×3 (scrollbar thumb while dragged, agent chat-bubble sender, `.btn-primary`), `rgba(139,92,246,…)` ×6 (agent chat bubble, `.btn-primary` border and glow, progress shimmer, two pinned utilities), `#7c3aed` and the caret's `#a78bfa`. Which of these are action, selection or focus (migrate to Signal) and which are decoration or status (2.5, remove) takes reading each site; the fill, focus and selection counts above are the migrate set. Fix: once the Signal tokens land as `@theme` colours, replace action, selection and focus uses with them, then delete `--accent-purple*`.

2.1 **Glow shadows.** Breaks: never glow shadows. Evidence: `shadow-\[0_0_[^\]]*\]` = 101/41, 61 distinct strings; by colour, violet 34, emerald 22, cyan 18, amber 9, ruby 6, slate 3, black 2, other 7. They include `StatusBadge`'s `TONE_DOT`, `PipelineCard`'s `TONE_ACCENT`, the gate and artifact modal halos, and primary buttons (`hover:shadow-\[0_0_` = 7/5). `App.css` adds five more: `.animate-pulse-glow` (static cyan halo; 16/12 uses), `.animate-pulse-glow-amber` (2/2), and the focus glows on `.input-field` and `.chat-input` and the hover glow on `.btn-primary`. `tokens.css` keeps four `--accent-*-glow` values, one of them used. Fix: delete the glows; dots and bars go solid, focus becomes a ring, and the `--accent-*-glow` tokens go.

2.2 **Gradient primary button.** Breaks: no gradient fills on buttons. Evidence: `.btn-primary` in `App.css` is `linear-gradient(135deg, var(--accent-purple), #7c3aed)` with a hover glow; `\bbtn-primary\b` = 15/14, all in `ask/`, `discovery/` and their modals. `AskCanvasNodeInspector` has one more gradient button (`bg-gradient-to-br from-violet-600 to-violet-700`). Fix: a solid Signal fill (`#5a60c2`, hover `#6269cc`, white label), the same one 2.3's primitive uses.

2.3 **Two primary spellings, no primitive.** Breaks: one primary action treatment (inferred from §4's "the accent = primary actions"; §4 does not name a Button primitive). Evidence: inline `\bbg-violet-600\b` = 35/21 and `\bhover:bg-violet-500\b` = 38/29 beside the 15 `.btn-primary`. Fix: a `Button` primitive in `src/components/ui/` with primary (Signal fill, white label), secondary, ghost and destructive variants; it is the natural place for 2.0's migration.

2.4 **Focus and selection are cyan.** Breaks: the accent = selection and focus. Evidence: `focus(-visible|-within)?:border-cyan-` = 81/32 against `…:border-violet-` = 19/11; `focus(-visible)?:ring-cyan` = 4/4 (including `SegmentedControl`), `ring-violet` = 0. `TabBar` selects with `border-cyan-500` plus `TONE_TEXT.cyan`, and `SegmentedControl` with `TONE_CHIP.cyan`. In `App.css`, `::selection`, `select option:checked` and the scrollbar thumb on hover are cyan. The 19 violet focus borders are drift too (2.0). Fix: one shared focus ring in Signal (`#6e78db`), and Signal selected states (border `#6972c0`, tint `rgba(90,96,194,.16)`) in `TabBar`, `SegmentedControl` and `App.css`.

2.5 **Violet still names states and decorates.** Breaks: neither the accent nor violet is a status, colour is spent only on meaning, and status hues resolve through `runStatus.ts`. Evidence: `tone=["']violet["']|tone:\s*['"]violet['"]|TONE_(CHIP|TEXT|BADGE|BORDER_L)\.violet|\['violet'\]` = 27/17. Among them: `discoveryProgress.ts` ("Interviewing"), `askLifecycle.ts` ("Asking", "Open"), `ticketPresentation.ts` (the "Ready" lane), `StorageProjectCard`/`StoragePanel` ("Reclaimable"), canvas node kinds in `canvas/types.ts`, and model or workflow names as violet `Chip`s (`PipelineCard`, `FeatureHeader`, `AskThreadCard`, `DiscoveryCard`, discovery views). `syncPanel.ts`'s violet entries are actions ("Sync", "Resolve with agent"), so they move to the accent (2.0) rather than going away. `App.css` tints the agent's chat bubble violet. `HeaderNavItem` tints icons violet, cyan or amber by an `accent` prop (4 uses in `TopBar`). Fix: lifecycle states to cyan or slate by meaning, names to slate chips, header icons and the agent bubble neutral; then drop `violet` from `RunStatusTone`.

2.6 **Amber for states that need nobody.** Breaks: amber = a human is needed, and only that. Evidence: amber tone references outside `runStatus.ts` (`(tone|accent)\s*[:=]\s*['"]amber['"]|: 'amber'`) = 25/16. Read one by one, these need nobody: `activitySync.ts` "reconnecting", `decomposeReview.ts` "Revised" and "blocked by …", `ticketPresentation.ts` "Blocked" ("waiting on an edge"), the `AskThreadCard` "network" chip, and `harnessVerdict.ts`'s "unrunnable" (multi-line, not in the count). Gate nodes, credential failures, PR conflicts and "Undecided — left for you" arguably do. Fix: in-motion states to cyan, dependency and change kinds to slate; §4 needs a home for "warning".

2.7 **Three reds and off-palette hues.** Breaks: ruby = failed or destructive; colour only for meaning. Evidence: ruby classes 290/74, rose 92/19, red 29/5 (`(?<![\w-])(text|bg|border|ring|from|to|via|fill|stroke|shadow|outline)-<hue>-`). Rose carries GateView's Abort and "decision blocked". `App.css` pins `text-red-400`, `bg-red-500/10` and two `border-red-500/*`. Yellow, orange and blue add 12/4 (`CodeEditorView` git letters and folder icons, `ErrorToast`'s orange accent, `GeneralTab`'s unpushed chip, a yellow button in `WorkflowList`). Fix: rose and red to ruby; orange and yellow to amber only where a human is needed, else slate.

2.8 **Emerald for healthy machines.** Breaks: emerald = finished well (§4 no longer names "healthy"). Evidence: `StatusBadge`'s `NON_RUN_TONES` maps `idle` and `active` to emerald. Fix: decide whether a healthy machine is "finished well" or slate, and record it in §4.

### 3. Decoration

3.1 **Decorative blurred blobs.** Breaks: never decorative blurred blobs. Evidence: `rounded-full[^"`]*(?<![-\w])blur-(\[\d+px\]|xl|2xl|3xl)` = 20 lines in 11 files: `ProjectSettingsShell` ×5, `EmptyStateCard` ×4, `ProjectHome` ×2, `NewProjectView` ×2, and one each in `RemoteRunInbox`, `WizardShell`, `ProviderSettings`, `PreferencesScreen`, `CreateFromZeroWizard`, `MachinesView`, `ProvidersPage`. Fix: delete them.

3.2 **Sparkles as ornament.** Breaks: never sparkle icons as ornament. Evidence: `<Sparkles\b` = 12/10 (`ProjectRail` ×2, `EmptyStateCard` ×2, `NewProjectView`, `CreateFromZeroWizard`, `GateView`, `AgentBadge`, `StartFeatureModal`, `ShortcutHelp`, `NewTerminalMenu`, `StartSessionButton`). Most sit beside a text label that already says what the control does. `AgentBadge` in compact mode shows only the sparkle, so there it may carry meaning. Fix: use the icon of the thing (a plus, a terminal, a file) or none.

3.3 **Gradient ornament.** Breaks: no gradient fills on buttons or text (strictly, these are on tiles, avatars and washes, which §4's wording does not name; they are ornament either way). Evidence: `\bbg-gradient-to-[a-z]+` = 12/8: icon tiles (`MachinesView` ×2, `PreferencesScreen`, `EmptyStateCard`), avatars (`AccountMenu`, `ProvidersPage`), a hover wash on a `ProjectHome` card, `ShortcutHelp`'s header bands ×2, the `EmptyStateCard` CTA overlay and sheen, and the `AskCanvasNodeInspector` button (2.2). The two `radial-gradient` uses are canvas dot grids and are functional. Fix: flat fills.

3.4 **`hover:scale`.** Breaks: never `hover:scale`. Evidence: `(?<![\w-])hover:scale-` = 4/1 and `group-hover:scale-` = 4/1, all in the `EmptyStateCard` hero tiles. Fix: a border or background shift on hover.

3.5 **Shimmer outside progress.** Breaks: shimmer only on an indeterminate progress bar. Evidence: the `EmptyStateCard` sample-project button sweeps a `via-white/10` gradient across itself on hover. The one allowed shimmer, `.animate-progress-shimmer` (1/1, `TerminalStatusOverlay`), runs its gradient through violet, which is decoration. Fix: delete the sheen; make the progress bar cyan (in motion).

3.6 **The first-run hero.** `EmptyStateCard` combines 3.1–3.5, two glows and a gradient icon tile, and its copy ("a modern, premium control center…") breaks PRODUCT.md's binding voice (direct, no hype). Fix: rebuild it as one solid panel, three plain actions and plain copy; it is the first thing a new user sees.

### 4. Type and copy

4.1 **All-caps letter-spaced labels.** Breaks: never all-caps letter-spaced labels; meaning not carried by letter case. Evidence: `\buppercase\b` = 276/109, and 275 of those lines also carry positive tracking (`tracking-(wide|wider|widest|\[[0-9.]+(em|px)\])` = 296/116 overall; `tracking-wider` 179, `tracking-widest` 66, `tracking-wide` 35, six arbitrary values). The primitives multiply it: `Chip` is always `uppercase tracking-wide` (`<Chip\b` = 64/24), `FieldLabel` is `uppercase tracking-wider` (49/17), `SectionCard` too (0 uses), and `App.css`'s `.chat-bubble-sender`. Fix: sentence case in `Chip` and `FieldLabel` first (two one-line changes), then sweep the hand-rolled ones.

4.2 **Text under 11px.** Breaks: never text under 11px. Evidence: `(?<![\w-])text-\[(?:[0-9]|10)(?:\.\d+)?px\]` = 346/126: `text-[10px]` 289/114, `text-[9px]` 53/28, `text-[10.5px]` 3, `text-[9.5px]` 1. `Chip size="sm"` is 10px (`<Chip\b[^>]*size="sm"` = 47/19 on one line) and `RailBadge` is 10px. Fix: raise `Chip` `sm` and `RailBadge` to 11px, then replace 9px and 10px with an 11px step.

4.3 **Ungoverned type scale.** Breaks: no rule directly; it is why 4.2 recurs. Evidence: 14 arbitrary sizes (`text-[11px]` 371/119, `[13px]` 42, `[12px]` 16, which equals `text-xs`, `[11.5px]` 15, `[12.5px]` 4, plus one-offs) and no type-size tokens. Fix: a small size scale as `@theme` keys in `tokens.css`.

4.4 **Title Case strings.** Breaks: sentence case for labels and headings. Evidence (heuristic, so approximate): two or more capitalised words with only short connectives between them, found 71 times as same-line JSX text, 23 as standalone JSX text lines, 14 in `label`/`title`/`aria-label`/`placeholder` attributes and 24 in object-literal `label`/`title` values, about 132 in all. Examples: "Workspace Bootstrap In Progress", "Configure Worktree Strategy", "Copy Complete Output", "Save Changes", "Provider Type", and shortcut and command-palette labels in `src/lib/shortcuts.ts` and `App.tsx`. Some hits are names (product or workflow names). Fix: sentence case, file by file.

4.5 **"Workspace" where the noun is Project.** Breaks: PRODUCT.md Vocabulary (the UI says Project); §4's translatable-copy rule. Evidence: `(?<![\w./-])[Ww]orkspaces?(?![\w/-])` on user-facing lines = 57 matches on 55 lines in 19 files, excluding the unused `Sidebar.tsx` (2 more); every line was read. About 45 lines name a Project (settings headings, bootstrap screens, the rail's "Workspaces" header, "Delete Workspace"). The other 10 use the word for something else and need their own noun: the app-wide data directory (`PreferencesScreen` ×3), the whole install (`McpConsentView` ×2), a Git host account (`ProvidersPage`), the Discovery pane (`DiscoveryWorkspaceRow`), a feature worktree path (`NewProjectView`), the command palette placeholder (`TopBar`) and the empty-state back label (`parentView.ts`). Fix: rename to Project; give the other 10 their own words.

4.6 **"Provider" where the thing is a Git host connection.** Breaks: PRODUCT.md (in code a ProviderInstance is a GitHub or GitLab connection), where the UI noun is still an Open Decision. Evidence: a scripted pass over string literals and JSX text found 48 occurrences on 46 lines in 21 files, a lower bound because multi-line JSX strings are missed (for example `PostReviewComment`'s "using your provider"). Examples: "Source Providers" (`ProvidersPage`, `TopBar` title), "Connect a Provider", "Provider Type", "No Providers Mapped", the wizard step label "Provider". Fix: settle the noun in PRODUCT.md, then rename.

4.7 **Redundant `font-sans`.** Breaks: UI text needs no class. Evidence: `\bfont-sans\b` = 24/10, including the root shell in `App.tsx`. Fix: delete.

### 5. Raw hex and `style=` tokens

5.1 **Arbitrary hex classes.** Breaks: never raw hex for design tokens. Evidence: `\[#[0-9a-fA-F]{3,8}\]` = 176/74, 13 distinct values. 113 restate tokens: `#08090c` ×52 (`--bg-app`), `#0d0f14` ×44 (`--bg-sidebar`), `#050608` ×17 (`--bg-well`). 63 are near-blacks with no token: `#050508` ×26, `#0b0d12` ×11, `#0a0c10` ×8, `#0c0d12` ×7, `#0a0a0e` ×4, `#12161e` ×3 and four singletons. Fix: expose the surface tokens from 1.4 as `@theme` colours, replace the 113, and fold the 63 onto the nearest surface.

5.2 **Tokens are bypassed.** Breaks: values live in `tokens.css` and are read from there. Evidence: TSX reaches a token only through 13 `bg-[var(--…)]` uses; `--accent-*` and `--text-*` are referenced from TSX zero times. Components colour through palette classes (cyan 540, violet 448, ruby 290, amber 259, emerald 259). The two neutral ladders disagree: `--text-*` are Tailwind gray (#f3f4f6 / #9ca3af / #6b7280), while components use slate and `text-white` (386), and the root shell sets `text-white` over `body`'s `--text-primary`. Fix: one neutral ladder as tokens; the status hues stay behind `runStatus.ts`'s `TONE_*` maps.

5.3 **`style=` props carrying design values.** Breaks: never `style=` for design tokens. Evidence: `style=\{\{` = 22/15. Two set a surface: `ShortcutHelp` (the dialog body) and `WizardShell` (a card that is already a `glass-panel`), each `background: 'rgba(18,22,30,0.75)'` plus a 12px backdrop blur. The previous record said all 22 were geometry; these two are not. Fix: classes (the raised surface for `ShortcutHelp`, plain `.glass-panel` for `WizardShell`).

5.4 **Colour values outside `tokens.css`.** Breaks: token values live in `tokens.css`. Evidence: `App.css` still declares 22 hex literals in rules: 11 in the `PINNED UTILITY COLOURS` block (32 utility classes re-declared at Tailwind v3 values) and 11 elsewhere (`.btn-primary`'s `#7c3aed` and `#fff`, the caret's `#a78bfa`, the native-control rules), plus rgba fills in `.nested-card`, `.bubble-code`, the chat bubbles, the scrollbar and the skeleton. Fix: move the values to tokens; whether to keep the v3 pins is a palette decision, per the block's own comment.

5.5 **Hex in library themes.** Breaks: no raw hex (lower priority: the libraries need values, not classes). Evidence: `['"]#[0-9a-fA-F]{3,8}['"]` = 18/3: `TerminalSurface` (xterm theme) ×11, `canvas/flowGraph.ts` ×5, `WorkflowCanvas` ×2. Fix: read them from the CSS custom properties at runtime, or from one exported token map.

5.6 **Token comments contradict §4.** Evidence: `tokens.css` labels `--accent-purple` "Active Connections", `--accent-emerald` "Active running agents", `--accent-ruby` "Off / Stopped / Connection Error" and the block "Neon Accents". Unused: `--accent-emerald`, `--accent-ruby`, three of the four `--accent-*-glow`, `--text-muted`, `--text-dark`, `--transition-spring`, `--transition-smooth`. Fix: drop the meanings from the comments (§4 owns them) and delete the unused tokens.

### 6. Form controls

6.1 **`!important` native-control rules beat call-site utilities.** Breaks: no raw hex for tokens; a call site's utilities should apply (the reason for PR #250's layers). Evidence: the unlayered rules at the end of `App.css` hold 17 `!important` declarations on `select`, `option`/`optgroup`, `input[type=text|search|email|password|url]`, `textarea` and placeholders (#0a0a0e fill, #e2e8f0 text, #475569 placeholder, a cyan checked option, and on `select` `appearance: none`, a chevron `background-image` and `padding-right: 28px`). A scripted pass over JSX tags found 128 matching fields in 54 files (`select` 45, `input type="text"` 62, `password` 7, `search` 2, `textarea` 12). 98 of them carry a `bg-*` utility and 94 a text-colour utility that never apply, and the 14 using `.input-field` lose its background too. Inputs without a `type` attribute (8) and number inputs (10) are not matched, so they render differently. Moving the rules into `@layer base` would let 31 of the 45 selects' padding utilities (`p-`/`px-`/`pr-`) override the 28px chevron gutter. That it would also lose the chevron `background-image` was reported to me and not verified here. Fix: move the rules to `@layer base` on token colours, give `select` its chevron and gutter through a class or a `pr-7` convention, then remove the dead `bg-*` and text utilities.

6.2 **Selection and body font as `!important`.** Evidence: `::selection` is an unlayered `!important` cyan (see 2.4), and the "WebKitGTK / Tauri compatibility layer" re-declares `body`'s font with `!important`, duplicating the `@layer base` rule. Fix: fold both into `@layer base`, with selection in violet.

### Outside the PR groups

- **Status vocabularies outside `runStatus.ts`.** §4 says status hues resolve through `runStatus.ts` and nowhere else. About a dozen modules keep their own label→tone tables, reusing its `RunStatusTone` type and `TONE_*` maps: `syncPanel`, `discoveryProgress`, `askLifecycle`, `ticketPresentation`, `decomposeReview`, `activitySync`, `pullRequestRow`, `harnessVerdict`, `staleness` and `pipelineCard` in `src/lib/`, plus `RemoteRunInbox`, `canvas/types`, `StorageProjectCard` and `ReviewFailureCard`. Whether reusing the `TONE_*` maps satisfies "through runStatus.ts" is for §4 to say; 2.5 and 2.6 are where these tables disagree with it.
- **No-op animation.** `NotificationBell`'s toast uses `animate-[fadeIn_120ms_ease-out]`, and no `fadeIn` keyframe exists (the keyframe is `fade-in`).
- **Stale written guidance.** `.design-sync/conventions.md` still teaches the retired system: violet = "agent working", emerald = "running agent", `style={{ background: 'var(--bg-panel)' }}` for tokens, uppercase `tracking-widest` small labels, `SectionCard` first, and `StatusBadge variant="pill"`, a prop that no longer exists.

### Resolved

- Dead CSS: `.app-container`, `.sidebar-*`, `.list-item`, `.panel-header`, `.glow-*`, `.text-glow-*`, `.supervisor-*`, `.agent-markdown`, `.card-grid`/`.metric-*` and the `translateX` hover motion were deleted in PR #250.
- Unlayered `App.css` rules silently beating utilities: rules moved into `@layer base` / `@layer components` in PR #250, so the amber "needs you" ring, ruby confirm borders and row status stripes now render.
- The `!important` colour safelist beating `hover:`/`focus:` variants: now a non-important `@layer utilities` block in PR #250. Its v3 values remain (5.4).
- Radius and shadow re-spellings on `.glass-panel` call sites (`rounded-lg/xl/2xl`, `shadow-2xl`), and the violet halo on the active rail project, were removed in PR #250. The rail item and the search box keep control radii (1.3).
- Token values moved verbatim to `src/styles/tokens.css` on this branch, and §4 now points there.
- `runStatus.ts` re-toned on this branch: `bootstrapping` amber→cyan, `verifying` violet→cyan, `published` violet→cyan. Violet names no run status in `META`, and amber in `META` marks only the settled needs-a-human statuses (plus `REVIEW_READY_META`).
- The App.css banner naming the system "High-Tech Dark Neon Glassmorphism" and its "Glassmorphism" section titles were replaced on this branch by a pointer to `tokens.css`, §4 and this file.
- Old §4 wording no longer in conflict: "emerald = running agents", four hues instead of six, "status dots pulse", and a "card-surface token" that never existed. §4 was rewritten.

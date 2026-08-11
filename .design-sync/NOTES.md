# design-sync notes — Demeteo

Repo-specific gotchas for future syncs. Read this before re-running anything.

## Shape: this repo is an app, not a component library

- Demeteo ships as a Tauri desktop app. There is **no published `dist/` entry**
  and no `main`/`module`/`exports` in `package.json`, so the converter runs with
  an explicit `--entry`.
- `.design-sync/entry.tsx` is that entry — a hand-maintained barrel over
  `src/components/ui/`. **It is also what anchors package resolution**: without
  `--entry` the converter looks for `node_modules/demeteo/package.json`, which
  never exists (npm won't self-install), and dies with ENOENT inside
  `lib/dts.mjs projectFor`. Adding a component to the sync means adding it to
  both `entry.tsx` and `componentSrcMap`.
- Scope is deliberately `src/components/ui/` only. The other ~150 components in
  `src/components/` call Tauri commands through `src/lib/` wrappers; those
  cannot resolve in a browser and would ship as floor cards.

## The stylesheet is an app stylesheet, not a DS stylesheet

- `cfg.cssEntry` points at `.design-sync/.cache/app.css`, which `cfg.buildCmd`
  produces by concatenating the app's **compiled** Tailwind output
  (`dist/assets/index-*.css`) with `.design-sync/card-surface.css`.
  `src/App.css` itself is unusable directly — it's `@import "tailwindcss"`
  source, not resolved CSS.
- The vite asset hash in `index-*.css` changes every build, hence the glob in
  `buildCmd`. Never pin the hashed filename in config.
- `card-surface.css` exists because the generated preview card template ends
  its `<head>` with an inline `body{background:#fff}` that wins on source
  order. Demeteo is dark-only, so without the `!important` override every card
  renders slate text on white. It also unsets the app shell's
  `height:100vh/overflow:hidden/display:flex` — correct for a desktop app,
  wrong for a preview card or an agent-built page.
- Do **not** re-add `min-height:100%` to `card-surface.css`. The body
  background propagates to the canvas anyway, and the min-height only padded
  every card out to a viewport of empty black.

## Preview authoring

- Previews import from `'demeteo'`; the converter shims that bare specifier to
  `window.Demeteo`. `lucide-react` imports resolve from `node_modules` and get
  bundled into the preview.
- **Tailwind only scans `src/`** (`@source "./**/*.{ts,tsx,js,jsx,html}"` in
  `App.css`). `.design-sync/previews/` is *not* scanned, so a utility class
  that appears nowhere under `src/` will not exist in the compiled CSS and
  fails silently. This is not theoretical: `bg-violet-500/80` in the Modal
  preview rendered a plain dark button and looked merely "a bit off" rather
  than broken. Opacity and shade variants are the usual casualty — the base
  class exists, the variant does not.

  **Run `node .design-sync/check-classes.mjs` before every capture.** It
  extracts literal `className` strings from `.design-sync/previews/*.tsx`,
  escapes each token the way Tailwind escapes selectors, and fails on any that
  has no rule in the compiled CSS — printing the nearest existing siblings, which
  is nearly always what you meant. It only sees literal strings, so keep preview
  classNames free of template interpolation.

  Classes confirmed absent despite looking plausible: `demeteo-scrollarea`
  (`ScrollArea` sets it as a styling hook, but no rule ships), `animate-fadeIn`,
  `glass-panel-hover` as a standalone rule (it exists only as `:hover`),
  `h-64`/`h-72` (the app tops out around `h-56`).
- `package-capture.mjs` shoots a fixed **900×700** viewport with
  `fullPage:false`. The large empty area below short content is the harness,
  not a defect — do not chase it. What *does* matter is width: keep content
  under ~850px or it clips.
- Components that scroll their own overflow (`CreateZeroStepHeader` has
  `overflow-x-auto`) get **clipped, not shrunk**, by a `max-w-*` frame. Its
  preview uses a full-width frame, and the component carries
  `cfg.overrides.CreateZeroStepHeader.cardMode = "column"` because it is
  genuinely a full-width element.

## Card modes

Most of this DS is full-width form furniture, so 13 components carry
`cfg.overrides.<Name>.cardMode = "column"` (one export per row at full card
width). `Modal` is `cardMode: "single"` with `primaryStory: "GateDialog"` —
it is `fixed inset-0` and portals to `document.body`, so two open modals on one
page would stack on top of each other.

**Changing `cfg.overrides` invalidates the stamped grade keys.** The next
`package-capture.mjs` then fails `[CONFIG_STALE]` and captures nothing. Run the
full `package-build.mjs` after any overrides edit — `lib/preview-rebuild.mjs`
alone does not re-stamp.

## A finding about the repo itself (not a sync issue)

`font-outfit` is used **80 times across 31 files** under `src/`, but Tailwind
generates font utilities from the `--font-*` theme keys, and `App.css` defines
`--font-heading` (not `--font-outfit`). So `.font-outfit` has no rule anywhere
and every one of those headings silently falls back to the body font (Inter)
instead of Outfit. The design-system previews use `font-heading`, which does
work. Fixing the app is out of scope for a sync, but it is worth a ticket.

## Grouping

All 25 components would otherwise land in `general` — `ui` is in the
converter's `GENERIC_DIR` set, so the directory name never becomes the group.
Grouping comes from frontmatter-only stubs in `.design-sync/docs/<Name>.md`
(`category: Primitives | Forms | Wizard`) wired through `cfg.docsMap`. A stub
with an empty body is deliberately falsy in `lib/emit.mjs`, so `.prompt.md`
still gets synthesized from the `.d.ts` — the stub regroups without replacing
the generated docs.

## Known render warns

Both are non-blocking and expected; a re-sync should not treat them as new:

- `[FONT_MISSING] "JetBrains Mono"` — it is only the *second* entry in
  `--font-mono: 'Fira Code', 'JetBrains Mono', monospace`. Fira Code ships, so
  the fallback never activates. Not worth shipping a second mono family.
- `[FONT_DANGLING] "symbols nerd font mono"` — the terminal's Nerd Font
  *symbols* fallback (PUA powerline/icon glyphs for xterm.js). Its `@font-face`
  survives the CSS scrape but its `url()` is a vite asset path that cannot be
  rewritten. It carries no design language and is irrelevant outside the
  terminal surface.

## Re-sync risks

- **The app build must run first.** `cfg.buildCmd` depends on `dist/assets/`
  being current; a stale `dist/` silently syncs an old stylesheet. When in
  doubt, rebuild.
- **`entry.tsx` and `componentSrcMap` can drift from `src/components/ui/`.**
  Neither is generated. A component added to `ui/` after this sync is invisible
  until someone adds it to both; a component deleted from `ui/` breaks the
  build at the barrel. Diff the directory against the barrel on every re-sync.
- **Tauri coupling can creep in.** A `ui/` primitive that gains an
  `@tauri-apps` import (directly or through `src/lib/`) will start rendering
  blank. If a previously-good card regresses to a floor card, check its imports
  before suspecting the pipeline.
- **`guidelinesGlob` is narrowed on purpose** to `docs/UX_JOURNEYS.md`. The
  default globs swept in 18 engineering docs (ARCHITECTURE, DECISIONS,
  RUNNER_DEV…) about Rust ports and SSH transports — noise for a design agent.
  Widen only toward genuinely design-facing docs.
- Playwright is installed into `.ds-sync/node_modules` (gitignored). A fresh
  clone re-installs it; there is no repo pin to match.

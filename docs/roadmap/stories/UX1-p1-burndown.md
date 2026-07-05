# Epic UX1 — High-severity UX defect burn-down 🔴

> **Roadmap source:** [03-roadmap-6-months.md § Epic UX1](../03-roadmap-6-months.md#epic-ux1--high-severity-ux-defect-burn-down-); rank 5 in [04-high-level-plan.md](../04-high-level-plan.md); ships **v1.1 (Sep)**.

**Outcome:** the app stops misleading users on core journeys — every P1 finding from the [July 2026 UX audit](../../ux-audit/findings.md) is closed, and the review half of the core journey (code browsing, artifacts, gates) works offline.

**Out of scope:** P2/P3 findings (Epics UX2/UX3); any new capability beyond what the fixes require.

**Epic acceptance:** all nine P1 findings closed with regression coverage; offline smoke test passes (docs panel, Browse Code, gate artifact review with network disabled); Escape/mouse-back never navigates beneath an open overlay.

**Why this matters now:** v1.1 is the credibility release — an honest agent table (Epics A1–A3) next to a dishonest UI undercuts it. This also unblocks Epic C1: the board binds cards to pipelines through the start-feature surface, and repo targeting that's collected-but-dropped (F2) plus the always-firing conflict detector (F3) must be real before the board multiplies launches through them.

**Source of truth for line numbers:** all references below are as of commit `82fa581` per `docs/ux-audit/findings.md` header — re-check line numbers against current `HEAD` before editing, since normal development will have shifted them.

---

## Story UX1.1 — Fix "Stop Step" mislabeling (F1)

**As a** user watching a stuck step, **I want** "Stop Step" to actually stop just that step (or be honestly relabeled), **so that** I don't accidentally kill my entire pipeline trying to nudge one step.

**References:** `docs/ux-audit/findings.md` F1.

**Status:** Not started.

**Tasks:**
- [ ] `FeatureDetail.tsx:437-452` (`handleStopStep`) currently calls `feature_cancel`, which the backend maps to `executor.feature_cancel` (`src-tauri/src/commands/features.rs:152`) — this cancels the *whole feature*, not the step.
- [ ] **Decide and implement one of:** (a) rename the button/confirm-dialog copy to "Cancel Feature" and remove the per-step framing entirely, or (b) implement a real per-step stop command in the backend (new Tauri command + `StepExecutor` support for interrupting a single running step without cancelling the feature).
- [ ] Add regression coverage: a test asserting the button's behavior matches its label (whichever option is chosen).

## Story UX1.2 — Repo targeting wired through, conflict detector fixed (F2, F3)

**As a** user launching a feature with specific repos selected, **I want** my selection to actually reach the backend (or be told plainly that it's informational), **so that** the "Customize" repo pickers aren't UI theater.

**References:** `docs/ux-audit/findings.md` F2, F3. This is explicit groundwork for Epic C1 (the board binds cards to pipelines through this same surface).

**Status:** Not started.

**Tasks:**
- [ ] `start_feature` (`src-tauri/src/commands/features.rs:100-114`) has no repo parameter. Three UI surfaces collect a selection that never reaches it: `ProjectHome.tsx:41,626-657,688-717` (`selectedRepos`, dropped at `handleStartFeature` `:279`), `StartFeatureModal.tsx:310` (`targetRepos`, dropped by `App.tsx:523-615`), and `StartFeatureModal.tsx:456`'s copy promising a Customize repo editor that doesn't exist.
- [ ] **Decide and implement one of:** (a) add a `target_repos` parameter to `start_feature` and thread it through to wherever repo scoping actually happens in the orchestrator, or (b) remove the pickers/checkboxes and relabel the chips as purely informational ("repos this feature will likely touch," no selection semantics).
- [ ] Once F2 is resolved, fix F3: `StartFeatureModal.tsx:273-302` computes `usedRepos` from `get_repositories_for_project(f.project_id)` — the *entire* project's repo list, not the specific active feature's repos — so any active feature marks every repo as conflicted. The guard `if (f.id !== /* self */ undefined)` at that call site is always true (comparing an id to `undefined`), which is itself a bug independent of the data issue.
- [ ] Fixing F3 requires a real per-feature repo association (depends on F2's resolution) — implement F2 first.
- [ ] Add regression coverage: launching two features against disjoint repos should not show a conflict badge; launching against overlapping repos should.

## Story UX1.3 — In-app docs render in production; shortcut system unified (F4, F5)

**As a** user of a packaged build, **I want** the Help panel and keyboard-shortcuts page to actually render content, **so that** in-app documentation isn't dead weight.

**References:** `docs/ux-audit/findings.md` F4, F5.

**Status:** Not started.

**Tasks:**
- [ ] F4: `DocsPanel.tsx:34-52` fetches `/src/docs/${slug}.md` (a Vite dev-server-only path) with a fallback to `/docs/${slug}.md`, neither of which exists in a packaged build (`public/` only has icons). Fix: import docs via Vite's `?raw` glob imports (bundled at build time) and render with `react-markdown` + `remark-gfm` — both are **already runtime dependencies** (used by `ArtifactViewer`), so this is a rewire, not a new dependency.
- [ ] F4 (continued): `SimpleMarkdown` (`DocsPanel.tsx:169-170`) skips all `|` table lines; `src/docs/keyboard-shortcuts.md` is one big table, so it currently renders empty even in dev. Switching to `react-markdown` fixes this as a side effect — verify it explicitly with a regression test/screenshot rather than assuming.
- [ ] F5: `lib/shortcuts.ts` (the registry `ShortcutHelp` renders) documents `Cmd+P`, `Cmd+R` "Reload data", `F11` fullscreen — none implemented in `hooks/useKeyboardShortcuts.ts`. Worse, unhandled `Cmd+R` falls through to the webview and reloads the app, dropping all UI state. Either implement these three or remove them from the registry.
- [ ] F5 (continued): the dispatcher implements `Cmd+Shift+F` and `Cmd+.` for the palette (`useKeyboardShortcuts.ts:72-77,97-99`) — add these to the registry (they're currently undocumented, the inverse problem).
- [ ] F5 (continued): the `?` help chord never fires — `useKeyboardShortcuts.ts:37` requires `!e.shiftKey`, but typing `?` requires Shift on US/most layouts. Fix the guard. (`Cmd+?` at `:101-104` works but isn't documented in the registry — add it.)
- [ ] F5 (continued): `ShortcutHelp.tsx` + `ShortcutsContext.tsx` (781 lines incl. tests) are never mounted — not exported from `context/index.tsx`, not imported by any view. Mount it on F1/`?` in place of the generic `DocsPanel` it currently falls through to. Make `useKeyboardShortcuts` consume the registry's existing `matchesEntryKeyboard` matcher (already exists for exactly this) instead of maintaining parallel logic.
- [ ] Add regression coverage: registry entries match dispatcher-implemented shortcuts 1:1 in both directions (a test that walks both lists, not just eyeballing).

## Story UX1.4 — Idempotent bootstrap retry (F6)

**As a** user whose project bootstrap fails, **I want** "Retry Build" to retry, not duplicate, **so that** I don't end up with multiple project rows for one project.

**References:** `docs/ux-audit/findings.md` F6.

**Status:** Not started.

**Tasks:**
- [ ] `NewProjectView.tsx:153-208` (`handleCreate`) always calls `create_project` first; "Retry Build" (`:291`) re-invokes `handleCreate` in full, creating a second (then third, ...) project row on each retry.
- [ ] Fix: Retry should reuse the `projectId` already captured at `:189` and only re-run `bootstrap_project`, not `create_project`.
- [ ] Related: cancelling at the strategy-proposal step leaves the already-created project invisible until the next app restart (it was never added to UI state) — fix this in the same pass since it's the same root cause (a project row created before the UI is ready to represent it).
- [ ] Add regression coverage: simulate a bootstrap failure, retry, assert exactly one project row exists.

## Story UX1.5 — Bundle Monaco for offline review (F34)

**As a** user on an offline or firewalled machine, **I want** Browse Code, artifact previews, and gate review to work without internet, **so that** the review half of my core journey doesn't silently hang.

**References:** `docs/ux-audit/findings.md` F34.

**Status:** Not started.

**Tasks:**
- [ ] `@monaco-editor/react` is used by `CodeEditorView.tsx:3-4` (Browse Code) and `ArtifactViewer.tsx:4` (artifact panes in Feature Detail **and** GateView), but `monaco-editor` is not a package dependency and no `loader.config` exists anywhere in the repo — the default loader fetches Monaco from the jsdelivr CDN at runtime.
- [ ] Add `monaco-editor` as a direct dependency in `package.json`.
- [ ] Call `loader.config({ monaco })` once at app startup (check `vite.config.ts` for any Monaco-related config that needs updating alongside this) so Vite bundles it instead of fetching from a CDN.
- [ ] Add an offline smoke test (network disabled) covering: Browse Code opens and renders, an artifact preview in Feature Detail renders, and the GateView artifact pane renders — this is an explicit epic-acceptance item, not optional.

## Story UX1.6 — Single Escape/overlay-priority stack (F35, supersedes F30; also closes F40)

**As a** user pressing Escape (or mouse-back) to close a modal, **I want** only that modal to close, **so that** I don't also get navigated to a different screen underneath it.

**References:** `docs/ux-audit/findings.md` F35 (supersedes the milder F30), F40 (same failure family, mouse-back specifically).

**Status:** Not started.

**Tasks:**
- [ ] `App.tsx:113-118` documents that per-modal ESC handlers "are expected to call `event.stopPropagation()`" — **none do**. `ui/Modal.tsx:11-16` and `PromptDialog.tsx:39-46` add their own window-level Escape listeners without stopping propagation; the global hook (`useKeyboardShortcuts.ts:26-29`) fires on the same keypress; none of these dialogs are tracked in `UIState`, so `pickEscapeAction` falls through to `navigate-back`.
- [ ] Concretely broken today: closing the attachment preview, replay-from-step modal, Publish-MR prompt, repo-selection modal, or dirty-repo warning also pops the navigation stack. `AgentTerminalDrawer` has no Escape handling at all, so Escape while it's open navigates the view *underneath* the drawer.
- [ ] Fix: implement one ordered Escape-priority stack that every overlay registers with on mount/unmount (extend `pickEscapeAction` to consult it), **or** have `Modal`/`PromptDialog` capture Escape in the capture phase and call `stopPropagation()` — pick one approach and apply it uniformly rather than patching each dialog individually (patching individually is how the current three-sources-of-truth mess happened).
- [ ] F40: `lib/shortcuts.ts:264-282` documents XButton1/XButton2 (mouse back/forward) as "Suppressed while any modal is open or a text field is focused," but `useMouseNavigation.ts:27-42` implements **no suppression at all** — this is a promise the registry makes that the code doesn't keep. The same overlay-registration mechanism built for Escape should also gate mouse-back/forward.
- [ ] Add regression coverage: for each overlay type (attachment preview, replay modal, publish prompt, repo-selection modal, dirty-repo warning, terminal drawer), assert Escape and mouse-back close only that overlay and leave the navigation stack untouched.

## Story UX1.7 — Delete dead parallel UI (F36)

**As a** maintainer, **I want** ~3,200 lines of unused parallel implementations removed, **so that** bug fixes applied to the dead code stop looking like fixes while changing nothing live.

**References:** `docs/ux-audit/findings.md` F36.

**Status:** Not started.

**Tasks:**
- [ ] Confirm (re-verify against current `HEAD`, don't trust the audit blindly) that nothing imports: `CreateFromZeroWizard.tsx` (328 lines) plus the fourteen `ui/CreateZero*.tsx` step/panel/hook files, `hooks/useCreateProjectWizard.ts` (253 lines), `lib/createProject.ts`, `CommandSelector.tsx` (241 lines), `TerminalStatusOverlay.tsx`, `ShortcutHelp.tsx` (381 lines) + `ShortcutsContext.tsx` (119 lines) — the last two are **not** dead per Story UX1.3, which mounts `ShortcutHelp`; do not delete those, wire them up instead (see UX1.3).
- [ ] Delete the confirmed-dead files (the wizard duplicate, `CommandSelector`, `TerminalStatusOverlay`, and their exclusive test files).
- [ ] Note the correctness-trap framing from the audit: the two wizards implement the same seven-step flow with different validation — a fix applied to the dead one previously would have looked like progress while changing nothing. Grep for any recent commits touching the dead files as a sanity check that this hasn't already happened.
- [ ] Confirm the build still passes (`tsc --noEmit`) and bundle size drops after deletion.

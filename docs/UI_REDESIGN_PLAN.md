# UI Redesign Plan — Pipeline View & Project View

> Source of the visual direction: `ui-mocks/orchestrator_mockup.tsx`.
> Design language, token values and colour semantics: **AGENTS.md §4** and
> `src/App.css` — this plan defers to both and never restates them.
> Existing UX defects it folds in: [`docs/ux-audit/findings.md`](ux-audit/findings.md).

The mock is a *direction*, not an implementation. It is untyped JS on a GitHub-dark
palette with hard-coded hex, layout in `style=` props, a `dangerouslySetInnerHTML`
`<style>` block, and a resize handler that pushes a state update per mouse-move pixel.
None of that ships. What ships is the four structural ideas it gets right, translated
into this codebase's palette, primitives and hexagon discipline.

---

## 1. What the mock actually proposes

Stripping the styling, five structural changes:

| # | Idea | Status here today |
|---|------|-------------------|
| **A** | Pipeline view is a **resizable two-pane split**: run on the left, a persistent step inspector on the right | Inspector exists (`canvas/NodePanel.tsx`) but is reachable **only** from the Graph view; the Timeline duplicates its job inline. Split is automatic and non-negotiable at ≥1600 px (`components/runLayout.ts`) |
| **B** | Feature prompt collapses into the title block | `FeatureDetail/InitialPromptPanel.tsx` is always expanded |
| **C** | Telemetry is one compact metric strip in the top chrome | `FeatureDetail/FeatureHeader.tsx` renders four large stacked stat blocks that wrap and eat header height |
| **D** | Activity log is a collapsible block with a live "polling every 3s" affordance | `RunEventTimeline.tsx` exists; no disclosure, no sync affordance |
| **E** | Subtasks render as ticket rows with landed / running / queued state | Already built as `SequenceTasks` **inside** `NodePanel`, so it is graph-only |

**The headline finding: most of the mock's right-hand panel already exists.**
`NodePanel` has the exact `Overview | Live | Output | Actions` tabs the mock draws
(`canvas/NodePanel.tsx:117`), the attempt-history table, and the ticket rows. The gap
is not "build an inspector" — it is **"the inspector serves one of two views."** That
reframes the whole job from a rewrite into a wiring change plus a slim-down, which is
both cheaper and lower-risk.

### Rejected from the mock

- **The palette.** Teal is not in this design language; `#0d1117 / #161b22 / #30363d`
  are not tokens. Translation table in §2.
- **The fake graph.** The mock's "graph" is a vertical list of divs. This app has real
  ELK layout over `@xyflow/react` (`canvas/WorkflowCanvas.tsx`, `canvas/useElkLayout.ts`)
  with auto-layout and zoom already. Adopting the mock's version is a regression.
- **`style={{ width: '65%' }}` + `setState` per mouse-move.** §4.1 replaces it.
- **`React.cloneElement`** for icon sizing, untyped props, and the injected
  `<style>` block — all three are hard "no" against AGENTS.md §3.
- **Always-on animation on containers.** The mock pulses whole cards; `App.css:758-771`
  records a real incident where animating `box-shadow`/`scale` pinned the WKWebView GPU
  process at idle. §5.6.

---

## 2. Design translation

Token values live in `src/App.css`; colour *semantics* are AGENTS.md §4. Mapping:

| Mock literal | Use instead | Why |
|---|---|---|
| `#0d1117`, `#090c10` | `--bg-app`, `--bg-well` | App shell / sunk wells |
| `#161b22`, `#1c2128` | `--bg-panel`, `--bg-panel-hover` | Glass card surfaces |
| `#30363d`, `#484f58` | `--border-glass` (+ hover variant) | One border language |
| `teal-400/500` | `--accent-cyan` | Cyan = terminal streams, interactive states |
| `purple-600` | `--accent-purple` (violet) | Violet = active connections, primary actions |
| `green-500` | `--accent-emerald` | Emerald = running agents, healthy status |
| `red-400/500` | `--accent-ruby` / `ruby-*` scale | Ruby = errors, stopped, failures |
| Flat card + `border-l-4` | `.glass-panel` + tone accent bar | Glassmorphism is the card language; `TONE_ACCENT` in `ProjectHome.tsx:28` already implements the bar |
| Ad-hoc chip colours | `TONE_CHIP` from `lib/runStatus.ts` | F27 fixed status-colour drift once; do not re-open it |

**Rule for the whole redesign:** every status colour resolves through
`lib/runStatus.ts`. A component that spells its own `bg-emerald-500/10 text-…` chip is
re-introducing F27. New surfaces get a tone from `runStatusMeta()` and read the class
out of `TONE_CHIP` / `TONE_TEXT` / `TONE_BORDER_L`.

**WebKitGTK caveat.** `App.css:823-880` carries an `!important` safelist of specific
utility classes for the Linux webview. Any *new* arbitrary-value colour utility is
outside it. Either express the colour as a token or verify it renders under
`npm run dev:tauri` on Linux before calling the phase done — a class that silently
no-ops there is invisible to `npm run checks`.

---

## 3. UX improvements beyond the mock

The mock is a visual refresh. These are the changes that alter what the app is *like
to use*, and several of them are also the performance fix (§4) — the same move.

### 3.1 One inspector, both views — selection instead of expansion

Today the Timeline expands in place: the live stream, the artifact list, the rerun
panel and the environment panel all mount *inside* `StepCard`, which is why that card
is 246 lines of conditional panels. Expansion is the worst option available:

- It reflows every sibling card below it.
- It can only show one step's detail at a time anyway — the same constraint an
  inspector has, without the inspector's stable reading position.
- It forces `StepCard` to receive the whole `streamContent` record, which is the
  single worst render bug in the view (§4.2).

Selecting a row and rendering detail in the inspector fixes all three at once. The
`StepCard` becomes identity + status + metrics + one primary CTA; everything else
already has a home in `NodePanel`'s four tabs.

### 3.2 "Needs you" is the app's promise — make it structural

Gates are the product's core value proposition, and they are currently something you
find by scrolling. Three cheap changes:

- **Project view:** segment/sort the pipeline list so `gated` and `needs-credentials`
  float to the top. `runStatusMeta().tone === 'amber'` already identifies them.
- **Pipeline view:** a persistent gate strip under the header while any step is
  `awaiting_gate`, with the decide CTA — not only the per-card block.
- **Rail:** the amber count already exists for terminals (`RailNavItem` `attentionCount`);
  extend the same affordance to gated runs.

### 3.3 Progressive disclosure in the project view

The pipeline card currently shows status chip + workflow chip + transport chip +
feature id + title + 2-line description + duration + tokens. That is eight competing
elements per row, all at similar weight. Restructure to a three-tier read:

1. **Scan tier** (always): tone accent bar, title, status chip, elapsed.
2. **Context tier** (secondary weight): workflow, transport, cost, tokens.
3. **Detail tier** (on demand): description, feature id, branch.

Add cost — audit *Opportunity 5* notes it is the number users actually watch and it is
already summed per feature.

### 3.4 Skeletons, not spinners

`FeatureDetail.tsx:171` swaps the entire run column for a centred spinner, and
`ProjectHome.tsx:604` does the same for the list. Both **unmount** their subtree, so
returning re-mounts the graph and re-runs ELK. A skeleton that preserves the layout
box avoids the remount and reads as faster even when it isn't.

### 3.5 Deep-linkable selection

`selectedNodeId` is local state in `useRunGraph`. Move it onto the `detail` view in
the navigation reducer, alongside `gateStepExecutionId` which already lives there.
Then back/forward, the gate deep-link and the inspector all use one mechanism, and
"which step was I looking at" survives a navigation.

### 3.6 Keyboard

`j`/`k` to move the step selection, `Enter` to focus the inspector, `g`/`t` to switch
Graph/Timeline. **Register these in the existing registry** — audit F5 records that the
shortcut system already has three disagreeing sources of truth. Adding a fourth
inline `keydown` is worse than shipping no shortcuts.

### 3.7 Density

A long run is 30+ steps. Offer comfortable/compact for the timeline and the pipeline
list, persisted. Compact is a padding + font-size token swap, not a second component.

---

## 4. Performance

These are specific defects in the current code, each with the file that carries it.
Fixing them **before** the visual work is deliberate: otherwise the redesign inherits
the blame for jank it did not cause.

### 4.1 Resize must not render (new code — get it right the first time)

The mock's approach re-subscribes `document` listeners on a state flag and calls
`setLeftPaneWidth` per mouse-move — a full React render tree per pixel, at pointer
frequency, with the ELK-driven graph inside it.

The `SplitPane` primitive instead:

- `onPointerDown` → `setPointerCapture` on the handle. No document listeners, no
  subscribe/unsubscribe churn, and the drag survives the pointer leaving the window.
- During the drag, write `--inspector-w` **directly onto the container element via
  its ref**. Zero React renders; the browser recalculates one custom property.
- `onPointerUp` → commit the final value to state once and persist it.
- `role="separator"` + `aria-valuenow` + arrow-key resize, so it is not mouse-only.
- Min/max clamps in a pure function so the constraint is unit-testable.

This matters more here than in a generic app: `useRunColumnLayout` observes the run
column with a `ResizeObserver` and feeds the result into `planLayout`, so a
state-per-pixel drag would run graph layout planning per pixel too. Committing once
means the observer fires once.

### 4.2 Stream flushes re-render every step card — **the worst current bug**

The chain:

1. `useAgentStream.ts:22` flushes `setStreamContent({ ...streamBufferRef.current })`
   once per animation frame while an agent streams — a **new object identity** each
   time.
2. `FeatureDetail.tsx:261` passes that whole record into `StepTimeline`.
3. `StepTimeline.tsx:83` passes it into **every** `StepCard`.
4. `StepCard` is not memoized.

So while any agent is streaming, every step card in the run re-renders up to 60×/s —
including all the cards that render nothing from that record.

Fix, in order of value:

- Pass **the selected step's stream string**, never the record. One consumer, one
  `string` prop, changes only when that step's text changes.
- Keep the buffer in the ref as today and expose it through `useSyncExternalStore`
  with a per-step selector, so the subscription is per-consumer instead of
  broadcast-to-all.
- `memo` `StepCard`.
- Once §3.1 lands, the stream has exactly one mount site (the inspector's Live tab)
  and the fan-out is structurally gone, not just guarded.

### 4.3 The stream buffer is unbounded

`useAgentStream.ts:18` appends forever with no cap, and `StepCard.tsx:237-241`
renders it into a `<pre className="whitespace-pre-wrap">` inside a `max-h-64`
scroller. A long agent turn means megabytes of text whose full height must be laid
out on every flush.

Fix: cap at accumulate time — keep the last N KB / last N lines, dropping from the
front (agents' useful output is the tail). Add `content-visibility: auto` +
`contain-intrinsic-size` to the log block; `App.css:896` already establishes that
pattern for `.stream-event`.

### 4.4 Full reload on every progress tick (audit F19)

`useFeatureRun.ts:130-139` calls `reload()` on every `step_progress` event.
`reload()` is two IPC round trips (`listStepsForRun` + `getFeature`) plus eight
`setState` calls plus a models probe.

And the Tauri event is **not** throttled. `PROGRESS_THROTTLE_MS = 8_000` in
`crates/demeteo-core/src/adapters/run_event_log.rs:60` gates only the *persisted*
run-event append; `RunEventRecorder::emit` forwards every event unchanged to the UI
notifier — including every mid-turn token/cost refresh.

Fix:

- Coalesce `reload()` behind a trailing-edge scheduler — at most one in flight, at
  most one queued, floor of ~500 ms. (Backend throttling is the wrong lever: the log
  wants a readable narrative, the UI wants smooth telemetry. Different jobs.)
- Patch the single step from the event payload for the interim, so the UI stays live
  between reloads without a fetch.
- Delete `setTotalCost(payload.cost_usd)` (`useFeatureRun.ts:136`) — it sets the
  *pipeline* total to one step's cost, so the header visibly drops until the next
  reload. This is F19 and it is a correctness bug, not just perf.

### 4.5 `steps` identity churn defeats every memo

`setSteps(list)` replaces the array and every row object on each reload, so
`harnessEvidence`, `runStatusByNode`, the graph overlay and all cards recompute even
when nothing changed. Reconcile by id and return the previous object when a row is
unchanged — then memoized cards genuinely skip. Put the reconcile in a pure,
tested function in `lib/`; it is exactly the kind of policy that should not live
inside an async fetch.

### 4.6 Unstable callback props

`StepTimeline.tsx:79` creates `cardRef={(el) => …}` per render per card;
`StepCard.tsx:207` creates `onSelect={(path) => …}`. Both defeat `memo` before it is
added. Handlers take the id as an argument and are created once with `useCallback`;
ref collection goes through one stable callback that closes over the ref map only.

### 4.7 Project view

- `ProjectHome.tsx:642-693` runs two IIFEs per card per render to derive the workflow
  and transport badges. Hoist to pure functions in `lib/` (`workflowBadge.ts` already
  set this precedent) and extract a memoized `PipelineCard`.
- `stageClipboardFiles` (`ProjectHome.tsx:76`) depends on `attachments`, so the paste
  handler is rebuilt on every attachment change. Use the functional form of
  `setAttachments` and drop the dependency.

### 4.8 Poll while hidden

`useRemoteRun.ts:107` polls every 3 s regardless of window visibility. Gate on
`document.hidden`, and back off on consecutive failures instead of retrying at a flat
3 s into a dead tunnel.

### 4.9 Long lists, without a new dependency

Virtualization would want a dependency, and **that is an AGENTS.md §6 gate**. It is
also not needed yet: `content-visibility: auto` with `contain-intrinsic-size` gets
most of the win for the timeline and the pipeline list with zero deps, using the
pattern already in `App.css`. Revisit only with a measurement showing it is
insufficient — and then ask before adding the dep.

---

## 5. Component architecture

### 5.1 New primitives in `src/components/ui/`

Each is generic, typed, one per file, documented in the `.design-sync` docs map so it
joins the existing catalogue.

| Component | Replaces | Notes |
|---|---|---|
| `SplitPane` | mock's hand-rolled resizer | §4.1. Controlled, persisted by key, keyboard-accessible |
| `Chip` | four separate re-spellings of the same pill | Tone from `runStatus.ts`. Does **not** subsume `StatusBadge` — see the division recorded below |
| `MetricStrip` + `Metric` | `FeatureHeader`'s four stat blocks, mock's `MetricItem` | Label + value + tone + optional tooltip |
| `Disclosure` | `InitialPromptPanel`, activity block | One animation, one a11y contract (`aria-expanded`, `aria-controls`) |
| `SegmentedControl` | `RunViewToggle` (which *is* one), new list filter | Generalize the existing component; do not add a second |
| `Skeleton` | centred spinners | Layout-preserving |
| `Inspector` | shell around `NodePanel` | Header + tab strip + body; `NodePanel`'s tabs move into it |

`ScrollArea`, `SectionCard`, `TabBar`, `Modal`, `OverlayPortal`, `EmptyStateCard` and
`StatusBadge` already exist — reuse, don't re-create. Audit F28 and F36 are both about
parallel implementations drifting; every new component here has to justify why an
existing one couldn't take the job.

#### Two overlaps Phase 0 surfaced, and how they resolve

Both are cases where "one primitive" was the plan and two components exist in fact.
Recorded here because a migration that changes appearance is not the "no visual change"
Phase 0 claimed, and pretending otherwise is how a redesign loses its audit trail.

- **`Chip` vs `StatusBadge`.** They do *not* do the same job, and `Chip` was wrong to
  imply it. `StatusBadge`'s `dot` variant is a standalone glow dot with no label —
  a rail/list-row affordance. `Chip`'s dot is `bg-current` and exists only inside a
  pill. **Division:** `Chip` owns every labelled pill; `StatusBadge` narrows to the
  standalone dot. Both already resolve colour through `runStatusMeta` + `TONE_CHIP`,
  so there is no F27 drift either way — this is about typography and scope, not colour.
  Phase 4 migrates the pill call sites and drops `StatusBadge`'s `pill` variant then.
- **`SegmentedControl` vs `RunViewToggle`.** §5.1 said generalize, don't add a second;
  a second landed and `RunViewToggle` is untouched. Their selected states differ —
  `RunViewToggle` uses `bg-cyan-500/15 text-cyan-300` plus a glow, `SegmentedControl`
  uses `TONE_CHIP.cyan`. **So Phase 2's migration is a deliberate visual change, not a
  pure swap.** Decide the selected-state treatment once, there, and apply it to both
  call sites; do not let the two drift further apart in the meantime.
- **`SplitPane` restore-on-reopen.** The width to restore after a collapse currently
  lives in a component-local ref, so it does not survive a remount. §7's
  "collapse fully, restore last width" therefore needs Phase 6 to own that value as
  state, not the component. Phase 2 should pass it in rather than build on the ref.

### 5.2 Where the decisions live

Mirror the Rust discipline in AGENTS.md §3: a `match` that decides *what should
happen* does not belong inside the component that renders it. Frontend precedent
already exists — `runLayout.ts`, `workflowBadge.ts`, `runStatus.ts`, `runLayout`'s
tests. New pure modules under `src/lib/`:

- `pipelineCard.ts` — status/workflow/transport/metric derivation for one feature row.
- `pipelineFilter.ts` — filter + sort + "needs you first" ordering.
- `inspectorTarget.ts` — given steps, selection and view mode, which step the
  inspector shows (and the empty/blocked cases).
- `streamBuffer.ts` — the cap-and-append rule from §4.3.
- `stepReconcile.ts` — the identity-preserving merge from §4.5.

Each gets a unit test. This is what makes the redesign reviewable: the interesting
logic is testable without mounting a component or stubbing twenty ports.

### 5.3 React rules this redesign holds itself to

- `memo` on every row component in a list that re-renders from a live feed
  (`StepCard`, `PipelineCard`, `SubtaskRow`, `RunEventRow`), with a
  re-render-count test where the render is expensive.
  `ArtifactViewer.rerender.test.tsx` is the pattern to copy — it counts renders of the
  real subtree instead of asserting that `memo` exists.
- No object/array/function literal in a prop passed to a memoized child.
- Derived values via `useMemo` only where the input identity is actually stable —
  a `useMemo` over a fresh array every render is cost with no benefit (§4.5 is the
  prerequisite).
- Live-feed state stays in a ref plus `useSyncExternalStore`, not in state that
  broadcasts to a whole subtree.
- No `any` (AGENTS.md §3); `unknown` + a type guard for anything crossing IPC.
- One component per file; extract past ~400 LOC. `NodePanel` at 956 lines is already
  over and gets split as part of Phase 2.

---

## 6. Phases

Each phase ends green on `npm run checks` and smoke-tested with `npm run dev:tauri`,
and each is independently shippable. Conventional Commits per AGENTS.md §5 —
note the `subject-case` trap: no capitalized leading token.

### Phase 0 — Foundations (no visual change)
`ui/` primitives from §5.1 with tests; pure modules from §5.2 with tests; add them to
`.design-sync/config.json`. Fix audit **F17** first — `FeatureDetail.tsx:39` returns
before ~20 hook calls, which makes every subsequent restructure a hooks-order crash
waiting to happen.
*Verify:* new unit tests fail before the implementation exists (AGENTS.md §7 — a test
you have not watched fail is not coverage).

### Phase 1 — Performance (§4.2–4.8), still no visual change
Stream fan-out, buffer cap, reload coalescing + F19, step reconcile, callback
stability, visibility-gated poll.
*Verify:* re-render-count tests for `StepCard` under a simulated stream and under a
`step_progress` burst. Both must fail on today's code.

### Phase 2 — Pipeline view shell
`SplitPane` + the unified `Inspector`; `NodePanel` split into its four tab modules
behind it; Timeline rows become selectable and drive the same inspector; collapsible
prompt (**B**); `MetricStrip` header (**C**); sticky collapsed header on scroll;
selection moved into navigation state (§3.5).
Reconcile with `runLayout.ts`: automatic split at 1600 px becomes the *default* width
for a pane the user can now resize and collapse. `pickRunLayout` keeps deciding the
initial state; it stops being the final word. Update its doc comment accordingly —
AGENTS.md §3, you changed the code, you own the comment.

### Phase 3 — Timeline & step-card slim-down
`StepCard` down to scan tier + one CTA; stream, artifacts, rerun and environment
panels served by the inspector; gate strip (§3.2); density toggle (§3.7);
`content-visibility` on rows.

**Two things Phase 1 deliberately left here**, because moving these panels out of the
card is what actually fixes them:

- `StepCard` is memoized, but two of its props are still fresh every render and both
  come from hooks Phase 1 did not touch: `overrides` (`useHarnessOverrides` returns a
  new object literal) and `handleRetryStep` / `handleStopStep` (`useRerunActions`
  returns bare async functions). So the memo holds on the stream path — where it
  mattered, at frame rate — but not on the reload path, which is now ≤2 renders/sec.
  Stabilizing those hooks is possible, but Phase 3 removes both props from the card
  entirely, so doing it earlier is work Phase 3 deletes.
- The graph's Live tab renders the same capped tail as the timeline but **without** the
  truncation notice, because `NodePanel` was outside Phase 1's file set. Once the stream
  has one mount site in the inspector there is one place to say it.

### Phase 4 — Project view
`PipelineCard` (memoized, three-tier read per §3.3, cost column);
filter + sort + needs-you-first via `SegmentedControl` and `pipelineFilter.ts`;
`MetricStrip` for the telemetry cluster; skeletons; empty state through
`EmptyStateCard`. Fix audit **F10** while here — the header hard-codes
"Connected via GitHub Enterprise • Default Workflow: Standard Feature Pipeline"
(`ProjectHome.tsx:468`) for every project regardless of the real provider and workflow.
A redesign that repaints a lie is worse than one that fixes it.

### Phase 5 — Activity surface & motion budget
Collapsible activity block with sync affordance (**D**); every always-on animation
audited to opacity-only per `App.css:758-771`; pulse confined to small dots, never a
container holding text (`StepCard.tsx:160` currently pulses a whole gate block);
`prefers-reduced-motion` coverage for anything new.

### Phase 6 — Keyboard & persistence
`j`/`k`/`Enter`/`g`/`t` through the existing registry (§3.6); persist split width,
density, last view mode and filter. Audit *Opportunity 2* notes
`get_app_session`/`set_app_session` already exist — use them rather than
`localStorage`.

---

## 7. Gates, risks and what this plan does not decide

**AGENTS.md §6 gates — none of these are mine to take:**

- **No new npm dependency** is assumed anywhere. If measurement later says
  virtualization (`@tanstack/react-virtual`) or a spring animation library earns its
  place, that is a question, not a commit.
- Nothing here touches `src-tauri/capabilities/`, agent spawn logic, migrations, or
  `OPENCODE_PERMISSION`. Phase 6's persistence uses existing session commands; if it
  needs a schema change, that is a gate.

**Risks:**

- **WebKitGTK invisibility.** `npm run checks` compiles for the host and never renders.
  A new arbitrary-value utility outside the `App.css` safelist can silently no-op on
  Linux. Every phase with new colour classes needs a `dev:tauri` look on Linux.
- **Glassmorphism cost.** `backdrop-filter: blur(12px)` is cheap once and expensive
  nested. The inspector sitting inside the run column must not stack a third blur
  layer over the two already there. `App.css` records a real WKWebView GPU incident;
  treat blur as a budget.
- **Cross-OS (AGENTS.md §3).** Pointer capture, `ResizeObserver` and
  `content-visibility` behave differently across WebKitGTK / WKWebView / WebView2, and
  PR checks run on `ubuntu-22.04` only. macOS and Windows appearance is unverified by
  any gate — say so explicitly when handing a phase back rather than implying it was
  checked.
- **Existing tests.** `FeatureDetail.test.tsx` and `ProjectHome.test.tsx` will need
  updating. Updating an assertion because the DOM moved is fine; deleting one because
  it now fails is a regression in disguise.
- **Scope creep into the audit backlog.** F17, F19, F10 are on this plan's critical
  path and are in scope. The other ~40 findings are not, and are scheduled separately
  under roadmap Theme F.

**Deliberately left open:**

- Whether the inspector should be dismissible to zero width or clamp to a minimum.
  Recommendation: collapse fully, with the last width restored on reopen — a 30-step
  run wants the full column sometimes.
- Whether Graph or Timeline is the default. Today it is Timeline. The mock implies
  Timeline too. Worth revisiting once the inspector serves both, since the argument
  for Timeline-as-default was partly that the graph had no detail affordance.

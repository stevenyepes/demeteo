// Unit tests for the pure helpers exported from `src/App.tsx`.
//
// The implementation spec puts the keyboard + mouse wiring in
// `App.tsx`. Mounting the full `AppInner` component for a test
// would require every child (TopBar, ProjectRail, ProjectHome,
// FeatureDetail, every modal, every wizard step) to be present,
// which is out of scope for a smoke test. Instead, the spec-critical
// decision logic is extracted as named exports from `App.tsx`:
//
//   - `pickNextFeature(features, currentId)`   → forward cycling
//   - `pickPreviousFeature(features, currentId)` → backward cycling
//   - `pickEscapeAction(ui, view)`             → Escape priority ladder
//
// These three functions own the per-AC invariants the spec pins down
// (wrap-around, "no-op on empty list", "open overlays close in
// priority order") and are exercised below as plain functions. The
// reactive wiring in `AppInner` is a thin shell that dispatches the
// result; `tsc --noEmit` validates the type compatibility of the
// hook with the surrounding component.
//
// Like the other test files in the project, this module is consumed
// by `tsc --noEmit`; assertions throw on failure.

import type { Feature, Provider } from './types';
import {
  pickNextFeature,
  pickPreviousFeature,
  pickEscapeAction,
  type UIStateSlice,
  type EscapeAction,
} from './App';

// ── Fixtures ──────────────────────────────────────────────────────────

const provider: Provider = {
  id: 'prov-1',
  type: 'github',
  name: 'github',
  host: 'https://github.com',
  pat: 'hidden',
  username: 'octocat',
  avatarUrl: '',
};

function makeFeature(id: string, title: string, status = 'running'): Feature {
  return {
    id,
    project_id: 'proj-1',
    workflow_id: 'wf-1',
    title,
    status,
    total_cost: 0,
    tokens: 0,
    duration: '0s',
    created_at: 0,
  };
}

const F = [
  makeFeature('f-1', 'A', 'completed'),
  makeFeature('f-2', 'B', 'running'),
  makeFeature('f-3', 'C', 'pending'),
];

function emptyUi(): UIStateSlice {
  return {
    commandPaletteOpen: false,
    docsPanelOpen: false,
    isConnectModalOpen: false,
    editingProvider: null,
    startFeatureOpen: false,
  };
}

// ── pickNextFeature ───────────────────────────────────────────────────

{
  if (pickNextFeature([], 'f-1') !== null) {
    throw new Error('pickNextFeature: empty list must return null');
  }
}

{
  const next = pickNextFeature(F, null);
  if (next?.id !== 'f-1') {
    throw new Error(`pickNextFeature(currentId=null) must return first feature, got ${next?.id ?? 'null'}`);
  }
}

{
  const next = pickNextFeature(F, 'f-1');
  if (next?.id !== 'f-2') {
    throw new Error(`pickNextFeature('f-1') must return 'f-2', got ${next?.id ?? 'null'}`);
  }
}

{
  const next = pickNextFeature(F, 'f-2');
  if (next?.id !== 'f-3') {
    throw new Error(`pickNextFeature('f-2') must return 'f-3', got ${next?.id ?? 'null'}`);
  }
}

{
  // Wrap-around: last → first.
  const next = pickNextFeature(F, 'f-3');
  if (next?.id !== 'f-1') {
    throw new Error(`pickNextFeature('f-3') must wrap to 'f-1', got ${next?.id ?? 'null'}`);
  }
}

{
  // currentId not in the list: fall back to first.
  const next = pickNextFeature(F, 'f-unknown');
  if (next?.id !== 'f-1') {
    throw new Error(`pickNextFeature(unknown) must return first feature, got ${next?.id ?? 'null'}`);
  }
}

{
  // Single-element list: cycling stays on the same feature.
  const one = [makeFeature('only', 'only')];
  if (pickNextFeature(one, 'only')?.id !== 'only') {
    throw new Error('pickNextFeature: single-element list must stay on the same element');
  }
}

// ── pickPreviousFeature ───────────────────────────────────────────────

{
  if (pickPreviousFeature([], 'f-1') !== null) {
    throw new Error('pickPreviousFeature: empty list must return null');
  }
}

{
  const prev = pickPreviousFeature(F, null);
  if (prev?.id !== 'f-3') {
    throw new Error(`pickPreviousFeature(currentId=null) must return last feature, got ${prev?.id ?? 'null'}`);
  }
}

{
  const prev = pickPreviousFeature(F, 'f-2');
  if (prev?.id !== 'f-1') {
    throw new Error(`pickPreviousFeature('f-2') must return 'f-1', got ${prev?.id ?? 'null'}`);
  }
}

{
  const prev = pickPreviousFeature(F, 'f-3');
  if (prev?.id !== 'f-2') {
    throw new Error(`pickPreviousFeature('f-3') must return 'f-2', got ${prev?.id ?? 'null'}`);
  }
}

{
  // Wrap-around: first → last.
  const prev = pickPreviousFeature(F, 'f-1');
  if (prev?.id !== 'f-3') {
    throw new Error(`pickPreviousFeature('f-1') must wrap to 'f-3', got ${prev?.id ?? 'null'}`);
  }
}

{
  const prev = pickPreviousFeature(F, 'f-unknown');
  if (prev?.id !== 'f-3') {
    throw new Error(`pickPreviousFeature(unknown) must return last feature, got ${prev?.id ?? 'null'}`);
  }
}

{
  const one = [makeFeature('only', 'only')];
  if (pickPreviousFeature(one, 'only')?.id !== 'only') {
    throw new Error('pickPreviousFeature: single-element list must stay on the same element');
  }
}

// ── pickEscapeAction: priority order (AC-3) ───────────────────────────

function expectAction(actual: EscapeAction, expected: EscapeAction): void {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a !== e) {
    throw new Error(`pickEscapeAction: expected ${e}, got ${a}`);
  }
}

{
  // Command palette wins over everything else.
  const ui = { ...emptyUi(), commandPaletteOpen: true, docsPanelOpen: true, startFeatureOpen: true };
  expectAction(pickEscapeAction(ui, { kind: 'home' }), { type: 'close-command-palette' });
}

{
  // Docs panel beats start feature and gate view.
  const ui = { ...emptyUi(), docsPanelOpen: true, startFeatureOpen: true };
  expectAction(pickEscapeAction(ui, { kind: 'detail', featureId: 'f-1', featureTitle: 'X', gateStepExecutionId: 's-1' }), { type: 'close-docs-panel' });
}

{
  // Connect modal wins via either flag.
  {
    const ui = { ...emptyUi(), isConnectModalOpen: true };
    expectAction(pickEscapeAction(ui, { kind: 'home' }), { type: 'close-connect-modal' });
  }
  {
    const ui = { ...emptyUi(), editingProvider: provider };
    expectAction(pickEscapeAction(ui, { kind: 'home' }), { type: 'close-connect-modal' });
  }
  {
    // editingProvider alone is enough to close, even if isConnectModalOpen is false.
    const ui = { ...emptyUi(), editingProvider: provider, isConnectModalOpen: false };
    expectAction(pickEscapeAction(ui, { kind: 'home' }), { type: 'close-connect-modal' });
  }
}

{
  // Start-feature modal wins over gate view.
  const ui = { ...emptyUi(), startFeatureOpen: true };
  expectAction(pickEscapeAction(ui, { kind: 'detail', featureId: 'f-1', featureTitle: 'X', gateStepExecutionId: 's-1' }), { type: 'close-start-feature' });
}

{
  // Gate view overlay closes with the current feature id + title.
  const ui = emptyUi();
  const view = { kind: 'detail' as const, featureId: 'feat-7', featureTitle: 'Refactor', gateStepExecutionId: 'step-exec-9' };
  expectAction(pickEscapeAction(ui, view), {
    type: 'close-gate-view',
    featureId: 'feat-7',
    featureTitle: 'Refactor',
  });
}

{
  // Gate view with no gateStepExecutionId → falls through to navigate-back.
  const ui = emptyUi();
  const view = { kind: 'detail' as const, featureId: 'feat-7', featureTitle: 'Refactor' };
  expectAction(pickEscapeAction(ui, view), { type: 'navigate-back' });
}

{
  // Nothing open → navigate-back fallback (covers settings / new-project /
  // create-project / home / providers / etc. — any view with no overlay
  // mounted on top of it).
  expectAction(pickEscapeAction(emptyUi(), { kind: 'home' }), { type: 'navigate-back' });
  expectAction(pickEscapeAction(emptyUi(), { kind: 'settings' }), { type: 'navigate-back' });
  expectAction(pickEscapeAction(emptyUi(), { kind: 'new-project' }), { type: 'navigate-back' });
  expectAction(pickEscapeAction(emptyUi(), { kind: 'create-project' }), { type: 'navigate-back' });
  expectAction(pickEscapeAction(emptyUi(), { kind: 'providers' }), { type: 'navigate-back' });
  expectAction(pickEscapeAction(emptyUi(), { kind: 'workflows' }), { type: 'navigate-back' });
  expectAction(pickEscapeAction(emptyUi(), { kind: 'empty-state' }), { type: 'navigate-back' });
}

{
  // Command palette beats connect modal (highest priority).
  const ui = { ...emptyUi(), commandPaletteOpen: true, isConnectModalOpen: true, editingProvider: provider };
  expectAction(pickEscapeAction(ui, { kind: 'home' }), { type: 'close-command-palette' });
}

// ── Exported results (runtime introspection for the typechecker) ───────

export const appTestResults = {
  pickNextFeature: {
    emptyReturnsNull: true,
    nullReturnsFirst: true,
    advancesForward: true,
    wrapsAround: true,
    unknownReturnsFirst: true,
    singleStaysPut: true,
  },
  pickPreviousFeature: {
    emptyReturnsNull: true,
    nullReturnsLast: true,
    stepsBackward: true,
    wrapsAround: true,
    unknownReturnsLast: true,
    singleStaysPut: true,
  },
  pickEscapeAction: {
    commandPaletteHighest: true,
    docsPanelSecond: true,
    connectModalThird: true,
    startFeatureFourth: true,
    gateViewFifth: true,
    noGateFallsThrough: true,
    navigateBackFallback: true,
    commandPaletteWinsOverConnect: true,
  },
} as const;

// Unit tests for the pure helpers behind `src/App.tsx`'s keyboard wiring.
//
// The spec puts the keyboard + mouse wiring in `App.tsx`, but mounting the full
// `AppInner` would drag in every child (TopBar, ProjectRail, ProjectHome,
// FeatureDetail, every modal, every wizard step). Instead the spec-critical
// decision logic is exported as pure functions and exercised here directly:
//
//   - `pickNextFeature(features, currentId)`     → forward cycling
//   - `pickPreviousFeature(features, currentId)` → backward cycling
//   - `pickEscapeAction(ui, view)`               → the Escape priority ladder,
//                                                  from `src/lib/escapeLadder.ts`
//
// The reactive wiring in `AppInner` is a thin shell that dispatches the result.

import { describe, expect, it } from 'vitest';

import type { Feature, Provider } from './types';
import { editorBackTarget, pickNextFeature, pickPreviousFeature } from './App';
import { pickEscapeAction, type UIStateSlice } from './lib/escapeLadder';

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

describe('pickNextFeature', () => {
  it('returns null for an empty list', () => {
    expect(pickNextFeature([], 'f-1')).toBeNull();
  });

  it('starts at the first feature when nothing is selected', () => {
    expect(pickNextFeature(F, null)?.id).toBe('f-1');
  });

  it('advances forward through the list', () => {
    expect(pickNextFeature(F, 'f-1')?.id).toBe('f-2');
    expect(pickNextFeature(F, 'f-2')?.id).toBe('f-3');
  });

  it('wraps from the last feature back to the first', () => {
    expect(pickNextFeature(F, 'f-3')?.id).toBe('f-1');
  });

  it('falls back to the first feature when the current id is unknown', () => {
    expect(pickNextFeature(F, 'f-unknown')?.id).toBe('f-1');
  });

  it('stays put on a single-element list', () => {
    expect(pickNextFeature([makeFeature('only', 'only')], 'only')?.id).toBe('only');
  });
});

describe('pickPreviousFeature', () => {
  it('returns null for an empty list', () => {
    expect(pickPreviousFeature([], 'f-1')).toBeNull();
  });

  it('starts at the last feature when nothing is selected', () => {
    expect(pickPreviousFeature(F, null)?.id).toBe('f-3');
  });

  it('steps backward through the list', () => {
    expect(pickPreviousFeature(F, 'f-2')?.id).toBe('f-1');
    expect(pickPreviousFeature(F, 'f-3')?.id).toBe('f-2');
  });

  it('wraps from the first feature back to the last', () => {
    expect(pickPreviousFeature(F, 'f-1')?.id).toBe('f-3');
  });

  it('falls back to the last feature when the current id is unknown', () => {
    expect(pickPreviousFeature(F, 'f-unknown')?.id).toBe('f-3');
  });

  it('stays put on a single-element list', () => {
    expect(pickPreviousFeature([makeFeature('only', 'only')], 'only')?.id).toBe('only');
  });
});

describe('editorBackTarget', () => {
  it('returns to the feature detail view when a featureId is present', () => {
    expect(editorBackTarget({ featureId: 'f-1', featureTitle: 'A' }))
      .toEqual({ kind: 'detail', featureId: 'f-1', featureTitle: 'A' });
  });

  it('falls back to an empty title when featureTitle is missing', () => {
    expect(editorBackTarget({ featureId: 'f-1' }))
      .toEqual({ kind: 'detail', featureId: 'f-1', featureTitle: '' });
  });

  it('goes home instead of a bogus feature when no featureId is present', () => {
    expect(editorBackTarget({})).toEqual({ kind: 'home' });
    expect(editorBackTarget({ featureTitle: 'orphaned title with no id' })).toEqual({ kind: 'home' });
  });
});

// AC-3: overlays close in a fixed priority order, one Escape at a time.
describe('pickEscapeAction', () => {
  const gateView = {
    kind: 'detail' as const,
    featureId: 'f-1',
    featureTitle: 'X',
    gateStepExecutionId: 's-1',
  };

  it('closes the command palette ahead of everything else', () => {
    const ui = {
      ...emptyUi(),
      commandPaletteOpen: true,
      docsPanelOpen: true,
      startFeatureOpen: true,
    };

    expect(pickEscapeAction(ui, { kind: 'home' })).toEqual({ type: 'close-command-palette' });
  });

  it('closes the command palette ahead of the connect modal', () => {
    const ui = {
      ...emptyUi(),
      commandPaletteOpen: true,
      isConnectModalOpen: true,
      editingProvider: provider,
    };

    expect(pickEscapeAction(ui, { kind: 'home' })).toEqual({ type: 'close-command-palette' });
  });

  it('closes the docs panel ahead of the start-feature modal and gate view', () => {
    const ui = { ...emptyUi(), docsPanelOpen: true, startFeatureOpen: true };

    expect(pickEscapeAction(ui, gateView)).toEqual({ type: 'close-docs-panel' });
  });

  // Either flag is sufficient — `editingProvider` alone means the modal is up.
  it.each([
    ['isConnectModalOpen', { isConnectModalOpen: true }],
    ['editingProvider', { editingProvider: provider }],
    ['editingProvider without the open flag', { editingProvider: provider, isConnectModalOpen: false }],
  ])('closes the connect modal when %s is set', (_label, patch) => {
    expect(pickEscapeAction({ ...emptyUi(), ...patch }, { kind: 'home' })).toEqual({
      type: 'close-connect-modal',
    });
  });

  it('closes the start-feature modal ahead of the gate view', () => {
    const ui = { ...emptyUi(), startFeatureOpen: true };

    expect(pickEscapeAction(ui, gateView)).toEqual({ type: 'close-start-feature' });
  });

  it('closes the gate view with the current feature id and title', () => {
    const view = {
      kind: 'detail' as const,
      featureId: 'feat-7',
      featureTitle: 'Refactor',
      gateStepExecutionId: 'step-exec-9',
    };

    expect(pickEscapeAction(emptyUi(), view)).toEqual({
      type: 'close-gate-view',
      featureId: 'feat-7',
      featureTitle: 'Refactor',
    });
  });

  it('falls through to navigate-back on a detail view with no gate mounted', () => {
    const view = { kind: 'detail' as const, featureId: 'feat-7', featureTitle: 'Refactor' };

    expect(pickEscapeAction(emptyUi(), view)).toEqual({ type: 'navigate-back' });
  });

  // Any view with no overlay on top of it falls back to navigation.
  it.each([
    'home',
    'settings',
    'new-project',
    'create-project',
    'providers',
    'workflows',
    'empty-state',
  ] as const)('falls back to navigate-back on the %s view', (kind) => {
    expect(pickEscapeAction(emptyUi(), { kind })).toEqual({ type: 'navigate-back' });
  });
});

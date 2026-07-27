/**
 * Undo/redo + dirty tracking (task P3.3). The reducer is where the subtle
 * claims live, so it is tested directly rather than through the builder:
 * a no-op edit is not history, undoing back to the saved shape is *clean*
 * again, and a new edit truncates the redo branch.
 */
import { describe, expect, it } from 'vitest';

import {
  canRedo,
  canUndo,
  graphHistoryReducer,
  HISTORY_LIMIT,
  initHistory,
  isDirty,
  serializeDefinition,
} from './graphHistory';
import type { WorkflowDefinitionV2 } from './types';

const base: WorkflowDefinitionV2 = {
  schema_version: 2,
  id: 'wf-h',
  name: 'History',
  nodes: [{ id: 'plan', type: 'agent', title: 'Plan' }],
  edges: [],
};

/** `base` plus `count` extra nodes — a cheap distinct definition. */
function withNodes(count: number): WorkflowDefinitionV2 {
  return {
    ...base,
    nodes: [
      ...base.nodes,
      ...Array.from({ length: count }, (_, i) => ({
        id: `extra-${i}`,
        type: 'gate',
        title: `Gate ${i}`,
      })),
    ],
  };
}

describe('graphHistoryReducer', () => {
  it('starts clean with nothing to undo or redo', () => {
    const h = initHistory(base);
    expect(canUndo(h)).toBe(false);
    expect(canRedo(h)).toBe(false);
    expect(isDirty(h)).toBe(false);
  });

  it('commits an edit, then undoes and redoes it', () => {
    let h = initHistory(base);
    h = graphHistoryReducer(h, { type: 'commit', def: withNodes(1) });
    expect(h.present.nodes).toHaveLength(2);
    expect(isDirty(h)).toBe(true);
    expect(canUndo(h)).toBe(true);

    h = graphHistoryReducer(h, { type: 'undo' });
    expect(h.present.nodes).toHaveLength(1);
    expect(canRedo(h)).toBe(true);

    h = graphHistoryReducer(h, { type: 'redo' });
    expect(h.present.nodes).toHaveLength(2);
    expect(canRedo(h)).toBe(false);
  });

  it('ignores a commit that changes nothing', () => {
    // The canvas re-derives and re-commits on gestures that land where they
    // started; those must not fill the undo stack or mark the graph dirty.
    let h = initHistory(base);
    h = graphHistoryReducer(h, { type: 'commit', def: { ...base } });
    expect(canUndo(h)).toBe(false);
    expect(isDirty(h)).toBe(false);
  });

  it('reads as clean again after undoing back to the saved shape', () => {
    // The reason dirty is a comparison and not a flag.
    let h = initHistory(base);
    h = graphHistoryReducer(h, { type: 'commit', def: withNodes(1) });
    expect(isDirty(h)).toBe(true);
    h = graphHistoryReducer(h, { type: 'undo' });
    expect(isDirty(h)).toBe(false);
  });

  it('marks the present as saved, making a later undo dirty again', () => {
    let h = initHistory(base);
    h = graphHistoryReducer(h, { type: 'commit', def: withNodes(1) });
    h = graphHistoryReducer(h, { type: 'saved' });
    expect(isDirty(h)).toBe(false);
    // Undoing away from a saved state is unsaved work in the other direction.
    h = graphHistoryReducer(h, { type: 'undo' });
    expect(isDirty(h)).toBe(true);
  });

  it('drops the redo branch when a new edit lands after an undo', () => {
    let h = initHistory(base);
    h = graphHistoryReducer(h, { type: 'commit', def: withNodes(1) });
    h = graphHistoryReducer(h, { type: 'undo' });
    expect(canRedo(h)).toBe(true);
    h = graphHistoryReducer(h, { type: 'commit', def: withNodes(2) });
    expect(canRedo(h)).toBe(false);
    expect(h.present.nodes).toHaveLength(3);
  });

  it('undo/redo on an empty stack are no-ops', () => {
    const h = initHistory(base);
    expect(graphHistoryReducer(h, { type: 'undo' })).toBe(h);
    expect(graphHistoryReducer(h, { type: 'redo' })).toBe(h);
  });

  it('bounds the remembered snapshots at HISTORY_LIMIT', () => {
    let h = initHistory(base);
    for (let i = 1; i <= HISTORY_LIMIT + 10; i += 1) {
      h = graphHistoryReducer(h, { type: 'commit', def: withNodes(i) });
    }
    expect(h.past).toHaveLength(HISTORY_LIMIT);
    // The oldest snapshots fell off the back, not the newest.
    expect(h.present.nodes).toHaveLength(HISTORY_LIMIT + 11);
  });

  it('resets for a plain load (clean) and for a restored draft (dirty)', () => {
    let h = initHistory(base);
    h = graphHistoryReducer(h, { type: 'commit', def: withNodes(1) });

    const loaded = graphHistoryReducer(h, { type: 'reset', def: base });
    expect(isDirty(loaded)).toBe(false);
    expect(canUndo(loaded)).toBe(false);

    const restored = graphHistoryReducer(h, {
      type: 'reset',
      def: withNodes(3),
      dirty: true,
    });
    expect(isDirty(restored)).toBe(true);
    expect(serializeDefinition(restored.present)).toBe(serializeDefinition(withNodes(3)));
  });
});

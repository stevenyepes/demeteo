/**
 * Undo/redo and dirty tracking for design mode (task P3.3, PRD §6.3).
 *
 * A pure past/present/future reducer over whole `WorkflowDefinitionV2`
 * snapshots. Snapshots rather than inverse operations because `graphEdits.ts`
 * (P3.1) already produces immutable `def → def'` values — the cheapest correct
 * history is to keep them, and it means a new edit operation gets undo for
 * free instead of needing an inverse written for it.
 *
 * Dirty state lives here too, compared against the **last saved** snapshot
 * rather than a boolean flag: undoing back to the saved shape correctly reads
 * as clean, which a flag can't express. This is the state the navigation guard
 * (audit F38) and the 30s draft autosave both read.
 *
 * Module-level reducer, driven by `useReducer` — the codebase's state idiom is
 * React Context + reducers, not an external store.
 */
import { useCallback, useMemo, useReducer } from 'react';

import type { WorkflowDefinitionV2 } from './types';

/** Cap on remembered snapshots. A workflow graph is tens of nodes, so this is
 *  kilobytes; the cap exists to bound a long editing session, not to save
 *  memory in any interesting way. */
export const HISTORY_LIMIT = 100;

export interface GraphHistory {
  past: WorkflowDefinitionV2[];
  present: WorkflowDefinitionV2;
  future: WorkflowDefinitionV2[];
  /** Serialized snapshot of the last saved definition — the dirty baseline. */
  saved: string;
}

/** Canonical serialization used for equality. Property order is stable
 *  because every producer builds nodes/edges the same way (`graphEdits`,
 *  the Rust migration), so `JSON.stringify` is a sound identity here. */
export function serializeDefinition(def: WorkflowDefinitionV2): string {
  return JSON.stringify(def);
}

export function initHistory(def: WorkflowDefinitionV2): GraphHistory {
  return { past: [], present: def, future: [], saved: serializeDefinition(def) };
}

export type HistoryAction =
  | { type: 'commit'; def: WorkflowDefinitionV2 }
  | { type: 'undo' }
  | { type: 'redo' }
  /** The definition was persisted — the present becomes the clean baseline. */
  | { type: 'saved' }
  /** Replace everything (loading a different workflow, restoring a draft). */
  | { type: 'reset'; def: WorkflowDefinitionV2; dirty?: boolean };

export function graphHistoryReducer(
  state: GraphHistory,
  action: HistoryAction,
): GraphHistory {
  switch (action.type) {
    case 'commit': {
      // A commit that changes nothing is not an edit. The canvas re-derives
      // and re-commits on plenty of no-op gestures (a click-drag that lands a
      // node back where it started); letting those pile up would fill the undo
      // stack with steps that appear to do nothing and would mark a pristine
      // workflow dirty.
      if (serializeDefinition(action.def) === serializeDefinition(state.present)) {
        return state;
      }
      const past = [...state.past, state.present];
      return {
        ...state,
        past: past.length > HISTORY_LIMIT ? past.slice(past.length - HISTORY_LIMIT) : past,
        present: action.def,
        // Any new edit invalidates the redo branch — standard linear history.
        future: [],
      };
    }
    case 'undo': {
      if (state.past.length === 0) return state;
      return {
        ...state,
        past: state.past.slice(0, -1),
        present: state.past[state.past.length - 1],
        future: [state.present, ...state.future],
      };
    }
    case 'redo': {
      if (state.future.length === 0) return state;
      return {
        ...state,
        past: [...state.past, state.present],
        present: state.future[0],
        future: state.future.slice(1),
      };
    }
    case 'saved':
      return { ...state, saved: serializeDefinition(state.present) };
    case 'reset':
      return {
        past: [],
        present: action.def,
        future: [],
        // A restored draft is dirty by definition: it is unsaved work. A plain
        // load is clean.
        saved: action.dirty ? '' : serializeDefinition(action.def),
      };
    default:
      return state;
  }
}

export function canUndo(h: GraphHistory): boolean {
  return h.past.length > 0;
}

export function canRedo(h: GraphHistory): boolean {
  return h.future.length > 0;
}

export function isDirty(h: GraphHistory): boolean {
  return serializeDefinition(h.present) !== h.saved;
}

export interface GraphHistoryController {
  definition: WorkflowDefinitionV2;
  /** Record an edit (a `graphEdits` result, a config-panel change). */
  commit: (def: WorkflowDefinitionV2) => void;
  undo: () => void;
  redo: () => void;
  canUndo: boolean;
  canRedo: boolean;
  /** The graph differs from the last saved snapshot. */
  dirty: boolean;
  /** Called after a successful save: the present becomes clean. */
  markSaved: () => void;
  /** Load a different definition, optionally as unsaved work (draft restore). */
  reset: (def: WorkflowDefinitionV2, opts?: { dirty?: boolean }) => void;
}

export function useGraphHistory(initial: WorkflowDefinitionV2): GraphHistoryController {
  const [history, dispatch] = useReducer(graphHistoryReducer, initial, initHistory);

  const commit = useCallback(
    (def: WorkflowDefinitionV2) => dispatch({ type: 'commit', def }),
    [],
  );
  const undo = useCallback(() => dispatch({ type: 'undo' }), []);
  const redo = useCallback(() => dispatch({ type: 'redo' }), []);
  const markSaved = useCallback(() => dispatch({ type: 'saved' }), []);
  const reset = useCallback(
    (def: WorkflowDefinitionV2, opts?: { dirty?: boolean }) =>
      dispatch({ type: 'reset', def, dirty: opts?.dirty }),
    [],
  );

  return useMemo(
    () => ({
      definition: history.present,
      commit,
      undo,
      redo,
      canUndo: canUndo(history),
      canRedo: canRedo(history),
      dirty: isDirty(history),
      markSaved,
      reset,
    }),
    [history, commit, undo, redo, markSaved, reset],
  );
}

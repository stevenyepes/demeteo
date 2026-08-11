import { useCallback, useEffect, useMemo, useRef } from 'react';

import type { NavigationMode } from '../../context/NavigationContext';
import {
  defaultInspectorSelection,
  inspectorTarget,
  type InspectorTarget,
} from '../../lib/inspectorTarget';
import type { AppView, StepExecution } from '../../types';

type DetailView = Extract<AppView, { kind: 'detail' }>;

/**
 * What an empty selection means, which "is it empty" cannot answer and the
 * seeding policy has to.
 *
 * `selectedStepId` being both optional *and* nullable is load-bearing.
 * `selectStep(null)` writes an explicit `null`; every route that rebuilds the
 * detail view from `featureId` alone — the gate overlay's close and decide
 * handlers, an awaiting-gate node activated on the canvas, the `gate_required`
 * listener — omits the field. So a run the user dismissed the inspector in and
 * a run they have just come back to from a gate are distinguishable, and only
 * here.
 *
 * Normalising the absent case to `null` at any point upstream collapses the two
 * and forces a choice between an inspector whose dismiss control does nothing
 * and one that says "no step selected" for the rest of the run —
 * UI_REDESIGN_PLAN §7 accepted a pane that never collapses on the understanding
 * that it would never be the second.
 */
type SelectionIntent = 'selected' | 'cleared' | 'unset';

function selectionIntent(selectedStepId: string | null | undefined): SelectionIntent {
  if (selectedStepId === undefined) return 'unset';
  if (selectedStepId === null) return 'cleared';
  return 'selected';
}

export interface StepSelection {
  /** What the inspector shows, empty reasons included. */
  target: InspectorTarget;
  /** Node id for the canvas overlay — the selection resolved to a graph node,
   *  so a selection made by execution id still highlights the right node. */
  selectedNodeId: string | null;
  /** Execution id for the timeline row, resolved the same way and for the
   *  mirror-image reason. */
  selectedExecutionId: string | null;
  /** `null` empties the inspector. Stable for the life of the view. */
  selectStep: (stepId: string | null) => void;
  /** Select, or clear when the id already resolves to what is shown. Stable. */
  toggleStep: (stepId: string) => void;
}

/**
 * The run's step selection, held on the `detail` view rather than in this
 * component tree (UI_REDESIGN_PLAN §3.5).
 *
 * **Every selection replaces.** A step click is a change of what the inspector
 * reads, not a destination: pushing one would put a history entry behind every
 * row a user glances at and leave Back walking a run step by step instead of
 * returning to where they came from.
 *
 * The writers never change identity, because they reach `navigate` through a ref
 * rather than closing over the view. They are handed to memoized `StepCard`s, so
 * a writer that changed when the selection changed would re-render every row in
 * the run on every click — the cost the memo exists to avoid, re-introduced by
 * the feature that needed the memo most.
 */
export function useStepSelection(input: {
  view: DetailView;
  steps: StepExecution[];
  navigate: (view: AppView, mode?: NavigationMode) => void;
}): StepSelection {
  const { view, steps, navigate } = input;

  const target = useMemo(
    () => inspectorTarget(steps, view.selectedStepId ?? null),
    [steps, view.selectedStepId],
  );

  const viewRef = useRef(view);
  viewRef.current = view;
  const targetRef = useRef(target);
  targetRef.current = target;

  const selectStep = useCallback(
    (stepId: string | null) => {
      navigate({ ...viewRef.current, selectedStepId: stepId }, 'replace');
    },
    [navigate],
  );

  const toggleStep = useCallback(
    (stepId: string) => {
      const shown = targetRef.current;
      const isShown =
        shown.kind === 'step' && (shown.step.id === stepId || shown.step.step_id === stepId);
      selectStep(isShown ? null : stepId);
    },
    [selectStep],
  );

  /**
   * A view that names no step opens on the one that deserves the reader's
   * attention. A view whose selection the user cleared stays cleared, which is
   * the whole of the inspector's dismiss control.
   */
  useEffect(() => {
    if (selectionIntent(view.selectedStepId) !== 'unset') return;
    if (steps.length === 0) return;
    const initial = defaultInspectorSelection(steps);
    if (initial) selectStep(initial);
  }, [view.selectedStepId, steps, selectStep]);

  return {
    target,
    selectedNodeId: target.kind === 'step' ? target.step.step_id : null,
    selectedExecutionId: target.kind === 'step' ? target.step.id : null,
    selectStep,
    toggleStep,
  };
}

import { useCallback, useEffect, useMemo, useState } from 'react';
import type { AppView, StepExecution } from '../../types';
import { usePersistedPref } from '../../hooks/usePersistedPref';
import { useRunEvents } from '../../hooks/useRunEvents';
import { getFeatureWorkflowGraph } from '../../lib/featureDetail';
import type { RunEventAssignments } from '../../lib/runEventAssignments';
import { runViewModePref } from '../../lib/uiPrefs';
import { replayCone, descendantIds } from '../canvas/graphOps';
import type { WorkflowDefinitionV2 } from '../canvas/types';
import { humanizeStepId } from './stepIdentity';
import type { ReplayTarget } from './useRerunActions';

/**
 * The run-mode graph: the pinned version's schema-v2 definition, the live
 * status overlay both run surfaces read, and what a click on a node means.
 *
 * The selection itself is not here. It lives on the `detail` view in navigation
 * state so it survives back/forward and a deep link, and so the timeline and
 * the canvas share one (UI_REDESIGN_PLAN §3.5) — this hook is handed the
 * resolved node id and a writer, and never holds either.
 */
export function useRunGraph(input: {
  featureId: string;
  featureTitle: string;
  steps: StepExecution[];
  navigate: (view: AppView) => void;
  startReplay: (target: ReplayTarget, previewNodes: Set<string> | null) => void;
  /** The node id the current selection resolves to, for the canvas overlay. */
  selectedNodeId: string | null;
  /** Select the node, or clear the selection when it is already the one shown. */
  toggleNode: (nodeId: string) => void;
  detachedAssignments: RunEventAssignments | null;
}) {
  const {
    featureId,
    featureTitle,
    steps,
    navigate,
    startReplay,
    toggleNode,
    detachedAssignments,
  } = input;
  /** Graph first: Phase 2 gives both surfaces the same inspector, which was the
   *  parity the timeline default was waiting on (UI_REDESIGN_PLAN §7,
   *  PRD §6.1). `canShowGraph` still gates it, so a run with no definition
   *  opens on the timeline — a fallback, not a default. The gate sits
   *  downstream of this value, so a stored `'graph'` cannot defeat it, and a
   *  run that falls back stores nothing: neither the toggle nor `g`/`t` is
   *  offered there. */
  const [viewMode, setViewMode] = usePersistedPref(runViewModePref, 'graph');
  // The pinned version's schema-v2 graph (P1.15 + `feature_workflow_graph`),
  // migrated backend-side. Null until loaded / when the feature has none.
  const [graphDef, setGraphDef] = useState<WorkflowDefinitionV2 | null>(null);
  // Single run-event consumer both run-mode surfaces share: the canvas overlay
  // reads node status from here, derived from the same `steps` snapshot the
  // timeline renders (plus failure classes from the `run_events` stream, P1.13).
  const {
    statusByNode: localStatusByNode,
    events: localRunEvents,
    assignments: localAssignments,
  } = useRunEvents(featureId, steps);
  const selectedAssignments = detachedAssignments ?? localAssignments;
  const { runStatusByNode, runAssignments } = useMemo(() => {
    const statuses = Object.fromEntries(
      Object.entries(localStatusByNode).map(([nodeId, status]) => {
        const { agentKind: _agentKind, effort: _effort, ...baseStatus } = status;
        const assignment = status.stepExecutionId
          ? selectedAssignments[status.stepExecutionId]
          : undefined;
        return [
          nodeId,
          assignment
            ? { ...baseStatus, agentKind: assignment.agentKind, effort: assignment.effort }
            : baseStatus,
        ];
      }),
    );
    const assignments: RunEventAssignments = {};
    for (const status of Object.values(statuses)) {
      if (!status.stepExecutionId) continue;
      const assignment = selectedAssignments[status.stepExecutionId];
      if (assignment) assignments[status.stepExecutionId] = assignment;
    }
    return { runStatusByNode: statuses, runAssignments: assignments };
  }, [localStatusByNode, selectedAssignments]);

  // Load the pinned version's v2 graph once per feature id — it's immutable
  // for the run's lifetime (runs pin their version forever, PRD §2), so a
  // single fetch suffices; live status rides on `runStatusByNode`, not this.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const def = await getFeatureWorkflowGraph(featureId);
        if (!cancelled) setGraphDef(def && def.nodes.length > 0 ? def : null);
      } catch (err) {
        // Soft failure: no graph → the toggle simply doesn't appear and the
        // timeline carries on. Legacy features with no workflow land here.
        if (!cancelled) setGraphDef(null);
        console.warn('feature_workflow_graph failed:', err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [featureId]);

  // The graph is offered only once there's a run to overlay and a definition
  // to draw. An *awaiting* gate node opens the existing full-screen `GateView`
  // (the actionable HITL path); every other node selects into the inspector.
  const canShowGraph = graphDef !== null && steps.length > 0;
  const graphMode = canShowGraph && viewMode === 'graph';

  const onNodeActivate = useCallback(
    (nodeId: string) => {
      const run = runStatusByNode[nodeId];
      if (run?.status === 'awaiting_gate' && run.stepExecutionId) {
        navigate({
          kind: 'detail',
          featureId,
          featureTitle,
          gateStepExecutionId: run.stepExecutionId,
        });
        return;
      }
      toggleNode(nodeId);
    },
    [runStatusByNode, navigate, featureId, featureTitle, toggleNode],
  );

  /**
   * Replay initiated from the inspector (P2.4): ring the whole downstream cone
   * on the canvas while the confirm modal is open, and count downstream by the
   * *graph*, which is accurate for a DAG where index arithmetic is not.
   *
   * A run with no definition has no cone to ring and falls back to the
   * timeline's own count — the steps after this one in index order — so the
   * confirm modal states a number either way rather than claiming zero.
   */
  const startReplayFromInspector = useCallback(
    (step: StepExecution) => {
      const node = graphDef?.nodes.find((n) => n.id === step.step_id) ?? null;
      const cone = graphDef ? replayCone(graphDef, step.step_id) : null;
      const downstreamCount = graphDef
        ? descendantIds(graphDef, step.step_id).size
        : Math.max(steps.length - 1 - step.step_index, 0);
      startReplay(
        { id: step.id, name: node?.title ?? humanizeStepId(step.step_id), downstreamCount },
        cone,
      );
    },
    [graphDef, steps, startReplay],
  );

  return {
    viewMode,
    setViewMode,
    graphDef,
    canShowGraph,
    graphMode,
    runStatusByNode,
    runAssignments,
    localRunEvents,
    onNodeActivate,
    startReplayFromInspector,
  };
}

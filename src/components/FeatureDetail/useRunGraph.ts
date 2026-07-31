import { useCallback, useEffect, useMemo, useState } from 'react';
import type { AppView, StepExecution } from '../../types';
import { useRunEvents } from '../../hooks/useRunEvents';
import { getFeatureWorkflowGraph } from '../../lib/featureDetail';
import { findActivePredecessor } from '../../lib/features';
import { replayCone, descendantIds } from '../canvas/graphOps';
import type { WorkflowDefinitionV2 } from '../canvas/types';
import { type RunViewMode } from '../RunViewToggle';
import type { ReplayTarget } from './useRerunActions';

/**
 * The run-mode graph: the pinned version's schema-v2 definition, the live
 * status overlay both run surfaces read, and the drill-down panel's selection.
 */
export function useRunGraph(input: {
  featureId: string;
  featureTitle: string;
  steps: StepExecution[];
  navigate: (view: AppView) => void;
  startReplay: (target: ReplayTarget, previewNodes: Set<string> | null) => void;
  openArtifact: (path: string, stepTitle: string | null) => void;
}) {
  const { featureId, featureTitle, steps, navigate, startReplay, openArtifact } = input;
  // Run-mode visualization toggle (P2.2). The list timeline stays the default
  // — it's better for skimming long linear runs and preserves muscle memory;
  // the graph is opt-in until Phase-2 parity is signed off (PRD §6.1).
  const [viewMode, setViewMode] = useState<RunViewMode>('timeline');
  // The pinned version's schema-v2 graph (P1.15 + `feature_workflow_graph`),
  // migrated backend-side. Null until loaded / when the feature has none.
  const [graphDef, setGraphDef] = useState<WorkflowDefinitionV2 | null>(null);
  // The node whose drill-down panel is open on the run-mode canvas (P2.3).
  // Null = no panel. A gate node routes to the full-screen `GateView` instead.
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  // Single run-event consumer both run-mode surfaces share: the canvas overlay
  // reads node status from here, derived from the same `steps` snapshot the
  // timeline renders (plus failure classes from the `run_events` stream, P1.13).
  const { statusByNode: runStatusByNode, events: localRunEvents } = useRunEvents(featureId, steps);

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
        // timeline (the default) carries on. Legacy features with no workflow
        // land here.
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
  // (the actionable HITL path); every other node opens the drill-down panel.
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
      setSelectedNodeId((prev) => (prev === nodeId ? null : nodeId));
    },
    [runStatusByNode, navigate, featureId, featureTitle],
  );

  // The node + its backing rows for the open drill-down panel. Prefer the exact
  // execution the overlay tracks (`stepExecutionId`); fall back to the newest
  // row for the node id when the run hasn't been observed live.
  const selectedNode = useMemo(
    () => graphDef?.nodes.find((n) => n.id === selectedNodeId) ?? null,
    [graphDef, selectedNodeId],
  );
  const selectedRun = selectedNodeId ? runStatusByNode[selectedNodeId] ?? null : null;
  const selectedStep = useMemo(() => {
    if (!selectedNodeId) return null;
    if (selectedRun?.stepExecutionId) {
      const exact = steps.find((s) => s.id === selectedRun.stepExecutionId);
      if (exact) return exact;
    }
    const matches = steps.filter((s) => s.step_id === selectedNodeId);
    return matches.length
      ? matches.reduce((a, b) => (b.updated_at >= a.updated_at ? b : a))
      : null;
  }, [selectedNodeId, selectedRun, steps]);

  // Replay initiated from the panel (P2.4): ring the whole downstream cone on
  // the canvas while the confirm modal is open, and count downstream by the
  // *graph* (accurate for DAGs) rather than the timeline's index arithmetic.
  const startReplayFromPanel = useCallback(() => {
    if (!selectedNode || !selectedStep) return;
    const cone = graphDef ? replayCone(graphDef, selectedNode.id) : null;
    const downstreamCount = graphDef ? descendantIds(graphDef, selectedNode.id).size : 0;
    startReplay({ id: selectedStep.id, name: selectedNode.title, downstreamCount }, cone);
  }, [selectedNode, selectedStep, graphDef, startReplay]);

  // An artifact clicked in the graph drill-down opens the same `ArtifactModal`
  // the timeline uses — one artifact surface, not two that drift apart. The
  // step title comes from the panel's own node so the modal captions it.
  const openArtifactFromPanel = useCallback(
    (artifactPath: string) => {
      openArtifact(artifactPath, selectedNodeId);
    },
    [openArtifact, selectedNodeId],
  );

  // The active ancestor (if any) blocking a retry/gate decision on the selected
  // node — the same guard the timeline's Retry button uses, surfaced as the
  // panel's disabled-button explanation (PRD §6.4).
  const selectedBlockedBy = useMemo(() => {
    if (!selectedStep) return null;
    const pred = findActivePredecessor(steps, selectedStep);
    return pred ? { step_id: pred.step_id, status: pred.status } : null;
  }, [selectedStep, steps]);

  return {
    viewMode,
    setViewMode,
    graphDef,
    canShowGraph,
    graphMode,
    runStatusByNode,
    localRunEvents,
    selectedNodeId,
    setSelectedNodeId,
    selectedNode,
    selectedRun,
    selectedStep,
    selectedBlockedBy,
    onNodeActivate,
    startReplayFromPanel,
    openArtifactFromPanel,
  };
}

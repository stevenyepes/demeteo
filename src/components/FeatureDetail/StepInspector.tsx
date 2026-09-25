import { useCallback } from 'react';

import { NodePanel } from '../canvas/NodePanel';
import type { NodeRunStatus, WorkflowDefinitionV2 } from '../canvas/types';
import type { InspectorTarget } from '../../lib/inspectorTarget';
import type { RunEventAssignments } from '../../lib/runEventAssignments';
import type { HarnessBaseline, StepExecution } from '../../types';
import { InspectorEmpty } from './InspectorEmpty';
import { inspectorNodeConfig, inspectorRunStatus } from './stepIdentity';
import type { AgentStreamStore } from './useAgentStream';
import type { HarnessOverrides } from './useHarnessOverrides';
import type { StepAssignment } from './useStepAssignment';

interface StepInspectorProps {
  featureId: string;
  target: InspectorTarget;
  graphDef: WorkflowDefinitionV2 | null;
  statusByNode: Record<string, NodeRunStatus>;
  /** Spawn evidence keyed by `step_execution_id`, exactly as `StepTimeline`
   *  takes it — the selection may name an older attempt, so the panel is served
   *  per execution and not per node. Required rather than optional: the read-only
   *  Assignment renders this and nothing else, so a caller that forgets it would
   *  silently turn every running node's assignment into "no launch evidence". */
  assignments: RunEventAssignments;
  streamStore: AgentStreamStore;
  /** What the gates said at the base commit, for the Output tab's reading of an
   *  environment failure. Optional here because `NodePanel` is served to the
   *  canvas as well, which has no baseline in hand. */
  harnessBaseline?: HarnessBaseline | null;
  /** The harness/model/effort the Actions tab's Assignment control reads.
   *  Optional for the same reason. */
  overrides?: HarnessOverrides;
  /** Writes that trio onto the selected node. Both are the run view's, held
   *  against `target` — a picker with nowhere to submit renders no control at
   *  all, so passing one without the other silently removes the surface. */
  assignment?: StepAssignment;
  /** Empties the pane rather than hiding it — there is no closed state. */
  onDeselect: () => void;
  onOpenEditorForPath: (filePath: string) => void;
  onOpenArtifact: (artifactPath: string, stepTitle: string) => void;
  onRetry: (stepExecutionId: string) => void;
  onReplay: (step: StepExecution) => void;
  onStop: () => void;
  onDecideGate: (stepExecutionId: string) => void;
  className?: string;
}

/**
 * The run's one step inspector, driven by `inspectorTarget` and served to both
 * run surfaces (UI_REDESIGN_PLAN §3.1).
 *
 * **The stream store is passed through, never subscribed to on the way.** Two
 * separate ceilings meet here and both are load-bearing.
 *
 * It may not move *up* into `FeatureDetail`: `useStreamText` wakes its consumer
 * once per animation frame while an agent streams, and `FeatureDetailView`
 * renders the timeline, so a subscription there would re-render every memoized
 * `StepCard` at frame rate and re-open the fan-out §4.2 exists to close —
 * invisibly, because the cards would still be memoized and the timeline's own
 * render-count test mounts the timeline alone.
 *
 * It may not stop *here* either, which the first argument does not imply. This
 * component is mounted for the whole run in both view modes and opens on the
 * step `defaultInspectorSelection` picks — the running one — so subscribing
 * here wakes `NodePanel` and whichever tab it is showing, at frame rate, with
 * no user action at all. `LiveTab` is the only consumer and is mounted only
 * when selected; the subscription belongs there.
 */
export function StepInspector({
  featureId,
  target,
  graphDef,
  statusByNode,
  assignments,
  streamStore,
  harnessBaseline,
  overrides,
  assignment,
  onDeselect,
  onOpenEditorForPath,
  onOpenArtifact,
  onRetry,
  onReplay,
  onStop,
  onDecideGate,
  className = '',
}: StepInspectorProps) {
  const nodeId = target.kind === 'step' ? target.step.step_id : '';
  const openArtifact = useCallback(
    (artifactPath: string) => onOpenArtifact(artifactPath, nodeId),
    [onOpenArtifact, nodeId],
  );

  if (target.kind !== 'step') return <InspectorEmpty reason={target.reason} className={className} />;

  const { blockedBy } = target;
  const isStreaming = target.step.status === 'running' || target.step.status === 'verifying';

  return (
    <NodePanel
      className={className}
      featureId={featureId}
      node={inspectorNodeConfig(graphDef, target.step)}
      run={inspectorRunStatus(
        target.step,
        statusByNode[target.step.step_id]?.errorClass,
        assignments[target.step.id],
      )}
      step={target.step}
      onClose={onDeselect}
      onOpenEditorForPath={onOpenEditorForPath}
      onOpenArtifact={openArtifact}
      harnessBaseline={harnessBaseline}
      overrides={overrides}
      assignment={assignment}
      streamStore={streamStore}
      isStreaming={isStreaming}
      blockedBy={blockedBy}
      // Every action is offered and `ActionsTab` decides which of them the
      // node's status allows. Repeating that judgement here would be the same
      // rule in two places, and the copy further from the buttons is the one
      // that goes stale.
      onRetry={() => onRetry(target.step.id)}
      onReplay={() => onReplay(target.step)}
      onStop={onStop}
      onDecideGate={() => onDecideGate(target.step.id)}
    />
  );
}

export default StepInspector;

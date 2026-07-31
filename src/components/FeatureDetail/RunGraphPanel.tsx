import type { RunEvent, StepExecution } from '../../types';
import { WorkflowCanvas } from '../canvas/WorkflowCanvas';
import { NodePanel } from '../canvas/NodePanel';
import type { NodeConfigV2, NodeRunStatus, WorkflowDefinitionV2 } from '../canvas/types';

interface RunGraphPanelProps {
  featureId: string;
  definition: WorkflowDefinitionV2;
  statusByNode: Record<string, NodeRunStatus>;
  highlightedNodeIds: Set<string> | null;
  graphBoxPx: number;
  selectedNodeId: string | null;
  selectedNode: NodeConfigV2 | null;
  selectedRun: NodeRunStatus | null;
  selectedStep: StepExecution | null;
  selectedBlockedBy: { step_id: string; status: string } | null;
  runEvents: RunEvent[];
  liveStream: string | undefined;
  onNodeActivate: (nodeId: string) => void;
  onCloseNode: () => void;
  onOpenEditorForPath: (filePath: string) => void;
  onOpenArtifact: (artifactPath: string) => void;
  onRetry: (() => void) | undefined;
  onReplay: (() => void) | undefined;
  onStop: (() => void) | undefined;
  onDecideGate: (() => void) | undefined;
}

/**
 * Height comes from the plan, not the column's leftovers: a `RIGHT` chain is
 * one ~64px row, and flexing to fill left it floating in ~900px of empty
 * canvas. The plan was made for `graphBox` — the space this element actually
 * has — so the computed height can't push it past the fold either. `style=`
 * carries a measurement, never a token; `min-h-[28rem]` keeps the floor in
 * the class list, mirroring `MIN_GRAPH_BOX_PX`.
 */
export function RunGraphPanel({
  featureId,
  definition,
  statusByNode,
  highlightedNodeIds,
  graphBoxPx,
  selectedNodeId,
  selectedNode,
  selectedRun,
  selectedStep,
  selectedBlockedBy,
  runEvents,
  liveStream,
  onNodeActivate,
  onCloseNode,
  onOpenEditorForPath,
  onOpenArtifact,
  onRetry,
  onReplay,
  onStop,
  onDecideGate,
}: RunGraphPanelProps) {
  return (
    <div
      className="flex min-h-[28rem] w-full shrink-0 overflow-hidden rounded-xl border border-white/5 bg-[#050608]/40"
      style={{ height: graphBoxPx }}
    >
      <div className="min-w-0 flex-1">
        <WorkflowCanvas
          definition={definition}
          statusByNode={statusByNode}
          onNodeActivate={onNodeActivate}
          selectedNodeId={selectedNodeId}
          highlightedNodeIds={highlightedNodeIds}
        />
      </div>
      {selectedNode && (
        <NodePanel
          featureId={featureId}
          node={selectedNode}
          run={selectedRun}
          step={selectedStep}
          onClose={onCloseNode}
          onOpenEditorForPath={onOpenEditorForPath}
          onOpenArtifact={onOpenArtifact}
          runEvents={runEvents}
          liveStream={liveStream}
          isStreaming={
            selectedStep?.status === 'running' || selectedStep?.status === 'verifying'
          }
          blockedBy={selectedBlockedBy}
          onRetry={onRetry}
          onReplay={onReplay}
          onStop={onStop}
          onDecideGate={onDecideGate}
        />
      )}
    </div>
  );
}

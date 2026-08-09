import { WorkflowCanvas } from '../canvas/WorkflowCanvas';
import type { NodeRunStatus, WorkflowDefinitionV2 } from '../canvas/types';

interface RunGraphPanelProps {
  definition: WorkflowDefinitionV2;
  statusByNode: Record<string, NodeRunStatus>;
  highlightedNodeIds: Set<string> | null;
  selectedNodeId: string | null;
  onNodeActivate: (nodeId: string) => void;
}

/**
 * The run-mode canvas, and nothing else.
 *
 * `min-h-[28rem]` is the CSS half of `MIN_GRAPH_BOX_PX` (`layoutDirection.ts`),
 * which floors the computed height at the same 448px so the container elk is
 * planned against is the space the box will really have. One decision spelled
 * in two languages: move either and the plan starts describing a box that does
 * not exist.
 *
 * Every other pixel of height is the host's to state, because the box shares a
 * `SplitPane` row with the inspector and only the row knows what is left.
 */
export function RunGraphPanel({
  definition,
  statusByNode,
  highlightedNodeIds,
  selectedNodeId,
  onNodeActivate,
}: RunGraphPanelProps) {
  return (
    <div className="h-full min-h-[28rem] w-full overflow-hidden rounded-xl border border-white/5 bg-[#050608]/40">
      <WorkflowCanvas
        definition={definition}
        statusByNode={statusByNode}
        onNodeActivate={onNodeActivate}
        selectedNodeId={selectedNodeId}
        highlightedNodeIds={highlightedNodeIds}
      />
    </div>
  );
}

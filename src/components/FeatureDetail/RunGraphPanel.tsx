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
 * **It states no height, not even a minimum.** It used to carry a
 * `min-h-[28rem]` mirroring `MIN_GRAPH_BOX_PX`, which was right while the host
 * always handed it a computed box; side by side the host now hands it a share
 * of the window instead, and a floor taller than that share overflows a row
 * whose parent clips — a short, wide window would lose the bottom of the graph
 * with no scrollbar to reach it. The floor still exists where it can be honoured:
 * `graphBoxHeight` clamps the stacked layout's stated height to the same 448px.
 *
 * The card is `glass-panel` because this is one of the run's three tracks and
 * they are peers; the canvas inside it is a `panel-field`, which is the same
 * well the terminal uses and was previously spelled here as a raw `#050608`
 * — a hex of a token that already existed (AGENTS.md §4).
 */
export function RunGraphPanel({
  definition,
  statusByNode,
  highlightedNodeIds,
  selectedNodeId,
  onNodeActivate,
}: RunGraphPanelProps) {
  return (
    <div className="glass-panel h-full min-h-0 w-full overflow-hidden">
      <div className="panel-field h-full w-full">
        <WorkflowCanvas
          definition={definition}
          statusByNode={statusByNode}
          onNodeActivate={onNodeActivate}
          selectedNodeId={selectedNodeId}
          highlightedNodeIds={highlightedNodeIds}
        />
      </div>
    </div>
  );
}

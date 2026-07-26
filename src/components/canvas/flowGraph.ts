/**
 * Pure transform from a schema-v2 workflow definition to the `nodes`/`edges`
 * arrays React Flow renders. Kept side-effect-free and separate from the
 * `WorkflowCanvas` component so it can be exhaustively fixture-tested without
 * mounting React Flow (which needs a real layout engine jsdom lacks).
 *
 * Run-mode status overlay (task P2.2) rides on the optional `statusByNode`
 * map — design mode (P2.1) simply passes nothing.
 */
import type { Edge, Node } from '@xyflow/react';
import { isEssenceEmpty, nodeEssence, type NodeEssence } from './nodeSummary';
import type { NodeRunStatus, WorkflowDefinitionV2 } from './types';

/** Data carried on each React Flow node into the `WorkflowNode` card. */
export interface WorkflowNodeData extends Record<string, unknown> {
  nodeType: string;
  title: string;
  /** Live run state (P2.2); undefined in design mode. */
  run?: NodeRunStatus;
  /** In the replay cone about to re-run (P2.4) — draws a "will re-run" ring. */
  highlighted?: boolean;
  /** Config-essence badges for design mode (P3.2); undefined in run mode,
   *  where the card's second row belongs to cost/duration instead. */
  essence?: NodeEssence;
}

export type WorkflowFlowNode = Node<WorkflowNodeData, 'workflow'>;

/** Fallback layout stride when a node carries no persisted position — matches
 *  the migration's `VERTICAL_SPACING` so an unpositioned graph still reads as
 *  the same column before elk auto-layout runs. */
const FALLBACK_STRIDE = 160;

export interface ToFlowGraphOptions {
  /** node id → live run state, for run-mode overlay (P2.2). */
  statusByNode?: Record<string, NodeRunStatus>;
  /** node ids in the replay cone to highlight before confirming (P2.4). */
  highlightedNodeIds?: Set<string> | null;
  /** Design mode (P3.2): put each node's config essence on its card, so the
   *  graph is scannable without opening the config panel (PRD §6.3). */
  showEssence?: boolean;
}

export function toFlowGraph(
  def: WorkflowDefinitionV2,
  opts: ToFlowGraphOptions = {},
): { nodes: WorkflowFlowNode[]; edges: Edge[] } {
  const nodes: WorkflowFlowNode[] = def.nodes.map((n, i) => {
    const essence = opts.showEssence ? nodeEssence(n) : null;
    return {
      id: n.id,
      type: 'workflow' as const,
      position: n.position ?? { x: 0, y: i * FALLBACK_STRIDE },
      data: {
        nodeType: n.type,
        title: n.title,
        run: opts.statusByNode?.[n.id],
        highlighted: opts.highlightedNodeIds?.has(n.id) ?? false,
        essence: essence && !isEssenceEmpty(essence) ? essence : undefined,
      },
    };
  });

  const edges: Edge[] = def.edges.map((e) => ({
    id: `${e.from}->${e.to}`,
    source: e.from,
    target: e.to,
    // A conditional edge (a `when` guard) reads as a labeled branch.
    ...(e.when ? { label: 'when', data: { when: e.when } } : {}),
  }));

  return { nodes, edges };
}

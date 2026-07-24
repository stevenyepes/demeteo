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
import type { WorkflowDefinitionV2 } from './types';

/** Data carried on each React Flow node into the `WorkflowNode` card. */
export interface WorkflowNodeData extends Record<string, unknown> {
  nodeType: string;
  title: string;
  /** Live run status (P2.2); undefined in design mode. */
  status?: string;
}

export type WorkflowFlowNode = Node<WorkflowNodeData, 'workflow'>;

/** Fallback layout stride when a node carries no persisted position — matches
 *  the migration's `VERTICAL_SPACING` so an unpositioned graph still reads as
 *  the same column before elk auto-layout runs. */
const FALLBACK_STRIDE = 160;

export interface ToFlowGraphOptions {
  /** node id → run status, for run-mode overlay (P2.2). */
  statusByNode?: Record<string, string>;
}

export function toFlowGraph(
  def: WorkflowDefinitionV2,
  opts: ToFlowGraphOptions = {},
): { nodes: WorkflowFlowNode[]; edges: Edge[] } {
  const nodes: WorkflowFlowNode[] = def.nodes.map((n, i) => ({
    id: n.id,
    type: 'workflow',
    position: n.position ?? { x: 0, y: i * FALLBACK_STRIDE },
    data: {
      nodeType: n.type,
      title: n.title,
      status: opts.statusByNode?.[n.id],
    },
  }));

  const edges: Edge[] = def.edges.map((e) => ({
    id: `${e.from}->${e.to}`,
    source: e.from,
    target: e.to,
    // A conditional edge (a `when` guard) reads as a labeled branch.
    ...(e.when ? { label: 'when', data: { when: e.when } } : {}),
  }));

  return { nodes, edges };
}

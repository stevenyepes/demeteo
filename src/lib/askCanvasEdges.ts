import type { AskCanvas, EdgeKind } from '../types';

export interface CanvasNeighbor {
  nodeId: string;
  title: string;
  kind: EdgeKind;
}

/**
 * Resolves a node's edges to neighbor titles for the inspector's Edges
 * section — `CanvasEdge` only carries ids, so this is the one place that
 * looks them up against `canvas.nodes`.
 */
export function edgesForNode(
  canvas: AskCanvas,
  nodeId: string,
): { incoming: CanvasNeighbor[]; outgoing: CanvasNeighbor[] } {
  const titleById = new Map(canvas.nodes.map((node) => [node.id, node.title]));

  const incoming: CanvasNeighbor[] = [];
  const outgoing: CanvasNeighbor[] = [];

  // Two independent tests, not an if/else: a self-edge is both directions at
  // once, and the chained form silently reported it as incoming only.
  for (const edge of canvas.edges) {
    if (edge.to === nodeId) {
      const title = titleById.get(edge.from);
      if (title !== undefined) incoming.push({ nodeId: edge.from, title, kind: edge.kind });
    }
    if (edge.from === nodeId) {
      const title = titleById.get(edge.to);
      if (title !== undefined) outgoing.push({ nodeId: edge.to, title, kind: edge.kind });
    }
  }

  return { incoming, outgoing };
}

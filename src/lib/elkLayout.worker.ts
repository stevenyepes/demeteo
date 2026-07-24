/**
 * Layered auto-layout for the workflow canvas, run off the main thread.
 *
 * elkjs is CPU-heavy for the layered algorithm; doing it in a Web Worker keeps
 * the battery-sensitive Tauri webview responsive (PRD §6.1, "elk in a worker").
 * The `elk.bundled` build is fully self-contained, so it runs entirely inside
 * this worker with no nested worker of its own.
 *
 * Protocol: the main thread posts `{ nodes, edges }` with node dimensions; the
 * worker replies with `{ id, x, y }` positions. Direction is top-to-bottom to
 * match the migration's vertical column and the DAG's natural flow.
 */
import ELK, { type ElkNode } from 'elkjs/lib/elk.bundled.js';

export interface ElkLayoutRequest {
  nodes: { id: string; width: number; height: number }[];
  edges: { id: string; source: string; target: string }[];
}

export type ElkLayoutResult = { id: string; x: number; y: number }[];

const elk = new ELK();

const LAYOUT_OPTIONS: Record<string, string> = {
  'elk.algorithm': 'layered',
  'elk.direction': 'DOWN',
  'elk.layered.spacing.nodeNodeBetweenLayers': '64',
  'elk.spacing.nodeNode': '48',
  'elk.layered.nodePlacement.strategy': 'NETWORK_SIMPLEX',
};

self.onmessage = async (event: MessageEvent<ElkLayoutRequest>) => {
  const { nodes, edges } = event.data;

  const graph: ElkNode = {
    id: 'root',
    layoutOptions: LAYOUT_OPTIONS,
    children: nodes.map((n) => ({ id: n.id, width: n.width, height: n.height })),
    edges: edges.map((e) => ({ id: e.id, sources: [e.source], targets: [e.target] })),
  };

  try {
    const laid = await elk.layout(graph);
    const positions: ElkLayoutResult = (laid.children ?? []).map((c) => ({
      id: c.id,
      x: c.x ?? 0,
      y: c.y ?? 0,
    }));
    self.postMessage(positions);
  } catch {
    // A layout failure must never wedge the canvas — reply with no moves and
    // the caller keeps the persisted positions.
    self.postMessage([] as ElkLayoutResult);
  }
};

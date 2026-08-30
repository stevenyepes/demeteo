import type { AskCanvas, CanvasNode, EdgeKind } from '../types';

/**
 * Where every Ask Canvas node sits, and the curve between each pair.
 *
 * Unlike `ticketGraphLayout.ts`'s dependency ranks, placement here is
 * author-declared: a node's `(stage, lane)` names its cell directly, so the
 * grid's shape comes from `canvas.stages`/`canvas.lanes` rather than a graph
 * traversal.
 */

export const NODE_W = 202;
export const NODE_H = 72;

const CELL_W = 240;
const CELL_H = 120;
const COL_GAP = 40;
const ROW_GAP = 24;
const PAD = 16;
/** Diagonal offset applied per extra occupant of a shared cell, so two nodes
 *  declared at the same `(stage, lane)` don't render on top of each other. */
const STACK_STEP = 14;

export interface AskCanvasNodePosition {
  id: string;
  x: number;
  y: number;
}

/** One declared (stage, lane) cell's background geometry — present even when
 *  no node occupies it (Acceptance Criterion 2). */
export interface AskCanvasCell {
  stage: number;
  lane: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface AskCanvasEdgeLayout {
  from: string;
  to: string;
  kind: EdgeKind;
  path: string;
}

export interface AskCanvasLayout {
  nodes: AskCanvasNodePosition[];
  cells: AskCanvasCell[];
  edges: AskCanvasEdgeLayout[];
  width: number;
  height: number;
}

function cellOrigin(stage: number, lane: number): { x: number; y: number } {
  return {
    x: PAD + stage * (CELL_W + COL_GAP),
    y: PAD + lane * (CELL_H + ROW_GAP),
  };
}

export function layoutAskCanvas(
  canvas: Pick<AskCanvas, 'stages' | 'lanes' | 'nodes' | 'edges'>,
): AskCanvasLayout {
  const cells: AskCanvasCell[] = canvas.stages.flatMap((_stage, stage) =>
    canvas.lanes.map((_lane, lane) => {
      const origin = cellOrigin(stage, lane);
      return { stage, lane, x: origin.x, y: origin.y, width: CELL_W, height: CELL_H };
    }),
  );

  const byCell = new Map<string, CanvasNode[]>();
  for (const node of canvas.nodes) {
    const key = `${node.stage}:${node.lane}`;
    const occupants = byCell.get(key);
    if (occupants) {
      occupants.push(node);
    } else {
      byCell.set(key, [node]);
    }
  }

  const placed = new Map<string, AskCanvasNodePosition>();
  for (const occupants of byCell.values()) {
    const origin = cellOrigin(occupants[0].stage, occupants[0].lane);
    occupants.forEach((node, index) => {
      placed.set(node.id, {
        id: node.id,
        x: origin.x + (CELL_W - NODE_W) / 2 + index * STACK_STEP,
        y: origin.y + (CELL_H - NODE_H) / 2 + index * STACK_STEP,
      });
    });
  }

  const edges: AskCanvasEdgeLayout[] = canvas.edges.map((edge) => ({
    from: edge.from,
    to: edge.to,
    kind: edge.kind,
    path: curve(placed.get(edge.from), placed.get(edge.to), edge.kind),
  }));

  const width = PAD * 2 + canvas.stages.length * CELL_W + Math.max(0, canvas.stages.length - 1) * COL_GAP;
  const height = PAD * 2 + canvas.lanes.length * CELL_H + Math.max(0, canvas.lanes.length - 1) * ROW_GAP;

  return { nodes: [...placed.values()], cells, edges, width, height };
}

/** Anchors at the node's left/right edges rather than top/bottom, since this
 *  grid flows across stage columns rather than down dependency ranks. A
 *  `goes_back` edge runs against that flow, so it dips above the row instead
 *  of cutting a straight line through whatever sits between the two stages. */
function curve(
  from: AskCanvasNodePosition | undefined,
  to: AskCanvasNodePosition | undefined,
  kind: EdgeKind,
): string {
  const a = from ?? { x: 0, y: 0 };
  const b = to ?? { x: 0, y: 0 };
  const y1 = a.y + NODE_H / 2;
  const y2 = b.y + NODE_H / 2;

  if (kind === 'goes_back') {
    const x1 = a.x;
    const x2 = b.x + NODE_W;
    const detour = Math.max(NODE_H, Math.abs(x1 - x2) / 4);
    const dipY = Math.min(y1, y2) - detour;
    return `M${round(x1)} ${round(y1)} C${round(x1)} ${round(dipY)}, ${round(x2)} ${round(dipY)}, ${round(x2)} ${round(y2)}`;
  }

  const x1 = a.x + NODE_W;
  const x2 = b.x;
  const bend = Math.max(16, (x2 - x1) / 2);
  return `M${round(x1)} ${round(y1)} C${round(x1 + bend)} ${round(y1)}, ${round(x2 - bend)} ${round(y2)}, ${round(x2)} ${round(y2)}`;
}

function round(value: number): number {
  return Math.round(value * 100) / 100;
}

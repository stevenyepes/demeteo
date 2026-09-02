import type { AskCanvas, CanvasNode, EdgeKind } from '../types';

/**
 * Where every Ask Canvas node sits, and the route between each pair.
 *
 * Unlike `ticketGraphLayout.ts`'s dependency ranks, placement here is
 * author-declared: a node's `(stage, lane)` names its cell directly, so the
 * grid's shape comes from `canvas.stages`/`canvas.lanes` rather than a graph
 * traversal.
 *
 * **Geometry comes from position; `kind` only picks a stroke.** An earlier
 * revision routed on `kind` — `goes_back` got a detour, everything else got a
 * forward bezier — which made a mislabelled edge a rendering failure rather
 * than a wrong colour. A model that types a right-to-left edge `hands_off`
 * (it does) then got `bend = max(16, negative)`, and the curve degenerated
 * into a straight diagonal across every card between the two ends. Whether an
 * edge runs forward is a fact about the two positions, so it is read from
 * them here and `EdgeKind` never reaches this module's routing decision.
 *
 * A lane is a band spanning every stage, not a row of cells — an empty
 * `(stage, lane)` is space inside its band, which is what
 * `docs/ask-canvas/probe/Main.html` draws and what lets a sparse canvas read
 * as sparse rather than as a broken table.
 */

export const NODE_W = 202;
export const NODE_H = 72;

const COL_W = 240;
const COL_GAP = 40;
/** Vertical space between two nodes declared in the same cell. They tile;
 *  they never stack, so no occupant can be hidden behind another. */
const NODE_GAP = 12;
/** Space above the first node in a band, which the lane label sits in, and
 *  below the last — the 46/22 split `docs/ask-canvas/probe/Main.html` uses. */
const BAND_PAD_TOP = 46;
const BAND_PAD_BOTTOM = 22;
/** Height of the channel below each band that return edges route through. */
const GUTTER_H = 34;
/** Separation between two return edges sharing one gutter. */
const GUTTER_STEP = 9;
const PAD = 16;
/** How far a forward edge overshoots its source before turning, so two edges
 *  leaving the same column at different rows do not share a vertical run. */
const ELBOW = 20;

export interface AskCanvasNodePosition {
  id: string;
  x: number;
  y: number;
}

/** One lane's full-width band. Present for every declared lane, occupied or
 *  not: an empty band reads as "nobody is acting here", which is the whole
 *  reason lanes are authored rather than inferred. */
export interface AskCanvasBand {
  lane: number;
  y: number;
  height: number;
}

/** One stage's column, for the header strip and the gridline under it. */
export interface AskCanvasColumn {
  stage: number;
  x: number;
  width: number;
}

export interface AskCanvasEdgeLayout {
  from: string;
  to: string;
  kind: EdgeKind;
  /** `true` when the direct route would have run through a card, so this one
   *  detoured through a gutter instead. The view draws both the same way; it
   *  is exposed so a test can name which route it is asserting about. */
  returns: boolean;
  path: string;
}

export interface AskCanvasLayout {
  nodes: AskCanvasNodePosition[];
  bands: AskCanvasBand[];
  columns: AskCanvasColumn[];
  edges: AskCanvasEdgeLayout[];
  width: number;
  height: number;
}

function columnX(stage: number): number {
  return PAD + stage * (COL_W + COL_GAP);
}

function cellKey(stage: number, lane: number): string {
  return `${stage}:${lane}`;
}

export function layoutAskCanvas(
  canvas: Pick<AskCanvas, 'stages' | 'lanes' | 'nodes' | 'edges'>,
): AskCanvasLayout {
  const byCell = new Map<string, CanvasNode[]>();
  for (const node of canvas.nodes) {
    const key = cellKey(node.stage, node.lane);
    const occupants = byCell.get(key);
    if (occupants) {
      occupants.push(node);
    } else {
      byCell.set(key, [node]);
    }
  }

  const columns: AskCanvasColumn[] = canvas.stages.map((_stage, stage) => ({
    stage,
    x: columnX(stage),
    width: COL_W,
  }));

  // A band is as tall as its fullest cell, so tiling occupants never pushes
  // one out of its own lane.
  const bands: AskCanvasBand[] = [];
  let cursor = PAD;
  for (let lane = 0; lane < canvas.lanes.length; lane += 1) {
    let tallest = 1;
    for (let stage = 0; stage < canvas.stages.length; stage += 1) {
      tallest = Math.max(tallest, byCell.get(cellKey(stage, lane))?.length ?? 0);
    }
    const height =
      BAND_PAD_TOP + BAND_PAD_BOTTOM + tallest * NODE_H + (tallest - 1) * NODE_GAP;
    bands.push({ lane, y: cursor, height });
    cursor += height + GUTTER_H;
  }

  const placed = new Map<string, AskCanvasNodePosition>();
  for (const occupants of byCell.values()) {
    const band = bands[occupants[0].lane];
    const top = band !== undefined ? band.y + BAND_PAD_TOP : PAD;
    const left = columnX(occupants[0].stage) + (COL_W - NODE_W) / 2;
    occupants.forEach((node, index) => {
      placed.set(node.id, {
        id: node.id,
        x: left,
        y: top + index * (NODE_H + NODE_GAP),
      });
    });
  }

  // Every detouring edge gets its own line in the gutter it uses, so two of
  // them leaving the same band stay legible.
  const gutterUse = new Map<number, number>();
  const cards = [...placed.values()];

  const edges: AskCanvasEdgeLayout[] = [];
  for (const edge of canvas.edges) {
    if (edge.from === edge.to) continue;
    const from = placed.get(edge.from);
    const to = placed.get(edge.to);
    if (from === undefined || to === undefined) continue;

    // Take the short route when it is clear, and only then. Deciding from
    // stage order instead would still have driven a straight run through
    // whatever a middle column happens to hold.
    const direct = directPath(from, to);
    const obstructed =
      to.x <= from.x ||
      cards.some((card) => card !== from && card !== to && crosses(direct, card));

    if (!obstructed) {
      edges.push({ from: edge.from, to: edge.to, kind: edge.kind, returns: false, path: direct });
      continue;
    }
    const band = bandBelow(bands, from.y);
    const slot = gutterUse.get(band) ?? 0;
    gutterUse.set(band, slot + 1);
    edges.push({
      from: edge.from,
      to: edge.to,
      kind: edge.kind,
      returns: true,
      path: gutterPath(from, to, gutterY(bands, band, slot)),
    });
  }

  const width =
    PAD * 2 + canvas.stages.length * COL_W + Math.max(0, canvas.stages.length - 1) * COL_GAP;
  // `cursor` already carries a trailing gutter, which doubles as the bottom
  // padding and as room for a return edge leaving the last band.
  const height = Math.max(cursor, PAD * 2);

  return { nodes: [...placed.values()], bands, columns, edges, width, height };
}

/** Index of the band a node sits in, by its top edge. */
function bandBelow(bands: readonly AskCanvasBand[], y: number): number {
  for (let i = bands.length - 1; i >= 0; i -= 1) {
    if (y >= bands[i].y) return i;
  }
  return 0;
}

function gutterY(bands: readonly AskCanvasBand[], band: number, slot: number): number {
  const below = bands[band];
  const base = below !== undefined ? below.y + below.height : 0;
  return base + GUTTER_H / 2 + (slot % 3) * GUTTER_STEP - GUTTER_STEP;
}

/** Right edge to left edge, in horizontal and vertical runs only. A same-row
 *  pair is one straight line, which is how `Main.html` draws its `H` edges;
 *  otherwise it turns in the channel just past the source, never at the
 *  midpoint, which is inside whatever column the midpoint lands in. */
function directPath(from: AskCanvasNodePosition, to: AskCanvasNodePosition): string {
  const x1 = from.x + NODE_W;
  const y1 = from.y + NODE_H / 2;
  const x2 = to.x;
  const y2 = to.y + NODE_H / 2;

  if (y1 === y2) return `M${r(x1)} ${r(y1)} H${r(x2)}`;
  return `M${r(x1)} ${r(y1)} H${r(x1 + ELBOW)} V${r(y2)} H${r(x2)}`;
}

/** Down out of the row, along a gutter, then up into the target's left edge.
 *  Both turns land between columns — a card is inset from its column, so the
 *  channel at `to.x - ELBOW` is empty in every band, whichever way the route
 *  travels through it. */
function gutterPath(
  from: AskCanvasNodePosition,
  to: AskCanvasNodePosition,
  gutter: number,
): string {
  const x1 = from.x + NODE_W / 2;
  const y1 = from.y + NODE_H;
  const x2 = to.x - ELBOW;
  const y2 = to.y + NODE_H / 2;

  return `M${r(x1)} ${r(y1)} V${r(gutter)} H${r(x2)} V${r(y2)} H${r(to.x)}`;
}

/** Whether any run of an orthogonal path passes through a card's box. The
 *  routes here are `M`/`H`/`V` only, so the corners are the whole geometry. */
function crosses(path: string, card: AskCanvasNodePosition): boolean {
  let x = 0;
  let y = 0;

  for (const [, op, args] of path.matchAll(/([MHV])\s*([-\d.\s]+)/g)) {
    const values = args.trim().split(/\s+/).map(Number);
    const prevX = x;
    const prevY = y;
    if (op === 'M') {
      [x, y] = values;
      continue;
    }
    if (op === 'H') x = values[0];
    else y = values[0];

    if (
      Math.min(prevX, x) < card.x + NODE_W &&
      Math.max(prevX, x) > card.x &&
      Math.min(prevY, y) < card.y + NODE_H &&
      Math.max(prevY, y) > card.y
    ) {
      return true;
    }
  }
  return false;
}

function r(value: number): number {
  return Math.round(value * 100) / 100;
}

// The grid's shape is author-declared: stage/lane counts come straight from
// the canvas, and a node's cell is whatever it names — no dependency
// traversal to pin here, so these assert derived relationships (containment,
// separation, routing) rather than pixel literals.

import { describe, expect, it } from 'vitest';

import { layoutAskCanvas, NODE_H, NODE_W } from './askCanvasLayout';
import type { AskCanvas, CanvasEdge, CanvasNode } from '../types';

function node(id: string, stage: number, lane: number, overrides: Partial<CanvasNode> = {}): CanvasNode {
  return {
    id,
    title: id,
    role: 'agent',
    path: null,
    stage,
    lane,
    ...overrides,
  };
}

function canvasOf(
  stages: string[],
  lanes: string[],
  nodes: CanvasNode[],
  edges: CanvasEdge[] = [],
): Pick<AskCanvas, 'stages' | 'lanes' | 'nodes' | 'edges'> {
  return { stages, lanes, nodes, edges };
}

function bandOf(layout: ReturnType<typeof layoutAskCanvas>, lane: number) {
  const band = layout.bands.find((candidate) => candidate.lane === lane);
  if (!band) throw new Error(`no band for lane ${lane}`);
  return band;
}

function columnOf(layout: ReturnType<typeof layoutAskCanvas>, stage: number) {
  const column = layout.columns.find((candidate) => candidate.stage === stage);
  if (!column) throw new Error(`no column for stage ${stage}`);
  return column;
}

function nodeOf(layout: ReturnType<typeof layoutAskCanvas>, id: string) {
  const placed = layout.nodes.find((candidate) => candidate.id === id);
  if (!placed) throw new Error(`node ${id} not placed`);
  return placed;
}

/** Every point a path visits. The routes are orthogonal, so the corners are
 *  the whole geometry — no sampling needed. */
function pointsOf(path: string): { x: number; y: number }[] {
  const points: { x: number; y: number }[] = [];
  let x = 0;
  let y = 0;
  for (const [, op, args] of path.matchAll(/([MHV])\s*([-\d.\s]+)/g)) {
    const values = args.trim().split(/\s+/).map(Number);
    if (op === 'M') {
      [x, y] = values;
    } else if (op === 'H') {
      x = values[0];
    } else {
      y = values[0];
    }
    points.push({ x, y });
  }
  return points;
}

/** Does any segment of `path` pass through the card at `(x, y)`? */
function crossesCard(path: string, card: { x: number; y: number }): boolean {
  const points = pointsOf(path);
  for (let i = 1; i < points.length; i += 1) {
    const a = points[i - 1];
    const b = points[i];
    const minX = Math.min(a.x, b.x);
    const maxX = Math.max(a.x, b.x);
    const minY = Math.min(a.y, b.y);
    const maxY = Math.max(a.y, b.y);
    if (minX < card.x + NODE_W && maxX > card.x && minY < card.y + NODE_H && maxY > card.y) {
      return true;
    }
  }
  return false;
}

describe('layoutAskCanvas', () => {
  it('places a node inside its lane band and its stage column', () => {
    const canvas = canvasOf(['stage 0', 'stage 1'], ['lane 0', 'lane 1'], [node('a', 1, 1)]);

    const layout = layoutAskCanvas(canvas);
    const band = bandOf(layout, 1);
    const column = columnOf(layout, 1);
    const placed = nodeOf(layout, 'a');

    expect(placed.x).toBeGreaterThanOrEqual(column.x);
    expect(placed.x + NODE_W).toBeLessThanOrEqual(column.x + column.width);
    expect(placed.y).toBeGreaterThanOrEqual(band.y);
    expect(placed.y + NODE_H).toBeLessThanOrEqual(band.y + band.height);
  });

  it('tiles two nodes declared in one cell instead of stacking them', () => {
    const canvas = canvasOf(['stage 0'], ['lane 0'], [node('a', 0, 0), node('b', 0, 0)]);

    const layout = layoutAskCanvas(canvas);
    const a = nodeOf(layout, 'a');
    const b = nodeOf(layout, 'b');

    const overlaps =
      a.x < b.x + NODE_W && a.x + NODE_W > b.x && a.y < b.y + NODE_H && a.y + NODE_H > b.y;
    expect(overlaps).toBe(false);

    // …and the band grew to hold both, rather than letting the second spill.
    const band = bandOf(layout, 0);
    expect(b.y + NODE_H).toBeLessThanOrEqual(band.y + band.height);
  });

  it('routes a backwards edge clear of every card between its ends', () => {
    // `hands_off` on a backwards edge is exactly what a model emits, and the
    // route must not depend on the label being right.
    const canvas = canvasOf(
      ['s0', 's1', 's2'],
      ['l0'],
      [node('a', 0, 0), node('b', 1, 0), node('c', 2, 0)],
      [{ from: 'c', to: 'a', kind: 'hands_off' }],
    );

    const layout = layoutAskCanvas(canvas);
    expect(layout.edges).toHaveLength(1);
    expect(layout.edges[0].returns).toBe(true);

    for (const id of ['a', 'b', 'c']) {
      expect(crossesCard(layout.edges[0].path, nodeOf(layout, id))).toBe(false);
    }
  });

  it('takes the short route forward when nothing is in the way', () => {
    const canvas = canvasOf(
      ['s0', 's1', 's2'],
      ['l0', 'l1'],
      [node('a', 0, 0), node('b', 1, 0), node('c', 2, 1)],
      [{ from: 'a', to: 'c', kind: 'hands_off' }],
    );

    const layout = layoutAskCanvas(canvas);
    expect(layout.edges[0].returns).toBe(false);
    expect(crossesCard(layout.edges[0].path, nodeOf(layout, 'b'))).toBe(false);
    for (const point of pointsOf(layout.edges[0].path)) {
      expect(point.y).toBeGreaterThanOrEqual(0);
      expect(point.y).toBeLessThanOrEqual(layout.height);
    }
  });

  it('detours a forward edge whose short route would run through a card', () => {
    // `b` sits in the target's row, between the turn and `c` — the case a
    // stage-order rule cannot see, because every stage here reads forward.
    const canvas = canvasOf(
      ['s0', 's1', 's2'],
      ['l0', 'l1'],
      [node('a', 0, 0), node('b', 1, 1), node('c', 2, 1)],
      [{ from: 'a', to: 'c', kind: 'hands_off' }],
    );

    const layout = layoutAskCanvas(canvas);
    expect(layout.edges[0].returns).toBe(true);
    expect(crossesCard(layout.edges[0].path, nodeOf(layout, 'b'))).toBe(false);
  });

  it('drops a self-edge rather than drawing one across its own card', () => {
    const canvas = canvasOf(
      ['s0'],
      ['l0'],
      [node('a', 0, 0)],
      [{ from: 'a', to: 'a', kind: 'goes_back' }],
    );

    expect(layoutAskCanvas(canvas).edges).toHaveLength(0);
  });

  it('declares a band for every lane, occupied or not', () => {
    const canvas = canvasOf(['s0', 's1'], ['l0', 'l1', 'l2'], [node('a', 0, 1)]);

    const layout = layoutAskCanvas(canvas);
    expect(layout.bands.map((band) => band.lane)).toEqual([0, 1, 2]);
    expect(layout.columns.map((column) => column.stage)).toEqual([0, 1]);
  });

  it('gives every node a position and keeps them all inside the canvas', () => {
    const canvas = canvasOf(
      ['s0', 's1'],
      ['l0', 'l1'],
      [node('a', 0, 0), node('b', 1, 0), node('c', 0, 1), node('d', 1, 1)],
    );

    const layout = layoutAskCanvas(canvas);
    expect(layout.nodes).toHaveLength(4);
    for (const placed of layout.nodes) {
      expect(placed.x + NODE_W).toBeLessThanOrEqual(layout.width);
      expect(placed.y + NODE_H).toBeLessThanOrEqual(layout.height);
    }
  });
});

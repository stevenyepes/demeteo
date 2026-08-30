// The grid's shape is author-declared: stage/lane counts come straight from
// the canvas, and a node's cell is whatever it names — no dependency
// traversal to pin here, so these assert derived relationships (containment,
// counts) rather than pixel literals.

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

function cellOf(layout: ReturnType<typeof layoutAskCanvas>, stage: number, lane: number) {
  const cell = layout.cells.find((candidate) => candidate.stage === stage && candidate.lane === lane);
  if (!cell) throw new Error(`no cell for ${stage},${lane}`);
  return cell;
}

function nodeOf(layout: ReturnType<typeof layoutAskCanvas>, id: string) {
  return layout.nodes.find((candidate) => candidate.id === id);
}

describe('layoutAskCanvas', () => {
  it('places a node inside its declared cell bounds', () => {
    const canvas = canvasOf(
      ['stage 0', 'stage 1'],
      ['lane 0', 'lane 1'],
      [node('a', 1, 1)],
    );

    const layout = layoutAskCanvas(canvas);
    const cell = cellOf(layout, 1, 1);
    const placed = nodeOf(layout, 'a');
    if (!placed) throw new Error('node a not placed');

    expect(placed.x).toBeGreaterThanOrEqual(cell.x);
    expect(placed.x + NODE_W).toBeLessThanOrEqual(cell.x + cell.width);
    expect(placed.y).toBeGreaterThanOrEqual(cell.y);
    expect(placed.y + NODE_H).toBeLessThanOrEqual(cell.y + cell.height);
  });

  it('reproduces the CanvasFocus arrangement: an empty cell still appears with no node', () => {
    // 4 stages x 3 lanes, no node declared at stage 2 / lane 0 (the `.jwait` cell).
    const canvas = canvasOf(
      ['Describe', 'Decompose', 'Run', 'Gate & merge'],
      ['The person', 'Demeteo', 'Coding agent'],
      [
        node('describe-0', 0, 0),
        node('decompose-0', 1, 0),
        node('gate-0', 3, 0),
        node('describe-1', 0, 1),
        node('run-1', 2, 1),
        node('describe-2', 0, 2),
      ],
    );

    const layout = layoutAskCanvas(canvas);
    const emptyCell = cellOf(layout, 2, 0);

    // No phantom node was added: the output node set is exactly the declared fixture nodes.
    expect(layout.nodes.map((n) => n.id).sort()).toEqual(canvas.nodes.map((n) => n.id).sort());

    // No placed node's bounds fall inside the undeclared (stage 2, lane 0) cell.
    for (const placed of layout.nodes) {
      const withinEmptyCell =
        placed.x < emptyCell.x + emptyCell.width &&
        placed.x + NODE_W > emptyCell.x &&
        placed.y < emptyCell.y + emptyCell.height &&
        placed.y + NODE_H > emptyCell.y;
      expect(withinEmptyCell).toBe(false);
    }

    expect(nodeOf(layout, 'run-1')).toBeDefined();
  });

  it('emits one cell per declared (stage, lane) pair regardless of occupancy', () => {
    const canvas = canvasOf(['s0', 's1', 's2'], ['l0', 'l1'], []);

    const layout = layoutAskCanvas(canvas);

    expect(layout.cells).toHaveLength(6);
    for (let stage = 0; stage < 3; stage += 1) {
      for (let lane = 0; lane < 2; lane += 1) {
        expect(layout.cells).toContainEqual(expect.objectContaining({ stage, lane }));
      }
    }
  });

  it('produces no layout.nodes entry for an id with no fixture node', () => {
    const canvas = canvasOf(['s0'], ['l0'], []);

    const layout = layoutAskCanvas(canvas);

    expect(layout.nodes).toEqual([]);
    expect(nodeOf(layout, 'ghost')).toBeUndefined();
  });

  it('emits one edge per input edge, each path starting with M', () => {
    const canvas = canvasOf(
      ['s0', 's1', 's2'],
      ['l0'],
      [node('a', 0, 0), node('b', 1, 0), node('c', 2, 0)],
      [
        { from: 'a', to: 'b', kind: 'hands_off' },
        { from: 'b', to: 'c', kind: 'hands_off' },
        { from: 'c', to: 'a', kind: 'goes_back' },
      ],
    );

    const layout = layoutAskCanvas(canvas);

    expect(layout.edges).toHaveLength(canvas.edges.length);
    for (const edge of layout.edges) {
      expect(edge.path).toMatch(/^M/);
    }
  });

  it('gives goes_back edges a distinct bend from hands_off edges', () => {
    const canvas = canvasOf(
      ['s0', 's1', 's2'],
      ['l0'],
      [node('a', 0, 0), node('b', 1, 0), node('c', 2, 0)],
      [
        { from: 'a', to: 'c', kind: 'hands_off' },
        { from: 'c', to: 'a', kind: 'goes_back' },
      ],
    );

    const layout = layoutAskCanvas(canvas);
    const forward = layout.edges.find((edge) => edge.kind === 'hands_off');
    const backward = layout.edges.find((edge) => edge.kind === 'goes_back');

    expect(forward?.path).not.toEqual(backward?.path);
  });

  it('places two nodes sharing a cell at non-overlapping positions without throwing', () => {
    const canvas = canvasOf(
      ['s0'],
      ['l0'],
      [node('a', 0, 0), node('b', 0, 0)],
    );

    const layout = layoutAskCanvas(canvas);

    const a = nodeOf(layout, 'a');
    const b = nodeOf(layout, 'b');
    if (!a || !b) throw new Error('both nodes should be placed');

    expect(a.x !== b.x || a.y !== b.y).toBe(true);
  });
});

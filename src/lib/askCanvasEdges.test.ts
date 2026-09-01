import { describe, expect, it } from 'vitest';

import { edgesForNode } from './askCanvasEdges';
import type { AskCanvas } from '../types';

function canvas(): AskCanvas {
  return {
    kind: 'architecture',
    title: 'Test canvas',
    stages: [],
    lanes: [],
    nodes: [
      { id: 'a', title: 'Decompose Feature', role: 'orchestration', path: null, stage: 0, lane: 0 },
      { id: 'b', title: 'Implement Step', role: 'agent', path: null, stage: 1, lane: 0 },
      { id: 'c', title: 'Gate & Merge', role: 'boundary', path: null, stage: 2, lane: 0 },
    ],
    edges: [
      { from: 'a', to: 'b', kind: 'hands_off' },
      { from: 'b', to: 'c', kind: 'hands_off' },
      { from: 'c', to: 'b', kind: 'goes_back' },
    ],
  };
}

describe('edgesForNode', () => {
  it('splits and titles incoming vs outgoing edges for a node with both', () => {
    const { incoming, outgoing } = edgesForNode(canvas(), 'b');

    expect(incoming).toEqual([
      { nodeId: 'a', title: 'Decompose Feature', kind: 'hands_off' },
      { nodeId: 'c', title: 'Gate & Merge', kind: 'goes_back' },
    ]);
    expect(outgoing).toEqual([{ nodeId: 'c', title: 'Gate & Merge', kind: 'hands_off' }]);
  });

  it('returns two empty arrays for a node with no edges', () => {
    const isolated: AskCanvas = { ...canvas(), nodes: [...canvas().nodes, { id: 'd', title: 'Isolated', role: 'agent', path: null, stage: 3, lane: 0 }] };

    const { incoming, outgoing } = edgesForNode(isolated, 'd');

    expect(incoming).toEqual([]);
    expect(outgoing).toEqual([]);
  });

  it('preserves an edge kind of goes_back in the returned neighbor', () => {
    const { outgoing } = edgesForNode(canvas(), 'c');

    expect(outgoing).toEqual([{ nodeId: 'b', title: 'Implement Step', kind: 'goes_back' }]);
  });

  it('skips an edge naming a node id not present in canvas.nodes', () => {
    const withDangling: AskCanvas = {
      ...canvas(),
      edges: [...canvas().edges, { from: 'ghost', to: 'b', kind: 'hands_off' }],
    };

    const { incoming } = edgesForNode(withDangling, 'b');

    expect(incoming.some((n) => n.nodeId === 'ghost')).toBe(false);
  });
});

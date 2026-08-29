/**
 * `AskCanvasView` integration coverage — Acceptance Criteria 2 and 7: the
 * `CanvasFocus.html`-shaped empty cell renders a bare slot, and the two edge
 * kinds render with distinguishable stroke classes.
 */
import { render, screen, cleanup } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { AskCanvasView } from './AskCanvasView';
import type { AskCanvas, CanvasNode } from '../../types';

afterEach(cleanup);

function node(id: string, stage: number, lane: number, overrides: Partial<CanvasNode> = {}): CanvasNode {
  return { id, title: id, role: 'agent', path: null, stage, lane, ...overrides };
}

describe('AskCanvasView', () => {
  it('renders a bare empty-cell slot for the CanvasFocus stage-2/lane-0 gap, with no node card inside', () => {
    const canvas: AskCanvas = {
      kind: 'journey',
      title: 'Ask canvas',
      stages: ['Describe', 'Decompose', 'Run', 'Gate & merge'],
      lanes: ['The person', 'Demeteo', 'Coding agent'],
      nodes: [
        node('describe-0', 0, 0),
        node('decompose-0', 1, 0),
        node('gate-0', 3, 0),
        node('describe-1', 0, 1),
        node('run-1', 2, 1),
        node('describe-2', 0, 2),
      ],
      edges: [],
    };

    const { container } = render(
      <AskCanvasView canvas={canvas} answerText="" selectedNodeId={null} onActivate={vi.fn()} />,
    );

    const emptyCells = screen.getAllByTestId('ask-canvas-empty-cell');
    expect(emptyCells.length).toBeGreaterThan(0);

    // No phantom card was rendered for an empty cell: exactly one card per declared node.
    const nodeForeignObjects = Array.from(container.querySelectorAll('foreignObject')).filter(
      (fo) => fo.querySelector('[data-state]') !== null,
    );
    expect(nodeForeignObjects).toHaveLength(canvas.nodes.length);

    // No node card's foreignObject overlaps an empty cell's bounds.
    for (const cell of emptyCells) {
      const cellBounds = {
        x: Number(cell.getAttribute('x')),
        y: Number(cell.getAttribute('y')),
        width: Number(cell.getAttribute('width')),
        height: Number(cell.getAttribute('height')),
      };
      for (const fo of nodeForeignObjects) {
        const foBounds = {
          x: Number(fo.getAttribute('x')),
          y: Number(fo.getAttribute('y')),
          width: Number(fo.getAttribute('width')),
          height: Number(fo.getAttribute('height')),
        };
        const overlaps =
          foBounds.x < cellBounds.x + cellBounds.width &&
          foBounds.x + foBounds.width > cellBounds.x &&
          foBounds.y < cellBounds.y + cellBounds.height &&
          foBounds.y + foBounds.height > cellBounds.y;
        expect(overlaps).toBe(false);
      }
    }
  });

  it('renders hands_off and goes_back edges with different classNames', () => {
    const canvas: AskCanvas = {
      kind: 'journey',
      title: 'Ask canvas',
      stages: ['s0', 's1', 's2'],
      lanes: ['l0'],
      nodes: [node('a', 0, 0), node('b', 1, 0), node('c', 2, 0)],
      edges: [
        { from: 'a', to: 'b', kind: 'hands_off' },
        { from: 'c', to: 'a', kind: 'goes_back' },
      ],
    };

    const { container } = render(
      <AskCanvasView canvas={canvas} answerText="" selectedNodeId={null} onActivate={vi.fn()} />,
    );

    const edgeLayer = container.querySelector('[data-testid="ask-canvas-edge-layer"]');
    expect(edgeLayer).not.toBeNull();
    const paths = edgeLayer!.querySelectorAll('path');
    expect(paths).toHaveLength(2);
    expect(paths[0].getAttribute('class')).not.toEqual(paths[1].getAttribute('class'));
  });

  it('renders stage and lane labels from the canvas', () => {
    const canvas: AskCanvas = {
      kind: 'journey',
      title: 'Ask canvas',
      stages: ['Describe', 'Decompose'],
      lanes: ['The person', 'Demeteo'],
      nodes: [],
      edges: [],
    };

    render(<AskCanvasView canvas={canvas} answerText="" selectedNodeId={null} onActivate={vi.fn()} />);

    expect(screen.getByText('Describe')).toBeInTheDocument();
    expect(screen.getByText('Decompose')).toBeInTheDocument();
    expect(screen.getByText('The person')).toBeInTheDocument();
    expect(screen.getByText('Demeteo')).toBeInTheDocument();
  });
});

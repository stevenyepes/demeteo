/**
 * `AskCanvasView` integration coverage: an unoccupied `(stage, lane)` leaves
 * its lane band empty rather than dropping the band, the two edge kinds stay
 * distinguishable, and a node's `(id, path)` verdict reaches its card.
 *
 * The structural assertion about `foreignObject` is the load-bearing one.
 * jsdom lays out no SVG, so the failure that motivated this file's rewrite —
 * cards inside a transformed `<g>`, rendered by the webview without the
 * transform and at a different scale — is invisible to every assertion about
 * appearance. What *is* checkable is the shape that caused it.
 */
import { render, screen, cleanup, fireEvent } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { AskCanvasView } from './AskCanvasView';
import type { AskCanvas, CanvasNode } from '../../types';

afterEach(cleanup);

function node(id: string, stage: number, lane: number, overrides: Partial<CanvasNode> = {}): CanvasNode {
  return { id, title: id, role: 'agent', path: null, stage, lane, ...overrides };
}

function cards(container: HTMLElement): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>('[data-state]'));
}

describe('AskCanvasView', () => {
  it('draws a band for every lane and leaves an unoccupied cell empty', () => {
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
      <AskCanvasView
        canvas={canvas}
        answerText=""
        canvasPaths={[]}
        selectedNodeId={null}
        onActivate={vi.fn()}
      />,
    );

    // One band per declared lane, whatever the occupancy — an empty lane is
    // the statement "nobody is acting here", which is why lanes are authored.
    expect(container.querySelectorAll('[data-testid="ask-canvas-band"]')).toHaveLength(3);
    // Exactly one card per declared node; nothing phantom for an empty cell.
    expect(cards(container)).toHaveLength(canvas.nodes.length);
  });

  it('keeps every card out of the SVG, so no ancestor transform can be dropped', () => {
    const canvas: AskCanvas = {
      kind: 'architecture',
      title: 'Ask canvas',
      stages: ['s0', 's1'],
      lanes: ['l0'],
      nodes: [node('a', 0, 0), node('b', 1, 0)],
      edges: [{ from: 'a', to: 'b', kind: 'hands_off' }],
    };

    const { container } = render(
      <AskCanvasView
        canvas={canvas}
        answerText=""
        canvasPaths={[]}
        selectedNodeId={null}
        onActivate={vi.fn()}
      />,
    );

    expect(container.querySelectorAll('foreignObject')).toHaveLength(0);
    for (const card of cards(container)) {
      expect(card.closest('svg')).toBeNull();
      expect(card.style.left).not.toBe('');
    }
  });

  it('renders hands_off and goes_back edges with different classNames and arrowheads', () => {
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
      <AskCanvasView
        canvas={canvas}
        answerText=""
        canvasPaths={[]}
        selectedNodeId={null}
        onActivate={vi.fn()}
      />,
    );

    const edgeLayer = container.querySelector('[data-testid="ask-canvas-edge-layer"]');
    expect(edgeLayer).not.toBeNull();
    const paths = edgeLayer!.querySelectorAll('path');
    expect(paths).toHaveLength(2);
    expect(paths[0].getAttribute('class')).not.toEqual(paths[1].getAttribute('class'));
    for (const path of paths) {
      expect(path.getAttribute('marker-end')).toMatch(/^url\(#/);
    }
  });

  it('renders stage names, lane names and the canvas title', () => {
    const canvas: AskCanvas = {
      kind: 'journey',
      title: 'Feature to Gate',
      stages: ['Describe', 'Decompose'],
      lanes: ['The person', 'Demeteo'],
      nodes: [],
      edges: [],
    };

    render(
      <AskCanvasView
        canvas={canvas}
        answerText=""
        canvasPaths={[]}
        selectedNodeId={null}
        onActivate={vi.fn()}
      />,
    );

    expect(screen.getByText('Describe')).toBeInTheDocument();
    expect(screen.getByText('Decompose')).toBeInTheDocument();
    expect(screen.getByText('The person')).toBeInTheDocument();
    expect(screen.getByText('Demeteo')).toBeInTheDocument();
    // The title had nowhere to render but the svg's aria-label before this.
    expect(screen.getByRole('heading', { name: 'Feature to Gate' })).toBeInTheDocument();
  });

  it('threads each verdict to its node by (node_id, path), and leaves a path-less node alone', () => {
    const canvas: AskCanvas = {
      kind: 'journey',
      title: 'Ask canvas',
      stages: ['s0'],
      lanes: ['l0'],
      nodes: [
        node('resolved-node', 0, 0, { path: 'src/lib/foo.ts' }),
        node('stale-verdict-node', 0, 0, { path: 'src/lib/bar.ts' }),
        node('no-verdict-node', 0, 0, { path: 'src/lib/baz.ts' }),
        node('no-path-node', 0, 0),
      ],
      edges: [],
    };

    const onActivate = vi.fn();
    render(
      <AskCanvasView
        canvas={canvas}
        answerText=""
        canvasPaths={[
          { node_id: 'resolved-node', path: 'src/lib/foo.ts', resolved: true },
          { node_id: 'stale-verdict-node', path: 'src/lib/bar.ts', resolved: false },
        ]}
        selectedNodeId={null}
        onActivate={onActivate}
      />,
    );

    expect(screen.getByText('resolved-node').closest('[data-state]')).toHaveAttribute(
      'data-state',
      'resting',
    );
    expect(screen.getByText('stale-verdict-node').closest('[data-state]')).toHaveAttribute(
      'data-state',
      'unresolved',
    );
    expect(screen.getByText('no-verdict-node').closest('[data-state]')).toHaveAttribute(
      'data-state',
      'unresolved',
    );

    // A node that never claimed a file has nothing to be stale about.
    const pathless = screen.getByText('no-path-node').closest('[data-state]')!;
    expect(pathless).toHaveAttribute('data-state', 'resting');
    fireEvent.click(pathless);
    expect(onActivate).toHaveBeenCalledWith('no-path-node');
  });
});

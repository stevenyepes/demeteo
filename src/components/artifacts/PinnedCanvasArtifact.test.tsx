/**
 * Unit tests for `PinnedCanvasArtifact` — the presentational renderer for a
 * `<message_id>.canvas.json` artifact. Driven against the component directly:
 * the only coverage these decisions had was `ArtifactViewer.test.tsx`'s
 * end-to-end mount through a mocked `invoke`, which reaches them through the
 * JSON parse and so cannot address them one at a time.
 *
 * Every case below was watched to fail first (AGENTS.md §7), each against the
 * draft that gets that one decision wrong:
 *
 * - "unknown" — against a draft rendering `{checkedCommitSha}` bare, which is
 *   the common case rather than an edge one: `verify_canvas_paths` returns
 *   `(None, None)` for a canvas whose nodes cite no paths, so a pin with no
 *   sha rendered an empty span beside the "Checked commit:" label.
 * - the two "no unresolved block" cases — against a draft rendering the amber
 *   block unconditionally, which showed an empty "Unresolved paths:" heading
 *   for a fully-resolved canvas.
 * - "lists exactly the unresolved paths" — against a draft mapping the whole
 *   `canvasPaths` list rather than the filtered one; it listed the resolved
 *   path under the amber heading too.
 * - "selects" — against `selectedNodeId={null}` / `onActivate={() => {}}`, the
 *   shape this component was extracted from, where the card carries
 *   `cursor-pointer` and stays `resting` through any number of clicks. That
 *   draft leaves "deselects" green, which is what the second case is for: it
 *   was watched to fail separately against a plain `setSelectedNodeId(id)`,
 *   where a node latches on and the live pane's toggle is gone.
 * - the duplicate-`node_id` case — against `key={p.node_id}`, which renders
 *   both rows but logs React's "same key" warning.
 */

import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { AskCanvas, CanvasPathVerdict } from '../../types';
import { PinnedCanvasArtifact } from './PinnedCanvasArtifact';

const canvas: AskCanvas = {
  kind: 'architecture',
  title: 'Pinned canvas',
  stages: ['01 · Orchestrator'],
  lanes: ['01 · The person'],
  nodes: [
    { id: 'n1', title: 'Node one', role: 'orchestration', path: 'src/one.ts', stage: 0, lane: 0 },
  ],
  edges: [],
};

function renderArtifact(props: Partial<{ canvasPaths: CanvasPathVerdict[]; checkedCommitSha: string | null }> = {}) {
  return render(
    <PinnedCanvasArtifact
      artifactPath="/tmp/artifacts/ask-canvas/thread-1/msg-1.canvas.json"
      body="{}"
      canvas={canvas}
      canvasPaths={props.canvasPaths ?? []}
      checkedCommitSha={props.checkedCommitSha ?? null}
    />,
  );
}

/** The node card, found through its title — `AskCanvasNode` carries the
 *  render state this feature's selection ring is painted from on `data-state`.
 *  Scoped to the grid, because the detail strip below it repeats the title of
 *  whichever node is selected. */
function nodeCard(title: string): HTMLElement {
  const grid = screen.getByTestId('ask-canvas-view');
  const label = within(grid).getByText(title);
  const card = label.closest('[data-state]');
  if (!(card instanceof HTMLElement)) throw new Error(`no node card for ${title}`);
  return card;
}

describe('PinnedCanvasArtifact', () => {
  it('renders "unknown" for a snapshot pinned with no checked commit sha', () => {
    renderArtifact({ checkedCommitSha: null });

    // The sha renders in its own span, so an exact-text query addresses the
    // fallback rather than the label line around it.
    expect(screen.getByText('unknown')).toBeInTheDocument();
  });

  it('renders the checked commit sha when the snapshot carries one', () => {
    renderArtifact({ checkedCommitSha: 'abc1234' });

    expect(screen.getByText('abc1234')).toBeInTheDocument();
    expect(screen.queryByText('unknown')).not.toBeInTheDocument();
  });

  it('renders no unresolved block for a snapshot whose nodes cite no paths', () => {
    renderArtifact({ canvasPaths: [] });

    expect(screen.queryByText('Unresolved paths:')).not.toBeInTheDocument();
  });

  it('renders no unresolved block when every cited path resolved', () => {
    renderArtifact({
      canvasPaths: [
        { node_id: 'n1', path: 'src/one.ts', resolved: true },
        { node_id: 'n2', path: 'src/two.ts', resolved: true },
      ],
    });

    expect(screen.queryByText('Unresolved paths:')).not.toBeInTheDocument();
    expect(screen.queryByText('src/two.ts')).not.toBeInTheDocument();
  });

  it('lists exactly the unresolved paths in the amber block', () => {
    renderArtifact({
      canvasPaths: [
        { node_id: 'n1', path: 'src/one.ts', resolved: true },
        { node_id: 'n2', path: 'src/gone.ts', resolved: false },
      ],
    });

    const block = screen.getByText('Unresolved paths:').parentElement;
    expect(block).toHaveTextContent('src/gone.ts');
    expect(block).not.toHaveTextContent('src/one.ts');
  });

  it('lists both verdicts when two share a node id, without a duplicate-key warning', () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});

    renderArtifact({
      canvasPaths: [
        { node_id: 'n1', path: 'src/gone.ts', resolved: false },
        { node_id: 'n1', path: 'src/also-gone.ts', resolved: false },
      ],
    });

    const block = screen.getByText('Unresolved paths:').parentElement;
    expect(block).toHaveTextContent('src/gone.ts');
    expect(block).toHaveTextContent('src/also-gone.ts');
    expect(
      consoleError.mock.calls.some((call) => call.some((arg) => String(arg).includes('same key'))),
    ).toBe(false);

    consoleError.mockRestore();
  });

  it('selects a path-bearing node on click, the way the live pane does', () => {
    renderArtifact({ canvasPaths: [{ node_id: 'n1', path: 'src/one.ts', resolved: true }] });

    expect(nodeCard('Node one')).toHaveAttribute('data-state', 'resting');

    fireEvent.click(nodeCard('Node one'));
    expect(nodeCard('Node one')).toHaveAttribute('data-state', 'selected');
  });

  it('deselects the same node on a second click, the way the live pane does', () => {
    renderArtifact({ canvasPaths: [{ node_id: 'n1', path: 'src/one.ts', resolved: true }] });

    fireEvent.click(nodeCard('Node one'));
    fireEvent.click(nodeCard('Node one'));

    expect(nodeCard('Node one')).toHaveAttribute('data-state', 'resting');
  });

  it('shows the selected node in a detail strip, so the click leads somewhere', () => {
    renderArtifact({ canvasPaths: [{ node_id: 'n1', path: 'src/one.ts', resolved: true }] });

    expect(screen.queryByTestId('pinned-canvas-node-detail')).not.toBeInTheDocument();

    fireEvent.click(nodeCard('Node one'));

    const detail = screen.getByTestId('pinned-canvas-node-detail');
    expect(detail).toHaveTextContent('Node one');
    expect(detail).toHaveTextContent('src/one.ts');
  });
});

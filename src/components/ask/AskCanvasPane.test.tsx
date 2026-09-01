/**
 * `AskCanvasPane` — Acceptance Criteria 3 and 4: a canvas-free completion
 * holds the prior canvas rather than clearing it, and no canvas ever renders
 * while the turn is `setting_up`/`working`.
 *
 * Both cases were watched to fail first: AC3 against a draft that reset the
 * held ref to `null` whenever `lastMessage?.canvas` was `null` (clearing on a
 * canvas-free completion instead of holding), and AC4 against a draft that
 * checked `phase` only *after* falling through to the held-canvas branch —
 * i.e. rendered the previous canvas underneath the fold rather than the fold
 * alone. Both drafts failed the corresponding assertion below before the
 * `phase !== null` early-return (AC4) and the render-time hold-not-clear
 * guard (AC3) were written to make them pass.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const resolveNodeMock = vi.fn();
vi.mock('../../lib/ask', () => ({
  resolveNode: (...args: unknown[]) => resolveNodeMock(...args),
}));

const fetchActiveFeaturesMock = vi.fn();
const listStepsForRunMock = vi.fn();
vi.mock('../../lib/features', () => ({
  fetchActiveFeatures: (...args: unknown[]) => fetchActiveFeaturesMock(...args),
  listStepsForRun: (...args: unknown[]) => listStepsForRunMock(...args),
}));

const navigateMock = vi.fn();
vi.mock('../../context', () => ({
  useNavigation: () => ({ navigate: navigateMock }),
}));

import { AskCanvasPane } from './AskCanvasPane';
import { NO_TURN } from '../../lib/askActivity';
import type { AskStreamStore } from './useAskStream';
import type { AskCanvas, AskMessageView, CanvasNode, NodeResolution } from '../../types';

afterEach(cleanup);

beforeEach(() => {
  resolveNodeMock.mockReset();
  fetchActiveFeaturesMock.mockReset();
  listStepsForRunMock.mockReset();
  navigateMock.mockReset();
  resolveNodeMock.mockReturnValue(new Promise(() => {}));
  fetchActiveFeaturesMock.mockResolvedValue([]);
  listStepsForRunMock.mockResolvedValue([]);
});

const fakeStore: AskStreamStore = {
  subscribe: () => () => {},
  read: () => NO_TURN,
};

function node(overrides: Partial<CanvasNode> = {}): CanvasNode {
  return { id: 'n0', title: 'Node 0', role: 'agent', path: null, stage: 0, lane: 0, ...overrides };
}

function canvas(title: string, nodes: CanvasNode[] = [node({ title })]): AskCanvas {
  return {
    kind: 'journey',
    title,
    stages: ['s0'],
    lanes: ['l0'],
    nodes,
    edges: [],
  };
}

function message(overrides: Partial<AskMessageView> = {}): AskMessageView {
  return {
    id: 'm1',
    thread_id: 't1',
    role: 'assistant',
    text: '',
    cost_usd: null,
    tokens: null,
    turn_activity: null,
    canvas_paths: null,
    checked_commit_sha: null,
    created_at: 0,
    prose: '',
    canvas: null,
    canvas_error: null,
    ...overrides,
  };
}

describe('AskCanvasPane', () => {
  it('holds turn N’s canvas across a canvas-free completion (AC3)', () => {
    const turnNCanvas = canvas('Turn N canvas');
    const { rerender } = render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({ canvas: turnNCanvas })}
        phase={null}
      />,
    );

    expect(screen.getByTestId('ask-canvas-view')).toBeInTheDocument();
    expect(screen.getByRole('img', { name: 'Turn N canvas' })).toBeInTheDocument();

    // Turn N+1 completes with no canvas of its own.
    rerender(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({ canvas: null })}
        phase={null}
      />,
    );

    expect(screen.getByTestId('ask-canvas-view')).toBeInTheDocument();
    expect(screen.getByRole('img', { name: 'Turn N canvas' })).toBeInTheDocument();
  });

  it('never renders the canvas while the turn is working, only the activity fold (AC4)', () => {
    render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({ canvas: canvas('should not show') })}
        phase="working"
      />,
    );

    expect(screen.queryByTestId('ask-canvas-view')).not.toBeInTheDocument();
    expect(screen.queryByRole('img')).not.toBeInTheDocument();
    expect(screen.getByTestId('turn-activity')).toBeInTheDocument();
  });

  it('never renders the canvas while setting up either', () => {
    render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({ canvas: canvas('should not show') })}
        phase="setting_up"
      />,
    );

    expect(screen.queryByTestId('ask-canvas-view')).not.toBeInTheDocument();
    expect(screen.getByTestId('turn-activity')).toBeInTheDocument();
  });

  it('opens the inspector on selecting a resolved node, and closes it on re-activation (AC 8/9)', async () => {
    const n0 = node({ id: 'n0', title: 'Reads the ticket board', role: 'agent', path: 'src/board.rs' });
    const n1 = node({ id: 'n1', title: 'Approves at the Gate', role: 'needs_human', path: null });
    const twoNodeCanvas: AskCanvas = {
      kind: 'journey',
      title: 'Onboarding',
      stages: ['s0'],
      lanes: ['l0'],
      nodes: [n0, n1],
      edges: [{ from: 'n0', to: 'n1', kind: 'hands_off' }],
    };
    resolveNodeMock.mockResolvedValue({
      kind: 'editor',
      machine_id: 'local',
      worktree_path: '/repo',
      branch: 'feature/x',
      default_branch: 'master',
      path: 'src/board.rs',
    } satisfies NodeResolution);

    render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({
          canvas: twoNodeCanvas,
          prose: 'Reads the ticket board before anything runs.',
          canvas_paths: [{ node_id: 'n0', path: 'src/board.rs', resolved: true }],
        })}
        phase={null}
      />,
    );

    expect(screen.queryByTestId('ask-canvas-node-inspector')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTitle('Reads the ticket board'));

    const inspector = await screen.findByTestId('ask-canvas-node-inspector');
    expect(inspector).toHaveTextContent('Reads the ticket board');
    expect(inspector).toHaveTextContent('Reads the ticket board before anything runs.');
    expect(inspector).toHaveTextContent('Approves at the Gate');
    expect(resolveNodeMock).toHaveBeenCalledWith({ threadId: 't1', messageId: 'm1', nodeId: 'n0' });

    fireEvent.click(screen.getByTitle('Reads the ticket board'));
    expect(screen.queryByTestId('ask-canvas-node-inspector')).not.toBeInTheDocument();
  });

  it('surfaces the moved-path sha copy inside the pane’s open inspector', async () => {
    const n0 = node({ id: 'n0', title: 'Reads the ticket board', role: 'agent', path: 'src/board.rs' });
    const oneNodeCanvas = canvas('Onboarding', [n0]);
    resolveNodeMock.mockResolvedValue({
      kind: 'moved',
      checked_commit_sha: 'abc1234567890def',
    } satisfies NodeResolution);

    render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({
          canvas: oneNodeCanvas,
          canvas_paths: [{ node_id: 'n0', path: 'src/board.rs', resolved: true }],
        })}
        phase={null}
      />,
    );

    fireEvent.click(screen.getByTitle('Reads the ticket board'));

    await waitFor(() => expect(screen.getByText(/moved since/i)).toBeInTheDocument());
    expect(screen.getByText(/abc12345/)).toBeInTheDocument();
  });

  it('never opens the inspector on a click against an unresolved node’s rendered card', () => {
    const n0 = node({ id: 'n0', title: 'Not spawned yet', role: 'agent', path: 'src/nope.rs' });
    const oneNodeCanvas = canvas('Onboarding', [n0]);

    render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({
          canvas: oneNodeCanvas,
          canvas_paths: [{ node_id: 'n0', path: 'src/nope.rs', resolved: false }],
        })}
        phase={null}
      />,
    );

    fireEvent.click(screen.getByTitle('Not spawned yet'));

    expect(screen.queryByTestId('ask-canvas-node-inspector')).not.toBeInTheDocument();
    expect(resolveNodeMock).not.toHaveBeenCalled();
  });
});

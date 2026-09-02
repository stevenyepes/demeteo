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
 *
 * Also covers the Pin/Export toolbar and pinned-canvases list: the pin
 * control's accessible name never reads "Pin to docs" (AC5), Pin sends the
 * *held* canvas's message id — not a stale one carried by a later
 * canvas-free completion — and Export never touches the artifact store
 * (AC4 for this ticket).
 *
 * The three "pins are still listed" / "the banner renders" cases were watched
 * to fail against the two early returns that preceded them — they were red
 * with the list and banner nested inside the held-canvas branch, which is the
 * state a thread switch lands in. The modal click-through was watched to fail
 * with `ArtifactRow`'s `onSelect` stubbed to a no-op, and it was the only
 * case that went red.
 *
 * "offers neither Pin nor Export while a turn runs" was watched to fail
 * against the toolbar gated on `held !== null` alone, where the buttons
 * outlived the canvas they act on; it was the only case that went red.
 *
 * "clears the banner once a later refresh succeeds" was watched to fail
 * against `refreshPinned` setting `error` and never clearing it; it too was
 * the only case that went red.
 *
 * The two title cases were watched to fail against a pane that rendered
 * `ArtifactRow` alone: with both pins' names being bare message ids, the
 * "tells two pins apart" assertions were red and the path-only case passed
 * either way, which is the pair that proves the title is what distinguishes
 * them rather than the row.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const resolveNodeMock = vi.fn();

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
import { exportAskCanvas, listPinnedAskCanvases, pinAskCanvas } from '../../lib/ask';
import { NO_TURN } from '../../lib/askActivity';
import type { AskStreamStore } from './useAskStream';
import type { AskCanvas, AskMessageView, CanvasNode, NodeResolution, PinnedCanvasEntry } from '../../types';

// A double per AGENTS.md §7: `listPinnedAskCanvases` defaults to an empty
// list (it fires unconditionally on every mount, whichever branch renders),
// but `pinAskCanvas`/`exportAskCanvas` default to a rejection so a call no
// test explicitly arranged fails loudly instead of resolving as if it had
// succeeded.
vi.mock('../../lib/ask', () => ({
  pinAskCanvas: vi.fn(),
  exportAskCanvas: vi.fn(),
  listPinnedAskCanvases: vi.fn(),
  resolveNode: (...args: unknown[]) => resolveNodeMock(...args),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

beforeEach(() => {
  vi.mocked(listPinnedAskCanvases).mockResolvedValue([]);
  vi.mocked(pinAskCanvas).mockRejectedValue(new Error('pinAskCanvas: no expectation set for this test'));
  vi.mocked(exportAskCanvas).mockRejectedValue(new Error('exportAskCanvas: no expectation set for this test'));
});

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

/** A `listPinnedAskCanvases` row. `title`/`pinned_at` default to `null` —
 *  the shape `list_pinned` degrades an unreadable entry to. */
function pin(path: string, overrides: Partial<PinnedCanvasEntry> = {}): PinnedCanvasEntry {
  return { path, title: null, pinned_at: null, ...overrides };
}

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
  it('holds turn N’s canvas across a canvas-free completion (AC3)', async () => {
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
    await waitFor(() => expect(listPinnedAskCanvases).toHaveBeenCalled());

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

  it('never renders the canvas while the turn is working, only the activity fold (AC4)', async () => {
    render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({ canvas: canvas('should not show') })}
        phase="working"
      />,
    );
    await waitFor(() => expect(listPinnedAskCanvases).toHaveBeenCalled());

    expect(screen.queryByTestId('ask-canvas-view')).not.toBeInTheDocument();
    expect(screen.queryByRole('img')).not.toBeInTheDocument();
    expect(screen.getByTestId('turn-activity')).toBeInTheDocument();
  });

  it('never renders the canvas while setting up either', async () => {
    render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({ canvas: canvas('should not show') })}
        phase="setting_up"
      />,
    );
    await waitFor(() => expect(listPinnedAskCanvases).toHaveBeenCalled());

    expect(screen.queryByTestId('ask-canvas-view')).not.toBeInTheDocument();
    expect(screen.getByTestId('turn-activity')).toBeInTheDocument();
  });

  it('has no Pin/Export toolbar when there is no held canvas yet', async () => {
    render(
      <AskCanvasPane store={fakeStore} threadId="t1" projectId="p1" lastMessage={message({ canvas: null })} phase={null} />,
    );
    await waitFor(() => expect(listPinnedAskCanvases).toHaveBeenCalled());

    expect(screen.getByTestId('ask-canvas-placeholder')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /demeteo/i })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Export' })).not.toBeInTheDocument();
  });

  it('never renders "Pin to docs" and labels the pin control to match /demeteo/i (AC5)', async () => {
    render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({ canvas: canvas('c') })}
        phase={null}
      />,
    );
    await waitFor(() => expect(listPinnedAskCanvases).toHaveBeenCalled());

    expect(screen.queryByText(/pin to docs/i)).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: /demeteo/i })).toBeInTheDocument();
  });

  it("clicking Pin pins the held canvas's message id, even after a canvas-free completion", async () => {
    vi.mocked(pinAskCanvas).mockResolvedValue('artifacts/pinned/m1.canvas.json');
    const { rerender } = render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({ id: 'm1', canvas: canvas('Turn N canvas') })}
        phase={null}
      />,
    );
    await waitFor(() => expect(listPinnedAskCanvases).toHaveBeenCalled());

    // Turn N+1 completes with no canvas of its own — the hold keeps turn N's
    // canvas and, with it, turn N's message id: pinning after this must not
    // pin the stale/lost id of a message that drew nothing.
    rerender(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({ id: 'm2', canvas: null })}
        phase={null}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: /demeteo/i }));

    await waitFor(() => expect(pinAskCanvas).toHaveBeenCalledWith('t1', 'm1'));
  });

  it('re-fetches the pinned list after a successful pin, and the new pin appears as an ArtifactRow', async () => {
    vi.mocked(pinAskCanvas).mockResolvedValue('artifacts/pinned/m1.canvas.json');
    vi.mocked(listPinnedAskCanvases)
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([pin('artifacts/pinned/m1.canvas.json', { title: 'Pinned at last' })]);

    render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({ id: 'm1', canvas: canvas('c') })}
        phase={null}
      />,
    );
    await waitFor(() => expect(listPinnedAskCanvases).toHaveBeenCalledTimes(1));
    expect(screen.queryByText('m1.canvas.json')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /demeteo/i }));

    await waitFor(() => expect(listPinnedAskCanvases).toHaveBeenCalledTimes(2));
    expect(await screen.findByText('m1.canvas.json')).toBeInTheDocument();
  });

  it('lists the thread’s pinned canvases when no canvas is held (M1)', async () => {
    vi.mocked(listPinnedAskCanvases).mockResolvedValue([pin('artifacts/ask-canvas/t1/m1.canvas.json')]);

    render(<AskCanvasPane store={fakeStore} threadId="t1" projectId="p1" lastMessage={null} phase={null} />);

    expect(await screen.findByText('m1.canvas.json')).toBeInTheDocument();
    expect(screen.getByTestId('ask-canvas-placeholder')).toBeInTheDocument();
  });

  it('lists the thread’s pinned canvases while a turn runs, still without a canvas (M1 + AC4)', async () => {
    vi.mocked(listPinnedAskCanvases).mockResolvedValue([pin('artifacts/ask-canvas/t1/m1.canvas.json')]);

    render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({ canvas: canvas('should not show') })}
        phase="working"
      />,
    );

    expect(await screen.findByText('m1.canvas.json')).toBeInTheDocument();
    expect(screen.getByTestId('turn-activity')).toBeInTheDocument();
    expect(screen.queryByTestId('ask-canvas-view')).not.toBeInTheDocument();
  });

  it('offers neither Pin nor Export while a turn runs, even holding a canvas', async () => {
    vi.mocked(listPinnedAskCanvases).mockResolvedValue([pin('artifacts/ask-canvas/t1/m1.canvas.json')]);

    render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({ id: 'm1', canvas: canvas('held but hidden') })}
        phase="working"
      />,
    );

    expect(await screen.findByText('m1.canvas.json')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /demeteo/i })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Export' })).not.toBeInTheDocument();
  });

  it('surfaces a failed pinned-list fetch in the alert banner, in every state', async () => {
    vi.mocked(listPinnedAskCanvases).mockRejectedValue(new Error('list_for_step: disk gone'));

    render(<AskCanvasPane store={fakeStore} threadId="t1" projectId="p1" lastMessage={null} phase={null} />);

    expect(await screen.findByRole('alert')).toHaveTextContent('list_for_step: disk gone');
  });

  it('clears the banner once a later refresh succeeds', async () => {
    vi.mocked(listPinnedAskCanvases)
      .mockRejectedValueOnce(new Error('list_for_step: disk gone'))
      .mockResolvedValue([pin('artifacts/ask-canvas/t2/m1.canvas.json')]);

    const { rerender } = render(
      <AskCanvasPane store={fakeStore} threadId="t1" projectId="p1" lastMessage={null} phase={null} />,
    );
    expect(await screen.findByRole('alert')).toHaveTextContent('list_for_step: disk gone');

    // A thread switch inside one mount: `refreshPinned` changes identity, the
    // mount effect re-fires, and this time the fetch resolves. The pane keeps
    // its `error` across that rerender, so only the success path can clear it.
    rerender(<AskCanvasPane store={fakeStore} threadId="t2" projectId="p1" lastMessage={null} phase={null} />);

    expect(await screen.findByText('m1.canvas.json')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('tells two pins apart by their canvas titles, and still opens the one clicked', async () => {
    vi.mocked(listPinnedAskCanvases).mockResolvedValue([
      pin('artifacts/ask-canvas/t1/m1.canvas.json', {
        title: 'Gate approval flow',
        pinned_at: 1_700_000_000_000,
      }),
      pin('artifacts/ask-canvas/t1/m2.canvas.json', {
        title: 'Worktree lifecycle',
        pinned_at: 1_700_000_060_000,
      }),
    ]);

    render(<AskCanvasPane store={fakeStore} threadId="t1" projectId="p1" lastMessage={null} phase={null} />);

    expect(await screen.findByText('Gate approval flow')).toBeInTheDocument();
    expect(screen.getByText('Worktree lifecycle')).toBeInTheDocument();
    expect(screen.getByText(new Date(1_700_000_000_000).toLocaleString())).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /m2\.canvas\.json/ }));

    expect(await screen.findByTestId('artifact-modal-title')).toHaveTextContent('m2.canvas.json');
  });

  it('renders a pin whose snapshot could not be read as a path-only row', async () => {
    vi.mocked(listPinnedAskCanvases).mockResolvedValue([
      pin('artifacts/ask-canvas/t1/m1.canvas.json'),
    ]);

    render(<AskCanvasPane store={fakeStore} threadId="t1" projectId="p1" lastMessage={null} phase={null} />);

    expect(await screen.findByRole('button', { name: /m1\.canvas\.json/ })).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('opens the artifact modal on the clicked pin', async () => {
    vi.mocked(listPinnedAskCanvases).mockResolvedValue([
      pin('artifacts/ask-canvas/t1/m1.canvas.json'),
      pin('artifacts/ask-canvas/t1/m2.canvas.json'),
    ]);

    render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({ id: 'm1', canvas: canvas('c') })}
        phase={null}
      />,
    );

    fireEvent.click(await screen.findByRole('button', { name: /m2\.canvas\.json/ }));

    expect(await screen.findByTestId('artifact-modal-title')).toHaveTextContent('m2.canvas.json');
  });

  it("clicking Export downloads a Blob built from exportAskCanvas's return value, without pinning (AC4)", async () => {
    vi.mocked(exportAskCanvas).mockResolvedValue('{"kind":"journey"}');
    const createObjectURL = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock-url');
    const revokeObjectURL = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});

    render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
        projectId="p1"
        lastMessage={message({ id: 'm1', canvas: canvas('c') })}
        phase={null}
      />,
    );
    await waitFor(() => expect(listPinnedAskCanvases).toHaveBeenCalled());

    fireEvent.click(screen.getByRole('button', { name: 'Export' }));

    await waitFor(() => expect(exportAskCanvas).toHaveBeenCalledWith('t1', 'm1'));
    expect(createObjectURL).toHaveBeenCalledTimes(1);
    const blobArg = createObjectURL.mock.calls[0][0];
    if (!(blobArg instanceof Blob)) throw new Error('expected Export to revoke a Blob URL');
    expect(blobArg.type).toBe('application/json');
    expect(clickSpy).toHaveBeenCalledTimes(1);
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:mock-url');
    expect(pinAskCanvas).not.toHaveBeenCalled();

    clickSpy.mockRestore();
    createObjectURL.mockRestore();
    revokeObjectURL.mockRestore();
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

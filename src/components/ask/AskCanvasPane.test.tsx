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
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { AskCanvasPane } from './AskCanvasPane';
import { NO_TURN } from '../../lib/askActivity';
import type { AskStreamStore } from './useAskStream';
import type { AskCanvas, AskMessageView } from '../../types';

afterEach(cleanup);

const fakeStore: AskStreamStore = {
  subscribe: () => () => {},
  read: () => NO_TURN,
};

function canvas(title: string): AskCanvas {
  return {
    kind: 'journey',
    title,
    stages: ['s0'],
    lanes: ['l0'],
    nodes: [{ id: 'n0', title, role: 'agent', path: null, stage: 0, lane: 0 }],
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
        lastMessage={message({ canvas: turnNCanvas })}
        phase={null}
      />,
    );

    expect(screen.getByTestId('ask-canvas-view')).toBeInTheDocument();
    expect(screen.getByRole('img', { name: 'Turn N canvas' })).toBeInTheDocument();

    // Turn N+1 completes with no canvas of its own.
    rerender(
      <AskCanvasPane store={fakeStore} threadId="t1" lastMessage={message({ canvas: null })} phase={null} />,
    );

    expect(screen.getByTestId('ask-canvas-view')).toBeInTheDocument();
    expect(screen.getByRole('img', { name: 'Turn N canvas' })).toBeInTheDocument();
  });

  it('never renders the canvas while the turn is working, only the activity fold (AC4)', () => {
    render(
      <AskCanvasPane
        store={fakeStore}
        threadId="t1"
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
        lastMessage={message({ canvas: canvas('should not show') })}
        phase="setting_up"
      />,
    );

    expect(screen.queryByTestId('ask-canvas-view')).not.toBeInTheDocument();
    expect(screen.getByTestId('turn-activity')).toBeInTheDocument();
  });
});

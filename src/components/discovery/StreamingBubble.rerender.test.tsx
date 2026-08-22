// The streaming bubble wakes on every coalesced frame of a turn. What must not
// follow it at that rate is the Markdown parse, over a document that is longer
// on every wake. A counting react-markdown stub is how `ArtifactViewer.rerender.test.tsx`
// pins the same property for the run surface.

import { act, cleanup, render } from '@testing-library/react';
import { useRef, type ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { NO_TURN, openTurn, type LiveTurn } from '../../lib/discoveryActivity';
import { StreamingBubble } from './StreamingBubble';
import type { DiscoveryStreamStore } from './useDiscoveryStream';

let markdownRenders = 0;

vi.mock('react-markdown', () => ({
  default: ({ children }: { children?: ReactNode }) => {
    markdownRenders += 1;
    return <div data-testid="markdown-body">{children}</div>;
  },
}));

function fakeStream() {
  let turn: LiveTurn = openTurn(Date.now(), 'working');
  const listeners = new Set<() => void>();
  const store: DiscoveryStreamStore = {
    subscribe: (_id, onChange) => {
      listeners.add(onChange);
      return () => listeners.delete(onChange);
    },
    read: () => turn,
  };
  return {
    store,
    delta(text: string) {
      turn = { ...turn, text: turn.text + text };
      for (const onChange of [...listeners]) onChange();
    },
  };
}

function Host({ store }: { store: DiscoveryStreamStore }) {
  const scroller = useRef<HTMLDivElement | null>(null);
  return <StreamingBubble store={store} discoveryId="d-1" scroller={scroller} />;
}

beforeEach(() => {
  markdownRenders = 0;
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

describe('a turn arriving one chunk per frame', () => {
  it('does not re-parse the markdown on every frame', () => {
    const { store, delta } = fakeStream();
    render(<Host store={store} />);
    const afterMount = markdownRenders;

    for (let i = 0; i < 30; i += 1) {
      act(() => void delta('word '));
      act(() => void vi.advanceTimersByTime(16));
    }

    // Thirty frames is 480 ms: one publication, or two. Never thirty, which is
    // what this cost before the throttle and what it costs again if it goes.
    const parses = markdownRenders - afterMount;
    expect(parses).toBeGreaterThanOrEqual(1);
    expect(parses).toBeLessThanOrEqual(2);
  });

  it('still shows every chunk once the interval is up', () => {
    const { store, delta } = fakeStream();
    const { getByTestId } = render(<Host store={store} />);

    act(() => void delta('Checked the tree '));
    act(() => void vi.advanceTimersByTime(16));
    act(() => void delta('first.'));
    act(() => void vi.advanceTimersByTime(500));

    expect(getByTestId('markdown-body').textContent).toBe('Checked the tree first.');
  });
});

describe('a turn that has said nothing yet', () => {
  it('renders the activity strip and no prose', () => {
    const store: DiscoveryStreamStore = { subscribe: () => () => {}, read: () => NO_TURN };
    const { getByTestId } = render(<Host store={store} />);

    expect(getByTestId('turn-activity')).toBeInTheDocument();
    expect(getByTestId('markdown-body').textContent).toBe('');
  });
});

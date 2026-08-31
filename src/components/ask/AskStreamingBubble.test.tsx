// `AskStreamingBubble` is the sole subscriber to a thread's live turn while
// one runs — mirrors `StreamingBubble.tsx`'s doc comment. This pins two
// things: it renders `AskActivityStrip` above the prose, and it never
// renders empty — the strip stands in for a turn that has said nothing yet.

import { act, cleanup, render, screen } from '@testing-library/react';
import { useRef, type ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { NO_TURN, openTurn, type LiveTurn, type ToolActivity } from '../../lib/askActivity';
import { AskStreamingBubble } from './AskStreamingBubble';
import type { AskStreamStore } from './useAskStream';

vi.mock('react-markdown', () => ({
  default: ({ children }: { children?: ReactNode }) => <div data-testid="markdown-body">{children}</div>,
}));

function fakeStream(initial: LiveTurn = openTurn(Date.now(), 'working')) {
  let turn: LiveTurn = initial;
  const listeners = new Set<() => void>();
  const store: AskStreamStore = {
    subscribe: (_id, onChange) => {
      listeners.add(onChange);
      return () => listeners.delete(onChange);
    },
    read: () => turn,
  };
  return {
    store,
    setTurn(next: LiveTurn) {
      turn = next;
      for (const onChange of [...listeners]) onChange();
    },
  };
}

function fetched(id: string, target: string): ToolActivity {
  return { id, kind: 'fetch', target, done: true, failed: false };
}

function Host({ store }: { store: AskStreamStore }) {
  const scroller = useRef<HTMLDivElement | null>(null);
  return <AskStreamingBubble store={store} threadId="t-1" scroller={scroller} />;
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

describe('AskStreamingBubble', () => {
  it('renders the activity strip even before any text has arrived', () => {
    const { store } = fakeStream();
    render(<Host store={store} />);

    expect(screen.getByTestId('ask-streaming-bubble')).toBeTruthy();
    expect(screen.getByTestId('turn-activity')).toBeTruthy();
  });

  it('shows delta text as it streams in, throttled rather than dropped', () => {
    const { store, setTurn } = fakeStream();
    render(<Host store={store} />);

    act(() => {
      setTurn(openTurn(Date.now(), 'working'));
      setTurn({ ...openTurn(Date.now(), 'working'), text: 'Reading the repo' });
    });
    act(() => {
      vi.advanceTimersByTime(300);
    });

    expect(screen.getByTestId('markdown-body').textContent).toBe('Reading the repo');
  });

  it('falls back to the resting turn (never crashes, never blank) for a store with nothing yet', () => {
    render(<Host store={{ subscribe: () => () => {}, read: () => NO_TURN }} />);

    expect(screen.getByTestId('ask-streaming-bubble')).toBeTruthy();
  });

  it('lists the distinct URLs the turn fetched, once each, in first-seen order', () => {
    const { store } = fakeStream({
      ...openTurn(Date.now(), 'working'),
      ledger: [
        fetched('c-1', 'https://v2.tauri.app/security/capabilities'),
        { id: 'c-2', kind: 'read', target: 'src/App.tsx', done: true, failed: false },
        fetched('c-3', 'https://docs.rs/serde'),
        fetched('c-4', 'https://v2.tauri.app/security/capabilities'),
      ],
    });
    render(<Host store={store} />);

    const urls = screen
      .getAllByTestId('ask-source')
      .map((element) => element.textContent);
    expect(urls).toEqual(['https://v2.tauri.app/security/capabilities', 'https://docs.rs/serde']);
  });

  it('renders no Sources list at all for a turn that fetched nothing', () => {
    const { store } = fakeStream({
      ...openTurn(Date.now(), 'working'),
      ledger: [{ id: 'c-1', kind: 'read', target: 'src/App.tsx', done: true, failed: false }],
    });
    render(<Host store={store} />);

    expect(screen.queryByTestId('ask-sources')).toBeNull();
    expect(screen.queryByText(/Sources/)).toBeNull();
  });
});

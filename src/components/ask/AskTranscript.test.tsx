// Acceptance Criterion: `AskTranscript.tsx` renders `.prose`, never raw
// `.text`, for assistant messages — a canvas-carrying turn's `.text` still
// has the canvas JSON glued to the end of it (`AskMessageView`'s own doc
// comment).

import { cleanup, render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { AskTranscript } from './AskTranscript';
import { NO_TURN } from '../../lib/askActivity';
import type { AskMessageView } from '../../types';
import type { AskStreamStore } from './useAskStream';

vi.mock('react-markdown', () => ({
  default: ({ children }: { children?: ReactNode }) => <div data-testid="markdown-body">{children}</div>,
}));

afterEach(cleanup);

const fakeStore: AskStreamStore = {
  subscribe: () => () => {},
  read: () => NO_TURN,
};

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

describe('AskTranscript', () => {
  it('renders the prose, not the raw text still carrying the canvas block', () => {
    render(
      <AskTranscript
        threadId="t1"
        pending={false}
        store={fakeStore}
        messages={[
          message({
            text: 'Here is the answer.\n```json\n{"kind":"journey"}\n```',
            prose: 'Here is the answer.',
          }),
        ]}
      />,
    );

    expect(screen.getByTestId('markdown-body').textContent).toBe('Here is the answer.');
    expect(screen.queryByText(/"kind":"journey"/)).toBeNull();
  });

  it("renders a user message's own text verbatim, unparsed", () => {
    render(
      <AskTranscript
        threadId="t1"
        pending={false}
        store={fakeStore}
        messages={[message({ role: 'user', text: 'What does this repo do?', prose: 'unused' })]}
      />,
    );

    expect(screen.getByTestId('ask-transcript-user').textContent).toContain('What does this repo do?');
  });

  it('renders a failed-to-parse canvas block beneath the bubble, without dropping the prose', () => {
    render(
      <AskTranscript
        threadId="t1"
        pending={false}
        store={fakeStore}
        messages={[
          message({
            prose: 'The turn still said something.',
            canvas_error: 'unexpected end of JSON input',
          }),
        ]}
      />,
    );

    expect(screen.getByTestId('markdown-body').textContent).toBe('The turn still said something.');
    expect(screen.getByTestId('ask-transcript-canvas-error').textContent).toContain(
      'unexpected end of JSON input',
    );
  });

  it('mounts the streaming bubble instead of nothing while a turn is pending', () => {
    render(<AskTranscript threadId="t1" pending={true} store={fakeStore} messages={[]} />);

    expect(screen.getByTestId('ask-streaming-bubble')).toBeTruthy();
  });
});

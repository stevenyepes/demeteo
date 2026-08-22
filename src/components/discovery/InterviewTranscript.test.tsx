// Real react-markdown here too: the point of these is which half of the
// transcript is parsed and which half is not, and a stub answers both the same.

import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { InterviewTranscript } from './InterviewTranscript';
import { NO_TURN } from '../../lib/discoveryActivity';
import type { TranscriptBlock, TranscriptBubble } from '../../lib/discoveryInterview';
import type { DiscoveryStreamStore } from './useDiscoveryStream';

afterEach(cleanup);

const STORE: DiscoveryStreamStore = {
  subscribe: () => () => {},
  read: () => NO_TURN,
};

function bubble(role: 'user' | 'assistant', text: string): TranscriptBubble {
  return {
    kind: 'bubble',
    key: `${role}-1`,
    role,
    text,
    meta: null,
    questionError: null,
    reseeded: false,
  };
}

function renderTranscript(blocks: TranscriptBlock[]) {
  return render(
    <InterviewTranscript
      discoveryId="d-1"
      blocks={blocks}
      openQuestion={null}
      pending={false}
      store={STORE}
      onPick={vi.fn()}
      onAnswerInOwnWords={vi.fn()}
    />,
  );
}

describe('an interviewer turn', () => {
  it('reaches the DOM as elements, not as literal marks', () => {
    renderTranscript([
      bubble(
        'assistant',
        'Parallel *topology* already exists — `workflow_v2` has fan-out edges.\n\n- `all_success`\n- `any_success`\n',
      ),
    ]);

    const bubbleEl = screen.getByTestId('transcript-assistant');
    expect(bubbleEl.querySelector('code')?.textContent).toBe('workflow_v2');
    expect(bubbleEl.querySelector('em')?.textContent).toBe('topology');
    expect(bubbleEl.querySelectorAll('li')).toHaveLength(2);
    expect(bubbleEl.textContent).not.toContain('`');
  });
});

describe('a turn whose question was refused', () => {
  it('says why instead of rendering the block that caused it', () => {
    renderTranscript([
      {
        ...bubble('assistant', 'Two things it leaves open.'),
        questionError: 'the question has no `text`',
      },
    ]);

    expect(screen.getByTestId('transcript-question-error').textContent).toContain(
      'the question has no `text`',
    );
    expect(screen.getByTestId('transcript-assistant').textContent).not.toContain('"question"');
  });
});

describe('a user turn', () => {
  const TYPED = 'Keep the **stars** and the `ticks` exactly as I typed them.';

  it('is not markdown-rendered', () => {
    renderTranscript([bubble('user', TYPED)]);

    const bubbleEl = screen.getByTestId('transcript-user');
    expect(bubbleEl.querySelector('code')).toBeNull();
    expect(bubbleEl.querySelector('strong')).toBeNull();
    expect(bubbleEl.textContent).toContain(TYPED);
  });

  it('keeps its own line breaks', () => {
    renderTranscript([bubble('user', 'one\ntwo')]);

    const bubbleEl = screen.getByTestId('transcript-user').querySelector('.chat-bubble');
    expect(bubbleEl?.className).toContain('whitespace-pre-wrap');
  });
});

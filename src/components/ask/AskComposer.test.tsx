// Acceptance Criterion 2: submitting the composer appends the user's message
// to the transcript before any `ask_agent_event` arrives, and the composer
// disables during both `setting_up` and `working`.

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { AskComposer } from './AskComposer';
import { sendAskTurn } from '../../lib/ask';
import type { AskMessage } from '../../types';

vi.mock('../../lib/ask', () => ({
  sendAskTurn: vi.fn(),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function message(overrides: Partial<AskMessage> = {}): AskMessage {
  return {
    id: 'm1',
    thread_id: 't1',
    role: 'user',
    text: 'What does this repo do?',
    cost_usd: null,
    tokens: null,
    turn_activity: null,
    canvas_paths: null,
    checked_commit_sha: null,
    created_at: 0,
    ...overrides,
  };
}

function renderComposer(props: Partial<React.ComponentProps<typeof AskComposer>> = {}) {
  const begin = vi.fn();
  const end = vi.fn();
  const onSent = vi.fn();
  const utils = render(
    <AskComposer threadId="t1" phase={null} begin={begin} end={end} onSent={onSent} {...props} />,
  );
  return { ...utils, begin, end, onSent };
}

describe('AskComposer', () => {
  it('appends the persisted user message synchronously, before any event could arrive', async () => {
    const stored = message();
    vi.mocked(sendAskTurn).mockResolvedValue(stored);
    const { onSent, begin } = renderComposer();

    fireEvent.change(screen.getByTestId('ask-composer'), {
      target: { value: 'What does this repo do?' },
    });
    fireEvent.click(screen.getByTestId('ask-composer-send'));

    expect(begin).toHaveBeenCalledWith('t1', 'setting_up');
    await waitFor(() => expect(onSent).toHaveBeenCalledWith(stored));
    expect(sendAskTurn).toHaveBeenCalledWith('t1', 'What does this repo do?');
    expect(screen.getByTestId<HTMLInputElement>('ask-composer').value).toBe('');
  });

  it('disables while the turn is setting_up', () => {
    renderComposer({ phase: 'setting_up' });

    expect(screen.getByTestId('ask-composer').hasAttribute('disabled')).toBe(true);
    expect(screen.getByTestId('ask-composer-send').hasAttribute('disabled')).toBe(true);
  });

  it('disables while the turn is working, the same as setting_up', () => {
    renderComposer({ phase: 'working' });

    expect(screen.getByTestId('ask-composer').hasAttribute('disabled')).toBe(true);
  });

  it('re-enables once phase returns to null, whatever ending caused it', () => {
    const { rerender } = renderComposer({ phase: 'working' });
    expect(screen.getByTestId('ask-composer').hasAttribute('disabled')).toBe(true);

    rerender(
      <AskComposer
        threadId="t1"
        phase={null}
        begin={vi.fn()}
        end={vi.fn()}
        onSent={vi.fn()}
      />,
    );

    expect(screen.getByTestId('ask-composer').hasAttribute('disabled')).toBe(false);
  });

  it('drops the optimistic turn and surfaces the error when the send is rejected', async () => {
    vi.mocked(sendAskTurn).mockRejectedValue(new Error('thread is closed'));
    const { end, onSent } = renderComposer();

    fireEvent.change(screen.getByTestId('ask-composer'), { target: { value: 'ping' } });
    fireEvent.click(screen.getByTestId('ask-composer-send'));

    await waitFor(() => expect(end).toHaveBeenCalledWith('t1'));
    expect(onSent).not.toHaveBeenCalled();
    expect(screen.getByRole('alert').textContent).toContain('thread is closed');
    expect(screen.getByTestId('ask-composer').hasAttribute('disabled')).toBe(false);
    expect(screen.getByTestId<HTMLInputElement>('ask-composer').value).toBe('ping');
  });

  it('disables the send button for blank input', () => {
    renderComposer();

    expect(screen.getByTestId('ask-composer-send').hasAttribute('disabled')).toBe(true);
  });
});

// `DISCOVERY_UI_SPEC.md` §6.6 records that the mock draws `1`/`2`/`3` keycaps
// and wires none of them. These pin the handler, and the rule that the
// free-text option submits nothing.

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { QuestionCard } from './QuestionCard';
import type { DiscoveryQuestion } from '../../types';

afterEach(cleanup);

const QUESTION: DiscoveryQuestion = {
  header: 'Identity',
  text: 'How should a client prove who it is?',
  options: [
    { id: 'keypair', label: 'An ed25519 keypair per client', description: 'One key per laptop.' },
    { id: 'token', label: 'One shared token', description: 'What the old sketch says.' },
    { id: 'ssh', label: 'Reuse the SSH host key', description: 'No new secret anywhere.' },
  ],
  recommended: 'keypair',
};

function renderCard(overrides: { live?: boolean; pending?: boolean } = {}) {
  const onPick = vi.fn();
  const onAnswerInOwnWords = vi.fn();
  render(
    <QuestionCard
      question={QUESTION}
      answer={null}
      live={overrides.live ?? true}
      pending={overrides.pending ?? false}
      onPick={onPick}
      onAnswerInOwnWords={onAnswerInOwnWords}
    />,
  );
  return { onPick, onAnswerInOwnWords };
}

describe('the number keys', () => {
  it('answer with the option at that position', () => {
    const { onPick } = renderCard();

    fireEvent.keyDown(window, { key: '2' });

    expect(onPick).toHaveBeenCalledWith(QUESTION.options[1]);
  });

  it('do nothing while a turn is still streaming', () => {
    const { onPick } = renderCard({ pending: true });

    fireEvent.keyDown(window, { key: '1' });

    expect(onPick).not.toHaveBeenCalled();
  });

  it('do nothing on a settled question', () => {
    const { onPick } = renderCard({ live: false });

    fireEvent.keyDown(window, { key: '1' });

    expect(onPick).not.toHaveBeenCalled();
  });

  it('leave typing alone', () => {
    const { onPick } = renderCard();
    const composer = document.createElement('input');
    document.body.appendChild(composer);

    fireEvent.keyDown(composer, { key: '2' });

    expect(onPick).not.toHaveBeenCalled();
    composer.remove();
  });

  it('ignore a key with no option behind it', () => {
    const { onPick } = renderCard();

    fireEvent.keyDown(window, { key: '9' });

    expect(onPick).not.toHaveBeenCalled();
  });
});

describe('answering in your own words', () => {
  it('focuses the composer and submits nothing', () => {
    const { onPick, onAnswerInOwnWords } = renderCard();

    fireEvent.click(screen.getByTestId('question-free-text'));

    expect(onAnswerInOwnWords).toHaveBeenCalledTimes(1);
    expect(onPick).not.toHaveBeenCalled();
  });

  it('is offered as an option rather than a fallback', () => {
    renderCard();

    expect(screen.getByText('Something else')).toBeTruthy();
    expect(
      screen.getByText(/takes it as written rather than fitting it to the nearest option/),
    ).toBeTruthy();
  });
});

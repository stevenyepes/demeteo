// "One open question at a time" is derived, not stored
// (`docs/TASKS_DISCOVERY.md`, "The interview turn contract"). These pin that
// derivation, and the rule that answering by option and answering in words are
// the same turn — differing only in what the next prompt carries.

import { describe, expect, it } from 'vitest';

import {
  buildTranscript,
  nothingLeftToSettle,
  openQuestionKey,
  optionForKey,
} from './discoveryInterview';
import type { DiscoveryMessageView, DiscoveryQuestion } from '../types';

const QUESTION: DiscoveryQuestion = {
  header: 'First move',
  text: 'Which do you want to settle first?',
  options: [
    { id: 'identity-first', label: 'Identity, then leases', description: '' },
    { id: 'leases-first', label: 'Leases, then identity', description: '' },
    { id: 'both', label: 'Both in one pass', description: '' },
  ],
  recommended: 'identity-first',
};

function assistant(
  id: string,
  prose: string,
  question: DiscoveryQuestion | null = null,
  settled = false,
): DiscoveryMessageView {
  return {
    id,
    discovery_id: 'dsc-1',
    role: 'assistant',
    content: prose,
    cost_usd: null,
    tokens: null,
    created_at: 0,
    prose,
    question,
    nothing_left_to_settle: settled,
    question_error: null,
  };
}

function user(id: string, content: string): DiscoveryMessageView {
  return {
    id,
    discovery_id: 'dsc-1',
    role: 'user',
    content,
    cost_usd: null,
    tokens: null,
    created_at: 0,
    prose: content,
    question: null,
    nothing_left_to_settle: false,
    question_error: null,
  };
}

describe('the open question', () => {
  it('is the last one with no answer recorded after it', () => {
    const blocks = buildTranscript([
      assistant('m1', 'Two things are open.', QUESTION),
      user('m2', 'Identity, then leases'),
      assistant('m3', 'Then the registry lands first.', QUESTION),
    ]);

    expect(openQuestionKey(blocks)).toBe('m3-question');
  });

  it('is nothing once every question has an answer after it', () => {
    const blocks = buildTranscript([
      assistant('m1', 'Two things are open.', QUESTION),
      user('m2', 'Identity, then leases'),
    ]);

    expect(openQuestionKey(blocks)).toBeNull();
  });
});

describe('how a question was settled', () => {
  it('records the option when the answer is one of the labels', () => {
    const [question] = buildTranscript([
      assistant('m1', '', QUESTION),
      user('m2', 'Both in one pass'),
    ]);

    expect(question).toMatchObject({ kind: 'question', answer: { optionId: 'both' } });
  });

  it('takes an answer in the user’s own words as written', () => {
    const [question] = buildTranscript([
      assistant('m1', '', QUESTION),
      user('m2', 'Neither — settle revocation first.'),
    ]);

    expect(question).toMatchObject({
      kind: 'question',
      answer: { optionId: null, text: 'Neither — settle revocation first.' },
    });
  });
});

describe('optionForKey', () => {
  it('maps the keycaps to the options in order', () => {
    expect(optionForKey('1', QUESTION)).toBe('identity-first');
    expect(optionForKey('3', QUESTION)).toBe('both');
  });

  it('answers nothing for a key with no option behind it', () => {
    expect(optionForKey('4', QUESTION)).toBeNull();
    expect(optionForKey('Enter', QUESTION)).toBeNull();
    expect(optionForKey('a', QUESTION)).toBeNull();
  });
});

describe('nothingLeftToSettle', () => {
  it('reads the last assistant turn, not any of them', () => {
    expect(
      nothingLeftToSettle([
        assistant('m1', 'Nothing left.', null, true),
        user('m2', 'One more thing.'),
        assistant('m3', 'Then here is a question.', QUESTION, false),
      ]),
    ).toBe(false);
  });
});

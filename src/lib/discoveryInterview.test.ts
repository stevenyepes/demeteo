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
    activity: null,
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
    activity: null,
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

// The meta line under a settled bubble is the only record a user gets of what
// a turn did once it stops streaming. It has to omit what was never measured
// rather than report it as zero, and it must never claim a re-seed happened.
describe('the meta line under a settled turn', () => {
  function spent(message: DiscoveryMessageView): DiscoveryMessageView {
    return { ...message, cost_usd: 0.31, tokens: 12400 };
  }

  function bubble(blocks: ReturnType<typeof buildTranscript>, key: string) {
    const block = blocks.find((b) => b.kind === 'bubble' && b.key === key);
    if (block === undefined || block.kind !== 'bubble') throw new Error(`no bubble ${key}`);
    return block;
  }

  it('says what the turn did beside what it cost', () => {
    const turn = {
      ...spent(assistant('m1', 'Here is what I found.')),
      activity: { reads: 6, edits: 0, writes: 0, ran: 2, commands: ['git log -20', 'rg auth'] },
    };
    expect(bubble(buildTranscript([turn]), 'm1').meta).toBe(
      '6 reads · ran 2 commands (git log, rg) · 12.4k tokens · $0.310',
    );
  });

  it('omits the activity entirely for a turn stored before it was collected', () => {
    const turn = spent(assistant('m1', 'Here is what I found.'));
    expect(turn.activity).toBeNull();
    expect(bubble(buildTranscript([turn]), 'm1').meta).toBe('12.4k tokens · $0.310');
  });

  it('says nothing at all about a turn nothing was recorded for', () => {
    expect(bubble(buildTranscript([assistant('m1', 'Hello.')]), 'm1').meta).toBeNull();
  });

  it('reports a re-seed only for the turn that was told of one', () => {
    const blocks = buildTranscript(
      [assistant('m1', 'First.'), user('m2', 'Go on.'), assistant('m3', 'Second.')],
      new Set(['m3']),
    );
    expect(bubble(blocks, 'm1').reseeded).toBe(false);
    expect(bubble(blocks, 'm2').reseeded).toBe(false);
    expect(bubble(blocks, 'm3').reseeded).toBe(true);
  });

  it('claims no re-seed when nothing said one happened', () => {
    const blocks = buildTranscript([assistant('m1', 'First.')]);
    expect(bubble(blocks, 'm1').reseeded).toBe(false);
  });
});

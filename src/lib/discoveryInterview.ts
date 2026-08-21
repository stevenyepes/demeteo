import type { DiscoveryMessageView, DiscoveryQuestion } from '../types';
import { formatCost, formatTokens } from './utils';

/**
 * The transcript as the interview column renders it, derived from the stored
 * messages and nothing else.
 *
 * **Which question is open is derived here, not stored.**
 * `docs/TASKS_DISCOVERY.md` ("The interview turn contract") settles that: the
 * open question is the last one with no answer recorded after it, so a retried
 * turn has nothing to reconcile and there is no `is_open` column to drift.
 *
 * **Answering by option and answering in words are the same turn.** The user
 * message that follows a question *is* its answer either way; the only
 * difference is whether that text is one of the options' labels, which is what
 * `optionId` records. Neither is a degraded form of the other
 * (`DISCOVERY_UI_SPEC.md` §6.7).
 */

export interface TranscriptBubble {
  kind: 'bubble';
  key: string;
  role: 'user' | 'assistant';
  text: string;
  /** Spend beneath the bubble, `null` when the harness reported none —
   *  distinct from `0`, which is a measurement. */
  meta: string | null;
  /** The turn carried a question block that would not parse. */
  questionError: string | null;
}

/** What settled a question: an option the user picked, or their own words. */
export interface QuestionAnswer {
  /** `null` when the answer matched no option — the free-text case. */
  optionId: string | null;
  text: string;
}

export interface TranscriptQuestion {
  kind: 'question';
  key: string;
  question: DiscoveryQuestion;
  /** `null` while nothing has been recorded after it. */
  answer: QuestionAnswer | null;
}

export type TranscriptBlock = TranscriptBubble | TranscriptQuestion;

export function buildTranscript(messages: DiscoveryMessageView[]): TranscriptBlock[] {
  const blocks: TranscriptBlock[] = [];

  messages.forEach((message, index) => {
    if (message.role === 'user') {
      blocks.push({
        kind: 'bubble',
        key: message.id,
        role: 'user',
        text: message.content,
        meta: null,
        questionError: null,
      });
      return;
    }

    const prose = message.prose.trim();
    if (prose.length > 0 || message.question === null) {
      blocks.push({
        kind: 'bubble',
        key: message.id,
        role: 'assistant',
        text: prose.length > 0 ? prose : message.content,
        meta: spendMeta(message),
        questionError: message.question_error,
      });
    }

    if (message.question) {
      blocks.push({
        kind: 'question',
        key: `${message.id}-question`,
        question: message.question,
        answer: answerAfter(messages, index, message.question),
      });
    }
  });

  return blocks;
}

/** The question awaiting the user, or `null` when nothing is outstanding. */
export function openQuestionKey(blocks: TranscriptBlock[]): string | null {
  for (let i = blocks.length - 1; i >= 0; i -= 1) {
    const block = blocks[i];
    if (block.kind === 'question' && block.answer === null) return block.key;
  }
  return null;
}

/**
 * Whether the interviewer has said there is nothing left to settle.
 *
 * Read off the last assistant turn rather than any: an earlier turn saying so
 * was answered by everything that came after it.
 */
export function nothingLeftToSettle(messages: DiscoveryMessageView[]): boolean {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i];
    if (message.role === 'assistant') return message.nothing_left_to_settle;
  }
  return false;
}

/**
 * The option a keycap selects. `1`–`9` map to the options in order; the free
 * text option carries `↵` and is not reachable this way, because it submits
 * nothing.
 */
export function optionForKey(key: string, question: DiscoveryQuestion): string | null {
  if (!/^[1-9]$/.test(key)) return null;
  const option = question.options[Number(key) - 1];
  return option ? option.id : null;
}

function answerAfter(
  messages: DiscoveryMessageView[],
  index: number,
  question: DiscoveryQuestion,
): QuestionAnswer | null {
  for (let i = index + 1; i < messages.length; i += 1) {
    const message = messages[i];
    if (message.role !== 'user') continue;
    const text = message.content;
    const picked = question.options.find((option) => option.label === text.trim());
    return { optionId: picked ? picked.id : null, text };
  }
  return null;
}

function spendMeta(message: DiscoveryMessageView): string | null {
  const parts: string[] = [];
  if (message.tokens !== null) parts.push(`${formatTokens(message.tokens)} tokens`);
  if (message.cost_usd !== null) parts.push(formatCost(message.cost_usd));
  return parts.length > 0 ? parts.join(' · ') : null;
}

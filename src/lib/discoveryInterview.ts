import type { DiscoveryMessageView, DiscoveryQuestion } from '../types';
import { formatActivitySummary } from './discoveryActivity';
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
  /** What the turn did and what it cost, beneath the bubble. `null` when
   *  neither was recorded — distinct from `0`, which is a measurement. */
  meta: string | null;
  /** Why the turn asked nothing that could be offered — a block that would
   *  not parse, or one that parsed and was refused. Rendered beneath the
   *  bubble, because the block it came from is not. */
  questionError: string | null;
  /** The harness had forgotten the session, so this turn carried the whole
   *  transcript in its prompt. Rare, and only ever known from the completion
   *  event — a turn read back from the database reports `false` because
   *  nothing stored it, not because it was resumed. */
  reseeded: boolean;
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

const NONE: ReadonlySet<string> = new Set();

/**
 * `reseeded` names the assistant messages whose turn had to carry the
 * transcript itself. It is a parameter rather than a field on the message
 * because nothing persists it: the caller holds what it heard on
 * `discovery_turn_completed` for as long as the workspace is open, and a turn
 * from an earlier session simply says nothing on the subject.
 */
export function buildTranscript(
  messages: DiscoveryMessageView[],
  reseeded: ReadonlySet<string> = NONE,
): TranscriptBlock[] {
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
        reseeded: false,
      });
      return;
    }

    const prose = message.prose.trim();
    if (prose.length > 0 || message.question === null) {
      blocks.push({
        kind: 'bubble',
        key: message.id,
        role: 'assistant',
        // The prose, never the stored turn: what separates them is the block,
        // which was addressed to Demeteo. A turn whose block would not parse
        // has prose and a `questionError`, and falling back to `content`
        // there is how the raw JSON reached the reader.
        text: prose,
        meta: turnMeta(message),
        questionError: message.question_error,
        reseeded: reseeded.has(message.id),
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

/**
 * What the turn did and what it cost (`DISCOVERY_UI_SPEC.md` §3.4.3), rendered
 * by the same formatter the live bubble uses, so a bubble does not change what
 * it says the moment it settles.
 *
 * Every part is omitted rather than zeroed when it was never recorded: a turn
 * stored before the activity column existed reads as a turn nothing is known
 * about, not as one that touched no files.
 */
function turnMeta(message: DiscoveryMessageView): string | null {
  const parts: string[] = [];
  const activity = formatActivitySummary(message.activity ?? null);
  if (activity !== null) parts.push(activity);
  if (message.tokens !== null) parts.push(`${formatTokens(message.tokens)} tokens`);
  if (message.cost_usd !== null) parts.push(formatCost(message.cost_usd));
  return parts.length > 0 ? parts.join(' · ') : null;
}

import React, { useEffect, useRef } from 'react';

import { formatActivitySummary } from '../../lib/askActivity';
import { formatCost, formatTokens } from '../../lib/utils';
import type { AskMessageView } from '../../types';
import { AgentMarkdown } from '../discovery/AgentMarkdown';
import { AskStreamingBubble } from './AskStreamingBubble';
import type { AskStreamStore } from './useAskStream';

interface AskTranscriptProps {
  threadId: string;
  messages: AskMessageView[];
  /** A turn on this thread is `setting_up`/`working` — the streaming bubble
   *  takes the tail instead of the last settled message. */
  pending: boolean;
  store: AskStreamStore;
}

/**
 * An Ask thread's messages as bubbles — `InterviewTranscript.tsx`'s bubble
 * rendering, with no question card to thread through: Ask has no concept of
 * an open question.
 *
 * **`.prose`, never `.text`.** A canvas-carrying turn's `.text` still has the
 * canvas JSON glued to the end of it — `.prose` is what `AskMessageView`
 * already stripped that from. A user's own message has no canvas to strip, so
 * `.text` is what they actually typed.
 */
export function AskTranscript({
  threadId,
  messages,
  pending,
  store,
}: AskTranscriptProps): React.ReactElement {
  const scroller = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const element = scroller.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [messages, pending]);

  return (
    <div
      ref={scroller}
      data-testid="ask-transcript"
      className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4"
    >
      {messages.map((message) => {
        const agent = message.role === 'assistant';
        const text = agent ? message.prose : message.text;
        const meta = agent ? turnMeta(message) : null;
        return (
          <div
            key={message.id}
            className={`flex flex-col ${agent ? 'items-start' : 'items-end'}`}
            data-testid={`ask-transcript-${message.role}`}
          >
            <div
              className={`chat-bubble min-w-0 ${agent ? 'agent' : 'user whitespace-pre-wrap'}`}
            >
              <div className="chat-bubble-sender">
                <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-current" />
                {agent ? 'Ask' : 'You'}
              </div>
              {agent ? <AgentMarkdown text={text} /> : text}
            </div>
            {meta && <p className="mt-1.5 mb-0 px-1 font-mono text-[10px] text-slate-600">{meta}</p>}
            {agent && message.canvas_error && (
              <p
                data-testid="ask-transcript-canvas-error"
                className="mt-1.5 mb-0 px-1 font-mono text-[10px] text-amber-200/90"
              >
                No canvas drawn: {message.canvas_error}
              </p>
            )}
          </div>
        );
      })}

      {pending && <AskStreamingBubble store={store} threadId={threadId} scroller={scroller} />}
    </div>
  );
}

/** `discoveryInterview.ts`'s `turnMeta`, over `AskMessageView` in place of
 *  `DiscoveryMessageView` — same formatter the live bubble uses, so a bubble
 *  does not change what it says the moment it settles. */
function turnMeta(message: AskMessageView): string | null {
  const parts: string[] = [];
  const activity = formatActivitySummary(message.turn_activity);
  if (activity !== null) parts.push(activity);
  if (message.tokens !== null) parts.push(`${formatTokens(message.tokens)} tokens`);
  if (message.cost_usd !== null) parts.push(formatCost(message.cost_usd));
  return parts.length > 0 ? parts.join(' · ') : null;
}

export default AskTranscript;

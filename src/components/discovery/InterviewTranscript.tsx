import React, { useEffect, useRef } from 'react';

import type { TranscriptBlock } from '../../lib/discoveryInterview';
import type { QuestionOption } from '../../types';
import { QuestionCard } from './QuestionCard';
import { StreamingBubble } from './StreamingBubble';
import type { DiscoveryStreamStore } from './useDiscoveryStream';

interface InterviewTranscriptProps {
  discoveryId: string;
  blocks: TranscriptBlock[];
  /** Which question the interview is waiting on, derived rather than stored. */
  openQuestion: string | null;
  pending: boolean;
  store: DiscoveryStreamStore;
  onPick: (option: QuestionOption) => void;
  onAnswerInOwnWords: () => void;
}

export function InterviewTranscript({
  discoveryId,
  blocks,
  openQuestion,
  pending,
  store,
  onPick,
  onAnswerInOwnWords,
}: InterviewTranscriptProps): React.ReactElement {
  const scroller = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const element = scroller.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [blocks, pending]);

  return (
    <div
      ref={scroller}
      data-testid="interview-transcript"
      className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4"
    >
      {blocks.map((block) => {
        if (block.kind === 'question') {
          const live = block.key === openQuestion;
          // One open question at a time: an unanswered question that is not
          // the open one has nothing to ask, and a live one is withheld while
          // a turn is still speaking.
          if (block.answer === null && (!live || pending)) return null;
          return (
            <QuestionCard
              key={block.key}
              question={block.question}
              answer={block.answer}
              live={live}
              pending={pending}
              onPick={onPick}
              onAnswerInOwnWords={onAnswerInOwnWords}
            />
          );
        }

        const agent = block.role === 'assistant';
        return (
          <div
            key={block.key}
            className={`flex flex-col ${agent ? 'items-start' : 'items-end'}`}
            data-testid={`transcript-${block.role}`}
          >
            <div
              className={`chat-bubble ${agent ? 'agent' : 'user'} whitespace-pre-wrap`}
            >
              <div className="chat-bubble-sender">
                <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-current" />
                {agent ? 'Interviewer' : 'You'}
              </div>
              {block.text}
            </div>
            {block.meta && (
              <p className="mt-1.5 mb-0 px-1 font-mono text-[10px] text-slate-600">{block.meta}</p>
            )}
            {block.questionError && (
              <p className="mt-1.5 mb-0 px-1 font-mono text-[10px] text-amber-200/90">
                Its question block would not parse, so nothing was offered to pick from.
              </p>
            )}
            {block.reseeded && (
              <p
                data-testid="transcript-reseeded"
                className="mt-1.5 mb-0 px-1 font-mono text-[10px] text-amber-200/90"
              >
                The harness had forgotten this session, so the turn carried the whole transcript.
              </p>
            )}
          </div>
        );
      })}

      {pending && (
        <StreamingBubble store={store} discoveryId={discoveryId} scroller={scroller} />
      )}
    </div>
  );
}

export default InterviewTranscript;

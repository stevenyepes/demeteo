import React, { useEffect } from 'react';

import { useStreamedTurn, type DiscoveryStreamStore } from './useDiscoveryStream';

interface StreamingBubbleProps {
  store: DiscoveryStreamStore;
  discoveryId: string;
  /** The transcript scroller, kept pinned to the bottom as text arrives. */
  scroller: React.RefObject<HTMLDivElement | null>;
}

/**
 * The partial turn, and **the only subscriber to the stream**.
 *
 * It is mounted solely while a turn runs, which is what keeps the frame-rate
 * wake off the transcript and off the ticket graph one column over —
 * `FeatureDetail/StepInspector.tsx` records the same constraint for the run
 * surface, where a subscription one level up re-rendered every card in the
 * list on every chunk.
 */
export function StreamingBubble({
  store,
  discoveryId,
  scroller,
}: StreamingBubbleProps): React.ReactElement {
  const text = useStreamedTurn(store, discoveryId);

  useEffect(() => {
    const element = scroller.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [text, scroller]);

  return (
    <div className="flex flex-col items-start" data-testid="streaming-bubble">
      <div className="chat-bubble agent whitespace-pre-wrap">
        <div className="chat-bubble-sender">
          <span
            aria-hidden="true"
            className="h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-current motion-reduce:animate-none"
          />
          Interviewer
        </div>
        {text}
        <span aria-hidden="true" className="stream-caret" />
      </div>
      <p className="mt-1.5 mb-0 px-1 font-mono text-[10px] text-slate-600">
        streaming · one-shot turn, resumed from the stored transcript
      </p>
    </div>
  );
}

export default StreamingBubble;

import React, { useEffect, useRef } from 'react';

import { useElapsed } from '../../hooks/useElapsed';
import { TurnActivityStrip } from './TurnActivityStrip';
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
 * wake — and the one-second elapsed tick — off the transcript and off the
 * ticket graph one column over. `FeatureDetail/StepInspector.tsx` records the
 * same constraint for the run surface, where a subscription one level up
 * re-rendered every card in the list on every chunk.
 *
 * **There is no state in which this renders an empty bubble.** A turn that has
 * said nothing and called nothing is the common case for a reasoning model,
 * and the strip above the prose is what stands in for it.
 *
 * Nothing is claimed beneath the bubble. The line that used to sit there said
 * the turn had been resumed from the stored transcript, on every turn
 * including the first, where no session existed to resume — and a re-seed is
 * a rare event a turn only learns about when it ends
 * (`DiscoveryTurnCompleted.reseeded`), which is where the transcript now
 * reports it.
 */
export function StreamingBubble({
  store,
  discoveryId,
  scroller,
}: StreamingBubbleProps): React.ReactElement {
  const turn = useStreamedTurn(store, discoveryId);
  // A turn already running when this mounted has no start this surface saw;
  // the mount is the earliest instant it can honestly count from.
  const mountedAt = useRef(Date.now());
  const elapsed = useElapsed(turn.startedAt === 0 ? mountedAt.current : turn.startedAt);

  useEffect(() => {
    const element = scroller.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [turn.text, scroller]);

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
        <TurnActivityStrip turn={turn} elapsedMs={elapsed} />
        {turn.text}
        <span aria-hidden="true" className="stream-caret" />
      </div>
    </div>
  );
}

export default StreamingBubble;

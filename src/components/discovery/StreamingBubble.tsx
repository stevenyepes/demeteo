import React, { useEffect, useRef } from 'react';

import { useElapsed } from '../../hooks/useElapsed';
import { useThrottledValue } from '../../hooks/useThrottledValue';
import { AgentMarkdown } from './AgentMarkdown';
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
  // The turn's Markdown is re-parsed four times a second, not once per frame.
  //
  // This component wakes on a coalesced animation frame for the whole length
  // of a turn, over text that is longer every wake, so parsing what has
  // arrived so far is work that grows with the turn: at frame rate a
  // two-minute turn pays for it some seven thousand times. The alternative
  // considered was leaving the partial turn as plain text and parsing once at
  // settle — cheaper still, but it makes the user read raw backticks and
  // asterisks for the entire turn, which is the bug this fixes, and then
  // reflows the whole bubble under them at the end.
  //
  // What is accepted instead: a bounded parse budget of roughly four parses a
  // second, and up to 250 ms between a delta landing and the prose showing it.
  // The activity strip and the caret above still tick every frame, so the
  // bubble never looks stalled while the text waits.
  const shownText = useThrottledValue(turn.text, 250);
  // A turn already running when this mounted has no start this surface saw;
  // the mount is the earliest instant it can honestly count from.
  const mountedAt = useRef(Date.now());
  const elapsed = useElapsed(turn.startedAt === 0 ? mountedAt.current : turn.startedAt);

  useEffect(() => {
    const element = scroller.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [turn.text, shownText, scroller]);

  return (
    <div className="flex flex-col items-start" data-testid="streaming-bubble">
      <div className="chat-bubble agent min-w-0">
        <div className="chat-bubble-sender">
          <span
            aria-hidden="true"
            className="h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-current motion-reduce:animate-none"
          />
          Interviewer
        </div>
        <TurnActivityStrip turn={turn} elapsedMs={elapsed} />
        <div className="stream-caret-host min-w-0">
          <AgentMarkdown text={shownText} />
        </div>
      </div>
    </div>
  );
}

export default StreamingBubble;

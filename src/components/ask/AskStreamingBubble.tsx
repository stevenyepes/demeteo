import React, { useEffect, useRef } from 'react';
import { Globe } from 'lucide-react';

import { useElapsed } from '../../hooks/useElapsed';
import { useThrottledValue } from '../../hooks/useThrottledValue';
import { sourcesOf, type LiveTurn } from '../../lib/askActivity';
import { AgentMarkdown } from '../discovery/AgentMarkdown';
import { AskActivityStrip } from './AskActivityStrip';
import { useStreamedTurn, type AskStreamStore } from './useAskStream';

interface AskStreamingBubbleProps {
  store: AskStreamStore;
  threadId: string;
  /** The transcript scroller, kept pinned to the bottom as text arrives. */
  scroller: React.RefObject<HTMLDivElement | null>;
}

/**
 * The partial turn, and **the only subscriber to the stream** —
 * `StreamingBubble.tsx`'s doc comment, verbatim, for Ask: mounted solely
 * while a turn runs, which is what keeps the frame-rate wake off the
 * transcript and off the canvas pane one column over.
 *
 * **There is no state in which this renders an empty bubble.** A turn that
 * has said nothing and called nothing yet is the common case right after a
 * turn opens, and `AskActivityStrip` above the prose is what stands in for
 * it.
 *
 * This is also the only place a turn's ledger is in hand, so {@link Sources}
 * hangs off it — the settled message the turn becomes carries the fetch
 * *counts* and not the URLs, so a list drawn anywhere else would have
 * nothing to draw.
 */
export function AskStreamingBubble({
  store,
  threadId,
  scroller,
}: AskStreamingBubbleProps): React.ReactElement {
  const turn = useStreamedTurn(store, threadId);
  // Re-parsed at a bounded rate rather than once per frame — see
  // `StreamingBubble.tsx` for why this is a budget, not a debounce.
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
    <div className="flex flex-col items-start" data-testid="ask-streaming-bubble">
      <div className="chat-bubble agent min-w-0">
        <div className="chat-bubble-sender">
          <span
            aria-hidden="true"
            className="h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-current motion-reduce:animate-none"
          />
          Ask
        </div>
        <AskActivityStrip turn={turn} elapsedMs={elapsed} />
        <div className="stream-caret-host min-w-0">
          <AgentMarkdown text={shownText} />
        </div>
        <Sources turn={turn} />
      </div>
    </div>
  );
}

/**
 * `docs/ask-canvas/probe/Streaming.html`'s `.srcs` block. Plain text rather
 * than anchors: a bare `<a href>` navigates the webview away from the app,
 * so a clickable source would have to route through
 * `@tauri-apps/plugin-opener`, which nothing in `src/` uses yet.
 */
function Sources({ turn }: { turn: LiveTurn }): React.ReactElement | null {
  const sources = sourcesOf(turn);
  if (sources.length === 0) return null;

  return (
    <div
      data-testid="ask-sources"
      className="mt-3 border-t border-white/5 pt-2.5"
    >
      <div className="mb-1.5 flex items-center gap-1.5 text-[10px] font-medium tracking-wide text-slate-500 uppercase">
        <Globe aria-hidden="true" className="h-3 w-3 shrink-0" />
        Sources &middot; {sources.length}
      </div>
      <ul className="m-0 flex list-none flex-col gap-1 p-0">
        {sources.map((source) => (
          <li
            key={source.url}
            data-testid="ask-source"
            className="min-w-0 truncate font-mono text-[11px] text-cyan-300/80"
          >
            {source.url}
          </li>
        ))}
      </ul>
    </div>
  );
}

export default AskStreamingBubble;

import React, { useEffect, useState } from 'react';

import { getDiscovery, getDiscoveryBoard } from '../../lib/discovery';
import { formatError } from '../../lib/errors';
import { progressText } from '../../lib/discoveryProgress';
import type { DiscoveryBoard, DiscoveryDetail } from '../../types';

interface DiscoveryViewProps {
  discoveryId: string;
  /** Carried on the view so the header can name the Discovery before
   *  `discovery_get` answers. */
  discoveryTitle: string;
}

/**
 * One Discovery's workspace — **a shell**. Phase 6 (`docs/TASKS_DISCOVERY.md`)
 * fills in the three columns `DISCOVERY_UI_SPEC.md` §3 specifies: the
 * interview transcript and its composer, the ticket graph, the board, and the
 * inspector. What is here is the route's landing pad and the two reads it
 * rests on, so the `AppView` arm added in Phase 5 lands somewhere real.
 */
export function DiscoveryView({
  discoveryId,
  discoveryTitle,
}: DiscoveryViewProps): React.ReactElement {
  const [detail, setDetail] = useState<DiscoveryDetail | null>(null);
  const [board, setBoard] = useState<DiscoveryBoard | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setDetail(null);
    setBoard(null);
    setError(null);
    Promise.allSettled([getDiscovery(discoveryId), getDiscoveryBoard(discoveryId)]).then(
      ([detailRes, boardRes]) => {
        if (cancelled) return;
        if (detailRes.status === 'fulfilled') setDetail(detailRes.value);
        else setError(formatError(detailRes.reason));
        if (boardRes.status === 'fulfilled') setBoard(boardRes.value);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [discoveryId]);

  const progress = board ? progressText(board) : null;

  return (
    <div className="flex flex-1 flex-col overflow-hidden bg-[#0a0c10]">
      <div className="flex shrink-0 items-center justify-between gap-6 border-b border-white/5 bg-[#0d0f14]/60 px-6 py-3.5">
        <div className="flex min-w-0 flex-col gap-1.5">
          <p className="font-mono text-[11px] text-slate-500">Discovery</p>
          <h1 className="truncate font-heading text-xl font-bold tracking-tight text-white">
            {detail?.discovery.title ?? discoveryTitle}
          </h1>
        </div>
        <span className="shrink-0 font-mono text-[11px] text-slate-400">
          {board ? `${board.tickets.length} tickets` : '—'}
        </span>
      </div>

      <div className="flex-1 overflow-y-auto p-6">
        {error ? (
          <p role="alert" className="font-mono text-xs text-ruby-200">
            {error}
          </p>
        ) : (
          <p className="font-mono text-xs text-slate-500">
            {progress ?? 'No tickets proposed yet.'}
          </p>
        )}
      </div>
    </div>
  );
}

export default DiscoveryView;

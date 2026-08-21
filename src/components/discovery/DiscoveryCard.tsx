import React from 'react';
import { Zap } from 'lucide-react';

import type { DiscoveryBoard, DiscoverySummary } from '../../types';
import type { RunStatusTone } from '../../lib/runStatus';
import {
  discoveryDetailLine,
  discoveryLifecycle,
  progressSegments,
  progressText,
} from '../../lib/discoveryProgress';
import { formatCost, formatTokens, relativeTime } from '../../lib/utils';
import { Chip } from '../ui/Chip';
import { TicketProgressBar } from './TicketProgressBar';

/**
 * The 4 px accent bar's fill and its glow, per lifecycle tone. Kept beside the
 * only component that draws one: it is a card treatment, not a status
 * vocabulary, so it does not belong next to `TONE_CHIP`.
 */
const TONE_ACCENT: Partial<Record<RunStatusTone, string>> = {
  violet: 'bg-violet-500 shadow-[0_0_10px_rgba(139,92,246,0.8)]',
  cyan: 'bg-cyan-500 shadow-[0_0_10px_rgba(6,182,212,0.8)]',
  slate: 'bg-slate-600 shadow-[0_0_10px_rgba(100,116,139,0.6)]',
};

interface DiscoveryCardProps {
  /** The list row: the Discovery, its turn count and its ticket counter, all
   *  from `discovery_list`. */
  discovery: DiscoverySummary;
  /** `null` until `discovery_board` answers. Only the detail line waits on it
   *  — the counter above arrives with the row, so the bar does not appear a
   *  beat after the card it sits in. */
  board: DiscoveryBoard | null;
  /** A turn is streaming right now — the only thing that pulses. */
  turnRunning: boolean;
  now: number;
  onOpen: (discoveryId: string, title: string) => void;
}

export function DiscoveryCard({
  discovery,
  board,
  turnRunning,
  now,
  onOpen,
}: DiscoveryCardProps): React.ReactElement {
  const ticketCount = discovery.progress.live + discovery.progress.dropped;
  const lifecycle = discoveryLifecycle(discovery, ticketCount, turnRunning);
  const progress = progressText(discovery.progress);
  const segments = progressSegments(discovery.progress);
  const detail = board ? discoveryDetailLine(board) : null;

  return (
    <button
      type="button"
      data-testid="discovery-card"
      onClick={() => onOpen(discovery.id, discovery.title)}
      className="glass-panel glass-panel-hover relative w-full overflow-hidden rounded-xl p-5 text-left"
    >
      <span
        aria-hidden="true"
        className={`absolute inset-y-0 left-0 w-1 ${TONE_ACCENT[lifecycle.tone] ?? ''}`}
      />

      <div className="flex items-start justify-between gap-4">
        <h3 className="min-w-0 flex-1 font-heading text-lg font-semibold text-white">
          {discovery.title}
        </h3>
        <div className="flex shrink-0 items-center gap-3">
          <Chip size="sm" tone={lifecycle.tone} dot pulse={lifecycle.live}>
            {lifecycle.label}
          </Chip>
          <span className="font-mono text-xs font-medium text-white">
            {relativeTime(discovery.updated_at, now)}
          </span>
        </div>
      </div>

      <div className="mt-2.5 flex flex-wrap items-center gap-3 text-[11px] text-slate-400">
        <Chip size="sm" tone="cyan">{discovery.agent_kind}</Chip>
        {discovery.model && <Chip size="sm" tone="violet">{discovery.model}</Chip>}
        <span className="text-slate-300">{turnCount(discovery.message_count)}</span>
        <span className="text-slate-300">{formatCost(discovery.total_cost)}</span>
        <span className="flex items-center gap-1 text-slate-300">
          <Zap className="h-3 w-3 text-cyan-400" aria-hidden="true" />
          {formatTokens(discovery.tokens)}
        </span>
      </div>

      {progress && (
        <div className="mt-3.5 flex items-center gap-3.5">
          <TicketProgressBar
            landedPct={segments.landedPct}
            inFlightPct={segments.inFlightPct}
            title={progress}
            className="w-40 shrink-0"
          />
          <span className="font-mono text-[11px] text-slate-400">{progress}</span>
        </div>
      )}

      {detail && <p className="mt-2.5 text-xs leading-relaxed text-slate-500">{detail}</p>}
    </button>
  );
}

/** How many turns have been taken. The count is of stored messages, which is
 *  what `DiscoverySummary` carries and what the transcript renders. */
function turnCount(messages: number): string {
  return messages === 1 ? '1 turn' : `${messages} turns`;
}

export default DiscoveryCard;

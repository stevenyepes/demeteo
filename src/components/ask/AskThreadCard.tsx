import React from 'react';
import { Zap } from 'lucide-react';

import { askLifecycle, turnCountLabel } from '../../lib/askLifecycle';
import type { RunStatusTone } from '../../lib/runStatus';
import { formatCost, formatTokens, relativeTime } from '../../lib/utils';
import type { AskThread } from '../../types';
import { Chip } from '../ui/Chip';

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

interface AskThreadCardProps {
  thread: AskThread;
  /** A turn is streaming right now — the only thing that pulses. */
  turnRunning: boolean;
  now: number;
  onOpen: (threadId: string) => void;
}

/**
 * One row of Project Home's Ask tab, built to the anatomy of `DiscoveryCard`
 * — same accent bar, same header, same meta strip in the same order.
 *
 * The two are separate components rather than one generic card because what
 * they show below the meta strip diverges: a Discovery carries a ticket
 * progress bar and a derived detail line, and a thread carries neither. A
 * shared card would take both as optional slots and end up being a layout with
 * two callers, which is the abstraction the divergence is telling us not to
 * build. What must not diverge is the *vocabulary*, and that is held by
 * `askLifecycle`/`discoveryProgress` agreeing on `RunStatusTone`.
 */
export function AskThreadCard({
  thread,
  turnRunning,
  now,
  onOpen,
}: AskThreadCardProps): React.ReactElement {
  const lifecycle = askLifecycle(thread, turnRunning);

  return (
    <button
      type="button"
      data-testid="ask-thread-card"
      onClick={() => onOpen(thread.id)}
      className="glass-panel glass-panel-hover relative w-full overflow-hidden rounded-xl p-5 text-left"
    >
      <span
        aria-hidden="true"
        className={`absolute inset-y-0 left-0 w-1 ${TONE_ACCENT[lifecycle.tone] ?? ''}`}
      />

      <div className="flex items-start justify-between gap-4">
        <h3 className="min-w-0 flex-1 font-heading text-lg font-semibold text-white">
          {thread.title}
        </h3>
        <div className="flex shrink-0 items-center gap-3">
          <Chip size="sm" tone={lifecycle.tone} dot pulse={lifecycle.live}>
            {lifecycle.label}
          </Chip>
          <span className="font-mono text-xs font-medium text-white">
            {relativeTime(thread.updated_at, now)}
          </span>
        </div>
      </div>

      <div className="mt-2.5 flex flex-wrap items-center gap-3 text-[11px] text-slate-400">
        <Chip size="sm" tone="cyan">{thread.agent_kind}</Chip>
        {thread.model && <Chip size="sm" tone="violet">{thread.model}</Chip>}
        <span className="text-slate-300">{turnCountLabel(thread.turn_count)}</span>
        <span className="text-slate-300">{formatCost(thread.cost_usd)}</span>
        <span className="flex items-center gap-1 text-slate-300">
          <Zap className="h-3 w-3 text-cyan-400" aria-hidden="true" />
          {formatTokens(thread.tokens)}
        </span>
        {/* Stated only when it is true. Network access is off by default, so a
            chip on every card would report the default rather than the
            exception the user is looking for. */}
        {thread.network && <Chip size="sm" tone="amber">network</Chip>}
      </div>
    </button>
  );
}

export default AskThreadCard;

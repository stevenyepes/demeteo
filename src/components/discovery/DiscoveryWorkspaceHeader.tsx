import React from 'react';
import { Code } from 'lucide-react';

import { discoveryLifecycle } from '../../lib/discoveryProgress';
import { formatCost, formatTokens } from '../../lib/utils';
import type { Discovery, DiscoveryBoard } from '../../types';
import { Chip } from '../ui/Chip';
import { Metric, MetricStrip } from '../ui/MetricStrip';

interface DiscoveryWorkspaceHeaderProps {
  discovery: Discovery;
  board: DiscoveryBoard | null;
  /** Every bubble and question card the transcript renders. */
  turnCount: number;
  turnRunning: boolean;
  onToggleOpen: () => void;
  /** Phase 7 owns the proposed-changes review, so the button appears only once
   *  something can receive the click. A control that opens nothing is the
   *  surface promising more than it carries. */
  onDecompose?: () => void;
  busy: boolean;
}

export function DiscoveryWorkspaceHeader({
  discovery,
  board,
  turnCount,
  turnRunning,
  onToggleOpen,
  onDecompose,
  busy,
}: DiscoveryWorkspaceHeaderProps): React.ReactElement {
  const tickets = board?.tickets ?? [];
  const started = tickets.filter((view) => view.ticket.state === 'started').length;
  const lifecycle = discoveryLifecycle(discovery, tickets.length, turnRunning);

  return (
    <header className="flex shrink-0 items-center justify-between gap-6 border-b border-white/5 bg-[#0d0f14]/60 px-6 py-3.5">
      <div className="flex min-w-0 flex-col gap-1.5">
        <p className="m-0 font-mono text-[11px] text-slate-500">Discovery</p>
        <div className="flex min-w-0 items-center gap-3">
          <h1 className="m-0 truncate font-heading text-xl font-bold tracking-tight text-white">
            {discovery.title}
          </h1>
          <Chip size="sm" tone={lifecycle.tone} dot pulse={lifecycle.live}>
            {lifecycle.label}
          </Chip>
          {tickets.length > 0 && (
            <Chip size="sm" tone="slate">
              {tickets.length} tickets · {started} started
            </Chip>
          )}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-4">
        <MetricStrip variant="inset">
          <Metric label="Turns" value={String(turnCount)} />
          <Metric label="Spend" value={formatCost(discovery.total_cost)} tone="emerald" />
          <Metric label="Tokens" value={formatTokens(discovery.tokens)} tone="cyan" />
        </MetricStrip>

        <button type="button" onClick={onToggleOpen} disabled={busy} className="btn-secondary">
          {discovery.status === 'open' ? 'Close discovery' : 'Reopen discovery'}
        </button>

        {onDecompose && (
          <button
            type="button"
            onClick={onDecompose}
            disabled={busy}
            className="btn-primary inline-flex items-center gap-2"
          >
            <Code className="h-3.5 w-3.5" aria-hidden="true" />
            Decompose
          </button>
        )}
      </div>
    </header>
  );
}

export default DiscoveryWorkspaceHeader;

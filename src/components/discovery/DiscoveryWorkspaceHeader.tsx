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
  /** How many turns have been taken — stored messages, the same count Project
   *  Home's card reads. See `turnCountLabel` for why a turn is one message and
   *  not one rendered block. */
  turnCount: number;
  turnRunning: boolean;
  onToggleOpen: () => void;
  /** Raise the proposed-changes review (§5.1). The **user** decides when, and
   *  it is offered from the first turn — the interviewer's
   *  `nothing_left_to_settle` is advisory and never gates this button. */
  onDecompose: () => void;
  /** A pass is running. It streams through the interview's own events, so the
   *  transcript is already showing the work; this only stops a second press. */
  decomposing: boolean;
  busy: boolean;
}

export function DiscoveryWorkspaceHeader({
  discovery,
  board,
  turnCount,
  turnRunning,
  onToggleOpen,
  onDecompose,
  decomposing,
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

        <button
          type="button"
          data-testid="discovery-decompose"
          onClick={onDecompose}
          disabled={busy || decomposing}
          className="btn-primary inline-flex items-center gap-2 disabled:cursor-not-allowed disabled:opacity-40"
        >
          <Code className="h-3.5 w-3.5" aria-hidden="true" />
          {decomposing ? 'Decomposing…' : 'Decompose'}
        </button>
      </div>
    </header>
  );
}

export default DiscoveryWorkspaceHeader;

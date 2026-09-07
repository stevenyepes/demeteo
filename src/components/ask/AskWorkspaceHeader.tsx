import React from 'react';
import { Plus, Settings, Trash2 } from 'lucide-react';

import { formatCost, formatTokens } from '../../lib/utils';
import type { AskThread } from '../../types';
import { BackButton } from '../ui/BackButton';
import { Chip } from '../ui/Chip';
import { Metric, MetricStrip } from '../ui/MetricStrip';

interface AskWorkspaceHeaderProps {
  thread: AskThread;
  onNewThread: () => void;
  onOpenSettings: () => void;
  /** Close an open thread, reopen a closed one — the caller reads
   *  `thread.status` for which, so this stays one button. */
  onToggleOpen: () => void;
  onDelete: () => void;
  /** A close, reopen or delete is in flight. */
  busy: boolean;
}

export function AskWorkspaceHeader({
  thread,
  onNewThread,
  onOpenSettings,
  onToggleOpen,
  onDelete,
  busy,
}: AskWorkspaceHeaderProps): React.ReactElement {
  const closed = thread.status === 'closed';

  return (
    <header className="flex shrink-0 items-center justify-between gap-6 border-b border-white/5 bg-[#0d0f14]/60 px-6 py-3.5">
      <div className="flex min-w-0 items-start gap-3">
        <BackButton className="mt-1" />
        <div className="flex min-w-0 flex-col gap-1.5">
        <p className="m-0 font-mono text-[11px] text-slate-500">Ask</p>
        <div className="flex min-w-0 items-center gap-3">
          <h1 className="m-0 truncate font-heading text-xl font-bold tracking-tight text-white">
            {thread.title}
          </h1>
          <Chip size="sm" tone="cyan">
            {thread.agent_kind}
          </Chip>
          {closed && (
            <Chip size="sm" tone="slate">
              Closed
            </Chip>
          )}
        </div>
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-4">
        <MetricStrip variant="inset">
          <Metric label="Turns" value={String(thread.turn_count)} />
          <Metric label="Spend" value={formatCost(thread.cost_usd)} tone="emerald" />
          <Metric label="Tokens" value={formatTokens(thread.tokens)} tone="cyan" />
        </MetricStrip>

        <button
          type="button"
          data-testid="ask-toggle-open"
          onClick={onToggleOpen}
          disabled={busy}
          className="btn-secondary text-[13px] disabled:cursor-not-allowed disabled:opacity-40"
        >
          {closed ? 'Reopen thread' : 'Close thread'}
        </button>

        <button
          type="button"
          data-testid="ask-delete-thread"
          aria-label="Delete thread"
          onClick={onDelete}
          disabled={busy}
          className="btn-secondary inline-flex items-center gap-2 text-ruby-200 disabled:cursor-not-allowed disabled:opacity-40"
        >
          <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
        </button>

        <button
          type="button"
          data-testid="ask-open-settings"
          aria-label="Thread settings"
          onClick={onOpenSettings}
          className="btn-secondary inline-flex items-center gap-2"
        >
          <Settings className="h-3.5 w-3.5" aria-hidden="true" />
        </button>

        <button
          type="button"
          data-testid="ask-new-thread"
          onClick={onNewThread}
          className="btn-primary inline-flex items-center gap-2"
        >
          <Plus className="h-3.5 w-3.5" aria-hidden="true" />
          New thread
        </button>
      </div>
    </header>
  );
}

export default AskWorkspaceHeader;
